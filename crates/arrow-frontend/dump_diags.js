// dump_diags.js — analyse one .ar file with the wasm frontend and print the raw JSON.
//
//   node dump_diags.js <arrow_frontend.wasm> <file.ar>
//
// Used by scripts/compare_wasm_frontend.ps1. Prints JSON on stdout and nothing else,
// so the caller can pipe it straight into ConvertFrom-Json.
'use strict';
const fs = require('fs');

const [, , wasmPath, arPath] = process.argv;
if (!wasmPath || !arPath) {
    console.error('usage: node dump_diags.js <arrow_frontend.wasm> <file.ar>');
    process.exit(2);
}

(async () => {
    const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
    const ex = instance.exports;

    const buf = new TextEncoder().encode(fs.readFileSync(arPath, 'utf8'));
    const ptr = ex.ar_alloc(buf.length);
    new Uint8Array(ex.memory.buffer, ptr, buf.length).set(buf);
    ex.ar_analyze(ptr, buf.length);
    const out = new Uint8Array(ex.memory.buffer, ex.ar_result_ptr(), ex.ar_result_len());
    process.stdout.write(new TextDecoder().decode(out));
    ex.ar_free(ptr, buf.length);
})().catch(e => { console.error(String(e)); process.exit(1); });
