// compare_tm_grammar.js — compare the TextMate grammar of the VS Code extension with the real lexer.
//
//   <electron-as-node> compare_tm_grammar.js --engine <dir> --grammar <tmLanguage.json>
//                      --wasm <arrow_frontend.wasm> [--verbose] <file.ar>...
//
// Used by scripts/compare_tm_grammar.ps1 (VSIX_update_plan.md 1-1).
//
// # Why
//
// The extension colours identifiers from the wasm lexer (semantic tokens), but **strings, comments
// and bracket matching come only from the TextMate grammar** — a second, regex-based lexer. When the
// two disagree, VS Code treats code as a string (or the other way round): brackets stop matching and
// comments turn white (`math_hover.ar`: a `{` inside `r"…"` opened an f-string expression that never
// closed). This script finds every such disagreement.
//
// # What is compared
//
// Every character is put in one of three classes, by each side:
//   S = inside a string literal, C = inside a comment, O = anything else.
// - wasm: the `str` / `comment` tokens of `analysis.tokens` (src/lexer/editor_tokens.rs).
// - grammar: the **outermost** `string.*` / `comment.*` scope of the TextMate token. Outermost, because
//   the lexer makes the whole f-string one `Str` token: an embedded `{expr}` is still part of it.
//
// A **SPILL** is a line whose first non-blank character the grammar puts in a string or comment while
// the lexer says it is code: the grammar carried a string/comment over from an earlier line. That is
// the failure that breaks a whole file, so it must be 0. Other differences (inside one line, e.g. the
// `r` of `r"…"` coloured as a variable) are counted and listed, but do not fail the run.
//
// The keyword check compares the grammar's keyword-like words with src/lexer/keyword.rs (task 2-5).
'use strict';
const fs = require('fs');
const path = require('path');

// ---- engine extraction ------------------------------------------------------
//
//   compare_tm_grammar.js --extract <node_modules.asar> <outDir>
//
// VS Code ships the same TextMate engine it uses for colouring (`vscode-textmate`, `vscode-oniguruma`)
// inside `resources/app/node_modules.asar`. Taking it from there needs no npm install / network, and
// tokenizes exactly as the installed VS Code does. The asar format is a 16-byte header, a JSON index,
// then the file bytes; `.asar.unpacked` holds files kept outside the archive (onig.wasm).
function extractFromAsar(asarPath, outDir, prefixes) {
    // ⚠ Electron's `fs` opens an `.asar` as a directory; read it as the plain file it is.
    process.noAsar = true;
    const fd = fs.openSync(asarPath, 'r');
    const head = Buffer.alloc(16);
    fs.readSync(fd, head, 0, 16, 0);
    const headerSize = head.readUInt32LE(4);
    const jsonSize = head.readUInt32LE(12);
    const json = Buffer.alloc(jsonSize);
    fs.readSync(fd, json, 0, jsonSize, 16);
    const index = JSON.parse(json.toString('utf8'));
    const base = 8 + headerSize;
    const unpacked = asarPath + '.unpacked';
    let count = 0;
    const walk = (node, rel) => {
        for (const [name, child] of Object.entries(node.files || {})) {
            const p = rel ? `${rel}/${name}` : name;
            if (child.files) { walk(child, p); continue; }
            if (!prefixes.some(pre => p === pre || p.startsWith(pre + '/'))) continue;
            const dst = path.join(outDir, ...p.split('/'));
            fs.mkdirSync(path.dirname(dst), { recursive: true });
            if (child.unpacked) {
                fs.copyFileSync(path.join(unpacked, ...p.split('/')), dst);
            } else {
                const buf = Buffer.alloc(child.size);
                fs.readSync(fd, buf, 0, child.size, base + Number(child.offset));
                fs.writeFileSync(dst, buf);
            }
            count++;
        }
    };
    walk(index, '');
    fs.closeSync(fd);
    return count;
}

if (process.argv[2] === '--extract') {
    const [, , , asarPath, outDir] = process.argv;
    const n = extractFromAsar(asarPath, outDir, ['vscode-textmate', 'vscode-oniguruma']);
    if (!fs.existsSync(path.join(outDir, 'vscode-textmate')) || !fs.existsSync(path.join(outDir, 'vscode-oniguruma'))) {
        console.error(`vscode-textmate / vscode-oniguruma not found in ${asarPath}`);
        process.exit(2);
    }
    console.log(`extracted ${n} file(s) to ${outDir}`);
    process.exit(0);
}

