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
  `ast.rs` AST node definitions; `interpreter.rs` re-export shim for `interpreter/`;
  `module_path.rs` **the import search rule** (CPython: absolute from the entry dir, relative from the
  file) and per-file module names — shared by the parser (every `[lang]`) and the runtime;
  `import_fs.rs` **the only place import processing touches the outside world** (files, env vars,
  cwd, Python's lib dirs) — the editor's wasm swaps it for host calls, so import code must not call
  `std::fs` / `std::env` directly; `arc_format.rs` the `.arc` file format reader (the embedded source,
  read by the import code in both builds)
- `cpp_header/` — reads **types** from C/C++ headers for `import[cpp-dll]`/`import[cpp-lib]`
  (`CType`/`CStructDef`/`CFnSig`, header parser, typedef loader, `cpp_config`). Split out of
  `interpreter/cpp_bridge` so the editor's wasm can use it; fs goes through `import_fs`
- `rs_crate/` — reads **types** from a Rust crate's source for `import[rs]` (`find_config`,
  `scan_all_sigs`, `make_stubs`, `load_types` = types only, no `cargo build`). Split out of
  `partial_compiler/rs_loader` for the editor's wasm
- `lexer/` — tokenizer: scan loop, keyword recognition (`lex_word()` in `keyword.rs`),
  literal/operator/symbol scanning, indentation tracking
- `parser/` — recursive-descent parser; imported-module loading happens here at parse time
  - `stmts/` — statement parsing (`core.rs` holds the `parse_stmt()` dispatch)
  - `exprs.rs` — expression precedence chain
  - `classes.rs` / `types.rs` — class/trait parsing, type-annotation parsing
  - `imports/` — `import[lang]` parsing + module resolution, **also used by the editor's wasm**
    (file access through `import_fs`; `packages.rs` the CPython package semantics: chains, binding,
    `from . import x`); `import_syntax.rs` — the `..a.b` module-path syntax
  - `cs_assembly/` — .NET DLL metadata reading (types for `import[cs-dll]`/`import[cs-proc]`
    and `--compile-cs` stub generation); pure Rust, so the editor's wasm includes it too
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
  - `cpp_bridge/` — C/C++ interop at run time: wrapper codegen, shim compile driver, the
    runtime methods of `CType`/`CStructDef` (header parsing itself is in `cpp_header/`)
  - `templates.rs` — template instantiation (`subst_stmt` / `subst_expr` clone-walk)
  - `ast_value.rs` — AST→Value reflection
  - `tests/` — interpreter integration tests, one file per topic
- `partial_compiler/` — `--compile` subsystem: `module_compiler.rs` orchestration + codegen
  eligibility; `llvm_codegen/` code generation (LLVM IR text → clang → DLL); `rs_loader/`
  `import[rs]` crate loader (builds the wrapper DLL with `cargo`; the type scanning is `rs_crate/`); `stub_gen.rs` `.ars` stub emission
