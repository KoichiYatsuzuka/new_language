---
name: partial-compile
description: Use when running `cargo run -- --compile`, working with .arc/.ars partial-compilation output, modifying src/partial_compiler/ (LLVM codegen, eligibility rules, the typed ABI, or the Rust-crate import[rs] loader), or explaining/demoing how an Arrow (.ar) module gets compiled to native machine code and dispatched natively on import. Includes the canonical physics.ar demo workflow and full codegen implementation reference.
---

# Partial Compilation

A `.ar` module can be partially compiled to native machine code with `--compile`.
This produces two files next to the source:

| Output file | Contents |
|-------------|----------|
| `{stem}.arc` | Compiled module: binary header + embedded source text, optionally with a native shared library embedded (v3 format; v1 is still read) |
| `{stem}.ars` | Type stub: function/class/trait signatures with `...` bodies (used by the VS Code extension and the static type checker) |

## Canonical demo

`examples/interop/test_modules/physics.ar` is the canonical module for demonstrating native compilation.
`examples/interop/importation.ar` (section 1) is the corresponding runner that shows the full workflow.

```bash
# Step 1 — run interpreted
cargo run --release -- examples/interop/importation.ar

# Step 2 — compile the module
cargo run --release -- --compile examples/interop/test_modules/physics.ar
# Output:
#   NativeLib: compiling 6 function(s): potential, kinetic, vel_dot, ...
#   NativeLib: 6 function(s) embedded in examples\interop\test_modules\physics.arc
#   Compiled : examples\interop\test_modules\physics.arc
#   Stub     : examples\interop\test_modules\physics.ars

# Step 3 — run again with native dispatch (same command as Step 1)
cargo run --release -- examples/interop/importation.ar
```

## How the compiled module is used

When a `.ar` file imports a module, the parser prefers `.arc` over `.ar`.
If the `.arc` is v1, the embedded DLL is extracted to a temp file at runtime and loaded via `libloading`.
The file is `%TEMP%\{stem}_tl_{content hash}.dll` and is **not rewritten when an identical one exists**
(`materialize_native_lib`, copy_alloc_speed_plan.md 7-8): loading a freshly written DLL costs 0.6–0.8 s (antivirus scan).
One process never gets the same path twice (a second identical module gets its own copy, so DLL globals are not shared).
Eligible functions are dispatched natively; all other functions tree-walk as usual.

```
import test_modules.physics                  # loads test_modules/physics.arc (parser)
test_modules.physics.total_energy(a, b, N)   # calls native code
```

For how the resulting module is *imported* (search order, cache keys, `import[rs]` vs `.arc`), see the `importation` skill.

---

# Partial Compile — Implementation Reference

This section describes the partial compile subsystem (`src/partial_compiler/`) in detail, derived from the actual source code.

---

## Overview

`--compile <file.ar>` takes a parsed Arrow module and produces two output files:

| File | Purpose |
|------|---------|
| `{stem}.arc` | Compiled module — binary blob consumed by the importer at runtime |
| `{stem}.ars` | Type stub — text read by the type checker and VS Code extension |

There is a single native backend, with a source-only fallback:

1. **clang** — emits LLVM IR text, then shells out to `clang -O3 -shared` → v3 `.arc` (DLL embedded)
2. **Source-only** — writes a v0 `.arc` (no native code) with a warning

(An inkwell/JIT backend existed behind `feature = "llvm"`; it was removed — building it
requires MSVC-built LLVM dev libraries, which are not obtainable in this environment.
`clang.exe` alone is enough for the path above.)

---

## Module Map

```
src/partial_compiler/
├── mod.rs            — public re-exports: compile, load_tlc, take_native_bytes, NativePayload
├── module_compiler.rs — .arc file format writer/reader + thread-local native cache
├── llvm_codegen.rs   — LLVM IR text generator (compiled to a DLL by clang)
├── stub_gen.rs       — .ars stub text generator
└── rs_loader.rs      — import[rs] native Rust crate loader
```

---

## The `.arc` Binary Format

`MAGIC = b"TLC\x00"` — present at byte 0 of every valid `.arc` file.

### Version 0 — source-only

```
[4 bytes]  magic    : b"TLC\x00"
[4 bytes]  version  : u32 LE  (0)
[4 bytes]  name_len : u32 LE
[name_len] name     : UTF-8 module name
[4 bytes]  src_len  : u32 LE
[src_len]  source   : UTF-8 source text
```

Written when native compilation is skipped (no eligible functions, or compiler unavailable).

### Version 1 — DLL embedded

Extends v0 with a native shared library (`.dll` / `.so` / `.dylib`) compiled by clang.

```
... (v0 header) ...
[4 bytes]  n_fns    : u32 LE
for each fn:
  [4 bytes]      fn_name_len : u32 LE
  [fn_name_len]  fn_name     : UTF-8
  [4 bytes]      n_params    : u32 LE
[4 bytes]  dll_len  : u32 LE
[dll_len]  dll_bytes: raw shared-library bytes
```

Still read (written by arrows before known_bugs.md #29); no longer written.

### Version 3 — DLL embedded, with the callback-table size

v1 plus the size of the callback table the DLL's generated code assumes (highest field it uses + 1,
`llvm_codegen::callbacks_required`), right after the v0 header:

```
... (v0 header, version 3) ...
[4 bytes]  n_callbacks : u32 LE
... (v1 from n_fns on) ...
```

