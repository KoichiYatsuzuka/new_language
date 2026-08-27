---
name: vscode-extension-dev
description: Use when adding or modifying a feature of the Arrow VS Code extension itself (vscode-extension/) — syntax highlighting, hover, inlay hints, completions, go-to-definition, diagnostics, semantic tokens, signature help, commands/keybindings, settings, or the built-in function stub. Covers the source-file map, how the wasm frontend supplies all analysis, build commands, and the mandatory VSIX packaging step. For just exercising existing analysis code against a .ar file, use the vscode-debug-runner skill instead.
---

# Developing the VS Code Extension

The extension lives entirely under `vscode-extension/`. It provides TextMate syntax highlighting
plus seven language features driven by the **real Arrow frontend**.

## The one thing to understand first

**The extension contains no Arrow-language logic.** All analysis happens in
`crates/arrow-frontend`, a wasm32 build of the same `src/lexer`, `src/parser` and
`src/type_check` that `cargo run` uses (pulled in with `#[path]`, not copied). The TypeScript
side only translates the analysis result into VS Code objects.

This is deliberate. Until 2026-08 the extension approximated the grammar with ~15 line-oriented
regexes in `analysis.ts` / `type_infer.ts`. Every new keyword had to be mirrored by hand, and it
did not keep up: 16 of the lexer's 55 keywords were missing, `protocol` was not recognised as a
declaration at all, and the scope model was flat enough to report `Variable 'x' is already
declared` for two unrelated `x`es. Those 5,471 lines are gone. **Do not reintroduce
language knowledge in TypeScript** — if the editor needs to know something about Arrow, teach the
Rust frontend and expose it through the analysis JSON.

## Source file map (`vscode-extension/src/`)

| File | Responsibility |
|------|-----------------|
| `extension.ts` | Entry point (`activate`): loads the wasm frontend + `builtins.ars` prelude, registers the seven providers and the Send-to-REPL command, schedules debounced diagnostics |
| `frontend.ts` | Loads `arrow_frontend.wasm` and exposes `analyze(source) → JSON`. Owns the raw C ABI (`ar_alloc` / `ar_analyze` / `ar_result_ptr` / …) |
| `wasm_providers.ts` | All seven providers, built on the analysis JSON: hover, inlay hints, semantic tokens, completion, signature help, go-to-definition, document symbols, diagnostics. Also the scope walk and the `builtins.ars` prelude |
| `debug_runner.ts` / `vscode_mock.ts` | Standalone CLI harness (see `vscode-debug-runner` skill) — `vscode_mock.ts` is used exclusively by `debug_runner.ts`, never by extension code |

Rust side of the same feature:

| File | Responsibility |
|------|-----------------|
| `crates/arrow-frontend/src/lib.rs` | `#[path]` declarations that pull `src/lexer`, `src/parser`, `src/type_check` into a dependency-light crate |
| `crates/arrow-frontend/src/analyze.rs` | Builds the analysis JSON (diagnostics / symbols / scopes / exprTypes / members) |
| `crates/arrow-frontend/src/wasm.rs` | The wasm C ABI. No `wasm-bindgen` — plain `extern "C"` + linear memory, so no extra toolchain is needed |
| `src/parser/editor_index.rs` | The declaration + scope + node-span side tables. **`editor` feature only**; the AST is not modified |
| `src/parser/editor_hooks.rs` | `note_var` / `note_def` / `note_field` / … — empty functions in the normal build |
| `src/parser/imports_editor.rs` | fs-free import parsing for the editor build |

## Where each feature gets its data

| Feature | Source in the analysis JSON |
|---|---|
| Diagnostics | `diagnostics` — `TypeChecker::check_program` errors and warnings |
| Hover | `symbols` (declaration + signature + docstring + access) and `symbols[].inferred` |
| Inlay hints | `symbols[].inferred` — the initializer's node-id resolved through `AstAnnotations` |
| Semantic tokens | `symbols` + the scope tree, matched against identifier occurrences |
| Completion | `scopes` for visible names; `members` for `.` access |
| Signature help | `symbols[].signature` and `members[].params` |
| Go to definition | `symbols[].at` |

## Adding a feature

