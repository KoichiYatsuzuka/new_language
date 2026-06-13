# Importation — Implementation Reference

This document describes how module importation works in the Arrow Rust implementation, derived directly from the source code.

---

## Overview

Import statements are processed in **two phases**:

1. **Parse time** — `src/parser/imports.rs` resolves the module file, parses its source, and embeds the resulting AST into the `Stmt::Import` or `Stmt::FromImport` node as `body`.
2. **Runtime** — `src/interpreter/exec.rs` executes the `body` in an isolated scope and collects all declared names as a `NamespaceData` object, then binds it to the module variable.

Type checking (`src/type_check/stmt.rs`) reads the `body` AST directly to collect member types without any additional file I/O.

---

## Syntax and Language Tags

```hv
import module.path                    # tl-auto (default)
import[ar]  module.path              # force .ar source
import[arc] module.path              # force .arc compiled
import[py]  module.path              # Python source
import[py-int] module.path           # Python type stubs (runtime via PyO3)
import[rs]  crate_name               # Rust crate
import[rs]  crate_name[0.2]          # Rust crate, specific version

from module import[lang] Name1, Name2 as N2
```

`parse_lang_bracket()` (imports.rs:267) reads the `[lang]` bracket and assembles hyphenated identifiers (e.g. `py-int`). If absent, the default is `"tl-auto"`.

`parse_version_bracket()` (imports.rs:359) reads `[X.Y.Z]` — only valid for `import[rs]`.

---

## AST Nodes

```
Stmt::Import {
    lang:      String,              // "tl-auto" | "ar" | "arc" | "py" | "py-int" | "rs" | "cpp-dll" | ...
    module:    Vec<String>,         // dotted path segments, e.g. ["os", "path"]
    with_file: Option<String>,      // header path for cpp-dll/cpp-lib only
    alias:     Option<String>,      // as alias
    body:      Vec<Stmt>,           // parsed module AST, embedded at parse time
}

Stmt::FromImport {
    lang:      String,
    module:    Vec<String>,
    with_file: Option<String>,
    names:     Vec<(String, Option<String>)>, // [(original_name, as_alias)]
    body:      Vec<Stmt>,           // parsed module AST, embedded at parse time
}
```

`body` is populated during parsing by `load_module()`. By the time the interpreter sees the node, the module has already been loaded and parsed — no further I/O is needed at runtime for `.ar` / `.arc` / `.py` modules.

---

## Module Loading Dispatch (`load_module`, imports.rs:306)

```
lang         → loader
──────────────────────────────────────────────
"tl-auto"    → load_tl_module        prefer .arc, fallback to .ar
"ar-auto"    → load_tl_module        (alias)
"tl"         → load_tl_source_module force .ar, skip .arc
"ar"         → load_tl_source_module (alias)
"tlc"        → load_tlc_module       force .arc, error if absent
"arc"        → load_tlc_module       (alias)
"py"         → load_python_module    Python source → converter → AST
"py-int"     → load_python_interface_module  .pyi → .py → empty on miss
"rs"         → load_rs_module        compile Rust crate → stub AST
"cpp-dll"    → parse_cpp_import      parse C header → stub AST
"cpp-lib"    → parse_cpp_import      (alias)
```

---

## `.ar` / `.arc` Module Resolution (`load_tl_module`, imports.rs:388)

Search directories: `source_dir` first, then `root_dir` (deduplicated when identical).

For each directory, candidates are tried in this order:

```
1. {dir}/{module_path}.arc    ← compiled module (preferred)
2. {dir}/{module_path}.ar     ← plain source
3. {dir}/{module_path}/__init__.ar  ← package
```

The first candidate that `exists()` wins.

**If `.arc`**: calls `partial_compiler::load_tlc()` to extract the embedded source text (and cache native bytes in a thread-local if v1/v2). The filename label becomes `<compiled:ModuleName>`.

**If `.ar`**: reads the file directly with `fs::read_to_string`.