`arc_format::parse` refuses a v3 file whose `n_callbacks` exceeds `arc_format::CALLBACK_TABLE_LEN`
("compiled by a newer arrow … recompile it with `arrow --compile`", a static import error in the CLI and the
VS Code extension alike). The table only grows at the end, so before #29 an older exe reading a newer `.arc`
called past its table and crashed (0xC0000005). ⚠ When you add a callback field, bump
`CALLBACK_TABLE_LEN` — a compile-time `assert!` in `native_api/mod.rs` stops the build otherwise.
Exes before v3 refuse v3 files (`unsupported .arc version 3`) instead of crashing.

⚠ **Version 2 is skipped** and still refused: a v2 format carrying LLVM bitcode existed for the removed inkwell
JIT backend (dropped in `a8035d95`), so a leftover file must not be misread as the new layout.

---

## Compilation Pipeline (`module_compiler.rs`)

### `compile(source, stmts, source_path)`

Entry point called by `main.rs` when `--compile` is given.

1. **Stub** — always runs first: calls `stub_gen::generate_stub(stmts)` and writes `{stem}.ars`.
2. **Native** — calls `compile_native(stmts)`: `llvm_codegen::generate_llvm_module(stmts)` → `clang` → `NativePayload::Dll`
3. **Write** — calls `write_tlc_native(…)` (v3) or `write_tlc_v0` (source-only) depending on outcome.
4. Returns `(hvc_path, hvs_path)`.

### `load_tlc(path)`

Called by the parser/importer when a `.arc` file is found.

1. Reads and parses binary data via `parse_tlc`.
2. If v1 or v3: inserts `(exports, NativePayload)` into `NATIVE_CACHE`.
3. Returns `(module_name, source_text)` — the source is then parsed and type-checked normally.

### Thread-local cache

```rust
thread_local! {
    static NATIVE_CACHE: RefCell<HashMap<String, (Vec<FnExport>, NativePayload)>>;
}
```

`take_native_bytes(module_name)` — consumed by `exec.rs` when a module is imported, to attach native dispatch to eligible functions.
`cache_native(module_name, exports, dll_bytes)` — used by `rs_loader` to pre-populate the cache for `import[rs]` modules.

---

## Eligibility Rules (`llvm_codegen.rs` — `stmt_eligible` / `expr_eligible`)

The eligibility check recurses into nested `if`, `while`, `for`, `match`, and `block` bodies. Control-flow expressions (`Expr::IfExpr`, `Expr::ForExpr`, etc.) are eligible and generate correct LLVM IR including `block_return` and `loop_yield` semantics.

### Function-level — entire function skipped

| Case | Reason |
|------|--------|
| Template function/gen (`template_params` non-empty) | Cannot monomorphize at compile time |
| Abstract function (`is_abstract = true`) | No body to compile |
| Method inside a template class | Class is not instantiated at compile time |

### Ineligible statements — any occurrence in the body skips the whole function

