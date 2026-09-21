'use strict';
// 一時的な調査用プローブ（使い終わったら消す）。
// signature help が「受け手つきの呼び出し」全般で出ないのか、モジュール限定かを切り分ける。
const path = require('path'), Module = require('module');
const EXT = __dirname;
const orig = Module._load.bind(Module);
Module._load = function (r, p, m) {
    if (r === 'vscode') return require(path.join(EXT, 'out_debug', 'vscode_mock'));
    return orig(r, p, m);
};
const { loadFrontend } = require(path.join(EXT, 'out_debug', 'frontend'));
const P = require(path.join(EXT, 'out_debug', 'wasm_providers'));
loadFrontend(EXT);
P.loadPrelude(path.join(EXT, 'builtins.ars'));

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

const SRC = [
    'class Counter:',
    '    mut n: int = 0',
    '    fn bump(mut self, by: int) -> int:',
    '        self.n = self.n + by',
    '        return self.n',
    '',
    'fn plain(a: int, b: str) -> int:',
    '    return a',
    '',
    'fn main() -> None:',
    '    mut c = Counter()',
    '    let x = c.bump(1)',
    '    let y = plain(1, "s")',
    '    print(x, y)',
    '',
].join('\n');

const doc = new Doc('probe.ar', SRC);
function sigAt(lineNo, marker) {
    const text = doc.lineAt(lineNo).text;
    const open = text.indexOf('(', text.indexOf(marker));
    const sh = P.provideSignatureHelp(doc, { line: lineNo, character: open + 1 });
    return sh ? sh.signatures.map(s => s.label) : null;
}
console.log('method call  c.bump(  =', JSON.stringify(sigAt(11, 'bump')));
console.log('plain call   plain(   =', JSON.stringify(sigAt(12, 'plain')));
console.log('ctor call    Counter( =', JSON.stringify(sigAt(10, 'Counter')));
