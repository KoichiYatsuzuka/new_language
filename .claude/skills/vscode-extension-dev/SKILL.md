---
name: vscode-extension-dev
description: Use when adding or modifying a feature of the Arrow VS Code extension itself (vscode-extension/) — syntax highlighting, hover, inlay hints, completions, go-to-definition, diagnostics, commands/keybindings, settings, or the built-in function stub. Covers the source-file map, build commands, and the mandatory VSIX packaging step. For just exercising existing analysis code against a .ar file, use the vscode-debug-runner skill instead.
---

# Developing the VS Code Extension

The extension lives entirely under `vscode-extension/`. It provides syntax highlighting plus type-inference-driven language features (hover, inlay hints, completions, go-to-definition, diagnostics) for `.ar` files.

## Source file map (`vscode-extension/src/`)

| File | Responsibility |
|------|-----------------|
| `extension.ts` | Extension entry point (`activate`); registers all language-feature providers and commands |
| `analysis.ts` | Document analysis engine — parses `.ar` source line-by-line into `HoverSymbol[]` (variables, functions, classes, imports, …) |
| `type_infer.ts` | Registers/implements the actual VS Code providers: hover, inlay hints, semantic tokens, completions, document symbols, signature help, go-to-definition, diagnostics |
| `builtins.ts` | Pure static tables of Arrow built-in types/functions/keywords; no imports or I/O, shared by both the extension runtime and the standalone debug runner |
| `native_module.ts` | Resolves types for `import[rs]` / `import[cpp-dll]` / `import[cpp-lib]` by reading `ar_config.json` and parsing Rust/C++ source directly |
| `cs_assembly.ts` | ECMA-335 (.NET assembly) metadata reader for `import[cs-dll]`/`import[cs-proc]`; ported from `src/parser/cs_assembly.rs` — keep the two in sync when the Rust side changes |
| `tokenizer.ts` | Standalone lexer used by the extension's analysis code (separate from the Rust lexer) |
| `debug_runner.ts` / `vscode_mock.ts` | Standalone CLI harness (see `vscode-debug-runner` skill) — `vscode_mock.ts` is used exclusively by `debug_runner.ts`, never by extension code |
| `test_goto_def.ts` | Standalone test runner for "Go to Definition" (`node run_goto_def.js <file.ar>`) |

## Convention — explicit `let`/`mut` in every rendered signature

Function/param hovers show each parameter's mutability explicitly, and this must stay consistent across all sources. A param with no qualifier renders as `let`; a writable one as `mut`. Keep this invariant when adding an import language or touching signature display:

- **Arrow** (`analysis.ts`): `normalizeParamMutability()` prepends the implicit `let` to unqualified params when building a function's `signature:` string (`self` and explicit `let`/`mut`/`const` are left as-is). `parseParams()` strips a param's default value (`stripParamDefault()`) so `= expr` never leaks into the shown type.
- **C++** (`native_module.ts` `parseCParam`): non-const `*`/`&` → `mut` (write-back), value or `const` → `let`.
- **Rust** (`native_module.ts` `rsParamsToHv`): `&mut T` → `mut`, `T`/`&T` → `let`.
- **C#** (`cs_assembly.ts`): `ELEMENT_TYPE_BYREF` (`ref`/`out`) → `mut`, else `let`.
- **Python/JS**: left unqualified — no value-vs-reference distinction to derive from.

Note `FUNC_DEF_RE` (`builtins.ts`) captures the param list with a greedy `\((.*)\)` (not `[^)]*`) so a param list containing parentheses — e.g. a call-expression default `let x: int = make()` — isn't truncated at the first inner `)`. Safe because Arrow return types never contain `()`.

## Non-TypeScript pieces

- `syntaxes/arrow.tmLanguage.json` — TextMate grammar for syntax highlighting. Update this when adding/renaming keywords; `TODO.md` keeps a checklist of keywords that must be reflected here.
- `language-configuration.json` — bracket matching, comment tokens, auto-closing pairs.
- `builtins.ars` — built-in function stub file that powers hover/completion/signature-help for built-ins. This is **separate** from `src/built_in_stab/*.ars` in the main Rust project — the two are not auto-synced, update both if a built-in changes.
- `package.json` `contributes` block — commands, keybindings, `languages`/`grammars` registration, and the `arrow.*` settings (e.g. `arrow.pythonLibraryPaths`).
- `TODO.md` — running checklist of extension features/keywords; update it when you finish a checklist item or add a new one.

## Build commands

```bash
cd vscode-extension
npm run compile        # tsc -p ./  → out/  (used by the real extension, and by make-vsix.ps1)
npm run watch          # tsc -watch -p ./
npm run compile:debug  # tsc -p tsconfig.debug.json → out_debug/ (standalone debug runner only)
```

Manually test inside VS Code with the "Run Extension" launch config (F5), or exercise the analysis code headlessly — see the `vscode-debug-runner` skill.

## Packaging (VSIX)

Per the project regulations, **any extension update requires recompiling and regenerating the VSIX**. Do this with the hand-rolled packaging script (not `vsce package`):

```bash
cd vscode-extension
pwsh ./make-vsix.ps1   # or: powershell -File make-vsix.ps1
```

`make-vsix.ps1`:
1. Runs `npm run compile`.
2. Assembles `[Content_Types].xml` + `extension.vsixmanifest` by hand.
3. Copies `package.json`, `language-configuration.json`, `out/*.js`, `syntaxes/arrow.tmLanguage.json`, and `builtins.ars` into a staging folder.
4. Zips the staging folder into `arrow-<version>.vsix` at the `vscode-extension/` root (version comes from `package.json`).

If you add a new runtime asset (e.g. another `.ars` stub or a new `out/` file that isn't `*.js`), add a corresponding `Copy-Item` line in `make-vsix.ps1` — files not copied into the staging folder will silently be missing from the packaged extension.
