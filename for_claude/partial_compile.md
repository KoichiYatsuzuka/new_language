# Partial Compile — Implementation Reference

This document describes the partial compile subsystem (`src/partial_compiler/`) in detail, derived from the actual source code.

---

## Overview

`--compile <file.hv>` takes a parsed Havakyrie module and produces two output files:

| File | Purpose |
|------|---------|
| `{stem}.hvc` | Compiled module — binary blob consumed by the importer at runtime |
| `{stem}.hvs` | Type stub — text read by the type checker and VS Code extension |

The compiler selects from three code paths, in priority order:

1. **inkwell JIT** (requires `feature = "llvm"`) — emits LLVM bitcode; no external tools
2. **clang fallback** — emits LLVM IR text, then shells out to `clang -O3 -shared`
3. **Source-only** — writes a v0 `.hvc` (no native code) with a warning

---

## Module Map

```
src/partial_compiler/
├── mod.rs            — public re-exports: compile, load_tlc, take_native_bytes, NativePayload
├── module_compiler.rs — .hvc file format writer/reader + thread-local native cache
├── llvm_codegen.rs   — LLVM IR text generator (clang fallback path)
├── inkwell_codegen.rs — inkwell JIT compiler (feature = "llvm", primary path)
├── stub_gen.rs       — .hvs stub text generator
└── rs_loader.rs      — import[rs] native Rust crate loader
```

---

## The `.hvc` Binary Format

`MAGIC = b"TLC\x00"` — present at byte 0 of every valid `.hvc` file.

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

### Version 2 — LLVM bitcode embedded

Identical wire layout to v1 — same export table encoding — but the payload bytes are LLVM bitcode instead of a native DLL. The inkwell path re-JITs them in-process at import time (no temp file needed).

---

## Compilation Pipeline (`module_compiler.rs`)

### `compile(source, stmts, source_path)`

Entry point called by `main.rs` when `--compile` is given.

1. **Stub** — always runs first: calls `stub_gen::generate_stub(stmts)` and writes `{stem}.hvs`.
2. **Native** — calls `compile_native(stmts)`:
   - If `feature = "llvm"`: tries `inkwell_codegen::get_bitcode(stmts)` → `NativePayload::Bitcode`
   - Falls back to `llvm_codegen::generate_llvm_module(stmts)` → `clang` → `NativePayload::Dll`
3. **Write** — calls `write_tlc_v2` / `write_tlc_v1` / `write_tlc_v0` depending on outcome.
4. Returns `(hvc_path, hvs_path)`.

### `load_tlc(path)`

Called by the parser/importer when a `.hvc` file is found.

1. Reads and parses binary data via `parse_tlc`.
2. If v1 or v2: inserts `(exports, NativePayload)` into `NATIVE_CACHE`.
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
%HvCallbacks = type { ptr, ptr, ..., ptr }   ; 35 function-pointer fields

@CB = internal global ptr null

define void @hv_init(ptr %cb) { store ptr %cb, ptr @CB; ret void }

define internal i64 @_tl_idiv(i64 %a, i64 %b) { ... }   ; Python floor-div
define internal i64 @_tl_imod(i64 %a, i64 %b) { ... }   ; Python modulo

declare double @llvm.pow.f64(double, double)
declare double @llvm.floor.f64(double)
```

`hv_init` is called by the interpreter at module load time to inject the `HvCallbacks` pointer.

### HvCallbacks — Field Index Table

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

### Generated Function Variants

For each eligible function `f`, the codegen emits up to three LLVM functions:

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

For method calls, the codegen additionally wraps the call with `CB_ARENA_SAVE` / `CB_ARENA_COMPACT` to safely reclaim temporaries created inside the callee.

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

`generate_stub(stmts)` walks top-level statements and emits valid `.hv` syntax with `...` bodies.

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
2. Otherwise, reading `hv_config.json` → `llvm.path` → `<path>/bin/clang[.exe]`.

If neither is found, native compilation is skipped and a v0 `.hvc` is written.

---

## `import[rs]` — Rust Crate Loader (`rs_loader.rs`)

`import[rs] some_crate` (or `import[rs "1.2"] some_crate` for version pinning) loads a native Rust crate at import time, without a pre-compiled `.hvc`.

### Steps

1. **Locate source** — reads `hv_config.json` → `rust.crates_path` (string or array of strings). Finds the crate directory in the Cargo registry cache.
2. **Scan signatures** — walks all `.rs` files, collecting `pub fn` and `pub struct` + `impl` blocks whose types are ABI-compatible.
3. **ABI-compatible types** — `i*/u*`, `f32`, `f64`, `bool`, `String`, `&str`, `&[u8]`, `Vec<u8>`, `[u8; N]`.
4. **RustCrypto Digest pattern** — if the crate re-exports `digest::Digest` and defines `pub type` aliases, synthesises one-shot hash functions (`sha256(input: str) -> str` returning lowercase hex).
5. **Generate wrapper** — writes a temporary Cargo project (`hv_rs_{stem}/`) with a `lib.rs` containing `#[no_mangle] pub unsafe extern "C" fn {name}_tl(args, n) -> i64` wrappers for every compatible function, and struct arenas backed by `OnceLock<Mutex<HashMap<i64, T>>>`.
6. **Compile** — runs `cargo build --release`.
7. **Cache** — reads the resulting `.dll`/`.so`/`.dylib` bytes, calls `cache_native(module_name, exports, dll_bytes)`, then cleans up the temp directory.
8. **Return stubs** — returns synthesised `Stmt::FnDef` / `Stmt::ClassDef` nodes for the type checker and interpreter to use.

### Struct ABI

Each Rust struct is exposed as a Havakyrie class with:
- A `__rs_handle__: mut int` field holding the arena key
- Public fields mirroring the Rust struct (get/set via separate `get_{field}` / `set_{field}` methods)
- An `__init__` method that stores the instance in the arena and calls `hv_init`
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
| `-2` | `TL_EXCEPTION` (exception propagation sentinel) |
| `≥ 3` | Dynamic value stored in `VALUE_ARENA` |

---

## Speedup Summary

| Workload | Compiled vs. interpreted |
|----------|-------------------------|
| Pure `int`/`float` loop | 100–200× (direct LLVM arithmetic) |
| Class-instance methods with typed fields + `_fast` | 10–50× (zero field-read callbacks) |
| Handle-heavy workloads (lists, dicts, mixed types) | 2–5× (call overhead eliminated) |

The primary bottleneck in handle-heavy code is the callback ABI itself (GEP + load + indirect call per operation). The `_fast` variant and approach-1 pre-reads eliminate this for typed class arithmetic.
