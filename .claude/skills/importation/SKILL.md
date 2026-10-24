---
name: importation
description: Use when writing, reading, or extending Arrow (.ar) import statements — `import[lang] module.path as alias` / `from module import[lang] Name` / relative `import ..pkg.mod` — including .py, .dll/.lib (C), .rs, C#, or Node.js interop. Explains the CPython-compatible module semantics (search from the entry directory, `import a.b` binds `a`, packages and submodule attributes, re-exports, `from . import x`) shared by every language (src/module_path.rs, src/parser/imports/packages.rs), what each `[lang]` tag loads, and how src/parser/imports/ and src/interpreter/exec/modules.rs implement it.
---

# Importation of .ar, .py, .dll (C language), .lib, .rs, and more

Import syntax: `import[lang] module.path as alias` / `from module import[lang] Name`.
The `[lang]` tag selects the source type; omitting it defaults to `ar-auto`.
A module path may start with dots (relative import): `from . import x` / `from ..lib import f` / `import ..a.b`.

## Module semantics = CPython (2026-10-02, phase 4)

Plan and decisions: `implementation_logs/IMPORT_RESOLUTION_PLAN.md`. The search rule's only definition is
[src/module_path.rs](../../../src/module_path.rs) (header doc); packages / binding are
[src/parser/imports/packages.rs](../../../src/parser/imports/packages.rs).

| Written | Searched | Binds |
|---|---|---|
| `import a.b.c` | **entry file's directory** (CPython `sys.path[0]`), then the language's external paths | `a` (package) |
| `import a.b.c as m` | same | `m = a.b.c` |
| `from a.b import x` | same | `x` (a name in `a.b`, else submodule `a.b.x`) |
| `from . import x` / `from .m import y` | the importing file's directory **only** | `x` / `y` |
| `from .. import x` / `import ..a.b` | one level up **only** (each extra dot = one more level; `...` lexes as one `Ellipsis` = 3) | as above (`import ..a.b` binds `a`) |

- **No implicit relative import**: `import util` in `pkg/mid.ar` does not look in `pkg/` — write
  `from . import util` or `import pkg.util`. External paths: Python `python.search_paths` / `PYTHONPATH` /
  site-packages, C# `csharp.lib_paths`, js-proc bridge resolution; config files are looked up from the
  entry directory upward. Dotted (relative) imports never use external paths.
- ⚠ Arrow extensions over CPython: `import .x` / `import ..x` exist; relative imports also work from the
  entry file and may go above the entry directory (needed so a file in a subdirectory can be the entry).
- **Packages**: `import a.b.c` loads `a` → `a.b` → `a.b.c` in order. The parser emits **unbound
  `Stmt::Import`s (`bind: None`) in front of the statement** (`Parser::pending_stmts`, drained by
  `parse_program`), so runtime / type checker / registry / expander process package bodies like any import.
  A directory without `__init__` is an empty namespace package. A package whose `__init__` is being loaded is
  not re-loaded (CPython lets `__init__` import its own submodules).
- **Binding is decided once**: `Stmt::Import::bind: Option<ImportBind { name, module }>` (parser). Runtime,
  type checker, registry (`scope.modules`), expander (`monomorph` keys `a.b.Box`) and `decl_names` read it.
  cpp / cs / js / rs keep their old binding (alias or last part; cpp: header stem).
- **Submodule attributes**: after `a.b` loads it is an attribute of `a` wherever it was imported
  (`Interpreter::attach_submodule` → `Interpreter::submodules`, read by `namespace_member`; copied into async
  tasks). Not stored inside `NamespaceData` (parent ↔ child cycle would break `deep_clone`).
- **Re-exports**: names a module binds with imports are members (`wrapper.core`, `from wrapper import Item`).
  The type registry follows re-exports to the declaring module (`TypeRegistry::chase_reexport`).
- Static errors: `ModuleHasNoMember` / `CannotImportName` (name is no declaration / re-export / submodule —
  members come from `module_member_cache` + `module_children`, built by `TypeChecker::namespace_type`),
  `UnimportedModuleType` (a type spelled through a module this file did not bind).
- Each statement carries `origin: ImportOrigin { level, base_dir, span }`; runtime lookups (py-int
  `sys.path`, C# bridge/host, js-proc config) use `base_dir` (entry dir for absolute imports). Statements not
  parsed from a file (REPL, tests) carry `base_dir: None` and use `Interpreter::add_python_search_dir`.

## Module identity

- **Module names are per file**: the path relative to the entry file's directory (`pkg.util` = CPython's
  `__name__`; levels above it are `__parent__`, e.g. `__parent__.util`). Modules found in Python's external
  paths keep the written name. The same file under any spelling is one module; two files claiming one name is
  an explicit error (`module_path::ModuleNames`). The runtime cache, the type registry and the expander key
  on this name. `source_module` keeps the written spelling when it differs (editor stub keys use it).
- ⚠ History: phase 1 (morning of 2026-10-02) searched the importing file's directory and never the entry
  directory, and phase 2 had "no re-exports"; phase 4 replaced both with CPython semantics.

## Quick reference

| Tag | Loads |
|-----|-------|
| *(none)* | `.arc` preferred, falls back to `.ar` or `__init__.ar` |
| `ar` / `arc` | Force `.ar` source only / force `.arc` compiled only |
| `py` | Python `.py` via converter |
| `py-int` | `.pyi`→`.py` for type checking only; runtime via PyO3 |
| `rs` | Rust crate — auto-compiles a wrapper DLL (requires `ar_config.json` with `rust.crates_path`) |
| `cpp-dll` / `cpp-lib` | C header (`Dir.Name` → `Dir/Name.h`) for type stubs; runtime via `cpp_bridge` |
| `js-proc` | Node.js subprocess via named-pipe NDJSON-RPC (see below) |
| `cs-proc` | .NET assembly via IPC (ECMA-335 metadata reader) |

For parser internals in general (not import-specific), see the `parser-internals` skill. For interpreter internals in general, see `interpreter-internals`.

---

# Importation — Implementation Reference

This section describes how module importation works in the Arrow Rust implementation, derived directly from the source code.

---

## Overview

Import statements are processed in **two phases**:

1. **Parse time** — `src/parser/imports/` resolves the module file (`dispatch.rs` → per-language loaders), parses its source, and embeds the resulting AST into the `Stmt::Import` or `Stmt::FromImport` node as `body`. The module-path syntax (`..a.b`) is `src/parser/import_syntax.rs`. ⚠ The VS Code extension runs **the same code** inside its wasm (editor_import_resolution_plan.md 3-2): file access goes through `src/import_fs.rs` (the wasm version asks the host, `vscode-extension/src/wasm_host.ts`), paths from config text through `import_fs::path_from_text`, and runtime-only work (`import[rs]`'s `cargo build`, registering a `.arc`'s DLL, writing `.ars`) is `#[cfg(not(feature = "editor"))]`. In the editor an import that fails does not stop the parse: `Parser::try_import` records it as a diagnostic at the position where the CLI stops.
2. **Runtime** — `src/interpreter/exec/modules.rs` executes the `body` in the module's own globals and collects its declared names (minus import-only names) as a `NamespaceData` object, then binds it to the module variable.

Type checking (`src/type_check/stmt/check.rs` / `resolve.rs`) reads the `body` AST directly to collect member types without any additional file I/O.

---

## Syntax and Language Tags