- `python_converter/` — Arrow → Python source converter
- `built_in_stab/` — `.ars` stubs for built-ins. **`builtins.ars` is the single declaration of the
  built-in functions**: embedded by `type_check/builtins.rs` (return types, `range`/`len` signatures)
  and handed to the VS Code extension through the wasm (`ar_builtins`). The other `.ars` files
  there are reference only — nothing loads them
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
src/  (264 files, 115992 lines)
  ar_config.rs (301)
  arc_format.rs (123)
  ast.rs (1652)
  decl_names.rs (229)
  expr_walk.rs (200)
  import_fs.rs (307)
  interpreter.rs (1338)
  main.rs (797)
  module_path.rs (295)
  prof.rs (558)
  py_stubs.rs (50)
  repl.rs (138)
  stmt_walk.rs (287)
  syntax_cov.rs (383)
  template_subst.rs (819)
  token.rs (560)
  built_in_stab/
    basic_traits.ars (96)
    built_in_const.ars (2)
    built_in_type.ars (33)
    builtins.ars (316)
    error.ars (77)
  cpp_header/
    config.rs (355)
    mod.rs (19)
    target.rs (185)
    typedef_loader.rs (414)
    types.rs (594)
    header_parser/
      decls.rs (602)
      mod.rs (671)
      preprocess.rs (254)
      structs.rs (378)
  frontend_tests/
    lexer_tests.rs (327)
    mod.rs (6)
    parser_tests.rs (1798)
    type_check_tests/
      access.rs (258)
      annotations.rs (321)
      bridge_mutability.rs (146)
      calls.rs (530)
      comparison.rs (237)
      decorators_generics.rs (305)
      generators.rs (168)
      guards_fntype.rs (417)
      mod.rs (50)
      modules.rs (369)
      ptr_types.rs (92)
      union_types.rs (512)
      variables.rs (670)
  interpreter/
    ast_value.rs (813)
    async_mgr.rs (500)
    built_in_types.rs (339)
    cs_dll_runtime.rs (532)
    cs_dll_runtime_stub.rs (68)
    cs_proc_runtime.rs (178)
    cs_proc_runtime_stub.rs (52)
    cs_values.rs (486)
    debugger.rs (532)
    encoding.rs (159)
    event_loop.rs (273)
    exceptions.rs (150)
    ffi_boundary.rs (658)
    js_proc_runtime.rs (217)
    js_proc_runtime_stub.rs (39)
    msvc_errors.rs (112)
    native_api_stub.rs (60)
    proc_bridge.rs (203)
    proc_values.rs (250)
    py_interop.rs (505)
    py_interop_stub.rs (87)
    resolver.rs (691)
    scope.rs (313)
    str_methods.rs (649)
    templates.rs (296)
    tw_stats.rs (311)
    vm_toplevel.rs (420)
    classes/
      async_manager_methods.rs (102)
      bytes_methods.rs (37)
      class_methods.rs (162)
      freeze.rs (215)
      frozen_list_methods.rs (90)
      instantiate.rs (134)
      lookup.rs (86)
      method_call.rs (559)
      mod.rs (98)
      object_methods.rs (314)
      ptr_methods.rs (82)
      set_methods.rs (149)
      string_methods.rs (616)
    cpp_bridge/
      codegen.rs (1031)
      compiler.rs (811)
      mod.rs (30)
      types.rs (339)
    eval/
      attrs.rs (764)
      builtins.rs (1096)
      calls.rs (896)
      core.rs (597)
      mod.rs (210)
      native.rs (493)
      native_stub.rs (65)
      py_builtins.rs (1266)
      subscript.rs (376)
    exec/
      blocks.rs (125)
      control_flow.rs (77)
      definitions.rs (1018)
      dispatch.rs (252)
      exceptions_async.rs (333)
      mod.rs (313)
      modules.rs (1197)
      vars.rs (311)
    functions/
      args.rs (452)
      deepcopy.rs (142)
      execution.rs (988)
      mod.rs (7)
      overload.rs (223)
    native_api/
      callbacks.rs (1374)
      mod.rs (524)
    ops/
      display.rs (395)
      equality.rs (571)
      hash.rs (570)
      mod.rs (179)
      operators.rs (634)
      typecheck.rs (489)
      unicode_printable.rs (158)
    tests/
      alias.rs (109)
      async_tests.rs (311)
      basics.rs (575)
      callables.rs (746)
      classes.rs (816)
      collections.rs (433)
      control_flow.rs (123)
      enum_defaults.rs (238)
      events_external.rs (53)
      exceptions.rs (519)
      expressions.rs (1172)
      file_io.rs (250)
      functions.rs (192)
      hashing.rs (387)
      indexing.rs (128)
      instances.rs (427)
      iterator.rs (142)
      mod.rs (513)
      modules.rs (397)
      mustbe.rs (192)
      primitives.rs (344)
      ptr.rs (123)
      pyobject.rs (213)
      set_type.rs (324)
      unpacking.rs (601)
    value/
      callables.rs (641)
      collections.rs (335)
      core.rs (488)
      exceptions.rs (41)
      flat.rs (107)
      instance.rs (553)
      meta.rs (93)
      meta_projection.rs (568)
      mod.rs (26)
      native.rs (771)
      objects.rs (175)
      ptr.rs (1126)
  lexer/
    chars.rs (45)
    editor_tokens.rs (144)
    keyword.rs (149)
    literal.rs (334)
    math.rs (328)
    mod.rs (24)
    scan.rs (458)
    symbol.rs (279)
  meta_expand/
    mod.rs (3654)
    modules.rs (142)
    monomorph.rs (759)
    ordinary.rs (374)
  parser/
    classes.rs (982)
    editor_hooks.rs (524)
    editor_index.rs (301)
    exprs.rs (1228)
    import_syntax.rs (87)
    mod.rs (434)
    py_stub_extract.rs (901)
    types.rs (713)
    cs_assembly/
      export_gen.rs (710)
      metadata.rs (318)
      mod.rs (307)
      model.rs (284)
      parse.rs (559)
      signature.rs (373)
      stub_gen.rs (397)
      xml_docs.rs (115)
    imports/
      ar_modules.rs (341)
      cpp.rs (373)
      cs_js_modules.rs (258)
      dispatch.rs (382)
      mod.rs (98)
      packages.rs (318)
      py_modules.rs (443)
    stmts/
      assignment.rs (195)
      control_flow.rs (158)
      core.rs (489)
      definitions.rs (249)
      functions.rs (321)
      metafn.rs (364)
      mod.rs (37)
  partial_compiler/
    mod.rs (19)
    module_compiler.rs (252)
    stub_gen.rs (353)
    llvm_codegen/
      context.rs (427)
      expr.rs (1062)
      function.rs (485)
      mod.rs (1163)
      stmt.rs (433)
    rs_loader/
      codegen.rs (758)
      loader.rs (229)
      mod.rs (44)
  python_converter/
    annotations.rs (147)
    builtins_check.rs (224)
    classes.rs (747)
    decorators.rs (99)
    expressions.rs (924)
    hoist.rs (123)
    mod.rs (51)
    param_rewrite.rs (82)
    statements.rs (1446)
    supers.rs (41)
    utils.rs (42)
  rs_crate/
    mod.rs (216)
    parse.rs (934)
    stubs.rs (220)
  type_check/
    annotations.rs (263)
    binop.rs (756)
    builtins.rs (214)
    call_check.rs (1985)
    decorator.rs (154)
    diagnostics.rs (57)
    errors.rs (1186)
    infer.rs (1698)
    members.rs (317)
    mod.rs (511)
    names.rs (295)
    scope.rs (618)
    state.rs (304)
    type_utils.rs (730)
    types.rs (988)
    registry/
      builder.rs (984)
      mod.rs (473)
    stmt/
      check.rs (2238)
      mod.rs (6)
      protocol.rs (469)
      resolve.rs (867)
  vm/
    chunk.rs (337)
    disasm.rs (159)
    mod.rs (33)
    op.rs (587)
    op_prof.rs (219)
    peephole.rs (387)
    run.rs (2036)
    compiler/
      block_expr.rs (466)
      calls.rs (367)
      control.rs (284)
      decls.rs (298)
      diag.rs (144)
      emit.rs (722)
      entry.rs (681)
      expr.rs (688)
      mod.rs (575)
      stmt.rs (758)
      stmt_assign.rs (334)

