'use strict';
// 一時的な調査用プローブ（使い終わったら消す）。
const fs = require('fs'), path = require('path'), Module = require('module');
const EXT = __dirname;
const orig = Module._load.bind(Module);
Module._load = function (r, p, m) {
    if (r === 'vscode') return require(path.join(EXT, 'out_debug', 'vscode_mock'));
    return orig(r, p, m);
};
const { loadFrontend } = require(path.join(EXT, 'out_debug', 'frontend'));
const P = require(path.join(EXT, 'out_debug', 'wasm_providers'));
const S = require(path.join(EXT, 'out_debug', 'stubs'));
loadFrontend(EXT);
P.loadPrelude(path.join(EXT, 'builtins.ars'));

const FP = path.join('d:', 'repository', 'TeX_editor', 'main.ar');
console.log('stubs loaded =', S.loadStubsFor(FP));

class Doc {
    constructor(fp, s) {
        this.fileName = fp; this.version = 1; this.languageId = 'arrow';
        this._l = s.replace(/\r\n/g, '\n').split('\n');
        if (this._l[this._l.length - 1] === '') this._l.pop();
        this.lineCount = this._l.length;
        this.uri = { fsPath: fp, toString() { return 'file://' + fp; } };
    }
    lineAt(n) { return { text: this._l[n] ?? '', range: null }; }
    getText(r) {
        if (!r) return this._l.join('\n');
        if (r.start.line === r.end.line) return (this._l[r.start.line] ?? '').slice(r.start.character, r.end.character);
        const o = [];
        for (let i = r.start.line; i <= r.end.line; i++) {
            const ln = this._l[i] ?? '';
            o.push(i === r.start.line ? ln.slice(r.start.character) : i === r.end.line ? ln.slice(0, r.end.character) : ln);
        }
        return o.join('\n');
    }
    getWordRangeAtPosition(p, re) {
        const line = this._l[p.line];
        if (line === undefined) return undefined;
        const r = new RegExp((re ?? /\w+/).source, 'g');
        let m;
        while ((m = r.exec(line)) !== null) {
            if (m.index <= p.character && m.index + m[0].length > p.character) {
                return { start: { line: p.line, character: m.index }, end: { line: p.line, character: m.index + m[0].length } };
            }
        }
        return undefined;
    }
}

const doc = new Doc(FP, fs.readFileSync(FP, 'utf8'));
const L = 89;                       // 0 始まり = ソースの 90 行目
const text = doc.lineAt(L).text;
console.log('line 90:', JSON.stringify(text));

const recv = text.indexOf('wp.');
const col = text.indexOf('open_dummy_pane');
const dump = h => JSON.stringify(h ? h.contents : null);

console.log('hover(wp)              =', dump(P.provideHover(doc, { line: L, character: recv + 1 })));
console.log('hover(open_dummy_pane) =', dump(P.provideHover(doc, { line: L, character: col + 3 })));

const paren = text.indexOf('(', col);
const sh = P.provideSignatureHelp(doc, { line: L, character: paren + 1 });
console.log('signatureHelp          =', sh ? JSON.stringify(sh.signatures.map(s => s.label)) : 'null');

const items = P.provideCompletionItems(doc, { line: L, character: recv + 3 });
console.log('completion(wp.)        =', items.length, 'items:', items.slice(0, 6).map(i => i.label + ' | ' + (i.detail ?? '')));
