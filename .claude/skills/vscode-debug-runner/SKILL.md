---
name: vscode-debug-runner
description: Use when debugging or testing the VS Code extension's type-inference/analysis code (inlay hints, hover, diagnostics) via the standalone debug runner (node run_debug.js), without launching VS Code itself. Also use after changing analysis.ts, type_infer.ts, or native_module.ts to know which npm script to recompile. For adding/modifying an extension feature itself or packaging the VSIX, use the vscode-extension-dev skill instead.
---

# VS Code Extension — Standalone Debug Runner

A CLI tool that exercises the extension's analysis code against a `.ar` file without running VS Code. It outputs ANSI-coloured source with inlay hints inserted inline, hover balloon content for every symbol, and a diagnostics list.

**Setup (one-time after changing analysis code):**

```bash
cd vscode-extension
npm run compile:debug     # compiles to out_debug/ with ES2019 target
```

**Run against any `.ar` file:**

```bash
node run_debug.js <path/to/file.ar>
# example
node run_debug.js ../examples/interop/importation.ar
```

**Output sections:**

| Section | What it shows |
|---------|---------------|
| `SOURCE` | Every source line with inlay hints inserted in cyan and semantic token colours (yellow = class/import, cyan = built-in type) |
| `HOVER REFERENCE` | Every symbol with its hover balloon content |
| `DIAGNOSTICS` | Errors / warnings with line:col |

**Colour legend (ANSI — requires a terminal that renders escape codes):**

- Yellow `■` — class name or imported identifier
- Cyan `■` — built-in type (`int`, `float`, `str`, …)
- Bright-cyan `■` — inlay hint (type inserted by inference)
- Red `■` / Yellow `■` — diagnostic error / warning

**Gotcha — `HOVER REFERENCE` hides symbols not declared on their own line.** The runner only prints a symbol if its name appears textually on `sym.line` (the declaration line it recorded). A parameter of a *multi-line* `fn(` signature has `sym.line` = the `fn` line, where its name doesn't appear, so it is silently skipped in HOVER REFERENCE even though it exists in the symbol table and hovers correctly in the real editor. Don't conclude "the symbol is missing" from HOVER REFERENCE alone — write a tiny harness that calls `provideHover(doc, {line, character})` at the real position instead. (If that harness loads `out_debug/analysis.js` directly, first install an `Array.prototype.at` polyfill like `run_debug.js` does — without it `DocumentAnalysis` throws, is caught, and returns an *empty* analysis, making every symbol look absent.)

**How `import[rs]` types are resolved:**

The extension reads `ar_config.json` (walked up from the document directory) to find `rust.crates_path`, then parses the crate's `src/lib.rs` directly. No `.ars` stub file is needed for Rust crates — the type information is read live from the Rust source.

**Re-compile reminder:** Run `npm run compile:debug` whenever `analysis.ts`, `type_infer.ts`, or `native_module.ts` change. The VSIX for the real extension uses `npm run compile` (separate `out/` directory).
