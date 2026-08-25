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
   after creating / moving / renaming / deleting files, rerun `./generate-codebase-map.ps1` from
   the repo root. Line counts are in parentheses — use them to plan partial reads (for files over
   ~300 lines, Grep for the anchor function and Read only that region with offset/limit).

## Module Roles

### Repo root
- `spec.md` — language specification
- `ar_config.json` — interpreter config (e.g. `rust.crates_path` for `import[rs]`)
- `run_examples.ps1` — batch-runs example scripts
- `generate-codebase-map.ps1` — regenerates the File Tree section below

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

### vscode-extension/
- `src/` — `extension.ts` activation/wiring; `analysis.ts` declaration/symbol analysis;
  `type_infer.ts` hover + inlay type inference; `tokenizer.ts`; `builtins.ts` built-in stubs;
  `native_module.ts`; `cs_assembly.ts`; `debug_runner.ts` (see `vscode-debug-runner` skill)
- `syntaxes/arrow.tmLanguage.json` — TextMate grammar (keyword highlighting)
- `make-vsix.ps1` — compile + package the VSIX (mandatory after extension changes)

### examples/ — feature-grouped examples (`*_error.ar` = error demos)
`basics/` core syntax; `typing/` type system; `classes/`; `collections/`; `exceptions/`;
`async/`; `interop/` `import[...]` demos + test modules + interop projects; `bench/` benchmarks;
`apps/` larger apps (spider solitaire); `practical_examples/`; `DxLib/` game-library interop;
`archived/` old examples kept for reference.

## File Tree (auto-generated)

Refresh with `./generate-codebase-map.ps1`. Do not edit by hand.