| Statement | Notes |
|-----------|-------|
| `import` | No cross-module inlining |
| Nested `fn` / `gen` definition | Requires runtime closure capture |
| Nested `class` / `trait` definition | Type-system structure, not native-compilable |
| `try` / `except` / `finally` | Exception protocol uses sentinel strings incompatible with LLVM IR |
| Bare `raise` (re-raise with no expression) | Same exception-protocol reason |
| `raise expr` where `expr` is not a positional constructor call | Only `raise ExcType(msg)` form is eligible; `raise x`, `raise X(kw=v)` are not |
| `static mut` declaration | Shared global cell keyed by source position; not representable in LLVM IR |
| `async` assignment (`target <- async->T: body`) | Thread-spawning semantics require the interpreter |
| Tuple-unpacking `let` / `mut` (`let x, y = expr`) | Not implemented in codegen |
| Multi-target `for` (`for x, y in iter`) | Only single-target `for` is lowered; `targets.len() > 1` fails the check |
| `x = ..` / `x += ..` / `freeze x` whose target is not a local (a module global) | The codegen only writes locals (`GenCtx::locals`). The check walks the body in order and allows a target only if it is a parameter or was declared earlier (`let` / `mut` / `const` / `for` target) — `EligScope` (known_bugs.md #23: the global write used to be silently dropped, and `+=` made clang fail for the whole module). `freeze` of a local is lowered like `let x = x` (`ar_bind_value` with `CopyFreeze`, so `__freeze__` runs; known_bugs.md #24 — it used to emit nothing) |

### Ineligible expressions — any occurrence in the body skips the whole function

| Expression | Notes |
|------------|-------|
| Keyword argument in any call (`f(x=1)`) | Only positional `CallArg::Positional` is supported |
| Set literal (`{a, b, c}`) | Not implemented |
| Slice (`a[i:j:k]`) | Not implemented |
| Lambda (`lambda x: expr`) | Requires closure capture |
| List / dict / set comprehension | Not implemented |
| `yield` as an expression | N/A in compiled context |
| `await` expression | Async semantics require the interpreter |
| F-string / format string | Not implemented |

### Additional restrictions for `gen` functions

A `gen` function is compiled with an **eager-accumulator** strategy (all `yield` values collected into a list, returned at once). The following make a `gen` body ineligible on top of the rules above:

| Statement | Reason |
|-----------|--------|
| `loop_yield` in body | Incompatible with eager accumulator; only plain `yield` is allowed |
| `block_return` in body | Same — only `yield` is the intended exit mechanism |

### Compiles but with reduced optimizations

These cases are **not** ineligible — the function compiles — but specific optimizations do not apply:

| Situation | What is lost |
|-----------|-------------|
| Parameter annotated with a trait type (not a concrete class) | Treated as opaque `Handle`; no fast field reads (`CB_GET_FLOAT_FIELD` / `CB_GET_INT_FIELD`), no `_fast` variant, all method calls go through `CB_CALL_METHOD` |
| Class-instance parameter whose fields are written in the body | Pre-read optimization and `_fast` variant are suppressed for that parameter (purity check `body_writes_param` fails) |
| `gen` function (any eligible gen) | Returns an eager list instead of a lazy generator; semantics differ from the interpreter's lazy protocol |

---

## LLVM IR Code Generator (`llvm_codegen.rs`)

### Internal Type System

```rust
enum Ty { Int, Float, Bool, Handle }
```

- `Int` → LLVM `i64` (directly)
- `Float` → LLVM `double` (directly)
- `Bool` → stored as LLVM `i64` handle (TL_TRUE = 1, TL_FALSE = 2); used as `i1` only for branches
- `Handle` → opaque `i64` (a tag into `VALUE_ARENA`)

Type annotation `"int"` maps to `Ty::Int`; `"float"` maps to `Ty::Float`; anything else maps to `Ty::Handle`. This determines how the variable is stored in its alloca and how arithmetic is specialized.

### Module Header

Every generated `.ll` file begins with:

```llvm
%ArCallbacks = type { ptr, ptr, ..., ptr }   ; 35 function-pointer fields

@CB = internal global ptr null

define void @ar_init(ptr %cb) { store ptr %cb, ptr @CB; ret void }

define internal i64 @_tl_idiv(i64 %a, i64 %b) { ... }   ; Python floor-div
define internal i64 @_tl_imod(i64 %a, i64 %b) { ... }   ; Python modulo

declare double @llvm.pow.f64(double, double)
declare double @llvm.floor.f64(double)
```

`ar_init` is called by the interpreter at module load time to inject the `ArCallbacks` pointer.

### ArCallbacks — Field Index Table

Each field is a function pointer; the index is used in `getelementptr` instructions.

| Index | Name | Signature |
|-------|------|-----------|
| 0 | `make_int` | `i64(i64)` |
| 1 | `make_float` | `i64(double)` |
| 3 | `make_str` | `i64(ptr, i32)` |
| 4 | `make_list` | `i64(ptr, i32)` |
| 5 | `make_tuple` | `i64(ptr, i32)` |
| 6 | `make_dict` | `i64(ptr, ptr, i32)` |
| 8 | `is_truthy` | `i32(i64)` |
| 9 | `binop` | `i64(i32, i64, i64)` |
| 10 | `unop` | `i64(i32, i64)` |
| 11 | `call_fn` | `i64(i64, ptr, i32)` |
| 12 | `get_attr` | `i64(i64, ptr, i32)` |
| 13 | `set_attr` | `void(i64, ptr, i32, i64)` |
| 14 | `subscript` | `i64(i64, i64)` |
| 15 | `get_global` | `i64(ptr, i32)` |
| 16 | `iter_from` | `i64(i64)` |
| 17 | `iter_next` | `i64(i64)` |
| 18 | `is_type` | `i64(i64, ptr, i32)` |
| 19 | `arena_save` | `i64()` |
| 20 | `arena_compact` | `i64(i64, i64)` |
| 22 | `to_int` | `i64(i64)` |
| 23 | `to_float` | `double(i64)` |
| 24 | `deep_copy` | `i64(i64)` |
| 27 | `list_append` | `i64(i64, i64)` |
| 28 | `raise_exc` | `i64(i64, i64)` |
| 29 | `make_cell` | `i64(i64)` |
| 30 | `get_cell` | `i64(i64)` |
| 31 | `set_cell` | `void(i64, i64)` |
| 32 | `call_method` | `i64(i64, ptr, i32, ptr, i32)` |
| 33 | `get_float_field` | `double(i64, ptr, i32)` |
| 34 | `get_int_field` | `i64(i64, ptr, i32)` |
| 35 | `flat_data_ptr` | `i64(i64)` |
| 36 | `flat_len` | `i64(i64)` |
| 37 | `fn_trampoline` | `ptr(i64)` |
| 38 | `to_bool` | `i32(i64)` — `True` / `False` only; anything else sets a TypeError (ffi plan 7-12) |
| 39 | `pending_error` | `i32()` — 1 if an argument conversion set an error; the cpp / rs wrappers then return without calling the native function (7-12) |
| 40 | `make_bytes` | `i64(ptr, i64)` — `char*` / `wchar_t*` return → bytes (4-3) |
| 41 | `make_ptr` | `i64(u64, ptr, i32)` — pointer return → borrowed Ptr[elem]; NULL → None (4-3) |
| 42 | `bytes_arg` | `ptr(i64)` — `const char*` argument from bytes / None (4-3) |
| 43 | `ptr_arg` | `u64(i64, ptr, i32)` — `void*` / struct pointer argument from Ptr / None (4-3) |
| 44 | `protocol_arg` | `void(i64, ptr, i32)` — builds an `ArPtrProtocol` argument from Ptr / None (4-4), or from a list / bytes as a copy that lives until the outermost native call returns (4-7) |
| 45 | `protocol_done` | `void(i64, u64)` — after an `ArPtrProtocol*` argument: address 0 → invalidate the Ptr (4-4); a `mut` list gets what C wrote (4-7) |
| 46 | `protocol_ret` | `i64(ptr, ptr, i32)` — an `ArPtrProtocol` return value → owned / static Ptr; struct names to check the element type (4-5) |
| 47 | `make_raw_instance` | `i64(i64, ptr, i64)` — by-value struct return with an exact layout → instance by copying the bytes (no constructor; 4-6) |
| 48 | `make_instance` | `i64(i64, ptr, i32)` — by-value struct return with pointer fields → instance from converted field handles (no constructor; 4-6) |
| 49 | `bytes_buf_arg` | `ptr(i64, ptr)` — `char*` output buffer: a copy of the bytes and its length; the wrapper writes it back as bytes of the same length (`write_handle` + `make_bytes`; 4-7) |

⚠ `to_int` (22) / `to_float` (23) accept numbers only (`float` truncates, `bool` is 0 / 1); a non-number (`None`, `str`, `list`, `Ptr` …) sets a TypeError and returns 0 (7-12 — it used to return 0 silently). New fields go **at the end**: the cpp / rs wrappers declare a prefix of the struct and the LLVM code indexes it.

### Generated Function Variants

For each eligible function `f`, the codegen emits up to six LLVM functions (`@f_pre` — the body of a `mut self`
method that receives remembered nested-receiver blocks, with `@f_impl` as its null-passing wrapper — is described
under "Remembered Block Addresses", copy_alloc_speed_plan.md 7-19):

#### `@f_impl` (internal)

The actual implementation. Receives parameters as `i64` handles (one per declared param).
If the return type annotation is `float`, the ABI is `double` (no boxing/unboxing in the hot path).
All other return types use `i64` handle ABI.

```llvm
define internal i64 @f_impl(i64 %_h0, i64 %_h1, ...) {
entry:
  ; alloca declarations
  ; parameter unwrapping (CB_TO_INT / CB_TO_FLOAT for typed params, store for handles)
  ; body
}
```

#### `@f_tl` (public — called by the interpreter)

Wrapper with the uniform calling convention expected by `exec.rs`:

```llvm
define [dllexport] i64 @f_tl(ptr %args, i32 %_n) {
  ; GEP each arg slot from %args
  ; call @f_impl(...)
  ; for float return: box via CB_MAKE_FLOAT
  ; ret i64 result
}
```

#### `@f_fast` (internal — emitted when class params are present)

Receives class instance fields as raw scalars instead of arena handles. This eliminates all `CB_GET_FLOAT_FIELD` / `CB_GET_INT_FIELD` callbacks in the hot path; the function is pure arithmetic and LLVM can inline and hoist it freely.

A `_fast` variant is only emitted if:
- At least one parameter is a class instance with typed (`int`/`float`) fields
- That parameter is never written in the function body (purity analysis via `body_writes_param`)

#### `@f_blk` (internal — block-pointer variant of a raw-layout class's method, copy_alloc_speed_plan.md 7-14)

`mut self` arrives as the instance's block address (`ptr %_bp_self`) and its fields are read/written with plain
`load` / `store` (no init/immutable checks); `let self` and `let` class params of raw-layout classes arrive flattened
like `_fast` (a snapshot at the call); `let int/float` params raw. Candidates: `blk_sig_of` (methods of this module's
raw-layout, all-public classes; no defaults; ret `None`/`int`/`float`). The variant is dropped (and the symbol evicted
from `blk_ok`, which `generate_llvm_module` re-generates around) if the body needs any callback (`blk_failed` via
`call_cb`), calls anything but `@llvm.*` (`calls_only_intrinsics`), or uses `self` / a flattened param as a value.

Call sites (`emit_blk_method_call`): receiver and class args must be locals or `local.attr`; numeric args must be
`numeric_leaf`. The code fetches each block (remembered `%_ip_x` / `%_apN`), then checks: non-null; all init bits set;
for `mut self` the IMMUTABLE bit clear; for a guarded receiver the head's `class_id` equals the guard cell (the desc
match alone cannot tell the main program's `V` from a module's `V` with the same fields); if a flattened non-receiver
arg is a `mut` value, no `__copy__` class exists (`CB_COPY_HOOKS`). Pass → `call @X_blk`; fail → the normal path
(`gen_method_call_expr`), which first fills the IC cells of null nested blocks and the empty guard cell so later
calls pass. `scripts/native_ir_abi_check.py` checks `_blk` calls too.

