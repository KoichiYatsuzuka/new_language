---
name: codebase-map
description: Use when you need to know where a file, module, or subsystem lives in the Arrow repository (Rust src/, Python impl_python/, examples/, vscode-extension/), or what a given module is responsible for. Includes per-file line counts for partial-read decisions. Read this before navigating an unfamiliar part of the tree instead of globbing blindly.
---

# Codebase Map

Two layers, maintained differently:

1. **Module Roles** (below) — hand-maintained at **directory level**, so file renames inside a
   module don't invalidate it. Update a role line when a directory's responsibility changes or a
   directory is added/removed.
2. **File Tree** (bottom) — **auto-generated**. Never hand-edit between the AUTO-TREE markers;
   after creating / moving / renaming / deleting files, rerun `./scripts/generate-codebase-map.ps1` from
   the repo root. Line counts are in parentheses — use them to plan partial reads (for files over
   ~300 lines, Grep for the anchor function and Read only that region with offset/limit).

## Module Roles

### Repo root
- `implementation_logs/` — 計画・実装ログ・引き継ぎ文書（`BYTECODE_VM_PLAN.md` / `IMPLEMENTATION_LOG.md` / `FUTURE_FEATURE.md` ほか）
- `docs/spec.md` — 言語仕様の概要（日本語）。⚠ **残タスクはここに書かない**（正本は `implementation_logs/FUTURE_FEATURE.md`）
- `ar_config.json` — interpreter config (e.g. `rust.crates_path` for `import[rs]`)
- `_archive/` — 役目を終えたスクリプトの退避先（理由と代替は `_archive/README.md`）
- `scripts/` — 検証・計測スクリプト（何をいつ走らせるかは `CLAUDE.md`）。`generate-codebase-map.ps1` もここ

### src/ — Rust implementation (primary)
- Root files: `main.rs` entry point / CLI; `repl.rs` REPL; `token.rs` Token enum + Span;
  `ast.rs` AST node definitions; `interpreter.rs` re-export shim for `interpreter/`
- `lexer/` — tokenizer: scan loop, keyword recognition (`lex_word()` in `keyword.rs`),
  literal/operator/symbol scanning, indentation tracking
- `parser/` — recursive-descent parser; imported-module loading happens here at parse time
  - `stmts/` — statement parsing (`core.rs` holds the `parse_stmt()` dispatch)
  - `exprs.rs` — expression precedence chain
  - `classes.rs` / `types.rs` — class/trait parsing, type-annotation parsing
  - `imports/` — `import[lang]` parsing + module resolution
  - `cs_assembly/` — .NET DLL inspection for `--compile-cs` stub generation
- `type_check/` — static type checker (runs between parse and exec)
  - `mod.rs` — ファサードのみ: `TypeChecker::check` / `check_with_warnings` + 組み込み登録。
    状態は3つのサブ構造体に分割され、相互依存はない
  - `registry/` — 宣言索引 `TypeRegistry`（クラス/trait/protocol/関数）。`builder.rs` の
    収集パスだけが書き込め、検査中は読み取り専用
  - `state.rs` — `CheckState`（スコープスタック・現在の関数/クラス・`block_return` 禁止深さ）
  - `diagnostics.rs` — `Diagnostics`（収集されたエラー・警告）
  - `stmt/` — statement checking (`check.rs` holds `check_stmt()`)
  - `members.rs` — 型のメンバー解決と Intersection 適合検査
  - `infer.rs` / `types.rs` / `call_check.rs` / `binop.rs` — inference, `InferredType`,
    call-site checking, operator typing
- `interpreter/` — tree-walk interpreter
  - `exec/` — statement execution (`dispatch.rs` holds `exec()`)
  - `eval/` — expression evaluation (`core.rs` holds `eval()`)
  - `functions/` / `classes/` / `value/` / `ops/` — calls & closures/generators,
    class/instance/method dispatch, runtime `Value` types, operator implementations
  - `native_api/` — ABI handle arena + `ArCallbacks` passed to native DLLs
  - `cpp_bridge/` — C/C++ interop: header parsing, shim compile driver
  - `templates.rs` — template instantiation (`subst_stmt` / `subst_expr` clone-walk)
  - `ast_value.rs` — AST→Value reflection
  - `tests/` — interpreter integration tests, one file per topic