impl_python/  (50 files, 16992 lines)
  __init__.py (0)
  __main__.py (92)
  ast.py (621)
  repl.py (55)
  token.py (372)
  interpreter/
    __init__.py (33)
    builtins.py (1196)
    cs_dll_runtime.py (291)
    cs_proc_runtime.py (243)
    encoding.py (73)
    env.py (139)
    exceptions.py (65)
    interpreter.py (3120)
    native_api.py (515)
    value.py (916)
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
    scan.py (194)
    symbol.py (149)
  parser/
    __init__.py (137)
    classes.py (335)
    cs_assembly.py (831)
    exprs.py (476)
    imports.py (573)
    stmts.py (634)
    types.py (308)
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
    scope.py (67)
    stmt.py (511)
    type_utils.py (82)
    types.py (335)

vscode-extension/  (8 files, 2647 lines; src/ + syntaxes/ only)
  src/
    debug_runner.ts (352)
    extension.ts (258)
    frontend.ts (193)
    vscode_mock.ts (292)
    wasm_host.ts (217)
    wasm_providers.ts (1005)
  syntaxes/
    arrow.tmLanguage.json (322)
    arrow-stub.tmLanguage.json (8)

examples/  (recursive .ar counts per category)
  apps/ (2 .ar)
  archived/ (72 .ar)
  async/ (11 .ar)
  basics/ (153 .ar)
  bench/ (26 .ar)
  classes/ (58 .ar)
  collections/ (33 .ar)
  debugger/ (6 .ar)
  exceptions/ (17 .ar)
  interop/ (197 .ar)
  practical_examples/ (8 .ar)
  repl/ (0 .ar)
  typing/ (120 .ar)
  (2 loose .ar at top level)