1. If the data you need is already in the analysis JSON, work only in `wasm_providers.ts`.
2. If it is not, add it in `crates/arrow-frontend/src/analyze.rs` (transcription only — no
   language judgements there either), and if the parser has to record something new, extend
   `src/parser/editor_index.rs` + `editor_hooks.rs`. Hooks must stay zero-cost in the normal
   build: take `&str` / `usize` arguments and put the whole body behind
   `#[cfg(feature = "editor")]`.
3. Rebuild and verify — see below.

⚠ Position hooks must be called **immediately after `expect_ident()`**. The position comes from
`prev_pos()`, so reading a type annotation or `as` alias first makes the symbol point at the wrong
token (this actually happened with `import[rs] libm[0.2]`, which pointed at `]`).

## Convention — explicit `let`/`mut` in every rendered signature

Function/param hovers show each parameter's mutability explicitly. A param with no qualifier
renders as `let`, a writable one as `mut`, and `mut self` keeps its `mut`. This is built in
`render_fn_signature()` in `src/parser/editor_hooks.rs` — one place, used by every consumer.

## Non-TypeScript pieces

- `syntaxes/arrow.tmLanguage.json` — TextMate grammar. **Still manual**: colouring runs before any
  analysis, so it is a genuinely separate system. Update it when adding/renaming keywords.
- `language-configuration.json` — bracket matching, comment tokens, auto-closing pairs.
- `builtins.ars` — built-in function stubs (`print`, `len`, …) that power hover/completion/
  signature-help for built-ins. It is **parsed by the real Arrow parser**, so it must be valid
  Arrow: bodies are `pass`, never `...` (an `...` body is only accepted when it is the entire
  body, so combining it with a docstring is a syntax error). Separate from
  `src/built_in_stab/*.ars`; the two are not auto-synced.
- `package.json` `contributes` — commands, keybindings, `languages`/`grammars`, `arrow.*` settings.

## Build commands

```bash
cd vscode-extension
npm run compile        # tsc -p ./  → out/  (the real extension; make-vsix.ps1 runs this)
npm run watch          # tsc -watch -p ./
npm run compile:debug  # tsc -p tsconfig.debug.json → out_debug/ (debug runner only)

# The analysis engine (make-vsix.ps1 rebuilds it automatically):
cd ../crates/arrow-frontend && cargo build --release --target wasm32-unknown-unknown
```

## Verification

| Check | Command | Must show |
|---|---|---|
| Editor agrees with `arrow.exe` | `./scripts/compare_wasm_frontend.ps1` | `INVENTED: 0`, `parse mismatch: 0` |
| No provider throws / no regressions | `ELECTRON_RUN_AS_NODE=1 "<VS Code>/Code.exe" stress.js` | `threw: 0`, `hover misses: 0`, `def misses: 0` |
| One file in detail | `node run_debug.js <file.ar>` | see `vscode-debug-runner` |

⚠ The `node` on PATH may be too old to compile the wasm (it needs post-MVP opcodes). VS Code
ships Node 24; run these with `ELECTRON_RUN_AS_NODE=1 "<VS Code>/Code.exe"` when in doubt.

## Packaging (VSIX)

Per the project regulations, **any extension update requires recompiling and regenerating the
VSIX**. Use the hand-rolled script (not `vsce package`):

```bash
cd vscode-extension
pwsh ./make-vsix.ps1   # or: powershell -File make-vsix.ps1
```

`make-vsix.ps1`:
1. Runs `npm run compile`.
2. **Rebuilds `arrow_frontend.wasm`** (`cargo build --release --target wasm32-unknown-unknown`).
   Free when nothing changed, and it is what stops a VSIX shipping with a frontend older than the
   interpreter. If `cargo` is absent it warns and packages the existing wasm.
3. Assembles `[Content_Types].xml` + `extension.vsixmanifest` by hand.
4. Copies `package.json`, `language-configuration.json`, `out/*.js`, `out/arrow_frontend.wasm`,
   `syntaxes/*.json`, `icons/*.svg` and `builtins.ars` into a staging folder.
5. Zips it into `arrow-<version>.vsix` at the `vscode-extension/` root.

If you add a new runtime asset, add a `Copy-Item` line **and** a `<Default Extension=…>` entry in
`[Content_Types].xml` — files not copied are silently missing from the packaged extension.

The result is self-contained: no Rust toolchain, no `arrow.exe`, no external process at runtime.
The `.wasm` imports nothing from the host (verified: import section is empty), so one file serves
Windows / macOS / Linux on x64 and ARM alike.