- `partial_compiler/` — `--compile` subsystem: `module_compiler.rs` orchestration + codegen
  eligibility; `llvm_codegen/` code generation (LLVM IR text → clang → DLL); `rs_loader/`
  `import[rs]` crate loader; `stub_gen.rs` `.ars` stub emission
- `python_converter/` — Arrow → Python source converter
- `built_in_stab/` — `.ars` stubs for built-ins (also consumed by the VS Code extension)
- `frontend_tests/` — lexer / parser / type-check tests

### impl_python/ — Python mirror implementation
Mirrors `src/` layer-by-layer (`lexer/`, `parser/`, `type_check/`, `interpreter/`,
`partial_compiler/`). Keep synchronized with Rust changes and update the `# git SHA:` header
lines in touched files (see regulations).

### crates/arrow-frontend/ — the editor's analysis engine
The Arrow frontend (`src/lexer`, `src/parser`, `src/type_check`) pulled in with `#[path]` and
built for `wasm32-unknown-unknown`. A separate crate because the root package depends on
`pyo3` / `libloading`, which do not build for wasm — those live entirely in `src/interpreter/`.
`analyze.rs` turns a parse + type-check into the JSON the extension consumes; `wasm.rs` is the
plain `extern "C"` ABI (no `wasm-bindgen`, so no extra toolchain).

### vscode-extension/
**Contains no Arrow-language logic** — all analysis comes from the wasm frontend above.
- `src/` — `extension.ts` activation/wiring; `frontend.ts` wasm loader; `wasm_providers.ts` all
  seven language-feature providers; `debug_runner.ts` + `vscode_mock.ts` (see the
  `vscode-debug-runner` skill)
- `builtins.ars` — built-in stubs, parsed by the real parser (bodies must be `pass`, not `...`)
- `syntaxes/arrow.tmLanguage.json` — TextMate grammar (keyword highlighting; still hand-maintained)
- `stress.js` — all-examples provider regression sweep
- `make-vsix.ps1` — rebuild the wasm + compile TS + package the VSIX (mandatory after changes)

### examples/ — feature-grouped examples (`*_error.ar` = error demos)
`basics/` core syntax; `typing/` type system; `classes/`; `collections/`; `exceptions/`;
`async/`; `interop/` `import[...]` demos + test modules + interop projects; `bench/` benchmarks;
`apps/` larger apps (spider solitaire); `practical_examples/`; `DxLib/` game-library interop;
`archived/` old examples kept for reference.

## File Tree (auto-generated)

Refresh with `./scripts/generate-codebase-map.ps1`. Do not edit by hand.

