---
name: importation
description: Use when writing, reading, or extending Arrow (.ar) import statements — `import[lang] module.path as alias` / `from module import[lang] Name` — including .py, .dll/.lib (C), .rs, C#, or Node.js interop. Explains what each `[lang]` tag loads and how src/parser/imports.rs and src/interpreter/exec.rs implement it, down to key line numbers.
---

# Importation of .ar, .py, .dll (C language), .lib, .rs, and more

Import syntax: `import[lang] module.path as alias` / `from module import[lang] Name`.
The `[lang]` tag selects the source type; omitting it defaults to `ar-auto`.

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

1. **Parse time** — `src/parser/imports.rs` resolves the module file, parses its source, and embeds the resulting AST into the `Stmt::Import` or `Stmt::FromImport` node as `body`.
2. **Runtime** — `src/interpreter/exec.rs` executes the `body` in an isolated scope and collects all declared names as a `NamespaceData` object, then binds it to the module variable.

Type checking (`src/type_check/stmt.rs`) reads the `body` AST directly to collect member types without any additional file I/O.

---

## Syntax and Language Tags

```ar
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
"js-proc"    → load_js_module        .ars stub (optional); runtime = Node.js IPC
"cs-proc"    → load_cs_proc_module   ECMA-335 DLL → type stubs; runtime = .NET IPC
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

**For `rs` and `arc` (v1) imports**: calls `take_native_bytes()` to dequeue the DLL bytes cached by the parser, writes them to a temp file, loads via `libloading::Library::new()`. Then for each `FnDef` in the body, looks up the symbol `{fn_name}_tl` in the loaded library and replaces the tree-walk `Value::Function` with a `Value::NativeFnRef`. For each `ClassDef`, registers methods via `register_native_method()`.

**For `arc` (v2 / LLVM bitcode)**: uses the Inkwell JIT path (`jit_from_bitcode`, `load_jit_module`). The JIT engine handle is kept alive in `self.jit_handles`.

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

For the C ABI value/struct-passing design behind this bridge (raw layout, zero-copy vs. shadow conversion, write-back), see the `c-abi-interop` skill.

---

## Node.js IPC サブプロセス (`import[js-proc]`)

### 概要

`import[js-proc]` は Node.js を子プロセスとして起動し、Windows 名前付きパイプ上の NDJSON-RPC で任意の JS モジュールを呼び出します。cs-proc の JS 版に相当します。

### パーサー側 (`src/parser/imports.rs` — `load_js_module`)

`lang == "js-proc"` のとき `load_js_module(module)` が呼ばれます。

```rust
fn load_js_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
    // モジュールパスを "seg1/seg2" 形式に結合
    let module_name = module.join("/");
    // .ars スタブが存在すれば静的型チェックに使用
    for dir in [&self.source_dir, &self.root_dir] {
        let stub = dir.join(format!("{}.ars", module_name));
        if stub.exists() {
            // 既存の load_stub_file() パターンで .ars をパース
            return self.parse_stub_file(&stub);
        }
    }
    Ok(vec![])  // スタブなし = 型チェックなし（実行時に動的取得）
}
```

スタブがない場合は空の body を返します。型チェックは行われず、インポート後のメンバーは全て動的型になります。

### ランタイム側 (`src/interpreter/exec.rs`)

#### `find_js_config`

`ar_config.json` を `python_search_dirs`（source_dir・root_dir）の順でウォークアップ検索し、`javascript` キーを読みます。

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

### ブリッジランタイム (`src/interpreter/js_proc_runtime.rs`)

#### `JsBridge` 構造体

```rust
pub struct JsBridge {
    _child:        std::process::Child,    // Node.js 子プロセス
    reader:        BufReader<std::fs::File>,
    writer:        BufWriter<std::fs::File>,
    next_id:       u64,
    pub bridge_script: PathBuf,
}
```

#### グローバルブリッジレジストリ

スレッドローカルではなくグローバルな `OnceLock<Mutex<HashMap<PathBuf, JsBridge>>>` を使用します。これにより AsyncManager が生成する OS スレッドからもブリッジにアクセスできます。

```rust
fn global_bridges() -> &'static Mutex<HashMap<PathBuf, JsBridge>> {
    static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, JsBridge>>> = OnceLock::new();
    BRIDGES.get_or_init(|| Mutex::new(HashMap::new()))
}
```

`cs_proc_runtime.rs` が `thread_local!` を使うのとは異なります。

#### 公開 API

```rust
pub fn launch_proc(node_exe: &Path, bridge_script: &Path, bridge_root: &Path) -> Result<(), String>
pub fn list_functions(bridge_key: &str, module_name: &str) -> Result<Vec<String>, String>
pub fn call_function(bridge_key: &str, module_name: &str, fn_name: &str, args: &[Value])
    -> Result<Value, String>
```

#### 名前付きパイプ接続 (Windows)

`open_pipe_client` は `CreateFileW` を最大20回リトライします。`ERROR_PIPE_BUSY` の場合は `WaitNamedPipeW(5000)` でパイプが空くのを待ちます。

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
| その他 | `{"t":"n"}` |

**JSON → Arrow** (`decode_result`):

| JSON タグ | Arrow 値 |
|-----------|----------|
| `"s"` | `Value::Str` |
| `"i"` | `Value::Int` |
| `"f"` | `Value::Float` |
| `"b"` | `Value::Bool` |
| `"n"` | `Value::None` |
| `"a"` | `Value::List` (再帰デコード) |
| `"o"` | `Value::List` (`"k=v"` 形式の文字列リスト) |

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
| JS-proc stub loader | `src/parser/imports.rs` | `load_js_module` |
| JS-proc config reader | `src/interpreter/exec.rs` | `find_js_config` |
| JS-proc bridge runtime | `src/interpreter/js_proc_runtime.rs` | all |
| JS-proc value dispatch (attr call) | `src/interpreter/classes.rs` | Namespace arm in `eval_method_call` |
| JS-proc value dispatch (direct call) | `src/interpreter/eval.rs` | `JsProcFn` arm in `eval_call` |
| IPC サーバースクリプト | `bridge/js_bridge.cjs` | all |
| LaTeX Workshop MathJax ブリッジ | `bridge/lw_math.cjs` | all |
