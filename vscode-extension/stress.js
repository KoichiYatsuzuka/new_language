// stress.js — 全例題に対して VS Code の言語機能 7 種を走らせる回帰チェック。
//
//   ELECTRON_RUN_AS_NODE=1 "<VS Code>/Code.exe" stress.js
//
// 目的は「落ちないこと」と「取りこぼしが増えていないこと」の 2 点。
//   threw        : 例外を投げた例題（0 でなければならない）
//   hover misses : 宣言の上で hover が出なかった件数（0 が正常）
//   def   misses : 宣言の上で go-to-definition が解決しなかった件数（0 が正常）
//   tag   misses : `import[lang]` タグごとの最小フィクスチャで別名が索引に載らなかった件数。
//                  例題が 1 本も無いタグ（実測: cpp-dll）はこれでしか守れない。
//   bind  misses : import が束縛する名前が索引に載っていない件数（0 が正常）。
//                  期待値は**ソースの行スキャン**で作る — パーサから取ると一致して
//                  しまい、まさにこの種の欠落を検出できない（`importBindings` の注記）。
//   no symbols   : 宣言が 1 つも取れなかった例題。ParseError 例題と、宣言を含まない
//                  例題（math_string.ar）だけが該当するのが正常。
//
// PATH 上の node が古い環境向けに、VS Code 同梱の Node で動かすことを想定している
// （wasm の新しめの命令を解釈できる必要がある）。
//
// out_debug/ を使うので、事前に `npx tsc -p tsconfig.debug.json` を通しておくこと。
'use strict';
const fs=require('fs'), path=require('path');
const Module=require('module');
const orig=Module._load.bind(Module);
Module._load=function(req,parent,isMain){
  if(req==='vscode') return require(path.join(__dirname,'out_debug','vscode_mock'));
  return orig(req,parent,isMain);
};
const {loadFrontend,frontendLoadError}=require('./out_debug/frontend');
const P=require('./out_debug/wasm_providers');

if(!loadFrontend(__dirname)){ console.error('load failed:',frontendLoadError()); process.exit(1); }

// 組み込みスタブも activate() と同じように読む。
// ⚠ ここが抜けていると、この掃引は**実際の拡張が決して見ない**「組み込みの無い世界」を
//    調べることになる。`int` / `str` / `float` / `bool` / `uint` / `set` / `slice` /
//    `path` / `type` は builtins.ars で `fn` として宣言されているので、prelude を読んで
//    初めて「型名が組み込み関数に当たる」経路が動く。読まないと退行を取り逃がす。
if(!P.loadPrelude(path.join(__dirname,'builtins.ars'))){
  console.error('WARNING: builtins.ars failed to load — builtin names will be missing');
}

class Doc{
  constructor(fp,src){ this.fileName=fp; this.version=1; this.languageId='arrow';
    const raw=src.replace(/\r\n/g,'\n'); this._l=raw.split('\n');
    if(this._l[this._l.length-1]==='') this._l.pop();
    this.lineCount=this._l.length;
    this.uri={fsPath:fp,toString(){return 'file://'+fp}};}
  lineAt(n){return {text:this._l[n]??'',range:null};}
  getText(r){ if(!r) return this._l.join('\n');
    if(r.start.line===r.end.line) return (this._l[r.start.line]??'').slice(r.start.character,r.end.character);
    const out=[]; for(let i=r.start.line;i<=r.end.line;i++){const ln=this._l[i]??'';
      out.push(i===r.start.line?ln.slice(r.start.character):i===r.end.line?ln.slice(0,r.end.character):ln);}
    return out.join('\n');}
  getWordRangeAtPosition(p,re){ const line=this._l[p.line]; if(line===undefined) return undefined;
    const r=new RegExp((re??/\w+/).source,'g'); let m;
    while((m=r.exec(line))!==null){ if(m.index<=p.character && m.index+m[0].length>p.character)
      return {start:{line:p.line,character:m.index},end:{line:p.line,character:m.index+m[0].length}};}
    return undefined;}
}