```ar
import module.path                    # ar-auto (default)
import[ar]  module.path              # force .ar source
import[arc] module.path              # force .arc compiled
import[py]  module.path              # Python source
import[py-int] module.path           # Python type stubs (runtime via PyO3)
import[rs]  crate_name               # Rust crate
import[rs]  crate_name[0.2]          # Rust crate, specific version
import ..module.path                 # relative: one level up from this file
import[py] .pkg.mod                  # relative works for every tag except rs

from module import[lang] Name1, Name2 as N2
from ..module import Name
```

`parse_lang_bracket()` (imports.rs:267) reads the `[lang]` bracket and assembles hyphenated identifiers (e.g. `py-int`). If absent, the default is `"ar-auto"`.

`parse_version_bracket()` (imports.rs:359) reads `[X.Y.Z]` — only valid for `import[rs]`.

---

## AST Nodes

```
Stmt::Import {
    lang:          String,          // "ar-auto" | "ar" | "arc" | "py" | "py-int" | "rs" | "cpp-dll" | ...
    module:        Vec<String>,     // Arrow/py: per-file module name (`pkg.util`); others: written path;
                                    // cpp: [resolved header path]
    source_module: Option<String>,  // written spelling when it differs (`..util`, cpp `A.B`)
    alias:         Option<String>,  // as alias
    body:          Vec<Stmt>,       // parsed module AST, embedded at parse time
    origin:        ImportOrigin,    // { level, base_dir, span } — search base used at runtime too
}

Stmt::FromImport {
    lang, module, source_module,
    names:         Vec<(String, Option<String>)>, // [(original_name, as_alias)]
    body, origin,
}
```

`body` is populated during parsing by `load_module()`. By the time the interpreter sees the node, the module has already been loaded and parsed — no further I/O is needed at runtime for `.ar` / `.arc` / `.py` modules.

---

## Module Loading Dispatch (`load_module`, `src/parser/imports/dispatch.rs`)

`load_module(lang, level, module, version)` returns a `LoadedModule { body, name }`; `name` becomes
`Stmt::Import::module`.

```
lang         → loader
──────────────────────────────────────────────
"ar-auto"    → load_tl_module        prefer .arc, fallback to .ar
"tl-auto"    → load_tl_module        (旧名の別名)
"tl"         → load_tl_source_module force .ar, skip .arc
"ar"         → load_tl_source_module (alias)
"tlc"        → load_tlc_module       force .arc, error if absent
"arc"        → load_tlc_module       (alias)
"py"         → load_python_module    Python source → converter → AST
"py-int"     → load_python_interface_module  .pyi → .py → empty on miss
"rs"         → load_rs_module        compile Rust crate → stub AST
"cpp-dll"    → parse_cpp_import      parse C header → stub AST
"cpp-lib"    → parse_cpp_import      (alias)
"js-proc"    → load_js_module        .ars stub (optional); runtime = Node.js IPC
"cs-proc"    → load_cs_proc_module   ECMA-335 DLL → type stubs; runtime = .NET IPC
```

---

## `.ar` / `.arc` Module Resolution (`load_tl_module`, `src/parser/imports/ar_modules.rs`)

Search directory: **the search base only** (`module_search_dirs(level)` = `module_path::search_base(source_dir, level)`).

Candidates are tried in this order:

```
1. {dir}/{module_path}.arc    ← compiled module (preferred)
2. {dir}/{module_path}.ar     ← plain source
3. {dir}/{module_path}/__init__.ar  ← package
```

The first candidate that `exists()` wins.

**If `.arc`**: calls `partial_compiler::load_tlc()` to extract the embedded source text (and cache native bytes in a thread-local if v1/v2). The filename label becomes `<compiled:ModuleName>`.

**If `.ar`**: reads the file directly with `fs::read_to_string`.

