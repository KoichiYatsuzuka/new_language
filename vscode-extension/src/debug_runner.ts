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

import * as fs from 'fs';
import * as path from 'path';
// At runtime 'vscode' is intercepted → vscode_mock; these imports are for types only
import type * as vscode from 'vscode';
import { SemanticTokenEntry } from './vscode_mock';
import { loadFrontend, frontendLoadError } from './frontend';
import {
    provideHover,
    provideInlayHints,
    provideDiagnostics,
    provideDocumentSemanticTokens,
    provideCompletionItems,
    provideDocumentSymbols,
    provideSignatureHelp,
    provideDefinition,
    loadPrelude,
    SEMANTIC_TOKENS_LEGEND,
} from './wasm_providers';

// ── ANSI helpers ──────────────────────────────────────────────────────────────

const A = {
    reset:   '\x1b[0m',
    bold:    '\x1b[1m',
    dim:     '\x1b[2m',
    red:     '\x1b[31m',
    green:   '\x1b[32m',
    yellow:  '\x1b[33m',
    blue:    '\x1b[34m',
    magenta: '\x1b[35m',
    cyan:    '\x1b[36m',
    gray:    '\x1b[90m',
    bYellow: '\x1b[93m',
    bGreen:  '\x1b[92m',
    bCyan:   '\x1b[96m',
    bWhite:  '\x1b[97m',
} as const;

function c(color: string, text: string): string { return `${color}${text}${A.reset}`; }

/** Semantic token type index → ANSI colour (indices follow SEMANTIC_TOKENS_LEGEND). */
const TOKEN_COLORS: Record<string, string> = {
    class:      A.bYellow,
    interface:  A.yellow,
    enum:       A.bYellow,
    enumMember: A.magenta,
    function:   A.bGreen,
    method:     A.bGreen,
    property:   A.cyan,
    parameter:  A.bCyan,
    variable:   A.bWhite,
    namespace:  A.blue,
    type:       A.cyan,
};

// ── Mock TextDocument ─────────────────────────────────────────────────────────

class MockTextDocument {
    private readonly _lines: string[];
    readonly version = 1;
    readonly languageId = 'arrow';
    readonly uri: { fsPath: string; toString(): string };
    readonly fileName: string;
    readonly lineCount: number;

    constructor(filePath: string, content: string) {
        this.fileName = filePath;
        const raw = content.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
        this._lines = raw.split('\n');
        if (this._lines[this._lines.length - 1] === '') this._lines.pop();
        this.lineCount = this._lines.length;
        const fp = filePath;
        this.uri = {
            fsPath: fp,
            toString() { return `file://${fp.replace(/\\/g, '/')}`; },
        };
    }

    lineAt(line: number): { text: string; range: unknown } {
        return { text: this._lines[line] ?? '', range: null };
    }

    getText(range?: unknown): string {
        if (!range) return this._lines.join('\n');
        const r = range as { start: { line: number; character: number }; end: { line: number; character: number } };
        if (r.start.line === r.end.line) {
            return (this._lines[r.start.line] ?? '').slice(r.start.character, r.end.character);
        }
        const parts: string[] = [];
        for (let i = r.start.line; i <= r.end.line; i++) {
            const ln = this._lines[i] ?? '';
            if (i === r.start.line)    parts.push(ln.slice(r.start.character));
            else if (i === r.end.line) parts.push(ln.slice(0, r.end.character));
            else                       parts.push(ln);
        }
        return parts.join('\n');
    }

