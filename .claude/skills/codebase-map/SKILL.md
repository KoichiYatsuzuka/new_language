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
src/  (259 files, 123228 lines)
  ar_config.rs (301)
  arc_format.rs (123)
  ast.rs (1684)
  decl_names.rs (229)
  expr_walk.rs (200)
  import_fs.rs (344)
  interpreter.rs (1334)
  main.rs (797)
  module_path.rs (295)
  prof.rs (558)
  py_stubs.rs (63)
  py_stubs_generated.rs (90)
  repl.rs (138)
  stmt_walk.rs (288)
  syntax_cov.rs (383)
  template_subst.rs (824)
  token.rs (565)
  built_in_stab/
    basic_traits.ars (96)
    built_in_const.ars (2)
    built_in_type.ars (33)
    builtins.ars (367)
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
    lexer_tests.rs (338)
    mod.rs (6)
    parser_tests.rs (1652)
    type_check_tests/
      access.rs (258)
      annotations.rs (354)
      bridge_mutability.rs (146)
      calls.rs (538)
      comparison.rs (237)
      decorators_generics.rs (305)
      generators.rs (168)
      guards_fntype.rs (417)
      mod.rs (51)
      modules.rs (366)
      ptr_types.rs (92)
      py_collections.rs (117)
      union_types.rs (512)
      variables.rs (702)
  interpreter/
    ast_value.rs (811)
    async_mgr.rs (500)
    built_in_types.rs (410)
    cs_dll_runtime.rs (532)
    cs_dll_runtime_stub.rs (68)
    cs_proc_runtime.rs (178)
    cs_proc_runtime_stub.rs (52)
    cs_values.rs (486)
    debugger.rs (532)
    event_loop.rs (273)
    exceptions.rs (307)
    ffi_boundary.rs (735)
    js_proc_runtime.rs (217)
    js_proc_runtime_stub.rs (39)
    msvc_errors.rs (112)
    native_api_stub.rs (71)
    proc_bridge.rs (203)
    proc_values.rs (249)
    py_interop.rs (2229)
    py_interop_stub.rs (178)
    resolver.rs (691)
    scope.rs (361)
    str_methods.rs (614)
    templates.rs (296)
    tw_stats.rs (311)
    vm_toplevel.rs (405)
    classes/
      async_manager_methods.rs (107)
      bytes_methods.rs (396)
      class_methods.rs (144)
      dict_methods.rs (139)
      freeze.rs (203)
      frozen_list_methods.rs (90)
      instantiate.rs (278)
      lookup.rs (86)
      method_call.rs (533)
      mod.rs (207)
      object_methods.rs (444)
      ptr_methods.rs (82)
      set_methods.rs (199)
      string_methods.rs (688)
    cpp_bridge/
      codegen.rs (1031)
      compiler.rs (811)
      mod.rs (30)
      types.rs (339)
    eval/
      attrs.rs (938)
      builtins.rs (1103)
      calls.rs (920)
      core.rs (562)
      mod.rs (215)
      native.rs (588)
      native_stub.rs (82)
      py_builtins.rs (1655)
      subscript.rs (366)
    exec/
      blocks.rs (125)
      control_flow.rs (96)
      definitions.rs (847)
      dispatch.rs (253)
      exceptions_async.rs (342)
      mod.rs (313)
      modules.rs (1220)
      vars.rs (419)
    functions/
      args.rs (306)
      deepcopy.rs (191)
      execution.rs (1101)
      mod.rs (7)
      overload.rs (223)
    native_api/
      callbacks.rs (2540)
      mod.rs (753)
    ops/
      codecs.rs (425)
      display.rs (416)
      equality.rs (621)
      format_spec.rs (400)
      hash.rs (639)
      mod.rs (179)
      operators.rs (883)
      typecheck.rs (523)
      unicode_printable.rs (158)
    tests/
      alias.rs (109)
      async_tests.rs (311)
      basics.rs (644)
      callables.rs (834)
      classes.rs (926)
      collections.rs (412)
      control_flow.rs (123)
      enum_defaults.rs (238)
      events_external.rs (53)
      exceptions.rs (605)
      expressions.rs (1212)
      file_io.rs (301)
      functions.rs (192)
      hashing.rs (387)
      indexing.rs (128)
      instances.rs (427)
      iterator.rs (178)
      mod.rs (521)
      modules.rs (254)
      mustbe.rs (192)
      primitives.rs (344)
      ptr.rs (123)
      pyobject.rs (213)
      set_type.rs (366)
      unpacking.rs (621)
    value/
      callables.rs (692)
      collections.rs (339)
      core.rs (508)
      exceptions.rs (41)
      flat.rs (107)
      instance.rs (583)
      meta.rs (93)
      meta_projection.rs (568)
      mod.rs (26)
      native.rs (777)
      objects.rs (248)
      ptr.rs (1126)
  lexer/
    chars.rs (45)
    editor_tokens.rs (152)
    keyword.rs (149)
    literal.rs (337)
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
    classes.rs (1008)
    editor_hooks.rs (526)
    editor_index.rs (369)
    exprs.rs (1466)
    import_syntax.rs (87)
    mod.rs (458)
    py_import_links.rs (224)
    py_stub_extract.rs (1480)
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
      ar_modules.rs (342)
      cpp.rs (402)
      cs_js_modules.rs (258)
      dispatch.rs (402)
      editor_kinds.rs (240)
      mod.rs (82)
      packages.rs (295)
      py_modules.rs (441)
    stmts/
      assignment.rs (195)
      control_flow.rs (158)
      core.rs (492)
      definitions.rs (256)
      functions.rs (321)
      metafn.rs (364)
      mod.rs (37)
  partial_compiler/
    mod.rs (19)
    module_compiler.rs (252)
    stub_gen.rs (353)
    llvm_codegen/
      context.rs (619)
      expr.rs (1439)
      function.rs (506)
      mod.rs (1547)
      stmt.rs (798)
    rs_loader/
      codegen.rs (758)
      loader.rs (229)
      mod.rs (44)
  rs_crate/
    mod.rs (216)
    parse.rs (934)
    stubs.rs (220)
  type_check/
    annotations.rs (299)
    binop.rs (843)
    builtins.rs (214)
    call_check.rs (2340)
    decorator.rs (154)
    diagnostics.rs (57)
    errors.rs (1240)
    infer.rs (1896)
    members.rs (317)
    mod.rs (531)
    names.rs (370)
    scope.rs (618)
    state.rs (304)
    type_utils.rs (755)
    types.rs (1098)
    registry/
      builder.rs (1014)
      mod.rs (481)
    stmt/
      check.rs (2288)
      mod.rs (6)
      protocol.rs (469)
      resolve.rs (973)
  vm/
    chunk.rs (342)
    disasm.rs (174)
    mod.rs (33)
    op.rs (613)
    op_prof.rs (224)
    peephole.rs (402)
    run.rs (2029)
    compiler/
      block_expr.rs (460)
      calls.rs (367)
      control.rs (290)
      decls.rs (362)
      diag.rs (146)
      emit.rs (722)
      entry.rs (670)
      expr.rs (701)
      mod.rs (584)
      stmt.rs (771)
      stmt_assign.rs (337)

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