<!-- BEGIN AUTO-TREE -->
```text
src/  (211 files, 69236 lines)
  ar_config.rs (244)
  ast.rs (1124)
  decl_names.rs (173)
  expr_walk.rs (172)
  interpreter.rs (763)
  main.rs (636)
  prof.rs (508)
  repl.rs (112)
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
    parser_tests.rs (953)
    type_check_tests/
      access.rs (99)
      annotations.rs (321)
      bridge_mutability.rs (127)
      calls.rs (276)
      comparison.rs (110)
      decorators_generics.rs (249)
      guards_fntype.rs (353)
      mod.rs (36)
      union_types.rs (473)
      variables.rs (167)
  interpreter/
    ast_value.rs (735)
    async_mgr.rs (386)
    built_in_types.rs (364)
    cs_dll_runtime.rs (335)
    cs_proc_runtime.rs (187)
    debugger.rs (519)
    event_loop.rs (255)
    exceptions.rs (106)
    ffi_boundary.rs (404)
    js_proc_runtime.rs (200)
    msvc_errors.rs (112)
    proc_bridge.rs (200)
    py_interop.rs (412)
    resolver.rs (616)
    scope.rs (180)
    str_methods.rs (649)
    templates.rs (937)
    tw_stats.rs (308)
    vm_toplevel.rs (426)
    classes/
      async_manager_methods.rs (102)
      class_methods.rs (102)
      freeze.rs (169)
      frozen_list_methods.rs (93)
      instantiate.rs (113)
      lookup.rs (86)
      method_call.rs (415)
      mod.rs (111)
      object_methods.rs (314)
      set_methods.rs (163)
      string_methods.rs (614)
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
      attrs.rs (536)
      builtins.rs (925)
      calls.rs (861)
      core.rs (435)
      mod.rs (195)
      native.rs (492)
      subscript.rs (335)
    exec/
      blocks.rs (124)
      control_flow.rs (55)
      definitions.rs (699)
      dispatch.rs (214)
      exceptions_async.rs (371)
      mod.rs (277)
      modules.rs (961)
      vars.rs (253)
    functions/
      args.rs (319)
      deepcopy.rs (118)
      execution.rs (630)
      mod.rs (7)
      overload.rs (223)
    native_api/
      callbacks.rs (948)
      mod.rs (476)
    ops/
      display.rs (307)
      equality.rs (97)
      mod.rs (30)
      operators.rs (469)
      typecheck.rs (292)
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
      indexing.rs (124)
      instances.rs (427)
      iterator.rs (122)
      mod.rs (315)
      mustbe.rs (164)
      primitives.rs (186)
      pyobject.rs (213)
      set_type.rs (324)
      unpacking.rs (601)
    value/
      callables.rs (313)
      collections.rs (194)
      core.rs (377)
      exceptions.rs (41)
      flat.rs (107)
      instance.rs (400)
      mod.rs (20)
      native.rs (482)
      objects.rs (137)
  lexer/
    chars.rs (45)
    keyword.rs (143)
    literal.rs (334)
    math.rs (328)
    mod.rs (20)
    scan.rs (426)
    symbol.rs (263)
  parser/
    classes.rs (736)
    exprs.rs (940)
    mod.rs (221)
    types.rs (623)
    cs_assembly/
      metadata.rs (296)
      mod.rs (275)
      parse.rs (261)
      signature.rs (237)
      stub_gen.rs (448)
      xml_docs.rs (115)
    imports/
      ar_modules.rs (264)
      cpp.rs (203)
      cs_js_modules.rs (249)
      dispatch.rs (186)
      mod.rs (386)
      py_modules.rs (138)
    stmts/
      assignment.rs (193)
      control_flow.rs (150)
      core.rs (324)
      definitions.rs (208)
      functions.rs (279)
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
      stubs.rs (179)
  python_converter/
    annotations.rs (71)
    classes.rs (241)
    expressions.rs (276)
    mod.rs (29)
    statements.rs (448)
    utils.rs (42)
  type_check/
    annotations.rs (263)
    binop.rs (154)
    call_check.rs (602)
    decorator.rs (144)
    diagnostics.rs (32)
    errors.rs (482)
    infer.rs (506)
    members.rs (316)
    mod.rs (207)
    scope.rs (156)
    state.rs (127)
    type_utils.rs (184)
    types.rs (498)
    registry/
      builder.rs (422)
      mod.rs (157)
    stmt/
      check.rs (737)
      mod.rs (6)
      protocol.rs (242)
      resolve.rs (189)
  vm/
    chunk.rs (308)
    disasm.rs (148)
    mod.rs (33)
    op.rs (488)
    op_prof.rs (203)
    peephole.rs (272)
    run.rs (1633)
    compiler/
      block_expr.rs (403)
      calls.rs (324)
      control.rs (284)
      decls.rs (378)
      diag.rs (127)
      emit.rs (642)
      entry.rs (611)
      expr.rs (486)
      mod.rs (397)
      stmt.rs (643)
      stmt_assign.rs (300)

impl_python/  (49 files, 16410 lines)
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
    imports.py (386)
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

vscode-extension/  (11 files, 6661 lines; src/ + syntaxes/ only)
  src/
    analysis.ts (1648)
    builtins.ts (151)
    cs_assembly.ts (742)
    debug_runner.ts (417)
    extension.ts (195)
    native_module.ts (1059)
    test_goto_def.ts (304)
    tokenizer.ts (97)
    type_infer.ts (1470)
    vscode_mock.ts (279)
  syntaxes/
    arrow.tmLanguage.json (299)

examples/  (recursive .ar counts per category)
  apps/ (2 .ar)
  archived/ (72 .ar)
  async/ (6 .ar)
  basics/ (34 .ar)
  bench/ (24 .ar)
  classes/ (12 .ar)
  collections/ (7 .ar)
  debugger/ (5 .ar)
  DxLib/ (0 .ar)
  exceptions/ (7 .ar)
  interop/ (40 .ar)
  practical_examples/ (8 .ar)
  repl/ (0 .ar)
  typing/ (15 .ar)
  (2 loose .ar at top level)

(repo root)
  ab_bench.ps1 (116)
  ab_bench_modes.ps1 (165)
  annot_diff.ps1 (58)
  annot_unresolved.ps1 (87)
  ar_config.json (32)
  bench.ps1 (36)
  bench_baseline.md (68)
  BYTECODE_VM_PLAN.md (1053)
  CLAUDE.md (71)
  compare_bytecode.ps1 (109)
  compare_import_paths.ps1 (125)
  compare_outputs.ps1 (127)
  compare_python_impl.ps1 (213)
  debug_session.ps1 (161)
  dump_native_ir.ps1 (92)
  force_gate.ps1 (133)
  generate-codebase-map.ps1 (100)
  IMPLEMENTATION_LOG.md (9433)
  PHASE_R1_RESULTS.md (741)
  PHASE5_PLAN.md (427)
  prof_dist.ps1 (180)
  README.md (255)
  REFACTORING_HANDOFF.md (133)
  repl_session.ps1 (58)
  run_examples.ps1 (53)
  scan_examples.ps1 (56)
  spec.md (570)
  stale_doc_refs.ps1 (97)
  syntax_cov.ps1 (214)
  tw_stats.ps1 (104)
  tw_stats_files.ps1 (57)
```
_Generated 2026-08-26 by generate-codebase-map.ps1_
<!-- END AUTO-TREE -->
