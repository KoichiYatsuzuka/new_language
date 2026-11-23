// dump_diags.js — analyse .ar files with the wasm frontend and print the raw JSON.
//
//   node dump_diags.js <arrow_frontend.wasm> <file.ar>                      # one file, JSON on stdout
//   node dump_diags.js <arrow_frontend.wasm> --list <files.txt> --out <out.jsonl>
//
// Used by scripts/compare_wasm_frontend.ps1. The one-file form prints JSON on stdout and nothing
// else, so the caller can pipe it straight into ConvertFrom-Json.
//
// The list form (gate_speed_plan.md 7-1) analyses every file named in <files.txt> (one absolute
// path per line, UTF-8) in ONE process and writes one JSON object per line to <out.jsonl>, in the
// same order: {"file": <path>, ok, parseError, parseErrorAt, diagnostics}, or {"file": <path>,
// "error": <text>} when the analysis threw. Starting the runtime and loading the wasm once instead of
// once per file is what makes the gate fast; the extension also keeps one wasm instance for every
// document it analyses, so this is closer to how the wasm is really used. ⚠ The output goes to a file,
// not stdout (no pipe for the caller to drain; vm-pitfalls §4).
//
// The wasm is driven through the extension's own host (vscode-extension/out/frontend.js and
// wasm_host.js, compiled by the gate), so the gate runs it exactly as the extension does: the
// file is analysed *as that file*, and imports are resolved from its directory by the same Rust
// code as arrow.exe, reading files through the host functions.
'use strict';
const fs = require('fs');
const path = require('path');

const argv = process.argv.slice(2);
const wasmPath = argv[0];
let arPath = null, listPath = null, outPath = null;
for (let i = 1; i < argv.length; i++) {
    if (argv[i] === '--list') { listPath = argv[++i]; }
    else if (argv[i] === '--out') { outPath = argv[++i]; }
    else if (arPath === null) { arPath = argv[i]; }
    else { arPath = undefined; }
}
const usage = 'usage: node dump_diags.js <arrow_frontend.wasm> <file.ar>\n' +
              '       node dump_diags.js <arrow_frontend.wasm> --list <files.txt> --out <out.jsonl>';
if (!wasmPath || arPath === undefined || (listPath === null) === (arPath === null) || (listPath !== null) !== (outPath !== null)) {
    console.error(usage);
    process.exit(2);
}

const ext = path.join(__dirname, '..', '..', 'vscode-extension', 'out');
const { loadFrontend, frontendLoadError, analyze } = require(path.join(ext, 'frontend.js'));
const { setHostCwd } = require(path.join(ext, 'wasm_host.js'));

if (!loadFrontend(path.dirname(ext), path.resolve(wasmPath))) {
    console.error(String(frontendLoadError()));
    process.exit(1);
}

// Only what the gate reads. The full result also has `members` (the module/class member tables),
// whose keys may differ only by case (class `Vec2`, module `vec2`); PowerShell 5.1's ConvertFrom-Json
// rejects such objects.
function analyzeOne(arFile) {
    const file = path.resolve(arFile);
    // arrow.exe runs with the file's directory as its working directory (compare_wasm_frontend.ps1).
    setHostCwd(path.dirname(file));
    const result = analyze(fs.readFileSync(file, 'utf8'), file);
    if (!result) { return null; }
    const { ok, parseError, parseErrorAt, diagnostics } = result;
    return { ok, parseError, parseErrorAt, diagnostics };
}

if (arPath !== null) {
    const r = analyzeOne(arPath);
    if (!r) {
        console.error('analyze failed');
        process.exit(1);
    }
    process.stdout.write(JSON.stringify(r));
} else {
    // Strip a UTF-8 BOM (PowerShell 5.1 writes one) and blank lines.
    const files = fs.readFileSync(listPath, 'utf8').replace(/^﻿/, '').split(/\r?\n/).filter((l) => l.trim() !== '');
    const fd = fs.openSync(outPath, 'w');
    try {
        for (const f of files) {
            let line;
            try {
                const r = analyzeOne(f);
                // ⚠ A failed analysis is reported, not skipped (the caller counts it as a disagreement).
                line = r ? Object.assign({ file: f }, r) : { file: f, error: 'analyze failed' };
            } catch (e) {
                line = { file: f, error: String(e && e.stack || e) };
            }
            fs.writeSync(fd, JSON.stringify(line) + '\n');
        }
    } finally {
        fs.closeSync(fd);
    }
}