scripts/  (検証・計測スクリプト。何をいつ走らせるかは CLAUDE.md)
  ab_bench.ps1 (116)
  ab_bench_modes.ps1 (165)
  annot_diff.ps1 (58)
  annot_unresolved.ps1 (87)
  check_ptr_protocol.ps1 (118)
  compare_bytecode.ps1 (135)
  compare_import_paths.ps1 (147)
  compare_outputs.ps1 (153)
  compare_python_impl.ps1 (552)
  compare_wasm_frontend.ps1 (301)
  cpp_cache_isolation.ps1 (135)
  debug_session.ps1 (161)
  dump_native_ir.ps1 (92)
  force_gate.ps1 (149)
  generate-codebase-map.ps1 (115)
  hash_eq_identity.ps1 (147)
  prof_attr_ic.ps1 (104)
  prof_dist.ps1 (180)
  repl_session.ps1 (58)
  run_extension_debug.ps1 (92)
  scan_examples.ps1 (83)
  stale_doc_refs.ps1 (146)
  syntax_cov.ps1 (215)
  test_build_gate.ps1 (72)
  tw_stats.ps1 (105)
  type_obligations.ps1 (171)

implementation_logs/  (計画・実装ログ・引き継ぎ文書)
  bench_baseline.md (68)
  bug_fix.md (911)
  BUGFIX_B1_B13.md (250)
  BYTECODE_VM_PLAN.md (1017)
  comptime_metafn_design.md (973)
  EDITOR_EXTERNAL_TYPES_PLAN.md (509)
  editor_import_resolution_plan.md (293)
  enum_member_type_plan.md (224)
  eval_core_extraction_plan.md (186)
  FUTURE_FEATURE.md (539)
  IMPLEMENTATION_LOG.md (9882)
  IMPORT_RESOLUTION_PLAN.md (103)
  PHASE_R1_RESULTS.md (741)
  PHASE5_PLAN.md (427)
  python_converter_coverage.md (1087)
  PYTHON_CONVERTER_LOG.md (1204)
  PYTHON_IMPL_SYNC_PLAN.md (207)
  REFACTORING_HANDOFF.md (133)
  type_binding_enforcement_plan.md (1826)
  type_check_redesign.md (5118)
  VSCODE_LEXER_UNIFICATION_PLAN.md (439)

(repo root)
  ar_config.json (38)
  CLAUDE.md (164)
  README.md (317)
```
_Generated 2026-10-22 by generate-codebase-map.ps1_
<!-- END AUTO-TREE -->