// ---- arguments --------------------------------------------------------------
const argv = process.argv.slice(2);
const opt = { files: [] };
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--engine') opt.engine = argv[++i];
    else if (a === '--grammar') opt.grammar = argv[++i];
    else if (a === '--wasm') opt.wasm = argv[++i];
    else if (a === '--keywords') opt.keywords = argv[++i];
    else if (a === '--list') opt.files.push(...fs.readFileSync(argv[++i], 'utf8').split(/\r?\n/).filter(Boolean));
    else if (a === '--verbose') opt.verbose = true;
    else opt.files.push(a);
}
if (!opt.engine || !opt.grammar || !opt.wasm) {
    console.error('usage: compare_tm_grammar.js --engine <dir> --grammar <json> --wasm <wasm> [--keywords <keyword.rs>] [--list <file>] <file.ar>...');
    process.exit(2);
}

const repo = path.join(__dirname, '..');
const vsctm = require(path.join(opt.engine, 'vscode-textmate'));
const onig = require(path.join(opt.engine, 'vscode-oniguruma'));
const ext = path.join(repo, 'vscode-extension', 'out');
const { loadFrontend, frontendLoadError, analyze } = require(path.join(ext, 'frontend.js'));

// ---- classification ---------------------------------------------------------

/** Per-line class strings ('S' / 'C' / 'O' per UTF-16 unit) from the wasm tokens. */
function lexerClasses(lines, tokens) {
    const out = lines.map(l => new Array(l.length).fill('O'));
    for (const t of tokens) {
        const k = t.kind === 'str' ? 'S' : t.kind === 'comment' ? 'C' : null;
        if (!k) continue;
        for (let ln = t.line; ln <= t.endLine && ln < lines.length; ln++) {
            const from = ln === t.line ? t.col : 0;
            const to = ln === t.endLine ? t.endCol : lines[ln].length;
            for (let c = from; c < to && c < lines[ln].length; c++) out[ln][c] = k;
        }
    }
    return out;
}

/** The class of one TextMate token: its outermost string / comment scope. */
function scopeClass(scopes) {
    for (const s of scopes) {
        if (s.startsWith('string.')) return 'S';
        if (s.startsWith('comment.')) return 'C';
    }
    return 'O';
}

function grammarClasses(grammar, lines) {
    let state = vsctm.INITIAL;
    return lines.map(line => {
        const r = grammar.tokenizeLine(line, state);
        state = r.ruleStack;
        const cls = new Array(line.length).fill('O');
        for (const t of r.tokens) {
            const k = scopeClass(t.scopes);
            for (let c = t.startIndex; c < t.endIndex && c < line.length; c++) cls[c] = k;
        }
        return cls;
    });
}

// ---- keyword check ----------------------------------------------------------

/** Words the lexer turns into keyword tokens (`"word" => Token::…` / `self.maybe_two_word(…)`). */
function lexerKeywords(keywordRs) {
    const src = fs.readFileSync(keywordRs, 'utf8');
    const words = new Set();
    for (const m of src.matchAll(/^\s*"([A-Za-z_]+)"\s*=>/gm)) words.add(m[1]);
    return words;
}

/**
 * Words the grammar colours as keywords: the alternatives of every `\b(a|b|c)\b` group in a rule
 * whose scope matches `scopeRe`.
 */
function grammarKeywords(grammarJson, scopeRe) {
    const words = new Set();
    const isKeywordScope = n => scopeRe.test(n || '');
    const visit = node => {
        if (Array.isArray(node)) return node.forEach(visit);
        if (!node || typeof node !== 'object') return;
        if (typeof node.match === 'string') {
            const scopes = [node.name, ...Object.values(node.captures || {}).map(c => c.name)];
            if (scopes.some(isKeywordScope)) {
                // ⚠ only the first group of `\b(…)\b` alternatives and the plain `\bword\b` forms: the
                //   captures after it name identifiers (`fn (name)`), not keywords.
                const group = node.match.match(/^\\b\(([A-Za-z_| ]+)\)/);
                const plain = node.match.match(/^\\b([A-Za-z_ ]+)\\b$/);
                const alts = group ? group[1].split('|') : plain ? [plain[1]] : [];
                for (const a of alts) for (const w of a.split(' ')) if (w) words.add(w);
            }
        }
        Object.values(node).forEach(visit);
    };
    visit(grammarJson);
    return words;
}