function walk(dir,out=[]){ for(const e of fs.readdirSync(dir,{withFileTypes:true})){
  const p=path.join(dir,e.name);
  if(e.isDirectory()){ if(e.name!=='archived') walk(p,out); }
  else if(e.name.endsWith('.ar')) out.push(p);} return out; }


// ── import が束縛する名前を**ソースから**割り出す ───────────────────────────────
//
// ⚠⚠ **ここだけはパーサを通さない。** この検査の目的は「パーサ側の索引に載り忘れた名前」を
//    見つけることなので、期待値もパーサから取ると**必ず一致してしまい何も検出しない**。
//    実際、`import[cpp-dll]` の別名が索引に載っていなかった欠落は、hover/def の母集団を
//    `provideDocumentSymbols` から取っていたために**probe されず miss にも数えられず**、
//    全ゲート緑のまま残り続けた（計画 §1-g）。だから素朴な行スキャンで独立に作る。
//
// ⚠ 近似で構わない（誤検出が出たらここを狭める）。狙いは網羅ではなく
//    「タグを足したときに索引フックだけ忘れる」形の再発を止めること。
function importBindings(lines){
  const out=[];
  for(let i=0;i<lines.length;i++){
    const t=lines[i].trim();
    if(t.startsWith('#')) continue;
    let m;
    if((m=/^import(?:\[[\w-]+\])?\s+([A-Za-z_][\w.]*)(?:\s*\[[^\]]*\])?(?:\s+as\s+([A-Za-z_]\w*))?\s*$/.exec(t))){
      out.push({name:m[2]??m[1].split('.').pop(), line:i});
    }else if((m=/^from\s+[A-Za-z_][\w.]*\s+import(?:\[[\w-]+\])?\s+(.+)$/.exec(t))){
      for(const part of m[1].split(',')){
        const a=/^([A-Za-z_]\w*)(?:\s+as\s+([A-Za-z_]\w*))?$/.exec(part.trim());
        if(a) out.push({name:a[2]??a[1], line:i});
      }
    }
  }
  return out;
}


// ── タグ別の最小フィクスチャ ─────────────────────────────────────────────────
//
// ⚠⚠ **例題ベースの検査だけでは届かないタグがある。** 実測（2026-09-20）で
//    `import[cpp-dll]` は `examples/archived/` 以外に**1 本も無い** — まさにこのタグの
//    索引フックが抜けていたのに、全ゲートが緑のままだった理由がこれ。
//    例題を足すには DLL の同梱が要るので、ここは**解析だけ**の最小ソースで代替する
//    （エディタビルドは import 先を読まないので、実体が無くても解析は通る）。
//
// ⚠ 新しい `import[lang]` タグを足したら、ここにも 1 行足すこと。
const TAG_FIXTURES = [
  ['ar-auto (bare)',  'import some_module\n',                          'some_module'],
  ['ar-auto (as)',    'import some_module as sm\n',                     'sm'],
  ['ar',              'import[ar] some_module as sm\n',                 'sm'],
  ['py',              'import[py] json as j\n',                         'j'],
  ['py-int',          'import[py-int] math\n',                          'math'],
  ['rs (version)',    'import[rs] libm[0.2] as lm\n',                    'lm'],
  ['cs-dll',          'import[cs-dll] Some.Bridge as sb\n',             'sb'],
  ['cs-proc',         'import[cs-proc] Shell as sh\n',                   'sh'],
  ['cpp-dll',         'import[cpp-dll] Dir.Header as hd\n',             'hd'],
  ['cpp-lib',         'import[cpp-lib] Dir.Header as hl\n',             'hl'],
  ['js-proc',         'import[js-proc] pkg as p\n',                      'p'],
  ['from (bare)',     'from some_module import thing\n',                 'thing'],
  ['from (as)',       'from some_module import thing as t\n',            't'],
  ['from[py]',        'from mod import[py] thing as t2\n',               't2'],
];