Either way, the source is tokenized and a new `Parser` is created with:
- `source_dir` = directory of the resolved file
- `module_cache`, `loading`, and `root_dir` cloned from the parent parser

After parsing, the child's `module_cache` is merged back into the parent.

**Circular import detection**: `self.loading` is a `HashSet<PathBuf>`. Before parsing a module, its absolute path is inserted; it is removed after parsing completes. If a path is already in `loading`, an error is returned immediately.

**Cache key**: `("ar-auto", abs_path)` for tl-auto, `("ar", abs_path)` for forced source, `("arc", abs_path)` for forced compiled.

`load_tl_source_module` and `load_tlc_module` are identical to `load_tl_module` but skip `.arc` or skip `.ar` respectively.

---

## Python Modules (`load_python_module`, imports.rs:614)

- Search: only `source_dir`, candidate is `{module_path}.py`
- Converts Python source via `python_converter::convert_python_source()`
- Cache key: `("py", abs_path)`

### Python Interface (`load_python_interface_module`, imports.rs:658)

Used by `import[py-int]`. The body is for type-checking only; the runtime uses PyO3.

Search order (via `python_search_dirs()`):
1. `source_dir`
2. Directories in `PYTHONPATH` env var
3. `$PYTHONHOME/Lib/site-packages`

For each directory, tries `.pyi` first, then `.py`. If nothing is found, returns an empty body (no type checking, PyO3 handles everything at runtime).

Parsing errors in `.pyi` / `.py` files are silently ignored (best-effort via `unwrap_or_default()`).

---

## Rust Crate Import (`import[rs]`)

### Syntax