vscode-extension/  (8 files, 3106 lines; src/ + syntaxes/ only)
  src/
    debug_runner.ts (352)
    extension.ts (266)
    frontend.ts (193)
    vscode_mock.ts (292)
    wasm_host.ts (220)
    wasm_providers.ts (1156)
  syntaxes/
    arrow.tmLanguage.json (619)
    arrow-stub.tmLanguage.json (8)

examples/  (recursive .ar counts per category)
  apps/ (2 .ar)
  archived/ (72 .ar)
  async/ (12 .ar)
  basics/ (168 .ar)
  bench/ (35 .ar)
  classes/ (66 .ar)
  collections/ (38 .ar)
  debugger/ (6 .ar)
  DxLib/ (0 .ar)
  exceptions/ (18 .ar)
  interop/ (317 .ar)
  practical_examples/ (9 .ar)
  repl/ (0 .ar)
  typing/ (120 .ar)
  (2 loose .ar at top level)

scripts/  (検証・計測スクリプト。何をいつ走らせるかは CLAUDE.md)
  ab_bench.ps1 (116)
  ab_bench_modes.ps1 (165)
  annot_diff.ps1 (58)
  annot_unresolved.ps1 (87)
  check_native_ir_abi.ps1 (92)
  check_ptr_protocol.ps1 (118)
  compare_bytecode.ps1 (135)
  compare_import_paths.ps1 (147)
  compare_outputs.ps1 (153)
  compare_python_impl.ps1 (642)
  compare_tm_grammar.ps1 (120)
  compare_wasm_frontend.ps1 (301)
  cpp_cache_isolation.ps1 (135)
  debug_session.ps1 (161)
  dump_native_ir.ps1 (92)
  force_gate.ps1 (149)
  gen_builtin_py_stubs.ps1 (14)
  generate-codebase-map.ps1 (115)
  hash_eq_identity.ps1 (149)
  native_slowdown.ps1 (117)
  prof_attr_ic.ps1 (104)
  prof_dist.ps1 (180)
  py_boundary_bench.ps1 (70)
  release.ps1 (229)
  repl_session.ps1 (58)
  run_extension_debug.ps1 (92)
  scan_examples.ps1 (83)
  speed_milestones.ps1 (155)
  speed_modes.ps1 (315)
  stale_doc_refs.ps1 (160)
  syntax_cov.ps1 (215)
  test_build_gate.ps1 (72)
  tw_stats.ps1 (105)
  type_obligations.ps1 (171)

implementation_logs/  (計画・実装ログ・引き継ぎ文書)
  bench_baseline.md (68)
  bug_fix.md (911)
  BUGFIX_B1_B13.md (250)
  BYTECODE_VM_PLAN.md (1021)
  comptime_metafn_design.md (973)
  EDITOR_EXTERNAL_TYPES_PLAN.md (509)
  editor_import_resolution_plan.md (293)
  enum_member_type_plan.md (224)
  eval_core_extraction_plan.md (186)
  ffi_value_exchange_plan.md (1851)
  FUTURE_FEATURE.md (558)
  IMPLEMENTATION_LOG.md (9882)
  IMPORT_RESOLUTION_PLAN.md (103)
  KNOWN_BUGS_LOG.md (285)
  known_problems.md (68)
  native_object_access_plan.md (634)
  native_slowdown_analysis.md (230)
  PHASE_R1_RESULTS.md (741)
  PHASE5_PLAN.md (427)
  python_builtins_plan.md (596)
  python_converter_coverage.md (1090)
  PYTHON_CONVERTER_LOG.md (1204)
  PYTHON_IMPL_SYNC_PLAN.md (207)
  REFACTORING_HANDOFF.md (133)
  type_binding_enforcement_plan.md (1826)
  type_check_redesign.md (5118)
  VSCODE_LEXER_UNIFICATION_PLAN.md (439)
  VSIX_update_plan.md (386)

(repo root)
  ar_config.json (38)
  CLAUDE.md (172)
  README.md (317)
```
_Generated 2026-11-08 by generate-codebase-map.ps1_
<!-- END AUTO-TREE -->