// ---- main -------------------------------------------------------------------
(async () => {
    await onig.loadWASM(fs.readFileSync(path.join(opt.engine, 'vscode-oniguruma', 'release', 'onig.wasm')).buffer);
    const grammarText = fs.readFileSync(opt.grammar, 'utf8');
    const registry = new vsctm.Registry({
        onigLib: Promise.resolve({
            createOnigScanner: s => new onig.OnigScanner(s),
            createOnigString: s => new onig.OnigString(s),
        }),
        loadGrammar: async () => vsctm.parseRawGrammar(grammarText, opt.grammar),
    });
    const grammar = await registry.loadGrammar(JSON.parse(grammarText).scopeName);
    if (!loadFrontend(path.dirname(ext), path.resolve(opt.wasm))) {
        console.error('frontend: ' + frontendLoadError());
        process.exit(2);
    }

    let spillFiles = 0, spillLines = 0, diffFiles = 0, diffChars = 0, checked = 0;
    const spillReport = [], diffReport = [];
    for (const file of opt.files) {
        const rel = path.relative(repo, file);
        // ⚠ the lexer drops a leading BOM, and so does VS Code.
        const text = fs.readFileSync(file, 'utf8').replace(/^﻿/, '');
        // ⚠ no path: tokens do not depend on imports, and reading them would make the run slow.
        const r = analyze(text);
        if (!r) { console.log(`ANALYZE-FAILED ${rel}`); process.exitCode = 1; continue; }
        checked++;
        const lines = text.split(/\r?\n/);
        const lex = lexerClasses(lines, r.tokens);
        const tm = grammarClasses(grammar, lines);

        const spills = [], diffs = [];
        let fileDiffChars = 0;
        for (let ln = 0; ln < lines.length; ln++) {
            const first = lines[ln].search(/\S/);
            if (first >= 0 && lex[ln][first] === 'O' && tm[ln][first] !== 'O') spills.push(ln + 1);
            let n = 0;
            for (let c = 0; c < lines[ln].length; c++) if (lex[ln][c] !== tm[ln][c]) n++;
            if (n > 0) { diffs.push({ line: ln + 1, n }); fileDiffChars += n; }
        }
        if (spills.length > 0) {
            spillFiles++; spillLines += spills.length;
            const shown = spills.slice(0, 5).join(', ') + (spills.length > 5 ? ', …' : '');
            spillReport.push(`  ${rel}  ${spills.length} line(s) from line ${spills[0]}  [${shown}]`);
            console.log(`SPILL   ${rel}  from line ${spills[0]} (${spills.length} lines)`);
        }
        if (diffs.length > 0) {
            diffFiles++; diffChars += fileDiffChars;
            const lineInfo = diffs.slice(0, 8).map(d => `${d.line}`).join(', ') + (diffs.length > 8 ? ', …' : '');
            diffReport.push(`  ${rel}  ${fileDiffChars} char(s) on ${diffs.length} line(s)  [${lineInfo}]`);
            if (opt.verbose) {
                for (const d of diffs.slice(0, 20)) {
                    const l = lines[d.line - 1];
                    console.log(`    ${rel}:${d.line}`);
                    console.log(`      src : ${l}`);
                    console.log(`      lex : ${lex[d.line - 1].join('')}`);
                    console.log(`      tm  : ${tm[d.line - 1].join('')}`);
                }
            }
        }
    }

    let kwFail = false;
    const kwLines = [];
    if (opt.keywords) {
        const lexKw = lexerKeywords(opt.keywords);
        const json = JSON.parse(grammarText);
        // Keyword-coloured words must all be lexer keywords. A lexer keyword may instead be coloured
        // as a built-in type (`Any`, `Self`, `Option`, `Union` are type keywords).
        const tmKw = grammarKeywords(json, /^(keyword|storage|constant\.language)\b/);
        const tmColoured = grammarKeywords(json, /^(keyword|storage|constant\.language|support\.type)\b/);
        const missing = [...lexKw].filter(w => !tmColoured.has(w)).sort();
        const extra = [...tmKw].filter(w => !lexKw.has(w)).sort();
        kwLines.push(`keywords      : lexer ${lexKw.size}, grammar ${tmKw.size}`);
        kwLines.push(`  not coloured (lexer keyword, absent from grammar): ${missing.join(' ') || '-'}`);
        kwLines.push(`  coloured but not a keyword (grammar only)        : ${extra.join(' ') || '-'}`);
        kwFail = missing.length > 0 || extra.length > 0;
    }

    console.log('-'.repeat(78));
    console.log(`files checked : ${checked}`);
    console.log(`SPILL         : ${spillLines} line(s) in ${spillFiles} file(s)   <- must be 0`);
    console.log(`other diffs   : ${diffChars} char(s) in ${diffFiles} file(s)   (report only)`);
    for (const l of kwLines) console.log(l);
    if (opt.keywords) console.log(`keyword lists : ${kwFail ? 'DIFFER' : 'match'}   <- must match`);
    if (spillReport.length > 0) { console.log(''); console.log('SPILLS (the grammar carries a string/comment into code):'); spillReport.forEach(l => console.log(l)); }
    if (opt.verbose && diffReport.length > 0) { console.log(''); console.log('OTHER DIFFS:'); diffReport.forEach(l => console.log(l)); }
    if (spillLines > 0 || kwFail || process.exitCode) process.exit(1);
    process.exit(0);
})().catch(e => { console.error(e && e.stack || String(e)); process.exit(2); });
