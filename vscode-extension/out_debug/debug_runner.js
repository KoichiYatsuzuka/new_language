"use strict";
/**
 * Standalone CLI debug runner for the VS Code extension analysis code.
 *
 * Run via:   node run_debug.js <path/to/file.ar>
 *
 * The run_debug.js bootstrap intercepts require('vscode') before this module
 * loads, so all extension modules (analysis.ts, type_infer.ts) receive the
 * vscode_mock implementations instead of the real VS Code API.
 *
 * Output:
 *   1. Source with ANSI colours (semantic tokens) and inlay hints inserted inline
 *   2. Hover balloon content for every symbol
 *   3. Diagnostics list
 */
Object.defineProperty(exports, "__esModule", { value: true });
const fs = require("fs");
const path = require("path");
const analysis_1 = require("./analysis");
const type_infer_1 = require("./type_infer");
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
    bCyan: '\x1b[96m',
    bWhite: '\x1b[97m',
};
function c(color, text) { return `${color}${text}${A.reset}`; }
// Semantic token type index → ANSI colour (must match SEMANTIC_TOKENS_LEGEND order)
// 0=class  1=type  2=variable
const TOKEN_COLORS = {
    0: A.bYellow,
    1: A.cyan,
    2: A.bWhite, // variable reference
};
// ── Mock TextDocument ─────────────────────────────────────────────────────────
class MockTextDocument {
    constructor(filePath, content) {
        this.version = 1;
        this.languageId = 'arrow';
        this.fileName = filePath;
        // Normalise line endings, strip single trailing blank line
        const raw = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
        this._lines = raw.split('\n');
        if (this._lines.at(-1) === '')
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
        const text = (_a = this._lines[line]) !== null && _a !== void 0 ? _a : '';
        return { text, range: null };
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
                return { start: { line: pos.line, character: m.index }, end: { line: pos.line, character: m.index + m[0].length } };
            }
        }
        return undefined;
    }
}
// ── Hover text extraction ─────────────────────────────────────────────────────
function extractHoverLines(hover) {
    const items = Array.isArray(hover.contents) ? hover.contents : [hover.contents];
    const lines = [];
    for (const item of items) {
        const raw = typeof item === 'string' ? item : item.value;
        // Strip code-fence markers but keep the content
        const stripped = raw
            .replace(/```\w*\n?/g, '')
            .trim();
        for (const ln of stripped.split('\n')) {
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
    const bottom = c(A.gray, `${indent}╰${'─'.repeat(width + 2)}╯`);
    const rows = lines.map(l => c(A.gray, `${indent}│`) +
        ` ${c(A.bCyan, l.padEnd(width))} ` +
        c(A.gray, '│'));
    return [top, ...rows, bottom].join('\n');
}
// ── Source renderer ───────────────────────────────────────────────────────────
function renderSourceLine(lineText, tokens, hints) {
    var _a;
    // Build per-character colour map from semantic tokens
    const colourAt = new Array(lineText.length).fill('');
    for (const tok of tokens) {
        const col = (_a = TOKEN_COLORS[tok.tokenType]) !== null && _a !== void 0 ? _a : '';
        for (let i = tok.char; i < tok.char + tok.len && i < lineText.length; i++) {
            colourAt[i] = col;
        }
    }
    // Sort hints ascending so we can insert left-to-right
    const sortedHints = [...hints].sort((a, b) => a.col - b.col);
    let hintIdx = 0;
    let result = '';
    let cur = '';
    const closeColour = () => { if (cur) {
        result += A.reset;
        cur = '';
    } };
    const openColour = (col) => { if (col !== cur) {
        closeColour();
        result += col;
        cur = col;
    } };
    for (let i = 0; i <= lineText.length; i++) {
        // Insert inlay hints due at this column
        while (hintIdx < sortedHints.length && sortedHints[hintIdx].col === i) {
            closeColour();
            result += A.bCyan + sortedHints[hintIdx].text + A.reset;
            hintIdx++;
        }
        if (i === lineText.length)
            break;
        const col = colourAt[i];
        openColour(col);
        result += lineText[i];
    }
    closeColour();
    return result;
}
// Return the column of the first standalone occurrence of `name` in `line`,
// using word boundaries so "a" doesn't match the "a" inside "add".
function wordPos(line, name) {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const m = new RegExp(`\\b${escaped}\\b`).exec(line);
    return m ? m.index : -1;
}
// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
    var _a, _b, _c, _d, _e;
    const filePath = process.argv[2];
    if (!filePath) {
        console.error('Usage: node run_debug.js <path/to/file.ar>');
        process.exit(1);
    }
    const absPath = path.resolve(filePath);
    if (!fs.existsSync(absPath)) {
        console.error(`File not found: ${absPath}`);
        process.exit(1);
    }
    const content = fs.readFileSync(absPath, 'utf8');
    const doc = new MockTextDocument(absPath, content);
    // Run all providers
    const analysis = await analysis_1.DocumentAnalysis.for(doc);
    const hintsAll = await (0, type_infer_1.provideInlayHints)(doc, { start: { line: 0, character: 0 }, end: { line: doc.lineCount, character: 0 } });
    const semTokens = await (0, type_infer_1.provideDocumentSemanticTokens)(doc);
    const tokenList = (_a = semTokens.tokenList) !== null && _a !== void 0 ? _a : [];
    const diagnostics = await (0, type_infer_1.provideDiagnostics)(doc);
    // Group by line
    const hintsByLine = new Map();
    for (const hint of hintsAll) {
        const ln = hint.position.line;
        if (!hintsByLine.has(ln))
            hintsByLine.set(ln, []);
        const label = typeof hint.label === 'string'
            ? hint.label
            : hint.label.map(p => p.value).join('');
        hintsByLine.get(ln).push({ col: hint.position.character, text: label });
    }
    const tokensByLine = new Map();
    for (const tok of tokenList) {
        if (!tokensByLine.has(tok.line))
            tokensByLine.set(tok.line, []);
        tokensByLine.get(tok.line).push(tok);
    }
    const diagsByLine = new Map();
    for (const d of diagnostics) {
        const ln = d.range.start.line;
        if (!diagsByLine.has(ln))
            diagsByLine.set(ln, []);
        diagsByLine.get(ln).push(d);
    }
    // Hover results keyed by line (first symbol per line)
    // Collect all hover results up front
    const hoverByLine = new Map();
    for (const sym of analysis.symbols) {
        const lineText = doc.lineAt(sym.line).text;
        const nameIdx = wordPos(lineText, sym.name);
        if (nameIdx < 0)
            continue;
        const pos = { line: sym.line, character: nameIdx };
        const hover = await (0, type_infer_1.provideHover)(doc, pos);
        if (!hover)
            continue;
        const hl = extractHoverLines(hover);
        if (hl.length === 0)
            continue;
        if (!hoverByLine.has(sym.line))
            hoverByLine.set(sym.line, []);
        hoverByLine.get(sym.line).push({ name: sym.name, hoverLines: hl });
    }
    const fileName = path.basename(absPath);
    const sep = c(A.gray, '═'.repeat(70));
    // ── Header ──
    process.stdout.write('\n' + sep + '\n');
    process.stdout.write(c(A.bold + A.bWhite, `  ARROW DEBUG:  ${fileName}`) + '\n');
    process.stdout.write(sep + '\n\n');
    // ── Legend ──
    process.stdout.write(c(A.bold, 'Legend: ') +
        c(A.bYellow, '■') + ' class/import  ' +
        c(A.cyan, '■') + ' built-in type  ' +
        c(A.bCyan, '■') + ' inlay hint  ' +
        c(A.red, '■') + ' error  ' +
        c(A.yellow, '■') + ' warning' +
        '\n\n');
    // ─────────────────────────────────────────────────────────────────────────
    // Part 1 — Annotated source
    // ─────────────────────────────────────────────────────────────────────────
    process.stdout.write(c(A.bold + A.yellow, '── SOURCE  (with inlay hints and semantic colours) ──') + '\n\n');
    const lineCount = doc.lineCount;
    const numWidth = String(lineCount).length;
    for (let ln = 0; ln < lineCount; ln++) {
        const raw = doc.lineAt(ln).text;
        // Diagnostics marker
        const diags = (_b = diagsByLine.get(ln)) !== null && _b !== void 0 ? _b : [];
        const diagMarker = diags.length > 0
            ? c(diags.some(d => d.severity === 0) ? A.red : A.yellow, ' ●')
            : '  ';
        const lineNum = c(A.gray, String(ln + 1).padStart(numWidth));
        const bar = c(A.gray, ' │ ');
        const rendered = renderSourceLine(raw, (_c = tokensByLine.get(ln)) !== null && _c !== void 0 ? _c : [], (_d = hintsByLine.get(ln)) !== null && _d !== void 0 ? _d : []);
        process.stdout.write(`${diagMarker}${lineNum}${bar}${rendered}\n`);
        // Hover balloons — one per symbol defined on this line
        const hovers = hoverByLine.get(ln);
        if (hovers) {
            for (const { name, hoverLines } of hovers) {
                const label = c(A.gray, `  ${'─'.repeat(numWidth)} │ `) + c(A.dim, `hover:${name}  `);
                process.stdout.write(label + '\n');
                process.stdout.write(renderBalloon(hoverLines, ' '.repeat(numWidth + 5)) + '\n');
            }
        }
        // Diagnostic messages
        for (const d of diags) {
            const col = d.severity === 0 ? A.red : A.yellow;
            const label = d.severity === 0 ? 'error' : 'warn ';
            const col0 = d.range.start.character;
            const arrow = ' '.repeat(numWidth + 4 + col0) + c(col, `^ [${label}] ${d.message}`);
            process.stdout.write(arrow + '\n');
        }
    }
    // ─────────────────────────────────────────────────────────────────────────
    // Part 2 — Full hover reference list
    // ─────────────────────────────────────────────────────────────────────────
    process.stdout.write('\n' + c(A.bold + A.yellow, '── HOVER REFERENCE ──') + '\n\n');
    const KIND_COLOR = {
        variable: A.cyan, function: A.green, class: A.bYellow,
        trait: A.magenta, enum: A.bYellow, new_type: A.bCyan, module: A.blue,
    };
    for (const sym of analysis.symbols) {
        const lineText = doc.lineAt(sym.line).text;
        const nameIdx = wordPos(lineText, sym.name);
        if (nameIdx < 0)
            continue;
        const pos = { line: sym.line, character: nameIdx };
        const hover = await (0, type_infer_1.provideHover)(doc, pos);
        const kindCol = (_e = KIND_COLOR[sym.kind]) !== null && _e !== void 0 ? _e : A.bWhite;
        const location = c(A.gray, `L:${String(sym.line + 1).padStart(4, '0')}`);
        const kind = c(kindCol, sym.kind.padEnd(9));
        const name = c(A.bold + A.bWhite, sym.name);
        const prefix = `  ${location}  ${kind}  ${name}`;
        if (!hover) {
            process.stdout.write(`${prefix}  ${c(A.gray, '(no hover)')}\n`);
            continue;
        }
        const hl = extractHoverLines(hover);
        if (hl.length === 1) {
            process.stdout.write(`${prefix}  ${c(A.bCyan, hl[0])}\n`);
        }
        else {
            process.stdout.write(`${prefix}\n`);
            process.stdout.write(renderBalloon(hl, '                       ') + '\n');
        }
    }
    // ─────────────────────────────────────────────────────────────────────────
    // Part 3 — Diagnostics summary
    // ─────────────────────────────────────────────────────────────────────────
    process.stdout.write('\n' + c(A.bold + A.yellow, `── DIAGNOSTICS  (${diagnostics.length}) ──`) + '\n\n');
    if (diagnostics.length === 0) {
        process.stdout.write(c(A.gray, '  (none)\n'));
    }
    for (const d of diagnostics) {
        const [col, label] = d.severity === 0 ? [A.red, 'error  '] : [A.yellow, 'warning'];
        const loc = c(A.gray, `L:${String(d.range.start.line + 1).padStart(4, '0')}:${String(d.range.start.character + 1).padStart(3, '0')}`);
        process.stdout.write(`  ${loc}  ${c(col, `[${label}]`)}  ${d.message}\n`);
    }
    process.stdout.write('\n' + sep + '\n\n');
}
main().catch(err => { console.error(err); process.exit(1); });
//# sourceMappingURL=debug_runner.js.map