Either way, the source is tokenized and a new `Parser` is created with:
- `source_dir` = directory of the resolved file (so its own imports start there)
- `module_cache`, `loading`, `root_dir`, `node_counter` and `module_names` shared with / cloned from the parent
- `pending_stmts` is per parser (a module's own package chains go in front of its own statements)

After parsing, the child's `module_cache` is merged back into the parent.

**Paths**: the found path is split into a display path (lexically normalized, relative — used in messages and
as the child's `source_dir`) and a key (absolute + normalized, `found_paths`). ⚠ Keys must be normalized:
`pkg/../a.ar` vs `a.ar` would otherwise parse twice, and a `..` cycle would evade circular-import detection.

**Circular import detection**: `self.loading` is a `HashSet<PathBuf>` of keys. Before parsing a module, its key is inserted; it is removed after parsing completes. If a key is already in `loading`, an error is returned immediately.

**Cache key**: `("ar-auto", key)` for ar-auto, `("ar", key)` for forced source, `("arc", key)` for forced compiled.

**Module name**: `name_module_file(key, written, local=true)` (see "Module identity" above).

`load_tl_source_module` and `load_tlc_module` are identical to `load_tl_module` but skip `.arc` or skip `.ar` respectively.

---

## Python Modules (`load_python_module`, `src/parser/imports/py_modules.rs`)

- Search (`python_import_dirs(from_dir, level)`): no dots → the **entry directory** then
  `python_search_dirs_from(entry)`'s external dirs (`python.search_paths` from the nearest ancestor
  `ar_config.json`, `PYTHONPATH`, site-packages, stdlib) — CPython's `sys.path`; dots → counted from
  `from_dir` only. Candidates `{module_path}.py` / `{module_path}/__init__.py`; a directory without
  `__init__.py` is a namespace package.
- `from_dir` (relative imports only) is the `.ar` file's directory for `import[py]`, and **the `.py` file's
  directory** for imports inside a Python module (`fill_python_imports`, which also inserts package chains /
  submodule imports in front of each statement). A module found in the entry directory or a relative base is
  named relative to the entry directory; one found in an external dir keeps the written name.
- Converts Python source via `python_converter::convert_python_source()`
- ⚠ Placement rules (task 10-16): an `import` inside a function or a module-level block
  (`if` / `try` …) and a `class` inside a module-level block are **conversion errors**
  (`reject_nested_import` / `reject_class_in_module_block`). A `class` inside a function is
  converted and runs on the VM (`Op::ClassDef` → `Interpreter::vm_class_def`). Arrow code may not
  use a Python function that returns a class it defines (`TypeErrorKind::PyClassFactoryFromArrow`);
  a decorated Python class has an open member set (`members_unresolved`).
- ⚠ Nested classes (task 10-18): `convert_class` returns a statement list — a class defined in a
  class body is hoisted in front of its outer class under its `__qualname__` (`Outer.Inner`), and the
  outer class gets a class variable `Inner` (`static mut`, typed `type[Outer.Inner]`). `Outer.Inner()` /
  `self.Inner()` reach it through the method-call fallback `Interpreter::class_var_class`. Any class-body
  statement other than methods, single-name class attributes, nested classes, docstrings, `...` and
  `pass` is a conversion error (it used to be dropped silently).
- ⚠ Unsupported builtins (python_builtins_plan.md 1-1): after a successful conversion,
  `check_unsupported_builtins` (`python_converter/builtins_check.rs`) folds the Python AST once and
  rejects the module if it **reads** a CPython 3.12 builtin that the Arrow runtime does not resolve
  (`is_runtime_builtin_name` is false — the list shrinks automatically as builtins are added). Names
  bound anywhere in the module, decorators, class bases and the `super` of `super().m()` are skipped;
  a module with `from m import *` is not checked. Structural conversion errors are reported first.
- Reference sharing (known_bugs.md #1, decision A): every implicit copy on store goes through
  `Interpreter::copy_for_store`, which does not copy while `running_python` is set. `vm::run` / `vm::resume_frame`
  set it from `Chunk::is_python` (module-statement chunks of a Python module get it in `try_run_module_stmt`) and
  `exec_module` sets it for a Python module body. `ClassValue::is_python` (set by `exec_class_def` from
  `in_python_module`) makes `values_eq` / `hash_into` use identity for instances without `__eq__` / `__hash__`.
  Arrow `let` (`frozen_copy`), `self` binding, native calls and `copy()` still copy.
- `del` (known_bugs.md #2 / #3; `convert_del_target` in `statements.rs`): `del o.a` → `__py_delattr(o, "a")`
  (`Interpreter::del_attr_val`: the class's `__delattr__`, then a data descriptor's `__delete__`, then
  `InstanceData::clear_field` puts the field back to unassigned — the boxed slot becomes `None`, a raw class clears its
  init-bitmap bit); `del d[k]` → `__py_delitem(d, k)` (`Interpreter::eval_delitem`: dict / list / slice /
  `__delitem__`); `del x` still warns and is ignored; tuple / list targets are flattened left to right. Class and
  module attributes cannot be deleted (runtime `TypeError`). A `del` in a class body is still a conversion error.
- `__setattr__` / `__delattr__` (known_bugs.md #10): `attr_assign_evaled` / `set_attr_val` / `del_attr_val` call a Python
  class's hook first (`Interpreter::py_attr_hook` — `ClassValue::is_python` only). `object.__setattr__(o, k, v)` /
  `object.__delattr__(o, k)` and `super().__setattr__` / `__delattr__` in an object-based class convert to
  `__py_object_setattr` / `__py_object_delattr` (`object_method_builtin` in `expressions.rs`), which store past the hook
  (`attr_assign_without_hook` / `del_attr_val(.., false)`); with a Python base, `Base.__setattr__(self, ..)` falls back to
  them in `eval_class_method` when the base has no hook. A literal name in `object.__setattr__(self, "x", v)` declares
  field `x` (`bypass_setattr_name` in `classes.rs`). The checker lets an assignment target on a hooked class name
  anything (`attr_is_assign_target`). `cached_property` stores through `__py_object_setattr`. The Arrow parser rejects
  the hooks in Arrow classes (`reject_attr_hook`).
- Static / class methods through an instance (known_bugs.md #5): for a Python class (`ClassValue::is_python`)
  `call_instance_method_evaled` sends `obj.f(..)` of a `static fn` / `class_method fn` to `eval_class_method` with the
  instance's class (static: no `self`; class method: `cls = type(obj)`), and `get_attr_without_hook` returns the plain
  function / a `BoundTarget::ClassMethod` bound to the class. The checker skips `StaticMethodOnInstance` for
  `TypeRegistry::is_python_class` (classes collected from an `import[py]` body). Arrow classes keep the class-only rule.
- `__getattr__` (known_bugs.md #11): `get_attr_val` calls `get_attr_without_hook` and, when that fails with an
  `AttributeError` (`Interpreter::is_attribute_error` — a message, or a raised exception in `current_exception`, which
  it clears), calls a Python class's `__getattr__(name)`. A method call on an unknown name reads through it
  (`call_instance_method_evaled`). `object.__getattribute__(o, k)` / `super().__getattribute__(k)` convert to
  `__py_object_getattribute` (`get_attr_without_hook`). The checker lets an instance of a hooked class read or call any
  name (`getattr_hook_type` — the hook's return annotation, else `Any`). `getattr(o, k, d)` / `hasattr` use the same
  `is_attribute_error`, so an `AttributeError` raised by a property getter or the hook is caught.
- `object` (python_builtins_plan.md 6-10): a call `object(..)` is converted to the internal builtin
  `__py_object(..)` (Arrow source may not call `object()`; calling the class value is a runtime error).
  `super().__init__(..)` in a class without a base (or with base `object`) becomes
  `__py_object_init(self, ..)` (CPython's `TypeError` on extra arguments); any other `super().m()` there is a
  conversion error. `super()` passes `*args` / `**kwargs` through like an ordinary call. `super(C, x).m(..)` (6-2) is
  the same rewrite with receiver `x` when `C` is the class being converted (`current_super_class`); any other `C` is
  a conversion error (`super_form` in `expressions.rs`; `builtins_check` skips the same shape).
- `f = staticmethod(f)` / `g = classmethod(f)` in a class body (6-13, `wrapped_method` in
  `python_converter/classes.rs`) mark an earlier method static / class (a different name adds a copy). Wrapping
  anything other than an earlier method of the class is a conversion error; `builtins_check` skips exactly this form.
- `except m.Err:` is converted to a dotted name, `except (A, B) as e:` to one handler per type
  (same body); a nested tuple is a conversion error (4-3 / 4-4).
- ⚠ Properties (python_builtins_plan.md 6-1, `python_converter/classes.rs`): a method with a
  decorator `@property` / `@x.setter` / `@x.getter` / `@x.deleter` / `@Base.x.getter` is
  renamed (`__property_get_x` / `__property_set_x` / `__property_del_x`), `x` becomes a placeholder
  `static mut x: Any = None`, and statements **after the class** build the built-in `property`
  (`C.x = property(C.__property_get_x)`, `C.x.__set_name__(C, "x")`, `C.x = C.x.setter(..)`). Reads and
  writes then go through the descriptor protocol (1-12). `self.x = v` does not make `x` a field; fields
  are also collected from `self.attr = ..` in methods other than `__init__` (not for class-attribute or
  method names). Decorators inside the property decorator apply to the method; decorators outside it
  wrap the built property (`C.x = logged(C.x)`, 6-6). Conversion errors: two property decorators,
  `@staticmethod` / `@classmethod` / `@abstractmethod` outside one, `@x.setter` on a method with another
  name or before `@property def x`, a static/class-method property, `@cached_property`.
- Class-body names (6-3, `classes.rs` + `class_scope.rs`): a class attribute whose initializer reads a name
  bound earlier in the class body (method / class attribute / nested class) gets a `None` placeholder and
  `C.x = <expr with those names rewritten to C.name>` after the class (`ClassNameRewriter`; lambda bodies and
  comprehension elements are not rewritten, as in Python). `alias = method` / `__repr__ = __str__` define the
  same method again under the new name; `name = lambda self: ..` becomes a method.
- Cache key: `("py", key)`

### Python Interface (`load_python_interface_module`, `src/parser/imports/py_modules.rs`)

Used by `import[py-int]`. The body is for type-checking only; the runtime uses PyO3.

Typing rule (ffi_value_exchange_plan.md 7-27 / 7-30, the user's decision): only annotated members, `.pyi` stubs and the
constructor's result get types; everything else is `Any`. Top-level functions are `Stmt::FnDef` stubs with parsed
parameters (`py_params`; a `float` parameter accepts `int`, `py_param_type`; `*args` / `**kwargs` → arity unchecked).
Type names the file does not declare as a class become `Any` (`py_unknown_types_to_any`), and the Python boundary
checker coerces an `int` result to a declared `float` (`ffi_boundary::coerce_numbers`, PEP 484). `parser/py_stub_extract.rs` turns a top-level `class` into a
`Stmt::ClassDef` stub (`py_class_stub`): methods with parsed parameters, fields from class-level `name: T` / `name = …` and
`self.name …` in methods, `@property` as a field; a base in the same file is merged, a foreign base or a decorator makes
the member set open (decorator marker `py-open-members:`). `.pyi` files use the same class stubs (the converter's classes
are dropped: `mut self`, lost bare annotations). Runtime `is` / `mustbe pc.Box` on a `Value::PyObject` compares the
class MRO names (and `__module__` when written through a module), `py_interop::py_class_mro`.

Search order: the same `python_import_dirs` as `import[py]` (no dots: the entry directory, `python.search_paths`,
`PYTHONPATH`, `$PYTHONHOME/Lib/site-packages`, stdlib/purelib; dots: the search base only).

For each directory, tries `.pyi` first, then `.py`. If nothing is found: a directory without `__init__` is a
namespace package (empty body, resolved); otherwise the bundled stub (`py_stubs`, only for non-relative imports);
otherwise **an error** (`cannot find its types` — put a `.pyi` next to the importing file). ⚠ Unresolved imports
are errors for every language since 2026-10-08 (`implementation_plans/editor_import_resolution_plan.md` 1-2): they
used to fall back to an empty body (no type checking) for `py-int` / `cpp-*` / `cs-*` / `js-proc`.
At runtime `py_interop::load_py_int_module` inserts `Interpreter::import_search_dirs(origin)` (the search base,
plus `python.search_paths` for non-relative imports) at the front of `sys.path`.

Parsing errors in `.pyi` / `.py` files are silently ignored (best-effort via `unwrap_or_default()`).

**Value conversion** (`src/interpreter/py_interop.rs` — `ar_to_py` / `py_to_tl`, ffi_value_exchange_plan.md 2-6):
set / frozenset ↔ Arrow set (a set with an element Arrow cannot hold stays a `PyObject`, like dict). Only `float`
instances become `float`, and a Python `int` that does not fit in i64 stays a `PyObject` — ⚠ both used to be converted
to `float` silently (`2**70`, `Decimal("0.1")`). Values Python cannot take (Arrow instances) are `TypeError` naming the
class (it used to say `'unknown'`).
A `.py` file's signatures come from the line-based extractor `parser/py_stub_extract.rs` (top-level `def` / `class`
only). An unannotated function returns `Any` — nothing is inferred from its body (ffi_value_exchange_plan.md 7-27 /
7-30), so a body on the `def` line (`def f(): return 1`) needs no special case (python_builtins_plan.md 1-19).

---

## Rust Crate Import (`import[rs]`)

### Syntax

```ar
import[rs] libm           # latest version in registry
import[rs] libm[0.2]      # specific version
import[rs] sha2           # RustCrypto hash crate (digest pattern auto-detected)
```

### Entry point: `load_rs_module` (imports.rs:333)

1. Cache-checks by `("rs", module_name)`.
2. Calls `partial_compiler::rs_loader::load(module_name, search_dirs, version)`.
3. Caches the returned stubs.

### `rs_loader::load` pipeline (rs_loader.rs:98)

#### Step 1 — Find crate source (`find_config`, rs_loader.rs:202)

Looks for `ar_config.json` in the entry file's directory and its ancestors (then the CWD). ⚠ It used to look in `source_dir` and `root_dir` only. Reads the `rust.crates_path` key, which may be a single string or an array of strings.

```json
{
  "rust": {
    "crates_path": "/path/to/cargo/registry/src/index.crates.io-..."
  }
}
```

Within `crates_path`:
- If a subdirectory matches `{crate_name}-*`, picks the latest by directory name (or the first that matches the requested version string).
- If a directory named exactly `{crate_name}` exists and no versioned directories are found, uses it as a local path.

Returns a `CrateSource::LocalPath` pointing to the resolved crate directory.

#### Step 2 — Prepare wrapper project (`prepare_wrapper`, rs_loader.rs:335)

Creates a temporary Cargo project in `$TMPDIR/ar_rs_{stem}/`:

```
ar_rs_{stem}/
├── Cargo.toml   — package + [lib] crate-type=["cdylib"] + dependency on target crate
└── src/
    └── lib.rs   — placeholder (overwritten later)
```

Runs `cargo metadata` to resolve the actual crate source directory via the resolved `manifest_path`.

#### Step 3 — Scan signatures (`scan_all_sigs`, rs_loader.rs:521)

Walks all `.rs` files under the crate's `src/` directory recursively.

Source: `src/rs_crate/parse.rs` (shared with the VS Code wasm). Signatures may span several lines
(`signature_text`; rustfmt-wrapped functions used to vanish).

**Free functions** (`parse_fn_sigs` / `classify_fn_sig`) — a top-level `pub fn` whose parameter and return types are
accepted (below) is callable. ⚠ Every other one (generic, `Vec<String>`, `&[i64]`, `&mut T`, tuples …) is returned as
`RsUnsupported` with a reason, put into the stub with a mark, and using it is a static `FfiUnsupported` error
("Rust function"). Before ffi_value_exchange_plan.md 2-5 they disappeared silently.

**Structs** (`parse_struct_sigs`) — every non-generic `pub struct Name {`, including ones without `pub` fields.
Construction: `pub fn new(..) -> Self` if present (`classify_method` returns `MethodKind::New`), else a struct
literal when every field is `pub` and passable; otherwise `ctor_unsupported` marks `__init__` (constructing is a static
error, a value returned by a method is still usable). ⚠ Before 2-5 `new` was never found (associated functions were
dropped), so a struct with private fields made the whole import fail at the wrapper build.

**Methods** (`collect_impl_items` / `classify_method`) — `&self` / `&mut self` receivers. `-> Self` returns the struct.
Associated functions other than `new`, methods taking `self` by value, generic methods and unsupported types are marked
("Rust method").

**Types by position** (`is_param_type` / `is_return_type` / `is_field_type`):
```
integers (i8..i128, u8..u128, isize, usize), f32, f64, bool, String   — everywhere
&str  &String  &[u8]  &[num]  &[String]  &[&str]                         — parameters only (&str also as a return)
Vec<u8>  Vec<num>  Vec<String>  Option<scalar>                           — parameters and return values
[u8; N]  Vec<bool>                                                       — return values only
```
(5-4 · D-16) bytes cross as `bytes` (`_bytes_arg` / `make_bytes`; they used to be a hex `str`). A numeric `Vec<T>` return
becomes an owned `Ptr[T]` (`_ptr_from_vec`: boxed slice + `_release_vec` through `protocol_ret`); a numeric slice / Vec
parameter takes a `Ptr[T]` of that element type (`protocol_arg`) or a list checked element by element (`_num_vec_arg`).
`Vec<String>` / `Vec<bool>` are expanded to lists. The wrapper's `ArCallbacks` mirror now reaches field 49
(`callback_mirror_keeps_the_positions` guards the positions). Stub types: `rust_type_to_ar` (returns / fields) and
`rust_param_type_to_ar` (parameters: `Union[Ptr[int32], list[int]]`).
Integers are range-checked both ways (`try_from` in the wrapper → OverflowError; they used to wrap around with `as`).

**Re-export whitelist** (`collect_reexports`, rs_loader.rs:411):
If `lib.rs` defines `pub fn` or `pub struct` directly, or uses `pub use ..::*` / `pub use ..::Name`, only those names are exposed. This prevents pulling in internal helpers.

**RustCrypto Digest pattern** (`collect_digest_fns`, rs_loader.rs:684):
If `lib.rs` re-exports `digest::Digest`, synthesises one-shot hash functions for each `pub type Alias = ...` in the crate. Function name is the snake_case of the alias (e.g. `Sha256` → `sha256`), signature is `(input: str) -> str` returning lowercase hex.

#### Step 4 — Generate and compile wrapper (`lib_rs`, rs_loader.rs:1293)

Writes an auto-generated `lib.rs` to the temp project. The generated code:

- Declares a `ArCallbacks` struct and a `static mut CB` pointer (set via `ar_init()`).
- For each **free function**: exports `{fn_name}_tl(args: *const i64, n: i32) -> i64`. Decodes handles to Rust types, calls the real function, encodes the return value back to a handle.
- For each **struct**:
  - One static `OnceLock<Mutex<HashMap<i64, StructName>>>` arena + atomic counter.
  - `{StructName}____init___tl` — constructs the struct (or **adopts** a value a method returned, waiting in the
    thread-local `ADOPT_{NAME}` slot), stores it in the arena keyed by a fresh integer, writes the key back into the HV
    instance as `__rs_handle__`, and copies the fields into the instance (`_sync_fields_{name}`).
  - `{StructName}__drop_tl` — removes the key from the arena.
  - `{StructName}__get_{field}_tl` / `{StructName}__set_{field}_tl` — field access. ⚠ `obj.field = v` from Arrow is
    routed to the setter (`attr_assign_evaled`), so the Rust value changes too (2-5; it used to change only the copy).
  - `{StructName}__{method}_tl` — method dispatch via arena lookup. `&mut self` methods copy the fields back
    afterwards (2-5; `obj.field` used to keep the value from construction time). A returned struct is put into the
    adoption slot and the class is called (⚠ it used to be rebuilt from its fields through the constructor: wrong with
    `new()`, a build error with private fields).

Runs `cargo build --release` on the temp project. On success, reads the resulting DLL/SO bytes and deletes the temp directory.

#### Step 5 — Cache and return stubs

Calls `cache_native(module_name, exports, dll_bytes)` to store the DLL bytes in a thread-local `NATIVE_CACHE` (in `module_compiler.rs`), keyed by module name.

Returns `Vec<Stmt>` stubs (`make_stubs`, rs_loader.rs:1035):
- `Stmt::FnDef` for each free function — empty body, HV types, `is_abstract: true`
- `Stmt::ClassDef` for each struct — with field stubs, `__init__`, `drop`, getter/setter, and method stubs
- Unsupported functions / methods / constructors carry the `FFI_UNSUPPORTED_MARK` docstring as their body (2-5)

These stubs are embedded in `Stmt::Import.body` and used by the type checker and runtime.

### Type mapping (Rust → Arrow)

| Rust type | Arrow type |
|-----------|---------------|
| `i*`, `u*`, `isize`, `usize` | `int` |
| `f32`, `f64` | `float` |
| `bool` | `bool` |
| `String`, `&str`, `&String` | `str` |
| `&[u8]`, `Vec<u8>`, `[u8; N]` | `bytes` (5-4; a hex `str` before) |
| numeric `Vec<T>` (return) / `&[T]`, `Vec<T>` (param) | `Ptr[T]` / `Union[Ptr[T], list[int or float]]` |
| `Vec<String>`, `Vec<bool>` | `list[str]`, `list[bool]` |
| `Option<T>` | `Option[T]` |

---

## Runtime Execution (`exec_module`, `src/interpreter/exec/modules.rs`)

All import variants go through `exec_module(lang, module, body, origin)`. Cache key: `(lang, PathBuf from module segments)` —
`module` is the per-file name the parser assigned, so two different files never share an entry.

States:
- `ModuleState::Loading` — set before execution to catch circular imports at runtime
- `ModuleState::Loaded(NamespaceData)` — cached after first execution

**For `.ar` / `.arc` / `py` imports**: runs the body AST in the module's own globals, collects the declared top-level names as `NamespaceData.members`. Names bound by the module's own imports are members too (re-exports, CPython). After loading, `attach_submodule` makes the module an attribute of its parent package; binding follows `Stmt::Import::bind` (a package `a` for `import a.b`).

**Runtime search** (`py-int`'s `sys.path`, the C# bridge `{Name}_native.dll` / host `{Name}_proc.exe`): `Interpreter::import_search_dirs(origin)` = the statement's search base, plus `python.search_paths` (nearest ancestor `ar_config.json`) for non-relative imports. js-proc looks for `ar_config.json` from the search base upward, and passes a JS file found at the search base to the bridge as an absolute path. ⚠ These used to search the entry file's directory (and the CWD for C#).

**For `rs` and `arc` imports** (v1 is the only native `.arc` version): calls `take_native_bytes()` to dequeue the DLL bytes cached by the parser, writes them to a temp file, loads via `libloading::Library::new()`. Then for each `FnDef` in the body, looks up the symbol `{fn_name}_tl` in the loaded library and replaces the tree-walk `Value::Function` with a `Value::NativeFnRef`. For each `ClassDef`, registers methods via `register_native_method()`.

**For `py-int` imports**: calls `py_interop::load_py_int_module()` which uses PyO3 to import the Python module directly and wraps all non-private attributes as `Value::PyObject`.

---

## `Stmt::Import` vs `Stmt::FromImport` at Runtime

**`Stmt::Import`**: binds the entire `NamespaceData` as a namespace value. Variable name = `alias` if present, otherwise the last segment of the module path.

**`Stmt::FromImport`**: calls `exec_module` the same way, then for each `(orig_name, alias)` looks up `orig_name` in the namespace members and binds to `alias` (or `orig_name` if no alias).

---

## C/C++ Imports (`import[cpp-dll]`, `import[cpp-lib]`)

Not covered by `rs_loader`. Parsed by `parse_cpp_import` (imports.rs:91):

1. Resolves dotted identifier to a header file path from the search base: `DxLib.DxLib` → `{source_dir}/DxLib/DxLib.h`, `..DxLib.DxLib` → `{source_dir}/../DxLib/DxLib.h`
2. Reads the header and calls `cpp_bridge::parse_header_full()` to extract C function signatures and struct definitions.
   Typedefs (`TCHAR` → `char`) and struct definitions (`RECT`) come from `cpp.system_headers` in `ar_config.json`
   (`cpp_header::load_system_types`); `cpp_header::add_system_structs` adds the system structs the header refers to
   (7-13). For `cpp-lib` with `precompile_macros`, the local headers it `#include`s are read too. **The stub and the
   runtime (`load_cpp_module`) both call `cpp_header::collect_header_decls`**, so they see the same functions (7-28).
   A function the C++ shim fails to compile is registered as an `unavailable` native function: calling it raises a
   `RuntimeError` with the reason (`NativeFnRef::missing_symbol_error`).
3. Generates `Stmt::FnDef` and `Stmt::ClassDef` stubs for the type checker. A parameter whose C default
   (`DEFAULTPARAM( = x )` / `= x`) could be read gets it as the stub default (`CFnSig::defaults`); at runtime an
   omitted argument gets the same value (`NativeFnRef::omitted_arg`). An unreadable default (a macro name) is required.

At runtime, `exec.rs` dispatches to the C/C++ bridge for actual calls (not via `exec_module`).

**Functions Arrow cannot call correctly yet** (C strings / pointers returned, `void*`, callbacks, output
buffers, `wchar_t*`, structs without an exact layout …) are marked in the stub and using them is a static
error (`TypeErrorKind::FfiUnsupported`); they are not generated in the wrapper either. One judgement,
`CFnSig::unsupported_reason` in `src/cpp_header/types.rs` (ffi_value_exchange_plan.md 1-2; see `c-abi-interop`).
⚠ Parameter macros (`DEFAULTPARAM( = x )`, SAL annotations) are stripped before the parameters are read
(`strip_parameter_macros`), but a function-pointer declarator (`void (* cb )( void )`: a group starting with `*` / `&`,
followed by `(` or `[`) is kept, so the parameter becomes `CType::FnPtr` named `cb` (7-29). It used to be stripped
as a macro call and the whole function was dropped silently (`unknown C type '( void'`).

The cpp-lib shim is compiled with retries: names quoted in the MSVC errors (`msvc_errors::extract_bad_fn_names`) that
are functions in the shim (`failed_fn_names`) are removed and the shim is rebuilt; an error naming no shim function
fails at once. ⚠ MSVC also quotes type names and namespaces (`HWND`, `DxLib`) — they used to be logged as removed
functions (7-29).

**Generated files** (`cpp_bridge/compiler.rs`):
- `cpp-dll`: the Rust wrapper is rebuilt into `%TEMP%` on every run and loads `{stem}.dll` next to the header.
- `cpp-lib`: the wrapper `ar_{stem}.dll`, the MSVC shim `ar_{stem}_shim.dll` and `ar_{stem}.syms` are written
  **next to the header and reused** (they are committed for `test_modules/vec_math`). The first line of `.syms`
  is a stamp (`cache_stamp`: FNV-1a of the generated shim + wrapper sources and the build config). A stale
  stamp — generator, header or config changed — rebuilds them (needs MSVC). The wrapper loads the shim
  **from its own directory**, so the files work wherever they are copied.
  ⚠ The stamp does not cover the implementation library (`*_x64.lib`): after rebuilding only the library,
  delete the `ar_*` files. (ffi_value_exchange_plan.md 1-5; before it, a stale cache was used forever and
  the wrapper embedded the shim's absolute path.)

For the C ABI value/struct-passing design behind this bridge (raw layout, zero-copy vs. shadow conversion, write-back), see the `c-abi-interop` skill.

---

## C# Imports (`import[cs-dll]`, `import[cs-proc]`)

Stubs come only from the managed assembly's ECMA-335 metadata (`src/parser/cs_assembly/` — `load_cs_module` in
`cs_js_modules.rs`); there is no `.ars` route (`--compile-cs` writes one for reading only). Both bridges share
the value rules in `src/interpreter/cs_values.rs` (ffi_value_exchange_plan.md 2-1; user doc: "C# の値の対応" in
`docs/grammar/09_imports.md`):
- **Nullable annotations** are read (`NullableAttribute` / `NullableContextAttribute`, `parse.rs::read_nullable`):
  `string?` / `Person?` → `Option[str]` / `Option[Person]`. The metadata signature alone cannot tell `string` from
  `string?`. A null for a non-`?` reference type is a `TypeError` at the boundary (also for assemblies without
  nullable annotations).
- **Unsupported signatures** (collections, tuples, delegates, type parameters, other assemblies' types, interfaces)
  are marked in the stub (`stub_gen.rs::cs_unsupported_reason`) and using them is `TypeErrorKind::FfiUnsupported`
  (`call_check.rs` via `TypeRegistry::ffi_unsupported_method`). ⚠ The mark is a docstring at the start of the method
  body, not a decorator as for C/C++: C# stubs are executed as classes and `exec_class_def` applies method decorators.
  `type_check::ffi_unsupported_reason` reads both forms.
- **Return values** are read by the declared type: a class handle becomes `Value::CsObject` (classes are looked up
  per bridge in `cs_values::register_classes`, called by `import_cs_dll` / `import_cs_proc`).
- **cs-dll lays out argument slots from the declared parameter types** (`cs_dll_runtime::encode_args`; `None` for
  `Option[str]` = `(0, -1)`), at most 8 slots. Constructors use the `__init__` overload with that many parameters
  (`cs_values::ctor_types`; the stub has one `__init__` per public constructor since 5-1 — the type checker ignores
  foreign constructors' arguments).
- **The NativeAOT exports are generated** (task 5-1): `arrow --gen-cs-exports X.dll -o ArrowExports.g.cs`
  (`cs_assembly/export_gen.rs`, CLI in `main.rs`). Stubs, `.ars` and the generator all read one member list,
  `cs_assembly/model.rs::assembly_model` (the stub walkers used to be duplicated), and one type decoder,
  `signature.rs::CsTy` (`CsTy::arrow()` is the stub spelling). Generated exports catch C# exceptions / argument
  errors and keep them for `arrow_bridge_take_error`, which `BridgeLib::take_error` reads after every call.
  A C# exception (`System.X: …`) becomes the built-in exception `ForeignError`; the bridge's own typed errors
  (`TypeError: …`) stay as they are (`built_in_types::builtin_exception_of`). Errors a cs-proc / js-proc host
  reports (`{"err": …}`, `proc_bridge::PipeConn::send_recv`) are always `ForeignError`; the cs-proc host sends
  the exception type too. `FfiTypeError` (stub/value mismatch) is a built-in exception deriving from `TypeError`
  (ffi_value_exchange_plan.md 7-4).
  `[UnmanagedCallersOnly]` methods (hand-written exports) are left out of the stub (`NullableInfo::unmanaged_entries`).
  Build: `examples/interop/cs_interop_test/build_bridge.ps1` (managed build without the generated file → generate →
  NativeAOT publish).
- **cs-dll values beyond scalars** (5-5 · D-16): the stub types come from `model.rs::stub_ann` (cs-dll only):
  `IntPtr` → `Option[Ptr[void]]`, numeric `T[]` → `Ptr[T]` (return) / `Union[Ptr[T], list[…]]` (param, parsed as
  `CsKind::Ptr` by `cs_values::decl_of`), `byte[]` → `bytes`, `List<T>` / non-numeric arrays → `list[T]`. Runtime
  (`cs_dll_runtime`): bytes and lists travel like strings (lists as the tagged JSON of `proc_values`, decoded by the
  shared `cs_values::decode_tagged`), numeric arrays as `ArPtrProtocol*` (`protocol_arg_declared` — an empty list takes
  the declared element type — and an out-param read with `ptr_from_protocol`), `IntPtr` as an address
  (`ptr_arg_address` / a borrowed `PtrHandle`). The generator emits `ArrowAbi.Array/WriteArray` (copy into
  `NativeMemory`, release `FreeArray`) and `ArrowJson.ParseList/WriteList`; projects link
  `bridge/ptr_protocol/ArPtrProtocol.cs`.
- Errors that carry an exception type keep it (`cs_values::tag_error`), so `except TypeError` catches them; other
  bridge errors are prefixed `CsProc:` / `CsDll:`.
- **The stub depends on the bridge** (task 5-2): `load_cs_assembly(path, CsBridge::Proc | Dll)`. cs-proc copies
  collections (`list` / `tuple` / `dict` / `set`, element types checked recursively in `stub_gen.rs::type_reason`)
  and `bytes` (`byte[]` — `CsTy::arrow` spells it `bytes`); cs-dll still marks them unsupported (5-5) and
  `cs_dll_runtime::dll_decl` rejects them at runtime. `cs_values::CsKind` has the container kinds.
- **Process bridges share one value representation**: `src/interpreter/proc_values.rs` (tags a / u / d / e / y on top of
  i / f / b / s / n / h; `Ptr` is a TypeError — no meaning in another process, D-5). cs-proc decodes by the declared
  type (`cs_proc_runtime::decode_result`, recursive); js-proc decodes by tag and `ffi_boundary::JavaScriptChecker`
  coerces numbers inside lists / dicts / sets / tuples (and turns a JS array into a declared tuple). The C# host
  (`examples/interop/cs_proc_test/ArrowPipeHost.cs`, shared by cs_probe) rebuilds arguments as the parameter's C# type
  and awaits `Task<T>`; `bridge/js_bridge.cjs` maps dicts to plain objects (all-str keys) or `Map`.

## Node.js IPC サブプロセス (`import[js-proc]`)

### 概要

`import[js-proc]` は Node.js を子プロセスとして起動し、Windows 名前付きパイプ上の NDJSON-RPC で任意の JS モジュールを呼び出します。cs-proc の JS 版に相当します。

### パーサー側 (`src/parser/imports/cs_js_modules.rs` — `load_js_module`)

`lang == "js-proc"` のとき `load_js_module(level, module)` が呼ばれます。
`.ars` スタブは**探索の起点**（`import_base(level)`）の `path/to/module.ars` だけを見ます
（⚠ 以前は `root_dir` も見ていた）。見つかればそれをパースして body にします。

スタブが無い・読めない・構文解析できない場合は**エラー**です（`cannot find the type stub`）。
Node.js 組み込みのモジュール（`path` など）も、使う関数を `.ars` に書きます（例: `examples/interop/path.ars`）。
⚠ 2026-10-08 より前は空の body（型なし）を返していた（`editor_import_resolution_plan.md` 1-2）。

### ランタイム側 (`src/interpreter/exec/modules.rs` — `import_js_proc`)

#### `find_js_config`（`src/interpreter/exec/mod.rs`）

`ar_config.json` を**探索の起点から祖先へ**（その後 CWD）探し、`javascript` キーを読みます。
⚠ 以前はエントリのディレクトリ（`python_search_dirs`）だけを見ていた。

```rust
fn find_js_config(search_dirs: &[PathBuf])
    -> Result<(PathBuf, PathBuf, PathBuf), String>
// 戻り値: (node_exe, bridge_script, bridge_root)
```

`ar_config.json` の対応フィールド:

```json
{
  "javascript": {
    "node_path":    "node",
    "bridge_script": "bridge/js_bridge.cjs",
    "bridge_root":  "vscode-extension"
  }
}
```

#### `exec_import` の `"js-proc"` ブランチ

```
1. find_js_config() → (node_exe, bridge_script, bridge_root)
2. bridge_key = canonicalize(bridge_script).to_string_lossy()
3. js_proc_runtime::launch_proc(node_exe, bridge_script, bridge_root)
     ├─ Node.js が起動し名前付きパイプサーバーをリッスン
     └─ "READY\n" を受信したらパイプクライアントを接続・キャッシュ
4. js_proc_runtime::list_functions(bridge_key, module_name)
     → ブリッジに {"op":"list","module":"out_debug/analysis"} を送信
     → エクスポート関数名リストを受信
5. 各関数名につき Value::JsProcFn { bridge_key, module_name, fn_name } を生成
6. NamespaceData::new(module_name, members) を alias に束縛
```

### ブリッジランタイム

パイプの下回り（起動ハンドシェイク・`send_recv`・名前付きパイプ接続・`Drop` での `quit` 送信）は
cs-proc と共通で、`src/interpreter/proc_bridge.rs` の `PipeConn` に集約されています。
`js_proc_runtime.rs` は js-proc 固有のリクエスト op・値エンコード・レジストリだけを持ちます。

#### 共有 `PipeConn` 構造体 (`src/interpreter/proc_bridge.rs`)

```rust
pub struct PipeConn {
    _child:  std::process::Child,   // 子プロセス（Node.js / C# host）を生存させる
    reader:  BufReader<std::fs::File>,
    writer:  BufWriter<std::fs::File>,
    next_id: u64,
}

// 起動済みの child を受け取り "READY" を読んでからパイプへ接続する
pub fn connect(child: Child, pipe_name: &str, tag: &str) -> Result<PipeConn, String>
// JSON を 1 行送って 1 行受け取る（id は自動付与、`err` は Err に変換）
pub fn send_recv(&mut self, req: Value, tag: &str) -> Result<Value, String>
```

`tag`（`"cs-proc"` / `"js-proc"`）はエラーメッセージの接頭辞に使われます。

#### グローバルブリッジレジストリ (`src/interpreter/js_proc_runtime.rs`)

スレッドローカルではなくグローバルな `OnceLock<Mutex<HashMap<PathBuf, PipeConn>>>` を使用します。これにより AsyncManager が生成する OS スレッドからもブリッジにアクセスできます。

```rust
fn global_bridges() -> &'static Mutex<HashMap<PathBuf, PipeConn>> {
    static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, PipeConn>>> = OnceLock::new();
    BRIDGES.get_or_init(|| Mutex::new(HashMap::new()))
}
```

C# の登録表も同じくプロセスで 1 つです（`ffi_value_exchange_plan.md` 6-2）: cs-proc のホスト（`cs_proc_runtime::procs`）と
cs-dll の DLL（`cs_dll_runtime::bridges`）。⚠ 以前はどちらも `thread_local!` で、非同期タスクのワーカーからは見えなかった
（cs-proc は `bridge not launched`、cs-dll の静的メソッドとコンストラクタは型スタブの本体へ落ちて黙って `None`）。今は
ブリッジが見つからなければ `CsDll: bridge DLL not loaded for '…'` のエラー。C# のクラスの表（`cs_values::CS_CLASSES`）は
クラスが `Rc` なのでスレッドごとのままで、ワーカーは送られてきた値から登録し直す（`cs_values::register_task_classes`）。

#### 公開 API

```rust
pub fn launch_proc(node_exe: &Path, bridge_script: &Path, bridge_root: &Path) -> Result<(), String>
pub fn list_functions(bridge_key: &str, module_name: &str) -> Result<Vec<String>, String>
pub fn call_function(bridge_key: &str, module_name: &str, fn_name: &str, args: &[Value])
    -> Result<Value, String>
```

#### 名前付きパイプ接続 (Windows)

`proc_bridge::open_pipe_client` は `CreateFileW` を最大20回リトライします。`ERROR_PIPE_BUSY` の場合は `WaitNamedPipeW(5000)` でパイプが空くのを待ち、それ以外は 8 回まで 150ms スリープして再試行します（cs/js で統一）。

#### 型エンコーディング

**Arrow → JSON** (`encode_arg`):

| Arrow 値 | JSON タグ |
|----------|-----------|
| `Int(n)` / `UInt(n)` | `{"t":"i","v":n}` |
| `Float(f)` | `{"t":"f","v":f}` |
| `Bool(b)` | `{"t":"b","v":b}` |
| `Str(s)` | `{"t":"s","v":s}` |
| `None` | `{"t":"n"}` |
| `List(items)` | `{"t":"a","v":[...]}` |
| その他（dict / tuple / set / インスタンス …） | `TypeError`（⚠ 以前は `{"t":"n"}` で JS に `null` が届いた・ffi_value_exchange_plan.md 2-3） |

非有限の float は `{"t":"f","v":"NaN"}` のように文字列で送る（JSON に NaN / ±Infinity は無い）。

**JSON → Arrow** (`decode_result`):

| JSON タグ | Arrow 値 |
|-----------|----------|
| `"s"` | `Value::Str` |
| `"i"` | `Value::Int` |
| `"f"` | `Value::Float` |
| `"b"` | `Value::Bool` |
| `"n"` | `Value::None` |
| `"a"` | `Value::List` (再帰デコード) |
| `"o"` | `TypeError`（⚠ 以前は `"k=v"` の文字列のリスト。受け渡しはフェーズ 5・5-2） |
| `"x"` | `TypeError`（関数・Symbol・BigInt。以前は文字列） |
| 読めない値・知らないタグ | エラー（⚠ 以前は `""` / `0` / 整数） |

JS の数値は整数と小数の区別が無い。ブリッジ（`bridge/js_bridge.cjs`）は安全な整数を `"i"`、それ以外を `"f"` で送り、
宣言型との差は `ffi_boundary::JavaScriptChecker`（`js_coerce`）が寄せる（`float` 宣言に整数値 → `Float`、`int` 宣言に
整数値の `Float` → `Int`、コンテナは要素へ再帰）。js-proc の型スタブの戻り値は Arrow の型として読む
（`collect_module_types`。⚠ 以前はプリミティブだけで、コンテナを返す関数は照合されなかった）。

### `Value::JsProcFn`

`src/interpreter/value.rs` に追加した新しい値バリアント:

```rust
Value::JsProcFn {
    bridge_key:  String,   // canonicalize(bridge_script) の文字列
    module_name: String,   // "out_debug/analysis" など
    fn_name:     String,   // "stripComment" など
}
```

#### 各ファイルでの対応

| ファイル | 対応箇所 |
|----------|----------|
| `value.rs` | `JsProcFn` バリアント追加。`deep_clone` は catch-all `other => other.clone()` で自動対応 |
| `ops.rs` | `is_truthy` → `true`、`type_name` → `"function"`、`value_matches_type_ann` の `"function"` アーム、`display` → `<js function 'module.name'>` |
| `eval.rs` | `eval_call` の match アームに `Value::JsProcFn` 追加 — 直接 `js_proc_runtime::call_function` を呼ぶ |
| `classes.rs` | `eval_method_call` の Namespace アームの match に `Value::JsProcFn` 追加 — アトリビュートメソッド呼び出し時に dispatch |

`classes.rs` への追加が必要な理由: `js_path.basename(...)` のような呼び出しは `Expr::Attr { object, attr }` 形式で、`eval_call` ではなく `eval_method_call` 経由になるため。

### ブリッジスクリプト (`bridge/js_bridge.cjs`)

⚠ **ブリッジは Arrow との接続が閉じたら自分で終了する**（socket `close`・ffi_value_exchange_plan.md 2-4）。Arrow のブリッジの表は
静的で終了時に破棄されず、`quit` は送られない。以前は実行ごとに node が残り、親の stderr を握ってパイプで読むスクリプトを
止めた。60 秒接続が無くても終了する。

IPC サーバー本体。起動引数: `node js_bridge.cjs <pipe_name> <bridge_root>`。

- **`Module._load` オーバーライド**: `require('vscode')` を `{bridge_root}/out_debug/vscode_mock` にリダイレクト（VS Code 拡張モジュールをホスト外で動かすため）
- **`loadModule(moduleName)`**: 上記の解決順序でモジュールをロード・キャッシュ
- **`handleRequest(req)`**: `list` / `call` / `quit` を処理。`call` は `await Promise.resolve(fn(...args))` で async 関数に透過対応
- 起動完了時に `process.stdout.write('READY\n')` でシグナル

### Promise の同期

Arrow の `<-` async 構文で OS スレッドを生成し、そのスレッドがブリッジの `send_recv`（ブロッキング I/O）を呼びます。ブリッジ側では `async function` の Promise を `await` で解決してから応答します。これにより Arrow 側では `block_return` で結果を受け取るだけで、Promise 同期が自動的に行われます。

```ar
mng <- async->str:
    block_return analysis.cleanTypeAnnotation("  List[int]  ")
mng.wait_for_finish()
print(mng.results[0])   # "List[int]"
```

### テストファイル

| ファイル | 内容 |
|----------|------|
| `examples/interop/js_proc_test.ar` | Node.js `path` モジュールと VS Code 拡張 `out_debug/analysis.js` の同期呼び出しテスト |
| `examples/interop/js_proc_async_test.ar` | AsyncManager と組み合わせた非同期呼び出しテスト |
| `examples/interop/math_render.ar` | LaTeX Workshop の MathJax を流用した TeX 数式 SVG レンダリング |
| `bridge/js_bridge.cjs` | IPC サーバー本体 |
| `bridge/lw_math.cjs` | LaTeX Workshop バンドル MathJax を使うカスタムブリッジモジュール |

---

## Key File Locations

| Subject | File | Key lines |
|---------|------|-----------|
| **Search rule / module identity (single definition)** | `src/module_path.rs` | header doc, `search_base`, `ModuleNames` |
| Module-path syntax `..a.b` (CLI + editor) | `src/parser/import_syntax.rs` | `parse_module_ref` |
| AST node definitions | `src/ast.rs` | `Stmt::Import` / `Stmt::FromImport` / `ImportOrigin` |
| Import parsing entry points | `src/parser/imports/dispatch.rs` / `cpp.rs` | `parse_import_stmt` / `parse_from_import_stmt` |
| Module loader dispatch | `src/parser/imports/dispatch.rs` | `load_module`, `name_module_file` |
| `.ar`/`.arc` file resolution | `src/parser/imports/ar_modules.rs` | `load_tl_module` … |
| Python loading | `src/parser/imports/py_modules.rs` | `load_python_module`, `fill_python_imports` |
| Module exports (no re-export) | `src/decl_names.rs` | `module_exports` |
| Rust crate loader entry | `src/partial_compiler/rs_loader.rs` | 98–198 |
| ABI compatibility check | `src/partial_compiler/rs_loader.rs` | 946–965 |
| Wrapper code generator | `src/partial_compiler/rs_loader.rs` | 1293–1675 |
| `.arc` binary format | `src/partial_compiler/module_compiler.rs` | ~1–170 |
| Runtime module execution | `src/interpreter/exec/modules.rs` | `exec_module`, `attach_submodule`, `import_search_dirs` |
| Packages / binding / `from . import x` | `src/parser/imports/packages.rs` | `chain_and_bind`, `from_import_submodules`, `from_dots_import` |
| Python interop runtime | `src/interpreter/py_interop.rs` | `load_py_int_module` |
| Type checker import handling | `src/type_check/stmt/check.rs` / `resolve.rs` | `Stmt::Import` arm, `closed_module`, `module_member_types`, `report_unimported_module_type` |
| JS-proc stub loader | `src/parser/imports/cs_js_modules.rs` | `load_js_module` |
| JS-proc config reader | `src/interpreter/exec/mod.rs` | `find_js_config` |
| JS-proc bridge runtime | `src/interpreter/js_proc_runtime.rs` | all |
| JS-proc value dispatch (attr call) | `src/interpreter/classes.rs` | Namespace arm in `eval_method_call` |
| JS-proc value dispatch (direct call) | `src/interpreter/eval.rs` | `JsProcFn` arm in `eval_call` |
| IPC サーバースクリプト | `bridge/js_bridge.cjs` | all |
| LaTeX Workshop MathJax ブリッジ | `bridge/lw_math.cjs` | all |
