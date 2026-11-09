// uses_probe.js — 全例題の**識別子を使っている位置**ごとに hover の 1 行目と定義へ移動の行き先を出す（name_scope_plan.md の 2-1）。
//
//   ELECTRON_RUN_AS_NODE=1 "<VS Code>/Code.exe" uses_probe.js <out.tsv>
//
// stress.js は宣言の位置しか調べないので、「使っている位置が別の宣言に結び付く」誤り（§0 の A1〜A3・A12）は
// そちらでは数えられない。変更の前後で出力を比べ、hover / 定義が消えた・変わった位置を全部見るための道具。
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
if(!P.loadPrelude()) console.error('WARNING: builtins.ars failed to load');

class Doc{
  constructor(fp,src){ this.fileName=fp; this.version=1; this.languageId='arrow';
    const raw=src.replace(/\r\n/g,'\n'); this._l=raw.split('\n');
    if(this._l[this._l.length-1]==='') this._l.pop();
    this.lineCount=this._l.length;
    this.uri={scheme:'file',fsPath:fp,toString(){return 'file://'+fp}};}
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
  if(e.isDirectory()) walk(p,out);
  else if(e.name.endsWith('.ar')) out.push(p);} return out; }

function hoverText(h){
  if(!h) return '-';
  const c=Array.isArray(h.contents)?h.contents:[h.contents];
  const s=c.map(x=>typeof x==='string'?x:(x.value??'')).join(' ');
  return s.replace(/```\w*/g,'').replace(/\s+/g,' ').trim().slice(0,120) || '(empty)';
}
function defText(loc){
  if(!loc) return '-';
  const l=Array.isArray(loc)?loc[0]:loc;
  if(!l||!l.range) return '?';
  const fp=(l.uri&&l.uri.fsPath)||'';
  return `${path.basename(fp)}:${l.range.start.line+1}`;
}

const root=path.join(__dirname,'..','examples');
const out=[];
for(const f of walk(root).sort()){
  const doc=new Doc(f,fs.readFileSync(f,'utf8'));
  const rel=path.relative(path.join(__dirname,'..'),f).replace(/\\/g,'/');
  for(let ln=0; ln<doc.lineCount; ln++){
    const text=doc._l[ln];
    const re=/[A-Za-z_][A-Za-z0-9_]*/g; let m;
    while((m=re.exec(text))!==null){
      const p={line:ln,character:m.index};
      let h, d;
      try{ h=P.provideHover(doc,p); d=P.provideDefinition(doc,p); }catch(e){ h={contents:['THREW '+e.message]}; }
      if(!h && !d) continue;
      out.push(`${rel}\t${ln+1}:${m.index+1}\t${m[0]}\t${hoverText(h)}\t${defText(d)}`);
    }
  }
}
fs.writeFileSync(process.argv[2]||'uses.tsv', out.join('\n')+'\n');
console.log('positions with hover/def:', out.length);
