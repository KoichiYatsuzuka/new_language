"use strict";
/**
 * Standalone test runner for provideDefinition ("Go to Definition").
 *
 * Run via:   node run_goto_def.js <path/to/file.ar>
 *
 * Reports every tested position: symbol name, (line:col), resolved file path,
 * resolved line/col inside that file, and whether it's a cross-file jump.
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
    red: '\x1b[31m',
    green: '\x1b[32m',
    yellow: '\x1b[33m',
    cyan: '\x1b[36m',
    gray: '\x1b[90m',
    bCyan: '\x1b[96m',
    bWhite: '\x1b[97m',
};
function c(col, text) { return `${col}${text}${A.reset}`; }
// ── MockTextDocument (mirrors debug_runner.ts) ────────────────────────────────
class MockTextDocument {
    constructor(filePath, content) {
        this.version = 1;
        this.languageId = 'arrow';
        this.fileName = filePath;
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
                return {
                    start: { line: pos.line, character: m.index },
                    end: { line: pos.line, character: m.index + m[0].length },
                };
            }
        }
        return undefined;
    }
}
// ── Helpers ───────────────────────────────────────────────────────────────────
/** Find the position of `name` as a whole word (word-boundary aware). */
function wordPos(line, name) {
    const esc = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const m = new RegExp(`\\b${esc}\\b`).exec(line);
    return m ? m.index : -1;
}
function extractLocation(loc) {
    var _a, _b, _c;
    // vscode.Location accepts Range or Position; the mock stores whichever was passed.
    const l = loc;
    const r = l.range;
    // Range has .start (Position); Position has .line directly
    const start = (_a = r['start']) !== null && _a !== void 0 ? _a : r;
    return {
        filePath: l.uri.fsPath,
        line: ((_b = start['line']) !== null && _b !== void 0 ? _b : 0) + 1,
        col: ((_c = start['character']) !== null && _c !== void 0 ? _c : 0) + 1,
    };
}
function relPath(absPath, base) {
    const rel = path.relative(base, absPath);
    return rel.startsWith('..') ? absPath : rel;
}
// ── Main ──────────────────────────────────────────────────────────────────────
async function main() {
    const filePath = process.argv[2];
    if (!filePath) {
        console.error('Usage: node run_goto_def.js <path/to/file.ar>');
        process.exit(1);
    }
    const absPath = path.resolve(filePath);
    if (!fs.existsSync(absPath)) {
        console.error(`File not found: ${absPath}`);
        process.exit(1);
    }
    const repoRoot = path.resolve(__dirname, '..', '..');
    const content = fs.readFileSync(absPath, 'utf8');
    const doc = new MockTextDocument(absPath, content);
    const mockDoc = doc;
    const analysis = await analysis_1.DocumentAnalysis.for(doc);
    const testCases = [];
    const seenDotAccess = new Set();
    // ── Build test cases from symbols ─────────────────────────────────────────
    for (const sym of analysis.symbols) {
        const lineText = mockDoc.lineAt(sym.line).text;
        // Use word-boundary search to avoid matching `calc` inside `py_calculator`, etc.
        const nameIdx = wordPos(lineText, sym.name);
        if (nameIdx < 0)
            continue;
        testCases.push({
            label: `[${sym.kind.padEnd(8)}] ${sym.name}`,
            srcLine: sym.line + 1,
            srcCol: nameIdx + 1,
            mockLine: sym.line,
            mockChar: nameIdx,
        });
        // For typed variables from external classes (C++/Rust/C#): test instance method calls
        if (sym.kind === 'variable' && sym.type) {
            const srcMap = analysis.classSourceMap;
            if (srcMap === null || srcMap === void 0 ? void 0 : srcMap.has(sym.type)) {
                const esc = sym.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
                const instRe = new RegExp(`\\b${esc}\\.([A-Za-z_]\\w*)`, 'g');
                for (let ln = 0; ln < mockDoc.lineCount; ln++) {
                    const text = mockDoc.lineAt(ln).text;
                    let m;
                    while ((m = instRe.exec(text)) !== null) {
                        const memberName = m[1];
                        const key = `inst:${sym.name}.${memberName}`;
                        if (seenDotAccess.has(key))
                            continue;
                        seenDotAccess.add(key);
                        const memberCol = m.index + m[0].length - memberName.length;
                        testCases.push({
                            label: `[inst-acc ] ${sym.name}:${sym.type}.${memberName}`,
                            srcLine: ln + 1,
                            srcCol: memberCol + 1,
                            mockLine: ln,
                            mockChar: memberCol,
                        });
                    }
                }
            }
        }
        // For module imports, also test every alias.member occurrence in the file
        if (sym.kind === 'module') {
            const esc = sym.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            // 1-level: module.member
            const dotRe = new RegExp(`\\b${esc}\\.([A-Za-z_]\\w*)`, 'g');
            for (let ln = 0; ln < mockDoc.lineCount; ln++) {
                const text = mockDoc.lineAt(ln).text;
                let m;
                while ((m = dotRe.exec(text)) !== null) {
                    const memberName = m[1];
                    const key = `${sym.name}.${memberName}`;
                    if (seenDotAccess.has(key))
                        continue;
                    seenDotAccess.add(key);
                    const memberCol = m.index + m[0].length - memberName.length;
                    testCases.push({
                        label: `[dot-acc ] ${sym.name}.${memberName}`,
                        srcLine: ln + 1,
                        srcCol: memberCol + 1,
                        mockLine: ln,
                        mockChar: memberCol,
                    });
                }
            }
            // 2-level: module.SubPart.member (e.g. cs_mod.ClassName.method)
            const dotDotRe = new RegExp(`\\b${esc}\\.([A-Za-z_]\\w*)\\.([A-Za-z_]\\w*)`, 'g');
            for (let ln = 0; ln < mockDoc.lineCount; ln++) {
                const text = mockDoc.lineAt(ln).text;
                let m2;
                while ((m2 = dotDotRe.exec(text)) !== null) {
                    const subName = m2[1];
                    const memberName = m2[2];
                    const key = `${sym.name}.${subName}.${memberName}`;
                    if (seenDotAccess.has(key))
                        continue;
                    seenDotAccess.add(key);
                    const memberCol = m2.index + m2[0].length - memberName.length;
                    testCases.push({
                        label: `[dot-acc2] ${sym.name}.${subName}.${memberName}`,
                        srcLine: ln + 1,
                        srcCol: memberCol + 1,
                        mockLine: ln,
                        mockChar: memberCol,
                    });
                }
            }
        }
    }
    const results = [];
    for (const tc of testCases) {
        const pos = { line: tc.mockLine, character: tc.mockChar };
        const raw = await (0, type_infer_1.provideDefinition)(doc, pos);
        results.push({ tc, loc: raw ? extractLocation(raw) : null });
    }
    // ── Print results ─────────────────────────────────────────────────────────
    const sep = c(A.gray, '═'.repeat(72));
    process.stdout.write('\n' + sep + '\n');
    process.stdout.write(c(A.bold + A.bWhite, `  GOTO DEFINITION TEST:  ${path.basename(absPath)}`) + '\n');
    process.stdout.write(sep + '\n\n');
    let pass = 0;
    let fail = 0;
    for (const { tc, loc } of results) {
        const srcPos = c(A.gray, `L${String(tc.srcLine).padStart(3, '0')}:C${String(tc.srcCol).padStart(3, '0')}`);
        const label = c(A.bCyan, tc.label.padEnd(36));
        if (!loc) {
            process.stdout.write(`  ${srcPos}  ${label}  ${c(A.yellow, 'undefined')}\n`);
            fail++;
        }
        else {
            const isSameFile = loc.filePath === absPath;
            const displayPath = relPath(loc.filePath, repoRoot);
            const target = `${displayPath}  L${loc.line}:C${loc.col}`;
            const tag = isSameFile
                ? c(A.gray, '[same-file]')
                : c(A.green, '[external ]');
            process.stdout.write(`  ${srcPos}  ${label}  ${tag}  ${c(A.bWhite, target)}\n`);
            pass++;
        }
    }
    const total = pass + fail;
    process.stdout.write('\n' + sep + '\n');
    process.stdout.write(`  Results: ${c(A.green, `${pass} resolved`)}  /  ` +
        `${c(A.yellow, `${fail} undefined`)}  /  total ${total}\n`);
    process.stdout.write(sep + '\n\n');
}
main().catch(err => { console.error(err); process.exit(1); });
//# sourceMappingURL=test_goto_def.js.map