```hv
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

Looks for `ar_config.json` in `source_dir` and `root_dir`. Reads the `rust.crates_path` key, which may be a single string or an array of strings.

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

**Free functions** — accepted when:
- Starts with `pub fn` at the top level of a file (not inside an `impl` block)
- No generic type parameters (`<`)
- All parameter types and return type are ABI-compatible (see below)

**Structs** — accepted when (`parse_struct_sigs`, rs_loader.rs:627):
- `pub struct Name {` at the top level (no generics)
- At least one `pub` field with an ABI-compatible type
- Constructor: `pub fn new(...) -> Self` in `impl Name { }` (preferred), otherwise all-pub-field struct literal

**Methods** — accepted when (`parse_method_line`, rs_loader.rs:871):
- Inside `impl Name {` (no trait impl, no generics)
- `&self` or `&mut self` receiver
- All param types ABI-compatible
- Return type is ABI-compatible or is a struct defined in the same crate

**ABI-compatible types** (`is_abi_compatible`, rs_loader.rs:946):
```
i8 i16 i32 i64 i128 isize
u8 u16 u32 u64 u128 usize
f32 f64
bool
String  &str  &String
&[u8]           → passed/received as HV str
Vec<u8>         → returned as hex str
[u8; N]         → returned as hex str
```

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
  - `{StructName}____init___tl` — constructs the struct, stores it in the arena keyed by a fresh integer, writes the key back into the HV instance as `__rs_handle__`.
  - `{StructName}__drop_tl` — removes the key from the arena.
  - `{StructName}__get_{field}_tl` / `{StructName}__set_{field}_tl` — field access.
  - `{StructName}__{method}_tl` — method dispatch via arena lookup.

Runs `cargo build --release` on the temp project. On success, reads the resulting DLL/SO bytes and deletes the temp directory.

#### Step 5 — Cache and return stubs

Calls `cache_native(module_name, exports, dll_bytes)` to store the DLL bytes in a thread-local `NATIVE_CACHE` (in `module_compiler.rs`), keyed by module name.

Returns `Vec<Stmt>` stubs (`make_stubs`, rs_loader.rs:1035):
- `Stmt::FnDef` for each free function — empty body, HV types, `is_abstract: true`
- `Stmt::ClassDef` for each struct — with field stubs, `__init__`, `drop`, getter/setter, and method stubs

These stubs are embedded in `Stmt::Import.body` and used by the type checker and runtime.

### Type mapping (Rust → Arrow)

| Rust type | Arrow type |
|-----------|---------------|
| `i*`, `u*`, `isize`, `usize` | `int` |
| `f32`, `f64` | `float` |
| `bool` | `bool` |
| `String`, `&str`, `&String` | `str` |
| `&[u8]`, `Vec<u8>`, `[u8; N]` | `str` |

---

## Runtime Execution (`exec_module`, exec.rs:~1321)

All import variants go through `exec_module`. Cache key: `(lang, PathBuf from module segments)`.

States:
- `ModuleState::Loading` — set before execution to catch circular imports at runtime
- `ModuleState::Loaded(NamespaceData)` — cached after first execution

**For `.ar` / `.arc` / `py` imports**: runs the body AST in a fresh scope, collects all declared top-level variables as `NamespaceData.members`.

**For `rs` and `hvc` (v1) imports**: calls `take_native_bytes()` to dequeue the DLL bytes cached by the parser, writes them to a temp file, loads via `libloading::Library::new()`. Then for each `FnDef` in the body, looks up the symbol `{fn_name}_tl` in the loaded library and replaces the tree-walk `Value::Function` with a `Value::NativeFnRef`. For each `ClassDef`, registers methods via `register_native_method()`.

**For `hvc` (v2 / LLVM bitcode)**: uses the Inkwell JIT path (`jit_from_bitcode`, `load_jit_module`). The JIT engine handle is kept alive in `self.jit_handles`.

**For `py-int` imports**: calls `py_interop::load_py_int_module()` which uses PyO3 to import the Python module directly and wraps all non-private attributes as `Value::PyObject`.

---

## `Stmt::Import` vs `Stmt::FromImport` at Runtime

**`Stmt::Import`**: binds the entire `NamespaceData` as a namespace value. Variable name = `alias` if present, otherwise the last segment of the module path.

**`Stmt::FromImport`**: calls `exec_module` the same way, then for each `(orig_name, alias)` looks up `orig_name` in the namespace members and binds to `alias` (or `orig_name` if no alias).

---

## C/C++ Imports (`import[cpp-dll]`, `import[cpp-lib]`)

Not covered by `rs_loader`. Parsed by `parse_cpp_import` (imports.rs:91):

1. Resolves dotted identifier to a header file path: `DxLib.DxLib` → `{source_dir}/DxLib/DxLib.h`
2. Reads the header and calls `cpp_bridge::parse_header_full()` to extract C function signatures and struct definitions.
3. Generates `Stmt::FnDef` and `Stmt::ClassDef` stubs for the type checker.

At runtime, `exec.rs` dispatches to the C/C++ bridge for actual calls (not via `exec_module`).

---

## Key File Locations

| Subject | File | Key lines |
|---------|------|-----------|
| AST node definitions | `src/ast.rs` | ~688–722 |
| Import parsing entry points | `src/parser/imports.rs` | 38–81, 215–264 |
| Module loader dispatch | `src/parser/imports.rs` | 306–328 |
| `.ar`/`.arc` file resolution | `src/parser/imports.rs` | 388–611 |
| Python loading | `src/parser/imports.rs` | 614–726 |
| Rust crate loader entry | `src/partial_compiler/rs_loader.rs` | 98–198 |
| ABI compatibility check | `src/partial_compiler/rs_loader.rs` | 946–965 |
| Wrapper code generator | `src/partial_compiler/rs_loader.rs` | 1293–1675 |
| `.arc` binary format | `src/partial_compiler/module_compiler.rs` | ~1–170 |
| Runtime module execution | `src/interpreter/exec.rs` | ~1321–1627 |
| Python interop runtime | `src/interpreter/py_interop.rs` | ~143–184 |
| Type checker import handling | `src/type_check/stmt.rs` | ~553–576 |