    getWordRangeAtPosition(
        pos: { line: number; character: number },
        regex?: RegExp,
    ): { start: { line: number; character: number }; end: { line: number; character: number } } | undefined {
        const line = this._lines[pos.line];
        if (line === undefined) return undefined;
        const re = new RegExp((regex ?? /\w+/).source, 'g');
        let m: RegExpExecArray | null;
        while ((m = re.exec(line)) !== null) {
            if (m.index <= pos.character && m.index + m[0].length > pos.character) {
                return {
                    start: { line: pos.line, character: m.index },
                    end:   { line: pos.line, character: m.index + m[0].length },
                };
            }
        }
        return undefined;
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

function extractHoverLines(hover: vscode.Hover): string[] {
    const items = Array.isArray(hover.contents) ? hover.contents : [hover.contents];
    const lines: string[] = [];
    for (const item of items) {
        const raw = typeof item === 'string' ? item : (item as { value: string }).value;
        for (const ln of raw.replace(/```\w*\n?/g, '').trim().split('\n')) {
            const t = ln.trim();
            if (t) lines.push(t);
        }
    }
    return lines;
}

function renderBalloon(lines: string[], indent: string): string {
    if (lines.length === 0) return '';
    const width = Math.max(...lines.map(l => l.length), 0);
    const top = c(A.gray, `${indent}╭${'─'.repeat(width + 2)}╮`);
    const mid = lines.map(l =>
        c(A.gray, `${indent}│`) + ' ' + c(A.bCyan, l.padEnd(width)) + ' ' + c(A.gray, '│'));
    const bot = c(A.gray, `${indent}╰${'─'.repeat(width + 2)}╯`);
    return [top, ...mid, bot].join('\n');
}

/** Paint one source line using the semantic tokens and inline the inlay hints. */
function renderSourceLine(
    lineNo: number,
    text: string,
    tokens: SemanticTokenEntry[],
    hints: { char: number; label: string }[],
): string {
    const legend = SEMANTIC_TOKENS_LEGEND.tokenTypes;
    // Build the coloured line right-to-left so earlier offsets stay valid.
    const marks = [
        ...tokens.map(t => ({ at: t.char, len: t.len, kind: 'tok' as const, type: legend[t.tokenType] })),
        ...hints.map(h => ({ at: h.char, len: 0, kind: 'hint' as const, label: h.label })),
    ].sort((a, b) => b.at - a.at || b.len - a.len);

    let out = text;
    for (const m of marks) {
        if (m.kind === 'hint') {
            out = out.slice(0, m.at) + c(A.dim + A.green, m.label) + out.slice(m.at);
        } else {
            const colour = TOKEN_COLORS[m.type] ?? A.bWhite;
            out = out.slice(0, m.at) + c(colour, out.slice(m.at, m.at + m.len)) + out.slice(m.at + m.len);
        }
    }
    return `  ${c(A.gray, String(lineNo + 1).padStart(4))} ${c(A.gray, '│')} ${out}`;
}

function header(title: string): void {
    console.log('\n' + c(A.bold + A.yellow, `── ${title} ` + '─'.repeat(Math.max(0, 62 - title.length))));
}

// ── Main ──────────────────────────────────────────────────────────────────────

function main(): void {
    const target = process.argv[2];
    if (!target) {
        console.error('usage: node run_debug.js <path/to/file.ar>');
        process.exit(2);
    }
    const filePath = path.resolve(target);
    const content = fs.readFileSync(filePath, 'utf8');
    const doc = new MockTextDocument(filePath, content) as unknown as vscode.TextDocument;

    // The extension root is one level above out_debug/.
    if (!loadFrontend(path.join(__dirname, '..'))) {
        console.error('failed to load arrow_frontend.wasm: ' + frontendLoadError());
        console.error('build it with: cd crates/arrow-frontend && cargo build --release --target wasm32-unknown-unknown');
        process.exit(1);
    }

    console.log('\n' + c(A.gray, '═'.repeat(70)));
    console.log(c(A.bold + A.bWhite, `  ARROW DEBUG:  ${path.basename(filePath)}`));
    console.log(c(A.gray, '═'.repeat(70)));

    // ---- 1 + 2 + 3: source with tokens/hints, then hovers ----
    const semantic = provideDocumentSemanticTokens(doc) as unknown as { tokenList: SemanticTokenEntry[] };
    const fullRange = { start: { line: 0, character: 0 }, end: { line: doc.lineCount, character: 0 } };
    const hints = provideInlayHints(doc, fullRange as unknown as vscode.Range);

    const tokensByLine = new Map<number, SemanticTokenEntry[]>();
    for (const t of semantic.tokenList) {
        const list = tokensByLine.get(t.line) ?? [];
        list.push(t);
        tokensByLine.set(t.line, list);
    }
    const hintsByLine = new Map<number, { char: number; label: string }[]>();
    for (const h of hints) {
        const p = h.position as unknown as { line: number; character: number };
        const label = typeof h.label === 'string' ? h.label : '';
        const list = hintsByLine.get(p.line) ?? [];
        list.push({ char: p.character, label });
        hintsByLine.set(p.line, list);
    }

    header('SOURCE  (semantic colours + inlay hints)');
    for (let i = 0; i < doc.lineCount; i++) {
        console.log(renderSourceLine(i, doc.lineAt(i).text, tokensByLine.get(i) ?? [], hintsByLine.get(i) ?? []));
    }

    // ---- 2: hover + 3: definition, probed at every declaration ----
    const outline = provideDocumentSymbols(doc);
    header('HOVER + GO-TO-DEFINITION  (probed at each declaration)');
    const probes: { name: string; line: number; char: number }[] = [];
    const walk = (nodes: vscode.DocumentSymbol[]) => {
        for (const n of nodes) {
            const r = n.selectionRange as unknown as { start: { line: number; character: number } };
            probes.push({ name: n.name, line: r.start.line, char: r.start.character });
            walk(n.children);
        }
    };
    walk(outline);
    for (const p of probes) {
        const pos = { line: p.line, character: p.char } as unknown as vscode.Position;
        const hov = provideHover(doc, pos);
        const def = provideDefinition(doc, pos);
        const defStr = def
            ? `→ L${(def.range as unknown as { start: { line: number } }).start.line + 1}`
            : c(A.red, '→ (none)');
        console.log(`  ${c(A.gray, `L${String(p.line + 1).padStart(4)}`)} ${c(A.bWhite, p.name.padEnd(18))} ${c(A.gray, defStr)}`);
        if (hov) console.log(renderBalloon(extractHoverLines(hov), '        '));
    }

    // ---- 4: outline ----
    header('DOCUMENT SYMBOLS  (outline)');
    const printOutline = (nodes: vscode.DocumentSymbol[], depth: number) => {
        for (const n of nodes) {
            const r = n.selectionRange as unknown as { start: { line: number } };
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
        if (line >= doc.lineCount) continue;
        const items = provideCompletionItems(doc, { line, character: 0 } as unknown as vscode.Position);
        const names = items.slice(0, 8).map(i => i.label).join(', ');
        console.log(`  ${c(A.gray, `L${String(line + 1).padStart(4)}`)} scope → ${c(A.bGreen, String(items.length))} items: ${c(A.gray, names)}${items.length > 8 ? c(A.gray, ' …') : ''}`);
    }
    // (b) dot access: every `x.` occurrence in the file
    let dotProbes = 0;
    for (let i = 0; i < doc.lineCount && dotProbes < 6; i++) {
        const text = doc.lineAt(i).text;
        // コメント行を拾わない（`# functions.ar — …` を受け手だと誤認しないため）。
        if (/^\s*#/.test(text)) continue;
        const m = /([A-Za-z_]\w*)\./.exec(text);
        if (!m) continue;
        dotProbes++;
        const pos = { line: i, character: m.index + m[0].length } as unknown as vscode.Position;
        const items = provideCompletionItems(doc, pos);
        const label = items.length ? c(A.bGreen, `${items.length} members`) : c(A.red, 'empty');
        console.log(`  ${c(A.gray, `L${String(i + 1).padStart(4)}`)} ${c(A.bWhite, m[0].padEnd(14))} → ${label} ${c(A.gray, items.slice(0, 6).map(x => x.label).join(', '))}`);
    }

    // ---- 6: signature help ----
    header('SIGNATURE HELP');
    let sigProbes = 0;
    for (let i = 0; i < doc.lineCount && sigProbes < 6; i++) {
        const text = doc.lineAt(i).text;
        const m = /([A-Za-z_]\w*)\(/.exec(text);
        if (!m) continue;
        const pos = { line: i, character: m.index + m[0].length } as unknown as vscode.Position;
        const help = provideSignatureHelp(doc, pos);
        if (!help || help.signatures.length === 0) continue;
        sigProbes++;
        const sig = help.signatures[0];
        console.log(`  ${c(A.gray, `L${String(i + 1).padStart(4)}`)} ${c(A.bCyan, sig.label)} ${c(A.gray, `(active param ${help.activeParameter})`)}`);
    }
    if (sigProbes === 0) console.log(c(A.gray, '  (no call sites resolved)'));

    // ---- 7: diagnostics ----
    const diags = provideDiagnostics(doc);
    header(`DIAGNOSTICS  (${diags.length})`);
    if (diags.length === 0) {
        console.log(c(A.green, '  none'));
    } else {
        for (const d of diags) {
            const r = d.range as unknown as { start: { line: number; character: number } };
            const sev = d.severity === 0 ? c(A.red, 'error  ') : c(A.yellow, 'warning');
            console.log(`  ${c(A.gray, `L${String(r.start.line + 1).padStart(4)}:${String(r.start.character + 1).padStart(3)}`)}  [${sev}]  ${d.message}`);
        }
    }

    console.log('\n' + c(A.gray, '═'.repeat(70)) + '\n');
}

main();