#### `@f_typed` (public — 統一 typed ABI、zero-TLS)

```llvm
define [dllexport] i32 @f_typed(ptr %_args, ptr %_ret, ptr %_err) {
  ; %_args: u64 スロット列（int は i64、float は f64 ビットパターンを同スロットに格納）
  ; %_ret:  戻り値スロット（生値を書き込む）
  ; %_err:  ErrSlot（raise 時に例外情報を書き込む）
  ; 戻り値: 0 = 正常、1 = raise 発生
}
```

**ErrSlot レイアウト**（`native_api.rs` の `#[repr(C)] ErrSlot` と一致必須）:

| offset | field | contents |
|--------|-------|----------|
| +0  | `type_ptr: ptr` | 例外クラス名（DLL 内静的文字列） |
| +8  | `type_len: i64` | |
| +16 | `msg_ptr: ptr`  | メッセージ（DLL 内静的文字列） |
| +24 | `msg_len: i64`  | |

**特性**:
- TLS・アリーナ・ハンドルを一切通らない。`GenCtx.typed_mode` 中に `call_cb` が呼ばれたら
  `typed_failed` が立ち、その関数の typed 変種は破棄される（自動検出）
- `raise Name("literal")` は ErrSlot への静的文字列書き込み + `ret i32 1` に展開される
- typed 同士のモジュール内呼び出しは `@callee_typed(args*, ret*, %_err)` を直接発行し、
  `status != 0` なら即 `ret i32 %st` で伝播する（C のエラー伝播と同型）。
  `%_err` ポインタを横流しするため最内の raise 情報がそのまま最外へ届く
- LLVM が typed 関数同士をインライン化できる（ネイティブ間呼び出しコストは実測 0ns）

**適格条件**（`generate_llvm_module` の `typed_candidates` — インタープリタ側
`exec.rs::build_typed_sig` と一致必須）:
- トップレベル関数（メソッド・ジェネレータは対象外）
- 全パラメータが `let` かつ `int`/`float` 注釈、デフォルト値なし
- 戻り値が `int`/`float`
- 本体がコールバック不要（fixpoint: 破棄された typed 関数を呼ぶ関数も連鎖的に破棄）