function checkTagFixtures(){
  const misses=[];
  for(const [label, src, bind] of TAG_FIXTURES){
    const doc=new Doc(`<fixture:${label}>`, src + '\nfn main() -> None:\n    pass\n');
    let names=new Set();
    try{
      (function w(ns){for(const n of ns){names.add(n.name); w(n.children);}})(P.provideDocumentSymbols(doc));
    }catch(e){ misses.push(`  TAG   THREW ${label}  ${String(e).split('\n')[0]}`); continue; }
    if(!names.has(bind)) misses.push(`  TAG   MISS  ${label}  '${bind}' not in the symbol index`);
  }
  return misses;
}

const files=walk(path.join(__dirname,'..','examples'));
let ok=0, failed=0, noSym=0, totalSym=0, totalHover=0, hoverMiss=0, totalDef=0, defMiss=0;
let totalBind=0, bindMiss=0;
const problems=[];

for(const f of files){
  const doc=new Doc(f,fs.readFileSync(f,'utf8'));
  try{
    const outline=P.provideDocumentSymbols(doc);
    const diags=P.provideDiagnostics(doc);
    P.provideDocumentSemanticTokens(doc);
    P.provideInlayHints(doc,{start:{line:0,character:0},end:{line:doc.lineCount,character:0}});

    // 全宣言で hover / definition / completion / signature を叩く
    const probe=[]; (function w(ns){for(const n of ns){probe.push(n.selectionRange.start); w(n.children);}})(outline);
    for(const p of probe){
      totalHover++; if(!P.provideHover(doc,p)) { hoverMiss++; problems.push(`  HOVER MISS  ${path.relative(process.cwd(),f)}  L${p.line+1}:${p.character+1}`);}
      totalDef++;   if(!P.provideDefinition(doc,p)) { defMiss++; problems.push(`  DEF   MISS  ${path.relative(process.cwd(),f)}  L${p.line+1}:${p.character+1}`);}
    }
    for(let i=0;i<doc.lineCount;i+=7){
      P.provideCompletionItems(doc,{line:i,character:doc.lineAt(i).text.length});
      P.provideSignatureHelp(doc,{line:i,character:doc.lineAt(i).text.length});
    }
    totalSym+=probe.length;

    // import が束縛する名前は**必ず**索引に載っていること（計画 #6）。
    const declared=new Set(); (function w(ns){for(const n of ns){declared.add(n.name); w(n.children);}})(outline);
    for(const b of importBindings(doc.getText().split('\n'))){
      totalBind++;
      if(!declared.has(b.name)){
        bindMiss++;
        problems.push(`  BIND  MISS  ${path.relative(process.cwd(),f)}  L${b.line+1}  '${b.name}' not in the symbol index`);
      }
    }

    if(probe.length===0 && doc.lineCount>5) { noSym++; problems.push(`  NO SYMBOLS  ${path.relative(process.cwd(),f)}`); }
    ok++;
  }catch(e){
    failed++;
    problems.push(`  THREW       ${path.relative(process.cwd(),f)}  ${String(e).split('\n')[0]}`);
  }
}
console.log(`files          : ${files.length}`);
console.log(`ok             : ${ok}`);
console.log(`threw          : ${failed}   <- must be 0`);
console.log(`no symbols     : ${noSym}`);
console.log(`symbols probed : ${totalSym}`);
console.log(`hover misses   : ${hoverMiss} / ${totalHover}`);
console.log(`def   misses   : ${defMiss} / ${totalDef}`);
console.log(`bind  misses   : ${bindMiss} / ${totalBind}   <- must be 0 (import が束縛する名前が索引に無い)`);
const tagMisses = checkTagFixtures();
console.log(`tag   misses   : ${tagMisses.length} / ${TAG_FIXTURES.length}   <- must be 0 (タグ別フィクスチャ)`);
problems.push(...tagMisses);
if(problems.length){ console.log('\nproblems:'); problems.slice(0,25).forEach(p=>console.log(p)); }
