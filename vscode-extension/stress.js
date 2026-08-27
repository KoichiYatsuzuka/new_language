// stress.js — 全例題に対して VS Code の言語機能 7 種を走らせる回帰チェック。
//
//   ELECTRON_RUN_AS_NODE=1 "<VS Code>/Code.exe" stress.js
//
// 目的は「落ちないこと」と「取りこぼしが増えていないこと」の 2 点。
//   threw        : 例外を投げた例題（0 でなければならない）
//   hover misses : 宣言の上で hover が出なかった件数（0 が正常）
//   def   misses : 宣言の上で go-to-definition が解決しなかった件数（0 が正常）
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

const files=walk(path.join(__dirname,'..','examples'));
let ok=0, failed=0, noSym=0, totalSym=0, totalHover=0, hoverMiss=0, totalDef=0, defMiss=0;
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
if(problems.length){ console.log('\nproblems:'); problems.slice(0,25).forEach(p=>console.log(p)); }
