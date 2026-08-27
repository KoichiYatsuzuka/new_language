"use strict";
/**
 * Standalone CLI debug runner for the VS Code extension.
 *
 * Run via:   node run_debug.js <path/to/file.ar>
 *
 * The run_debug.js bootstrap intercepts require('vscode') before this module
 * loads, so the extension modules receive the vscode_mock implementations
 * instead of the real VS Code API.
 *
 * It exercises all seven language features against the wasm frontend
 * (`crates/arrow-frontend`), which is the same lexer/parser/type-checker that
 * `cargo run` uses. That means a disagreement seen here is a real disagreement,
 * not an artefact of a second implementation.
 *
 * Output sections:
 *   1. Source with semantic-token colours and inlay hints inserted inline
 *   2. Hover balloon for every declaration
 *   3. Go-to-definition targets
 *   4. Document symbols (outline)
 *   5. Completion probes (scoped names and dot-access members)
 *   6. Signature help probes
 *   7. Diagnostics
 */
Object.defineProperty(exports, "__esModule", { value: true });
const fs = require("fs");
const path = require("path");
const frontend_1 = require("./frontend");
const wasm_providers_1 = require("./wasm_providers");
// ── ANSI helpers ──────────────────────────────────────────────────────────────
const A = {
    reset: '\x1b[0m',
    bold: '\x1b[1m',
    dim: '\x1b[2m',
    red: '\x1b[31m',
    green: '\x1b[32m',
    yellow: '\x1b[33m',
    blue: '\x1b[34m',
    magenta: '\x1b[35m',
    cyan: '\x1b[36m',
    gray: '\x1b[90m',
    bYellow: '\x1b[93m',
    bGreen: '\x1b[92m',
    bCyan: '\x1b[96m',
    bWhite: '\x1b[97m',
};
function c(color, text) { return `${color}${text}${A.reset}`; }
/** Semantic token type index → ANSI colour (indices follow SEMANTIC_TOKENS_LEGEND). */
const TOKEN_COLORS = {
    class: A.bYellow,
    interface: A.yellow,
    enum: A.bYellow,
    enumMember: A.magenta,
    function: A.bGreen,
    method: A.bGreen,
    property: A.cyan,
    parameter: A.bCyan,
    variable: A.bWhite,
    namespace: A.blue,
    type: A.cyan,
};
// ── Mock TextDocument ─────────────────────────────────────────────────────────
class MockTextDocument {
    constructor(filePath, content) {
        this.version = 1;
        this.languageId = 'arrow';
        this.fileName = filePath;
        const raw = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
        this._lines = raw.split('\n');
        if (this._lines[this._lines.length - 1] === '')
            this._lines.pop();
        this.lineCount = this._lines.length;
        const fp = filePath;
        this.uri = {
            fsPath: fp,
            toString() { return `file://${fp.replace(/\\/g, '/')}`; },
        };
    }
    lineAt(line) {
        var _a;
        return { text: (_a = this._lines[line]) !== null && _a !== void 0 ? _a : '', range: null };
    }
    getText(range) {
        var _a, _b;
        if (!range)
            return this._lines.join('\n');
        const r = range;
        if (r.start.line === r.end.line) {
            return ((_a = this._lines[r.start.line]) !== null && _a !== void 0 ? _a : '').slice(r.start.character, r.end.character);
        }
        const parts = [];
        for (let i = r.start.line; i <= r.end.line; i++) {
            const ln = (_b = this._lines[i]) !== null && _b !== void 0 ? _b : '';
            if (i === r.start.line)
                parts.push(ln.slice(r.start.character));
            else if (i === r.end.line)
                parts.push(ln.slice(0, r.end.character));
            else
                parts.push(ln);
        }
        return parts.join('\n');
    }
    getWordRangeAtPosition(pos, regex) {
        const line = this._lines[pos.line];
        if (line === undefined)
            return undefined;
        const re = new RegExp((regex !== null && regex !== void 0 ? regex : /\w+/).source, 'g');
        let m;
        while ((m = re.exec(line)) !== null) {
            if (m.index <= pos.character && m.index + m[0].length > pos.character) {
                return {
                    start: { line: pos.line, character: m.index },
                    end: { line: pos.line, character: m.index + m[0].length },
                };
            }
        }
        return undefined;
    }
}
// ── Rendering ─────────────────────────────────────────────────────────────────
function extractHoverLines(hover) {
    const items = Array.isArray(hover.contents) ? hover.contents : [hover.contents];
    const lines = [];
    for (const item of items) {
        const raw = typeof item === 'string' ? item : item.value;
        for (const ln of raw.replace(/```\w*\n?/g, '').trim().split('\n')) {
            const t = ln.trim();
            if (t)
                lines.push(t);
        }
    }
    return lines;
}
function renderBalloon(lines, indent) {
    if (lines.length === 0)
        return '';
    const width = Math.max(...lines.map(l => l.length), 0);
    const top = c(A.gray, `${indent}╭${'─'.repeat(width + 2)}╮`);
    const mid = lines.map(l => c(A.gray, `${indent}│`) + ' ' + c(A.bCyan, l.padEnd(width)) + ' ' + c(A.gray, '│'));
    const bot = c(A.gray, `${indent}╰${'─'.repeat(width + 2)}╯`);
    return [top, ...mid, bot].join('\n');
}
/** Paint one source line using the semantic tokens and inline the inlay hints. */
function renderSourceLine(lineNo, text, tokens, hints) {
    var _a;
    const legend = wasm_providers_1.SEMANTIC_TOKENS_LEGEND.tokenTypes;
    // Build the coloured line right-to-left so earlier offsets stay valid.
    const marks = [
        ...tokens.map(t => ({ at: t.char, len: t.len, kind: 'tok', type: legend[t.tokenType] })),
        ...hints.map(h => ({ at: h.char, len: 0, kind: 'hint', label: h.label })),
    ].sort((a, b) => b.at - a.at || b.len - a.len);
    let out = text;
    for (const m of marks) {
        if (m.kind === 'hint') {
            out = out.slice(0, m.at) + c(A.dim + A.green, m.label) + out.slice(m.at);
        }
        else {
            const colour = (_a = TOKEN_COLORS[m.type]) !== null && _a !== void 0 ? _a : A.bWhite;
            out = out.slice(0, m.at) + c(colour, out.slice(m.at, m.at + m.len)) + out.slice(m.at + m.len);
        }
    }
    return `  ${c(A.gray, String(lineNo + 1).padStart(4))} ${c(A.gray, '│')} ${out}`;
}
function header(title) {
    console.log('\n' + c(A.bold + A.yellow, `── ${title} ` + '─'.repeat(Math.max(0, 62 - title.length))));
}
// ── Main ──────────────────────────────────────────────────────────────────────
function main() {
    var _a, _b, _c, _d;
    const target = process.argv[2];
    if (!target) {
        console.error('usage: node run_debug.js <path/to/file.ar>');
        process.exit(2);
    }
    const filePath = path.resolve(target);
    const content = fs.readFileSync(filePath, 'utf8');
    const doc = new MockTextDocument(filePath, content);
    // The extension root is one level above out_debug/.
    if (!(0, frontend_1.loadFrontend)(path.join(__dirname, '..'))) {
        console.error('failed to load arrow_frontend.wasm: ' + (0, frontend_1.frontendLoadError)());
        console.error('build it with: cd crates/arrow-frontend && cargo build --release --target wasm32-unknown-unknown');
        process.exit(1);
    }
    console.log('\n' + c(A.gray, '═'.repeat(70)));
    console.log(c(A.bold + A.bWhite, `  ARROW DEBUG:  ${path.basename(filePath)}`));
    console.log(c(A.gray, '═'.repeat(70)));
    // ---- 1 + 2 + 3: source with tokens/hints, then hovers ----
    const semantic = (0, wasm_providers_1.provideDocumentSemanticTokens)(doc);
    const fullRange = { start: { line: 0, character: 0 }, end: { line: doc.lineCount, character: 0 } };
    const hints = (0, wasm_providers_1.provideInlayHints)(doc, fullRange);
    const tokensByLine = new Map();
    for (const t of semantic.tokenList) {
        const list = (_a = tokensByLine.get(t.line)) !== null && _a !== void 0 ? _a : [];
        list.push(t);
        tokensByLine.set(t.line, list);
    }
    const hintsByLine = new Map();
    for (const h of hints) {
        const p = h.position;
        const label = typeof h.label === 'string' ? h.label : '';
        const list = (_b = hintsByLine.get(p.line)) !== null && _b !== void 0 ? _b : [];
        list.push({ char: p.character, label });
        hintsByLine.set(p.line, list);
    }
    header('SOURCE  (semantic colours + inlay hints)');
    for (let i = 0; i < doc.lineCount; i++) {
        console.log(renderSourceLine(i, doc.lineAt(i).text, (_c = tokensByLine.get(i)) !== null && _c !== void 0 ? _c : [], (_d = hintsByLine.get(i)) !== null && _d !== void 0 ? _d : []));
    }
    // ---- 2: hover + 3: definition, probed at every declaration ----
    const outline = (0, wasm_providers_1.provideDocumentSymbols)(doc);
    header('HOVER + GO-TO-DEFINITION  (probed at each declaration)');
    const probes = [];
    const walk = (nodes) => {
        for (const n of nodes) {
            const r = n.selectionRange;
            probes.push({ name: n.name, line: r.start.line, char: r.start.character });
            walk(n.children);
        }
    };
    walk(outline);
    for (const p of probes) {
        const pos = { line: p.line, character: p.char };
        const hov = (0, wasm_providers_1.provideHover)(doc, pos);
        const def = (0, wasm_providers_1.provideDefinition)(doc, pos);
        const defStr = def
            ? `→ L${def.range.start.line + 1}`
            : c(A.red, '→ (none)');
        console.log(`  ${c(A.gray, `L${String(p.line + 1).padStart(4)}`)} ${c(A.bWhite, p.name.padEnd(18))} ${c(A.gray, defStr)}`);
        if (hov)
            console.log(renderBalloon(extractHoverLines(hov), '        '));
    }
    // ---- 4: outline ----
    header('DOCUMENT SYMBOLS  (outline)');
    const printOutline = (nodes, depth) => {
        for (const n of nodes) {
            const r = n.selectionRange;
            console.log(`  ${'  '.repeat(depth)}${c(A.bYellow, n.name)} ${c(A.gray, `[${n.detail}]  L${r.start.line + 1}`)}`);
            printOutline(n.children, depth + 1);
        }
    };
    printOutline(outline, 0);
    // ---- 5: completion ----
    header('COMPLETION');
    // (a) scoped names: probe the last line of each function body-ish region
    const scopeProbeLines = probes.filter(p => p.name !== '__init__').slice(0, 4).map(p => p.line + 1);
    for (const line of scopeProbeLines) {
        if (line >= doc.lineCount)
            continue;
        const items = (0, wasm_providers_1.provideCompletionItems)(doc, { line, character: 0 });
        const names = items.slice(0, 8).map(i => i.label).join(', ');
        console.log(`  ${c(A.gray, `L${String(line + 1).padStart(4)}`)} scope → ${c(A.bGreen, String(items.length))} items: ${c(A.gray, names)}${items.length > 8 ? c(A.gray, ' …') : ''}`);
    }
    // (b) dot access: every `x.` occurrence in the file
    let dotProbes = 0;
    for (let i = 0; i < doc.lineCount && dotProbes < 6; i++) {
        const text = doc.lineAt(i).text;
        // コメント行を拾わない（`# functions.ar — …` を受け手だと誤認しないため）。
        if (/^\s*#/.test(text))
            continue;
        const m = /([A-Za-z_]\w*)\./.exec(text);
        if (!m)
            continue;
        dotProbes++;
        const pos = { line: i, character: m.index + m[0].length };
        const items = (0, wasm_providers_1.provideCompletionItems)(doc, pos);
        const label = items.length ? c(A.bGreen, `${items.length} members`) : c(A.red, 'empty');
        console.log(`  ${c(A.gray, `L${String(i + 1).padStart(4)}`)} ${c(A.bWhite, m[0].padEnd(14))} → ${label} ${c(A.gray, items.slice(0, 6).map(x => x.label).join(', '))}`);
    }
    // ---- 6: signature help ----
    header('SIGNATURE HELP');
    let sigProbes = 0;
    for (let i = 0; i < doc.lineCount && sigProbes < 6; i++) {
        const text = doc.lineAt(i).text;
        const m = /([A-Za-z_]\w*)\(/.exec(text);
        if (!m)
            continue;
        const pos = { line: i, character: m.index + m[0].length };
        const help = (0, wasm_providers_1.provideSignatureHelp)(doc, pos);
        if (!help || help.signatures.length === 0)
            continue;
        sigProbes++;
        const sig = help.signatures[0];
        console.log(`  ${c(A.gray, `L${String(i + 1).padStart(4)}`)} ${c(A.bCyan, sig.label)} ${c(A.gray, `(active param ${help.activeParameter})`)}`);
    }
    if (sigProbes === 0)
        console.log(c(A.gray, '  (no call sites resolved)'));
    // ---- 7: diagnostics ----
    const diags = (0, wasm_providers_1.provideDiagnostics)(doc);
    header(`DIAGNOSTICS  (${diags.length})`);
    if (diags.length === 0) {
        console.log(c(A.green, '  none'));
    }
    else {
        for (const d of diags) {
            const r = d.range;
            const sev = d.severity === 0 ? c(A.red, 'error  ') : c(A.yellow, 'warning');
            console.log(`  ${c(A.gray, `L${String(r.start.line + 1).padStart(4)}:${String(r.start.character + 1).padStart(3)}`)}  [${sev}]  ${d.message}`);
        }
    }
    console.log('\n' + c(A.gray, '═'.repeat(70)) + '\n');
}
main();
//# sourceMappingURL=debug_runner.js.map