<!-- BEGIN AUTO-TREE -->
```text
src/  (220 files, 80783 lines)
  ar_config.rs (244)
  ast.rs (1240)
  decl_names.rs (173)
  expr_walk.rs (172)
  interpreter.rs (946)
  main.rs (661)
  prof.rs (557)
  repl.rs (118)
  stmt_walk.rs (277)
  syntax_cov.rs (379)
  token.rs (531)
  built_in_stab/
    basic_traits.ars (96)
    built_in_const.ars (2)
    built_in_type.ars (33)
    error.ars (77)
  frontend_tests/
    lexer_tests.rs (315)
    mod.rs (6)
    parser_tests.rs (1088)
    type_check_tests/
      access.rs (99)
      annotations.rs (321)
      bridge_mutability.rs (127)
      calls.rs (276)
      comparison.rs (148)
      decorators_generics.rs (275)
      guards_fntype.rs (353)
      mod.rs (36)
      union_types.rs (473)
      variables.rs (188)
  interpreter/
    ast_value.rs (739)
    async_mgr.rs (386)
    built_in_types.rs (368)
    cs_dll_runtime.rs (335)
    cs_proc_runtime.rs (187)
    debugger.rs (519)
    event_loop.rs (255)
    exceptions.rs (111)
    ffi_boundary.rs (413)
    js_proc_runtime.rs (200)
    msvc_errors.rs (112)
    proc_bridge.rs (200)
    py_interop.rs (422)
    resolver.rs (680)
    scope.rs (180)
    str_methods.rs (649)
    templates.rs (988)
    tw_stats.rs (308)
    vm_toplevel.rs (426)
    classes/
      async_manager_methods.rs (102)
      class_methods.rs (119)
      freeze.rs (169)
      frozen_list_methods.rs (90)
      instantiate.rs (113)
      lookup.rs (86)
      method_call.rs (451)
      mod.rs (111)
      object_methods.rs (314)
      set_methods.rs (149)
      string_methods.rs (611)
    cpp_bridge/
      codegen.rs (499)
      compiler.rs (632)
      config.rs (330)
      mod.rs (32)
      typedef_loader.rs (324)
      types.rs (185)
      header_parser/
        decls.rs (453)
        mod.rs (255)
        preprocess.rs (252)
        structs.rs (376)
    eval/
      attrs.rs (552)
      builtins.rs (916)
      calls.rs (861)
      core.rs (441)
      mod.rs (198)
      native.rs (492)
      subscript.rs (344)
    exec/
      blocks.rs (125)
      control_flow.rs (40)
      definitions.rs (840)
      dispatch.rs (218)
      exceptions_async.rs (371)
      mod.rs (313)
      modules.rs (962)
      vars.rs (296)
    functions/
      args.rs (371)
      deepcopy.rs (142)
      execution.rs (871)
      mod.rs (7)
      overload.rs (223)
    native_api/
      callbacks.rs (955)
      mod.rs (476)
    ops/
      display.rs (307)
      equality.rs (547)
      hash.rs (553)
      mod.rs (31)
      operators.rs (545)
      typecheck.rs (329)
    tests/
      alias.rs (109)
      async_tests.rs (183)
      basics.rs (136)
      callables.rs (670)
      classes.rs (357)
      collections.rs (304)
      control_flow.rs (123)
      enum_defaults.rs (179)
      events_external.rs (53)
      exceptions.rs (328)
      expressions.rs (1172)
      file_io.rs (235)
      functions.rs (192)
      hashing.rs (358)
      indexing.rs (124)
      instances.rs (427)
      iterator.rs (122)
      mod.rs (320)
      mustbe.rs (164)
      primitives.rs (186)
      pyobject.rs (213)
      set_type.rs (324)
      unpacking.rs (601)
    value/
      callables.rs (430)
      collections.rs (335)
      core.rs (380)
      exceptions.rs (41)
      flat.rs (107)
      instance.rs (484)
      mod.rs (20)
      native.rs (482)
      objects.rs (137)
  lexer/
    chars.rs (45)
    editor_tokens.rs (144)
    keyword.rs (143)
    literal.rs (334)
    math.rs (328)
    mod.rs (24)
    scan.rs (454)
    symbol.rs (263)
  parser/
    classes.rs (897)
    editor_hooks.rs (424)
    editor_index.rs (250)
    exprs.rs (1018)
    imports_editor.rs (228)
    mod.rs (289)
    types.rs (680)
    cs_assembly/
      metadata.rs (296)
      mod.rs (275)
      parse.rs (261)
      signature.rs (237)
      stub_gen.rs (451)
      xml_docs.rs (115)
    imports/
      ar_modules.rs (264)
      cpp.rs (205)
      cs_js_modules.rs (249)
      dispatch.rs (187)
      mod.rs (410)
      py_modules.rs (138)
    stmts/
      assignment.rs (193)
      control_flow.rs (150)
      core.rs (358)
      definitions.rs (215)
      functions.rs (320)
      mod.rs (35)
  partial_compiler/
    mod.rs (15)
    module_compiler.rs (361)
    stub_gen.rs (338)
    llvm_codegen/
      context.rs (427)
      expr.rs (1043)
      function.rs (485)
      mod.rs (1099)
      stmt.rs (433)
    rs_loader/
      codegen.rs (468)
      loader.rs (312)
      mod.rs (100)
      parse.rs (639)
      stubs.rs (181)
  python_converter/
    annotations.rs (71)
    classes.rs (350)
    decorators.rs (93)
    expressions.rs (720)
    mod.rs (36)
    param_rewrite.rs (82)
    statements.rs (626)
    supers.rs (41)
    utils.rs (42)
  type_check/
    annotations.rs (263)
    binop.rs (613)
    call_check.rs (1383)
    decorator.rs (145)
    diagnostics.rs (32)
    errors.rs (908)
    infer.rs (1202)
    members.rs (317)
    mod.rs (270)
    scope.rs (273)
    state.rs (281)
    type_utils.rs (601)
    types.rs (719)
    registry/
      builder.rs (616)
      mod.rs (195)
    stmt/
      check.rs (1808)
      mod.rs (6)
      protocol.rs (459)
      resolve.rs (474)
  vm/
    chunk.rs (314)
    disasm.rs (149)
    mod.rs (33)
    op.rs (551)
    op_prof.rs (205)
    peephole.rs (380)
    run.rs (1839)
    compiler/
      block_expr.rs (419)
      calls.rs (350)
      control.rs (284)
      decls.rs (297)
      diag.rs (127)
      emit.rs (718)
      entry.rs (640)
      expr.rs (508)
      mod.rs (565)
      stmt.rs (699)
      stmt_assign.rs (334)

impl_python/  (49 files, 16411 lines)
  __init__.py (0)
  __main__.py (92)
  ast.py (608)
  repl.py (55)
  token.py (372)
  interpreter/
    __init__.py (33)
    builtins.py (1125)
    cs_dll_runtime.py (233)
    cs_proc_runtime.py (243)
    env.py (139)
    exceptions.py (65)
    interpreter.py (3025)
    native_api.py (515)
    value.py (872)
    cpp_bridge/
      __init__.py (29)
      compiler.py (408)
      config.py (114)
      header_parser.py (760)
      loader.py (652)
      types.py (148)
  lexer/
    __init__.py (21)
    chars.py (28)
    keyword.py (40)
    literal.py (115)
    math.py (6)
    scan.py (191)
    symbol.py (149)
  parser/
    __init__.py (131)
    classes.py (335)
    cs_assembly.py (831)
    exprs.py (476)
    imports.py (387)
    stmts.py (634)
    types.py (304)
  partial_compiler/
    __init__.py (12)
    codegen.py (11)
    module_compiler.py (11)
    rs_loader.py (1408)
    stub_gen.py (11)
  type_check/
    __init__.py (102)
    binop.py (84)
    call_check.py (208)
    decorator.py (71)
    errors.py (200)
    infer.py (190)
    scope.py (48)
    stmt.py (506)
    type_utils.py (78)
    types.py (335)

vscode-extension/  (7 files, 2241 lines; src/ + syntaxes/ only)
  src/
    debug_runner.ts (350)
    extension.ts (225)
    frontend.ts (133)
    vscode_mock.ts (289)
    wasm_providers.ts (937)
  syntaxes/
    arrow.tmLanguage.json (299)
    arrow-stub.tmLanguage.json (8)

examples/  (recursive .ar counts per category)
  apps/ (2 .ar)
  archived/ (72 .ar)
  async/ (7 .ar)
  basics/ (54 .ar)
  bench/ (26 .ar)
  classes/ (42 .ar)
  collections/ (18 .ar)
  debugger/ (6 .ar)
  DxLib/ (0 .ar)
  exceptions/ (7 .ar)
  interop/ (76 .ar)
  practical_examples/ (8 .ar)
  repl/ (0 .ar)
  typing/ (87 .ar)
  (2 loose .ar at top level)

scripts/  (検証・計測スクリプト。何をいつ走らせるかは CLAUDE.md)
  ab_bench.ps1 (116)
  ab_bench_modes.ps1 (165)
  annot_diff.ps1 (58)
  annot_unresolved.ps1 (87)
  compare_bytecode.ps1 (135)
  compare_import_paths.ps1 (135)
  compare_outputs.ps1 (140)
  compare_python_impl.ps1 (383)
  compare_wasm_frontend.ps1 (200)
  debug_session.ps1 (161)
  dump_native_ir.ps1 (92)
  force_gate.ps1 (134)
  generate-codebase-map.ps1 (115)
  hash_eq_identity.ps1 (147)
  prof_attr_ic.ps1 (104)
  prof_dist.ps1 (180)
  repl_session.ps1 (58)
  run_extension_debug.ps1 (92)
  scan_examples.ps1 (66)
  stale_doc_refs.ps1 (131)
  syntax_cov.ps1 (215)
  tw_stats.ps1 (105)
  type_obligations.ps1 (168)

implementation_logs/  (計画・実装ログ・引き継ぎ文書)
  bench_baseline.md (68)
  bug_fix.md (911)
  BUGFIX_B1_B13.md (250)
  BYTECODE_VM_PLAN.md (1016)
  FUTURE_FEATURE.md (353)
  IMPLEMENTATION_LOG.md (9882)
  PHASE_R1_RESULTS.md (741)
  PHASE5_PLAN.md (427)
  PYTHON_IMPL_SYNC_PLAN.md (207)
  REFACTORING_HANDOFF.md (133)

(repo root)
  ar_config.json (32)
  CLAUDE.md (144)
  README.md (317)
```
_Generated 2026-09-19 by generate-codebase-map.ps1_
<!-- END AUTO-TREE -->
