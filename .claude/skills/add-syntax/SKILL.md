---
name: add-syntax
description: Use when adding a new keyword, statement, or expression form to the Arrow language — gives the layer-by-layer checklist of exact insertion points (lexer → parser → type checker → interpreter → partial compiler → impl_python → examples → VS Code extension → tests) so the change can be made without reading whole subsystem references.
---

# Adding a New Syntax Feature — Touchpoint Checklist

Cross-cutting recipe for a new keyword/statement/expression. Read this INSTEAD of opening
`parser-internals` / `type-checking` / `interpreter-internals` up front; open those only when a
design decision inside one layer needs them (e.g. where in the precedence chain an operator goes).

## Step 0 — Grep an analogous feature (the live checklist)

Pick the nearest existing feature and Grep its Token/Stmt name across `src/`. The hit list IS the
up-to-date touchpoint list — trust it over any doc, including this one. Canonical templates:

| New feature shape | Grep for | Example |
|---|---|---|
| Named declaration statement (`alias`, `new_type`-like) | `NewTypeDef` and `NewType` | `new_type Name: Type` |
| Control-flow keyword inside blocks | `BlockReturn` / `LoopYield` | `block_return v` |
| Operator-introduced statement | `AsyncAssign` / `LeftArrow` | `x <- async->T:` |

Read only the matched regions with offset/limit — files max out around 1,000 lines; never read whole files.

## Step 1 — Token + AST first, then let the compiler enumerate the rest

Add the `Token` and `Stmt`/`Expr` variants, then `cargo build`. Most dispatch matches are
**exhaustive** — the compile errors list every site you must update. Known sites:

| Layer | File / function | Note |
|---|---|---|
| Token enum | `src/token.rs` — variant list, doc-comment block near top, keyword→str map in the reverse-lookup match (`Token::X => Some("kw")`) | |
| Lexer | `src/lexer/keyword.rs` — `lex_word()` match arm `"kw" => Token::X` | two-word keywords use `maybe_two_word()` |
| AST | `src/ast.rs` — `Stmt`/`Expr` variant + entry in the doc-comment variant list | |
| Parser dispatch | `src/parser/stmts/core.rs` — `parse_stmt()` match arm | expression forms: precedence chain in `src/parser/exprs.rs` (see `parser-internals`) |
| Parser impl | `src/parser/stmts/definitions.rs` (declarations) or the topic-matching sibling module | |
| Type check | `src/type_check/stmt/check.rs` — `check_stmt()` match arm (exhaustive) | name visible before use? add a pre-pass in `src/type_check/mod.rs` (see `NewTypeDef` pre-pass there) |
| Interpreter exec | `src/interpreter/exec/dispatch.rs` — `exec()` match arm → `exec_x()` in a sibling module (exhaustive) | expressions: `src/interpreter/eval/core.rs` `eval()` |
| Template clone-walk | `src/interpreter/templates.rs` — `subst_stmt()` / `subst_expr()` (exhaustive) | |
| AST reflection | `src/interpreter/ast_value.rs` — Stmt→Value namespace match (exhaustive) | |

**Wildcard sites that will NOT compile-error** (check manually via the Step-0 grep):

- `src/partial_compiler/stub_gen.rs` — `_ => None` fallbacks; add an arm only if the construct can
  appear at module top level and must survive into `.ars` stubs.
- `src/partial_compiler/module_compiler.rs` — codegen eligibility: functions containing the new
  construct are silently skipped from native compilation unless handled; usually leaving it
  unsupported is fine, but verify the skip is graceful.

## Step 2 — Mandatory non-Rust follow-ups (regulations)

1. **Python implementation** — mirror the change in `impl_python/` (`token.py`, `ast.py`,
   `lexer/`, `parser/`, `type_check/`, `interpreter/`), then update the `# git SHA:` header line
   in the touched files to the current Rust-side commit SHA.
2. **Examples** — `examples/<category>/feature.ar` demonstrating the syntax; if an error pattern
   was implemented, also `feature_error.ar`. Categories: `basics/`, `typing/`, `classes/`,
   `collections/`, `exceptions/`, `async/`, `interop/`, `apps/`, `bench/`.
3. **VS Code extension** (`vscode-extension/`) — **most of this is now automatic.**

   The extension analyses `.ar` files with `crates/arrow-frontend` compiled to wasm, and
   that crate `#[path]`-includes the very files you just edited (`src/lexer`, `src/parser`,
   `src/type_check`). So parsing, type checking, diagnostics, hover, inlay hints,
   completion, signature help, go-to-definition and semantic tokens pick up the new syntax
   **with no TypeScript changes at all** — `make-vsix.ps1` rebuilds the wasm for you.

   What still needs a hand:
   - `syntaxes/arrow.tmLanguage.json` — add to the keyword alternation (and a dedicated
     capture rule if the construct declares a name; see the `new_type` rule). TextMate
     colouring runs before any analysis, so it is a genuinely separate system.
   - **Only if the construct declares a name**: add a `note_def` / `note_var` /
     `note_field` call at the parse site so the declaration lands in
     `src/parser/editor_index.rs`. Call it **immediately after `expect_ident()`** —
     the position comes from `prev_pos()`, so reading a type annotation first makes the
     symbol point at the wrong token. Pick the matching `EditorKind`, or add one.
   - **Only if you changed `import` syntax**: mirror it in `src/parser/imports_editor.rs`
     (the fs-free import parser the editor build uses). `scripts/compare_wasm_frontend.ps1`
     fails if the two stop accepting the same syntax.
   - Then regenerate the VSIX via `make-vsix.ps1` (mandatory).

   Verify with `scripts/compare_wasm_frontend.ps1` (editor and `arrow.exe` must agree on
   every example) and `vscode-extension/stress.js` (no provider throws; no new hover /
   go-to-definition misses).
4. **Tests** — `src/frontend_tests/lexer_tests.rs`, `parser_tests.rs`,
   `frontend_tests/type_check_tests/`, and `src/interpreter/tests/<topic>.rs`.
   Run `cargo test <name>` for the touched areas, then the full `cargo test`.
5. **Codebase map** — whenever the work performed file operations (created, moved, renamed, or
   deleted files), or added/removed a file's responsibility, rerun `./scripts/generate-codebase-map.ps1`
   from the repo root to regenerate the file tree in the `codebase-map` skill; if a directory's
   responsibility changed, also update the hand-written Module Roles section there.

## Step 3 — Verify

- `cargo run -- -src examples/<category>/feature.ar` and the `_error` variant.
- `python -m impl_python examples/<category>/feature.ar` for the Python mirror.
