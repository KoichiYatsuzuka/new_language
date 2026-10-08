// dump_diags.js — analyse one .ar file with the wasm frontend and print the raw JSON.
//
//   node dump_diags.js <arrow_frontend.wasm> <file.ar>
//
// Used by scripts/compare_wasm_frontend.ps1. Prints JSON on stdout and nothing else,
// so the caller can pipe it straight into ConvertFrom-Json.
//
// The wasm is driven through the extension's own host (vscode-extension/out/frontend.js and
// wasm_host.js, compiled by the gate), so the gate runs it exactly as the extension does: the
// file is analysed *as that file*, and imports are resolved from its directory by the same Rust
// code as arrow.exe, reading files through the host functions.
'use strict';
const fs = require('fs');
const path = require('path');

const [, , wasmPath, arPath] = process.argv;
if (!wasmPath || !arPath) {
    console.error('usage: node dump_diags.js <arrow_frontend.wasm> <file.ar>');
    process.exit(2);
}

const ext = path.join(__dirname, '..', '..', 'vscode-extension', 'out');
const { loadFrontend, frontendLoadError, analyze } = require(path.join(ext, 'frontend.js'));
const { setHostCwd } = require(path.join(ext, 'wasm_host.js'));

if (!loadFrontend(path.dirname(ext), path.resolve(wasmPath))) {
    console.error(String(frontendLoadError()));
    process.exit(1);
}
const file = path.resolve(arPath);
// arrow.exe runs with the file's directory as its working directory (compare_wasm_frontend.ps1).
setHostCwd(path.dirname(file));
const result = analyze(fs.readFileSync(file, 'utf8'), file);
if (!result) {
    console.error('analyze failed');
    process.exit(1);
}
// Only what the gate reads. The full result also has `members` (the module/class member tables),
// whose keys may differ only by case (class `Vec2`, module `vec2`); PowerShell 5.1's ConvertFrom-Json
// rejects such objects.
const { ok, parseError, parseErrorAt, diagnostics } = result;
process.stdout.write(JSON.stringify({ ok, parseError, parseErrorAt, diagnostics }));