**インタープリタ側ディスパッチ**（`eval.rs dispatch_native_evaled` 冒頭）:
モジュールロード時に `{name}_typed` シンボルを解決して `NativeFnRef.typed_fn_ptr` に
キャッシュ。呼び出し時は引数を u64 スロット配列（スタック上 `[0u64; 16]`）に詰めて
直接呼び出し、status != 0 なら `ErrSlot::to_error_string()`（`"TypeName: msg"` 形式）で
既存の raise 経路へ合流する。実行時型が合わなければハンドル経路へフォールバック。

**AST インラインキャッシュ（関数ポインタ焼き込み）**:
`Expr::Call` は `cache: NativeCallCache`（`ast.rs`、`RefCell<Option<Arc<dyn Any>>>`）を持つ。
`eval_call`（`eval.rs`）は呼び出し先が以下を満たすと初回実行時に `Arc<NativeFnRef>` を
このスロットへ焼き込む:

- `Expr::Ident` 呼び出しで、バインディングが**不変**（`let` / import 束縛）
- `typed_sig` あり + `typed_fn_ptr != 0`
- 全引数が位置引数、引数数一致（≤16）

以後の実行は `dispatch_native_typed_exprs` へ直行し、スコープ検索・Value マッチ・
組み込み名チェックをすべて跳ばす。不変バインディングは再代入・再宣言とも禁止のため
キャッシュ無効化は不要。`NativeCallCache::clone()` は**空キャッシュ**を返す
（FnValue へのボディ複製・async の deep_clone ごとに再解決させ、DLL 生存期間の問題を防ぐ）。
型不一致時は評価済みスロットを Value に復元してハンドル経路へフォールバックする
（引数式の副作用は二重実行されない）。

**計測**（partial_call_overhead.ar / `sink += noop0()`）:
ハンドル経路 461ns → typed ABI 204ns → インラインキャッシュ **~93ns**/call。

**検証例**: `examples/interop/typed_abi.ar` + `examples/interop/test_modules/typed_abi_module.ar`

**cpp ブリッジ（import[cpp-lib] / cpp-dll）への適用**:
`cpp_bridge/codegen.rs` の `gen_dll_fn` は、全プリミティブシグネチャ
（params ∈ {int, long, float, double}、ret ∈ {void, int, long, float, double}）の C 関数に
`{name}_typed` ラッパーも生成する（`cpp_typed_eligible` — インタープリタ側
`exec.rs::build_cpp_typed_sig` と条件一致必須）。

- シンボルは初回呼び出し時に static へキャッシュ（従来は毎呼び出し GetProcAddress）
- 引数は u64 スロットからの純キャストで、CB コールバック（TLS）を一切通らない
- C 関数は raise しないため status は常に 0（シンボル欠落時のみ 1）
- `void` 戻り値は `AbiTy::Void` → Arrow 側 `None`

**ネイティブ→C の高速パス（`ar_call_fn`）**: コンパイル済み Arrow コードが
CB_CALL_FN 経由で C 関数を呼ぶ場合、`ar_call_fn` は typed シグネチャがあれば
引数ハンドルを**1回の STATE ボロー**で u64 スロットにデコードして `{name}_typed` を
直接呼ぶ。enter/exit_native_call・per-arg unmarshal CB・結果 marshal CB が消え、
STATE アクセスは ~8回 → 1–2回になる（実測: DrawPixel 1回あたり 0.83µs → 0.42µs）。

**キャッシュ注意**: ラッパー DLL（`ar_{stem}.dll`）は存在チェックのみの永続キャッシュ。
codegen 変更後は削除して再生成させること（shim はソース比較で自動再利用される）。

### Approach-1 Pre-reads

At function entry, for each class-instance parameter that satisfies the purity condition, the codegen reads all typed fields once via a single callback per field, stores the values into stack allocas, and inserts them into `preread_fields`. Subsequent `Expr::Attr` accesses on those params emit plain `load` instructions instead of callback calls.

This is the primary source of speedup for methods operating on typed class instances (e.g., physics simulations).

### Type Specialization for Binary Operators

When both operands have a concrete native type (`Int` or `Float`), the codegen emits direct LLVM instructions:

| Operation | Int → LLVM | Float → LLVM |
|-----------|-----------|-------------|
| `+` | `add i64` | `fadd double` |
| `-` | `sub i64` | `fsub double` |
| `*` | `mul i64` | `fmul double` |
| `/` | `sitofp` → `fdiv` (always float) | `fdiv double` |
| `//` | `call @_tl_idiv` | `fdiv` → `floor` |
| `%` | `call @_tl_imod` | `fsub` / `floor` / `fmul` |
| `**` | (fallback) | `call @llvm.pow.f64` |
| `==`, `<`, etc. | `icmp s{eq,lt,le,gt,ge}` | `fcmp o{eq,lt,le,gt,ge}` |
| bitwise | `and/or/xor/shl/ashr i64` | (fallback) |

When either operand is a `Handle`, the codegen falls back to `CB_BINOP` with the appropriate opcode integer.

### Generator Functions (`gen f(...)`)

Generator bodies compiled natively use an **eager-accumulator** strategy:
- A list alloca is pre-allocated at function entry.
- Each `yield` statement appends to that list via `CB_LIST_APPEND`.
- The function returns the accumulated list as an `i64` handle.

This differs from the interpreter's lazy generator protocol. The compiled `gen` returns a complete list in one call rather than yielding values one at a time.

### Method Symbol Naming

Class methods are exported using a name-mangled symbol:

```
{ClassName}__{method_name}
```

For example, `class Vec2D` → `fn dot` is exported as `Vec2D__dot_impl` / `Vec2D__dot_tl`.

### Intra-module Direct Calls

When one eligible function calls another eligible function in the same module, the codegen emits a direct `call` to `@callee_impl` instead of routing through `CB_CALL_FN` or `CB_CALL_METHOD`. This eliminates the callback overhead for intra-module calls.

For `let` (immutable) parameters, the codegen emits a `CB_DEEP_COPY` before the call, matching the interpreter's immutable-argument semantics.

Direct calls (functions and methods, every return type) are wrapped with `CB_ARENA_SAVE` / `CB_ARENA_COMPACT`
to reclaim temporaries created inside the callee **only when the callee can push** (`pushing_fns`, a fixpoint
over the module — native_object_access_plan.md 9-2, copy_alloc_speed_plan.md 7-12) or an operand was copied for
a `let` parameter. A callee that pushes nothing (pure-numeric `int` functions, methods that only write numeric
fields and return `None`, functions returning an argument) gets a bare `call`; a returned handle is then a
pre-existing slot. When the callee pushes nothing but an operand *may* be copied, the save point is taken lazily
inside the copying slow path (`CallSave::Lazy`) and the rewind after the call runs only if it was taken — the
read-only fast path (7-7) pays nothing. A nested native call (compiled code → callback → user code → compiled
function) cuts the arena back to its own entry when it returns (`leave_native_call`, known_bugs.md #25), so
callbacks not counted as pushes cannot leak through a skipped rewind.

**Fallback regions (copy_alloc_speed_plan.md 7-17).** A slow path whose pushes are temporaries rewinds them itself
and does not count as a push site, so a function that pushes only on slow paths stays out of `pushing_fns`
(`begin_fallback_region` / `end_fallback_region`: `CB_ARENA_SAVE` at the start, `CB_ARENA_COMPACT(0, save)` at the
end, `push_sites` restored). Used for: the miss of a nested receiver's direct read/write (`BlockSrc::Field`), the miss
of a `_blk` call whose result is a raw `int`/`float` or unused, the by-name branch of a guarded call when the result
is a raw number (`emit_guarded_method_call_typed`) or unused (a statement expression — `discarded_func` marks the
outermost call's `func` node), and the miss of a checked nested assignment (below). A region that stored a handle
into a local (`handle_store_log` grew) is not rewound and counts as before. A handle result that is used is never
inside a region.

**Checked nested assignment (7-17 ②).** `a.b.x = <expr>` where `a` is a local, `a.b` a raw-layout class and `<expr>`
only literals, numeric locals and reads `L.f` / `L.c.f` of raw-layout numeric fields combined with `+ - *`
(`checked_num_ty`): the target's and every read receiver's block is fetched first (`CB_ATTR_BLOCK` /
`CB_INST_BLOCK`, remembered addresses reused), then one check per block — non-null and `(head & mask) == init bits`
with the read fields' init bits plus `IMMUTABLE` for the target — then the fast path reads, computes and writes with
no call (setting the target's init bit). Miss → fill the empty nested IC cells, then the old path (`a.b` pushed →
value → `emit_field_assign`) inside a fallback region. The fetch callbacks run no user code, so fetching before the
value is not observable. (`emit_nested_field_assign`, 8-7, still covers values that read no field.)

### Loop Rewinds (native_object_access_plan.md 9-1 / copy_alloc_speed_plan.md 7-15)

A loop whose body can push to the arena (`push_sites` grew) takes a save point just before the loop (inserted at
`loop_pre_pos`) and rewinds to it every round (`gen_loop_body`). Locals that the body stored handles into
(`handle_store_log`) are kept: the latch calls `CB_COMPACT_MANY`, which re-pushes them and writes the new handles back.
When nothing is kept, a `for` over an iterator merges the rewind into the next round's fetch: the header calls
`CB_ITER_NEXT_AT` (field 84) with a per-loop slot that is reset to -1 on entry and set to the save point only when the
loop rewinds (`new_loop_fuse_slot` / `loop_fuse_slot`), so one callback per round instead of two. `break` skips the
header, so the last round's values stay until the enclosing rewind (as before). The fetch moves the element out of the
iterator's snapshot (`NativeIter::Items`) instead of cloning it.

**Handle runs (copy_alloc_speed_plan.md 7-20).** A `for` (statement or expression) starts with `CB_ITER_BEGIN`
(field 87, `emit_for_begin`) instead of `iter_from`. For the types that are copied when the loop starts (list / tuple /
str / dict keys / fixed_list — the VM also iterates over that copy) it pushes **every element to the arena** at once
(`push_run`: `fixed_handle` gives `None` / bools / small ints their fixed handles, the rest are written straight into
the arena's spare capacity — list elements by `write_clone`, which clones instances / strs / numbers in place, 7-21)
and writes the address and count of the handle
array (`NativeIter::Run`, alive until `iter_drop`) into a `[2 x i64]` scratch; the header (`emit_for_next`) then reads
`run[i]` per round with no callback. Only when the slot holds a save point (body pushes, nothing kept) does the run path
call `CB_ARENA_COMPACT(0, save)` first. Other iterables (set, generators, `__iter__`) return count -1 and keep the
`CB_ITER_NEXT_AT` path. ⚠ The loop's save point is taken after `CB_ITER_BEGIN`, so the run's slots sit below it and
survive every per-round rewind; they go with the enclosing rewind. ⚠ With no callback at the header, a remembered block
of the loop variable is dropped only because the header's `store i64 %h, ptr %_al_x` matches `finish_block_cache`'s
store pattern — keep that store in that exact form. ⚠ Like `range` loops and `while`, the run path does not look at a
pending error per round: after an error the remaining rounds run, but every callback is a no-op and block fetches
return null, so nothing is written.

### Compound Assignment of Numeric Fields (copy_alloc_speed_plan.md 7-16)

`x.f op= v` on a numeric field of a class *without* raw layout (raw layouts read/write the block directly, 8-3/8-4)
uses `CB_NUM_FIELD_PTR` (field 85): after the right-hand side is evaluated, one callback returns the address of the
`i64` / `f64` inside the slot's `Value::Int` / `Value::Float` — only on an IC hit (public field), for an initialized
slot whose mutable flag is set (a `let` field or a frozen instance has it cleared) and whose value matches the field
kind. The generated code loads, applies the op and stores with no call in between. Null → the old path (typed read
with the *same* IC cell, so a miss fills it, then `emit_field_assign`). Only when the receiver is a name (the VM
evaluates a non-name receiver twice) and the op keeps the field's type (`int`: `+ - *` with an `int` rhs; `float`:
`+ - * /`) — `fused_compound_ok` / `emit_fused_num_compound`.

### Remembered Block Addresses (copy_alloc_speed_plan.md 8-5 / 7-13)

Numeric fields of this module's raw-layout classes are read and written at the instance's block address
(`CB_INST_BLOCK` field 78 for a handle, `CB_ATTR_BLOCK` field 79 for a nested receiver `a.b` without pushing `a.b`).
The address is remembered per function in an alloca and fetched only when the alloca is null:
`%_ip_x` per local variable (8-5) and `%_apN` per (parent variable, attribute) for nested receivers (7-13,
`cacheable_field`). Only a value freshly loaded from the variable's alloca is remembered (`fresh_var_of`).
Sites that need several nested receivers of the same parent variable at once — a `_blk` call's receiver and
arguments (`self.pos.add_scaled(self.vel, dt)`) and a checked nested assignment's target and reads — go through
`emit_block_srcs` (7-18): when any of the group's slots is null, one `CB_ATTR_BLOCKS` call (field 86,
`void(i64 parent, i32 n, ptr reqs, ptr out)`) walks the parent once and fills every slot; `reqs` is a DLL constant
`@_abN = [n x { ptr name, i64 len, ptr ic_cell, ptr desc, ptr class_cell }]`, and each entry's result equals
`CB_ATTR_BLOCK` with the same arguments (a miss stays null, so the next site fetches it alone). The same attribute
twice, different parents, or a slot remembered under another class are fetched one by one.
`finish_block_cache` scans the finished body and inserts `store ptr null` after (1) every store to the variable
and (2) **every `call` except `CB_INST_BLOCK` / `CB_ATTR_BLOCK` / `CB_ATTR_BLOCKS` / `CB_ARENA_SAVE` / `@X_blk` /
`@llvm.*`** (`cb_field_of_call`, which reads the callback index from the `getelementptr inbounds %ArCallbacks` line;
unknown callees count as "other"). (2) keeps the nested child alive and current (only calls can replace a parent's
field) and keeps the error guard: the fetch callbacks return null when an error is pending, which sends the code to
the slow path that does nothing — a remembered address used after another call would skip that guard (known_bugs.md
#26). `CB_ARENA_SAVE` only reads the arena length, and a `_blk` body calls nothing and writes only numeric fields of
the blocks it was given (7-19).

**Handing blocks to a callee (7-19).** A `mut self` method whose body remembers `self.<attr>` blocks is emitted as
`@X_pre(<params>, ptr %_pb0, ..)`: the slots for those `(attr, class)` pairs start from the parameters instead of
null (a `; @pre-blocks` marker after the parameter stores is replaced once the slots are known), and `@X_impl`
becomes a one-line wrapper that calls `@X_pre` with null pointers (so `_tl` and every other caller are unchanged).
The list per method (`pre_lists`) is what the previous round of the `generate_llvm_module` fixpoint found
(`pre_found` — it depends only on the callee's own body, so it settles one round later). A direct call `self.m(..)`
— the receiver is the fresh value of `%_al_self` and `m` takes `mut self` (no copy) — loads the caller's slots for
the same pairs right before the call (`call_impl`; an intervening call has already nulled them) and calls `@m_pre`.
Other receivers use `@m_impl`. `native_ir_abi_check.py` checks `_pre` calls too.

### Short-circuit Operators

`and` and `or` generate proper LLVM basic-block structure:

```
and:  eval left → if falsy, skip right → store result
or:   eval left → if truthy, skip right → store result
```

### Control-flow Expression Code Generation

`Expr::Block`, `Expr::IfExpr`, `Expr::ForExpr`, `Expr::WhileExpr`, `Expr::MatchExpr` all generate a result alloca at function entry and a merge label at the exit. `block_return` stores into the result alloca and branches to the merge label. `loop_yield` appends to a list alloca (pre-allocated when `body_has_loop_yield` is true).

---

## Stub Generator (`stub_gen.rs`)

`generate_stub(stmts)` walks top-level statements and emits valid `.ar` syntax with `...` bodies.

### What is preserved in stubs

| Source construct | Stub output |
|-----------------|-------------|
| `fn f(x: int) -> float` | `fn f(x: int) -> float:\n    ...` |
| `gen g(x)` | `gen g(x):\n    ...` |
| `class C(Base)->C` | Full class stub with fields and method signatures |
| `trait T` | Trait stub with method signatures |
| `new_type N: Original` | `new_type N: Original` |
| `enum E` | Enum stub with variant values |
| Parameter defaults | `= ...` |
| Template params | `[T, U: Constraint]` |
| Access sections | `public:` / `private:` / `protected:` headers |

Top-level variable declarations and executable statements are silently skipped (not visible from outside the module).

Access section headers are suppressed when all members in the class/trait are `public` — the section markers add no information in that case.

---

## clang Invocation (`module_compiler.rs` — `invoke_clang`)

```
clang -O3 -shared -o <output.dll> <input.ll>   [+ -Wno-dll-attribute-on-redeclaration on Windows]
                                               [+ -fPIC on Linux/macOS]
```

`clang` is located by:
1. Calling `clang --version` — if it exits 0, use `clang` from `PATH`.
2. Otherwise, reading `ar_config.json` → `llvm.path` → `<path>/bin/clang[.exe]`.

If neither is found, native compilation is skipped and a v0 `.arc` is written.

---

## `import[rs]` — Rust Crate Loader (`rs_loader.rs`)

`import[rs] some_crate` (or `import[rs "1.2"] some_crate` for version pinning) loads a native Rust crate at import time, without a pre-compiled `.arc`. For the module-loading side of this (`load_rs_module`, cache keys, dispatch table), see the `importation` skill.

### Steps

1. **Locate source** — reads `ar_config.json` → `rust.crates_path` (string or array of strings). Finds the crate directory in the Cargo registry cache.
2. **Scan signatures** — walks all `.rs` files, collecting `pub fn` and `pub struct` + `impl` blocks whose types are ABI-compatible.
3. **ABI-compatible types** — `i*/u*`, `f32`, `f64`, `bool`, `String`, `&str`, `&[u8]`, `Vec<u8>`, `[u8; N]`.
4. **RustCrypto Digest pattern** — if the crate re-exports `digest::Digest` and defines `pub type` aliases, synthesises one-shot hash functions (`sha256(input: str) -> str` returning lowercase hex).
5. **Generate wrapper** — writes a temporary Cargo project (`ar_rs_{stem}/`) with a `lib.rs` containing `#[no_mangle] pub unsafe extern "C" fn {name}_tl(args, n) -> i64` wrappers for every compatible function, and struct arenas backed by `OnceLock<Mutex<HashMap<i64, T>>>`.
6. **Compile** — runs `cargo build --release`.
7. **Cache** — reads the resulting `.dll`/`.so`/`.dylib` bytes, calls `cache_native(module_name, exports, dll_bytes)`, then cleans up the temp directory.
8. **Return stubs** — returns synthesised `Stmt::FnDef` / `Stmt::ClassDef` nodes for the type checker and interpreter to use.

### Struct ABI

Each Rust struct is exposed as a Arrow class with:
- A `__rs_handle__: mut int` field holding the arena key
- Public fields mirroring the Rust struct (get/set via separate `get_{field}` / `set_{field}` methods)
- An `__init__` method that stores the instance in the arena and calls `ar_init`
- A `drop` method that removes it from the arena

---

## Handle Constants

These integer sentinels are shared between native code and the interpreter:

| Value | Meaning |
|-------|---------|
| `0` | `None` |
| `1` | `True` |
| `2` | `False` |
| `-1` | `StopIteration` |
| `-2` | `TL_EXCEPTION` (returned by a handle-returning `_impl` that raised; **not** how exceptions are detected) |
| `≥ 3` | Dynamic value stored in `VALUE_ARENA` |

### Exception propagation (known_bugs.md #27 / #28)

A compiled `raise` calls `CB_RAISE`, which stores the exception in the thread's native-call state (`pending_raise`), then
returns at once in the function's own return type (`emit_exception_return`: `ret double 0.0` for `float`, `ret i64 -2`
otherwise). Receivers look at the **state**, never at the return value:

- **Interpreter entries** (`eval/native.rs`, both function paths) call `take_error_or_raise()` after every native call
  (one state read; the formatting path is `#[cold]`). The method entry (`call_native_method_at`) checks
  `take_error` then `take_pending_raise`, as it always did. ⚠ Before #28 the function entries looked at the pending
  raise only when the result was `-2`, so an `int`-returning function (its `_tl` boxes the raw `-2`) returned `-2`.
- **Direct calls between compiled functions** (`call_impl` / `_pre` / `_fast`): if the callee may raise, the caller
  calls `CB_EXC_PENDING` (field 88) right after the call (after `rewind_after_call`, so the rewind's `phi` keeps its
  blocks) and returns at once when it is set (`check_exc_after_call`). "May raise" (`failing_fns`) is a fixpoint in
  `generate_llvm_module`, next to `pushing_fns`: a function whose body has `raise`, or that directly calls a may-raise
  function. `_fast` bodies are tracked separately (key `<sym>#fast`). Functions with no `raise` reachable through
  direct calls get no check (the `speed_modes` kernels' IR is byte-identical).
- ⚠ Errors stored by a **callback** (not a `raise`) are deliberately not counted: as before, later callbacks do nothing
  and the entry returns the error, but the compiled code keeps running until it returns (direct memory writes still
  happen). Counting every callback put a check after almost every call and slowed k1_step / k2_energy by 8–12%
  (known_bugs.md #38).
- typed variants (`_typed`, zero-TLS) never call `_impl`; they propagate through the `ErrSlot` status.

---

## Speedup Summary

| Workload | Compiled vs. interpreted |
|----------|-------------------------|
| Pure `int`/`float` loop | 100–200× (direct LLVM arithmetic) |
| Class-instance methods with typed fields + `_fast` | 10–50× (zero field-read callbacks) |
| Handle-heavy workloads (lists, dicts, mixed types) | 2–5× (call overhead eliminated) |

The primary bottleneck in handle-heavy code is the callback ABI itself (GEP + load + indirect call per operation). The `_fast` variant and approach-1 pre-reads eliminate this for typed class arithmetic.
