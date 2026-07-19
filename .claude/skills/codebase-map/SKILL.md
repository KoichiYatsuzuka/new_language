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
  - `mod.rs` — entry point + pre-passes (e.g. `new_type` registration)
  - `stmt/` — statement checking (`check.rs` holds `check_stmt()`)
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
  eligibility; `llvm_codegen/` and `inkwell_codegen/` code generation; `rs_loader/`
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
src/  (178 files, 55473 lines)
  ast.rs (992)
  interpreter.rs (568)
  main.rs (494)
  repl.rs (85)
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
      bridge_mutability.rs (127)
      calls.rs (276)
      comparison.rs (110)
      decorators_generics.rs (249)
      guards_fntype.rs (353)
      mod.rs (35)
      union_types.rs (473)
      variables.rs (167)
  interpreter/
    ast_value.rs (731)
    async_mgr.rs (339)
    built_in_types.rs (412)
    cs_dll_runtime.rs (335)
    cs_proc_runtime.rs (187)
    debugger.rs (374)
    event_loop.rs (216)
    exceptions.rs (106)
    js_proc_runtime.rs (202)
    msvc_errors.rs (112)
    proc_bridge.rs (200)
    py_interop.rs (412)
    scope.rs (105)
    str_methods.rs (649)
    templates.rs (850)
    classes/
      freeze.rs (176)
      instantiate.rs (172)
      lookup.rs (93)
      method_call.rs (761)
      mod.rs (105)
      object_methods.rs (393)
      string_methods.rs (602)
    cpp_bridge/
      codegen.rs (499)
      compiler.rs (632)
      config.rs (330)
      mod.rs (32)
      typedef_loader.rs (324)
      types.rs (185)
      header_parser/
        decls.rs (417)
        mod.rs (237)
        preprocess.rs (254)
        structs.rs (319)
    eval/
      attrs.rs (465)
      builtins.rs (573)
      calls.rs (586)
      control_expr.rs (334)
      core.rs (303)
      mod.rs (194)
      native.rs (632)
      subscript.rs (340)
    exec/
      blocks.rs (136)
      control_flow.rs (220)
      definitions.rs (640)
      dispatch.rs (424)
      exceptions_async.rs (333)
      mod.rs (318)
      modules.rs (786)
      vars.rs (253)
    functions/
      args.rs (305)
      deepcopy.rs (122)
      execution.rs (393)
      mod.rs (7)
      overload.rs (225)
    native_api/
      callbacks.rs (940)
      mod.rs (476)
    ops/
      display.rs (311)
      equality.rs (102)
      mod.rs (30)
      operators.rs (457)
      typecheck.rs (274)
    tests/
      alias.rs (109)
      async_tests.rs (183)
      basics.rs (123)
      callables.rs (419)
      classes.rs (357)
      collections.rs (304)
      control_flow.rs (123)
      enum_defaults.rs (179)
      events_external.rs (57)
      exceptions.rs (328)
      expressions.rs (682)
      file_io.rs (256)
      functions.rs (192)
      indexing.rs (124)
      instances.rs (427)
      iterator.rs (133)
      mod.rs (130)
      mustbe.rs (164)
      primitives.rs (186)
      pyobject.rs (213)
      set_type.rs (324)
      unpacking.rs (601)
    value/
      callables.rs (265)
      collections.rs (198)
      core.rs (353)
      exceptions.rs (50)
      flat.rs (113)
      instance.rs (408)
      mod.rs (20)
      native.rs (311)
      objects.rs (143)
  lexer/
    chars.rs (45)
    keyword.rs (143)
    literal.rs (334)
    math.rs (359)
    mod.rs (20)
    scan.rs (426)
    symbol.rs (263)
  parser/
    classes.rs (734)
    exprs.rs (915)
    mod.rs (203)
    types.rs (623)
    cs_assembly/
      metadata.rs (302)
      mod.rs (275)
      parse.rs (264)
      signature.rs (243)
      stub_gen.rs (450)
      xml_docs.rs (119)
    imports/
      ar_modules.rs (245)
      cpp.rs (206)
      cs_js_modules.rs (187)
      dispatch.rs (190)
      mod.rs (411)
      py_modules.rs (141)
    stmts/
      assignment.rs (193)
      control_flow.rs (153)
      core.rs (327)
      definitions.rs (210)
      functions.rs (282)
      mod.rs (35)
  partial_compiler/
    mod.rs (18)
    module_compiler.rs (366)
    stub_gen.rs (338)
    inkwell_codegen/
      context.rs (209)
      expr.rs (516)
      function.rs (174)
      mod.rs (527)
      stmt.rs (351)
    llvm_codegen/
      context.rs (352)
      expr.rs (926)
      function.rs (458)
      mod.rs (761)
      stmt.rs (439)
    rs_loader/
      codegen.rs (476)
      loader.rs (323)
      mod.rs (97)
      parse.rs (644)
      stubs.rs (186)
  python_converter/
    annotations.rs (78)
    classes.rs (244)
    expressions.rs (269)
    mod.rs (29)
    statements.rs (449)
    utils.rs (49)
  type_check/
    binop.rs (154)
    call_check.rs (541)
    decorator.rs (146)
    errors.rs (482)
    infer.rs (388)
    mod.rs (769)
    scope.rs (150)
    type_utils.rs (187)
    types.rs (498)
    stmt/
      check.rs (747)
      mod.rs (6)
      protocol.rs (242)
      resolve.rs (130)

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
  async/ (3 .ar)
  basics/ (14 .ar)
  bench/ (7 .ar)
  classes/ (9 .ar)
  collections/ (7 .ar)
  DxLib/ (0 .ar)
  exceptions/ (3 .ar)
  interop/ (25 .ar)
  practical_examples/ (8 .ar)
  typing/ (11 .ar)
  (2 loose .ar at top level)

(repo root)
  ar_config.json (32)
  CLAUDE.md (70)
  generate-codebase-map.ps1 (100)
  README.md (255)
  run_examples.ps1 (50)
  spec.md (570)
```
_Generated 2026-07-19 by generate-codebase-map.ps1_
<!-- END AUTO-TREE -->
