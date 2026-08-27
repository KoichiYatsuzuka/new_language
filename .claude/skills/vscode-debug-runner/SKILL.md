---
name: vscode-debug-runner
description: Use when debugging or testing the VS Code extension's language features (hover, inlay hints, semantic tokens, completion, signature help, go-to-definition, diagnostics) without launching VS Code — via the per-file debug runner (run_debug.js) or the all-examples regression sweep (stress.js). Also use after changing wasm_providers.ts, frontend.ts, or the Rust frontend to know which build steps to rerun. For adding/modifying an extension feature itself or packaging the VSIX, use the vscode-extension-dev skill instead.
---

# VS Code Extension — Headless Harnesses

Two tools exercise the extension's providers without running VS Code. Both drive
`wasm_providers.ts`, which is the same code the real extension uses, against
`crates/arrow-frontend` compiled to wasm — so what you see here is what the editor does.

⚠ **Use VS Code's own Node.** The `node` on PATH may predate the wasm opcodes the Rust
standard library emits (v11 on this machine fails with `Invalid opcode`). VS Code ships Node 24:

```bash
export ELECTRON_RUN_AS_NODE=1
"$LOCALAPPDATA/Programs/Microsoft VS Code/Code.exe" run_debug.js <file.ar>
```

## Setup (after changing analysis code)

```bash
cd vscode-extension
npm run compile:debug     # tsc -p tsconfig.debug.json → out_debug/

# If you changed src/lexer, src/parser, src/type_check, or crates/arrow-frontend:
cd ../crates/arrow-frontend && cargo build --release --target wasm32-unknown-unknown
```

`frontend.ts` finds the wasm at `out/arrow_frontend.wasm` (packaged) and falls back to
`crates/arrow-frontend/target/wasm32-unknown-unknown/release/arrow_frontend.wasm` (dev tree), so
the runner picks up a fresh `cargo build` without repackaging the VSIX.

## 1. `run_debug.js` — one file, all seven features

```bash
node run_debug.js ../examples/classes/protocol.ar
```

| Section | What it shows |
|---------|---------------|
| `SOURCE` | Every line with semantic-token colours and inlay hints inserted inline |
| `HOVER + GO-TO-DEFINITION` | Hover balloon and definition target for **every** declaration in the outline |
| `DOCUMENT SYMBOLS` | The outline tree (nesting comes from the parser's scope tree) |
| `COMPLETION` | Scoped-name probes, plus a `.`-access probe per receiver found |
| `SIGNATURE HELP` | Resolved call signatures with the active parameter index |
| `DIAGNOSTICS` | Errors / warnings with line:col |

Colour legend: bright-yellow = class, yellow = trait/protocol, magenta = enum member,
bright-green = function/method, cyan = field/type, bright-cyan = parameter, blue = module,
dim-green = inlay hint.

The probe list comes from `provideDocumentSymbols`, so unlike the old runner it does **not**
depend on a symbol's name appearing textually on its recorded line — parameters of multi-line
signatures are probed correctly.

## 2. `stress.js` — every example, regression counters

```bash
node stress.js
```

Runs all seven providers over every `.ar` under `examples/` and prints:

```
files          : 164
ok             : 164
threw          : 0     <- must be 0
no symbols     : 3
symbols probed : 2486
hover misses   : 0 / 2486
def   misses   : 0 / 2486
```

- `threw` must be 0.
- `hover misses` / `def misses` must be 0. A non-zero count usually means a declaration hook was
  called at the wrong moment and its recorded position no longer sits on the name
  (see `vscode-extension-dev`, "Adding a feature").
- `no symbols` is expected for exactly three files: the two deliberate `ParseError` examples
  (`alias_error.ar`, `functions_errors.ar`) and `math_string.ar`, which declares nothing.

## What each harness will NOT catch

Both use the analysis JSON, so they verify the editor is self-consistent — not that it agrees with
the interpreter. For that run `./scripts/compare_wasm_frontend.ps1`, which compares the wasm
frontend's diagnostics against `arrow.exe` on every example and fails if the editor invents an
error or rejects code the compiler accepts.

## Interpreting a blank result

If `wasm loaded : false` appears, the wasm was not found or the runtime rejected it; the reason is
in `frontendLoadError()`. Providers return empty rather than throwing in that state, so a totally
empty output means "frontend missing", not "file has no symbols".
