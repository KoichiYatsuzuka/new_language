# Interpreter — Implementation Reference

`src/interpreter/` is a tree-walk interpreter.  
**Input**: `Vec<Stmt>` AST  
**Output**: side effects + final `Value`

Execution is split across two entry points: `exec(stmt)` for statements and `eval(expr)` for expressions.

---

## Module Map

```
src/interpreter/
├── mod.rs         — Interpreter struct, Value enum, Var, all shared type definitions
├── eval.rs        — expr evaluation: eval() + attr_assign()
├── exec.rs        — stmt execution: exec() dispatch + all statement handlers
├── functions.rs   — fn/gen call, argument binding, overload dispatch, auto-cast
├── classes.rs     — class instantiation, method lookup, built-in type method dispatch
├── ops.rs         — is_truthy, type_name, display/repr, apply_unary, apply_binop, values_eq
├── str_methods.rs — str method dispatch (split, join, format, regex, ...)
├── scope.rs       — push/pop scope, get_var, declare_var, assign_var, freeze_var
├── exceptions.rs  — make_error_class, get_context_lines, exc_matches
├── async_mgr.rs   — AsyncManagerData, AsyncStatus, thread spawning via std::thread
├── native_api.rs  — i64 handle arena, HvCallbacks struct, C callback implementations
├── py_interop.rs  — PyO3 runtime bridge (import[py-int] execution)
├── debugger.rs    — break_point REPL, step mode, DBG_MODE thread-local
├── msvc_errors.rs — MSVC-style error message formatting
└── tests.rs       — interpreter integration tests
```

---

## Interpreter Struct (`mod.rs`)

```rust
pub struct Interpreter {
    scopes:         Vec<HashMap<String, Var>>,   // lexical scope stack
    module_cache:   HashMap<(String, PathBuf), ModuleState>,
    static_cells:   HashMap<(String, u32, u32), Rc<RefCell<Value>>>, // static mut keyed by Span
    current_class:  Option<String>,              // set/restored around method calls
    call_stack:     Vec<StackFrame>,             // traceback
    jit_handles:    Vec<...>,                    // keeps JIT engines alive
    python_search_dirs: Vec<PathBuf>,
    ...
}
```

`Value` enum covers: `Int(i64)`, `UInt(u64)`, `Float(f64)`, `Bool(bool)`, `Str(String)`, `None`, `List(Rc<RefCell<Vec<Value>>>)`, `Dict(Rc<RefCell<DictData>>)`, `Tuple(Rc<TupleData>)`, `Set(Rc<RefCell<Vec<Value>>>)`, `Slice(SliceValue)`, `Function(Rc<FnValue>)`, `OverloadedFn(...)`, `GeneratorFn(...)`, `Generator(Rc<RefCell<GeneratorState>>)`, `Class(Rc<ClassValue>)`, `Instance(Rc<RefCell<InstanceData>>)`, `Namespace(Rc<NamespaceData>)`, `NativeFn(Rc<NativeFnRef>)`, `PyObject(...)`, `...`

`Var` wraps a `Value` plus a mutability flag; mutable variables may store a shared `Rc<RefCell<Value>>` cell for closure capture.

---

## Scope (`scope.rs`)

`scopes` is a `Vec<HashMap<String, Var>>`. Index 0 is global; the tail is the innermost local scope.

| Op | Behaviour |
|----|-----------|
| `push_scope` | Appends a new empty HashMap |
| `pop_scope` | Removes the tail (never removes index 0) |
| `get_var(name)` | Searches tail → head (lexical lookup) |
| `declare_var` | Inserts into `scopes.last_mut()` |
| `assign_var` | Searches tail → head; error if not found or immutable |
| `freeze_var` | Sets mutability flag to false; error if captured by closure |

---

## Statement Execution (`exec.rs`)

`exec(stmt)` is a `match` dispatcher. Key handlers:

| Stmt | Handler | Notes |
|------|---------|-------|
| `Let` / `Const` | `exec_let` | `let` deep-copies mutable values |
| `Mut` | inline | Always `deep_copy_value` |
| `Static` | `exec_static_var` | Cell keyed by `(name, line, col)`; shared across all calls |
| `Assign` / `AttrAssign` | inline / `attr_assign` | Mutability checked |
| `If` | `exec_if_stmt` | Evaluates branches in order |
| `While` / `For` | `exec_while_stmt` / `exec_for_stmt` | Increment/decrement `LOOP_DEPTH` |
| `Match` | `exec_match_stmt` | Value-case or type-pattern arms |
| `Try` | `exec_try` | Catches `RAISE_SENTINEL` error strings; finally always runs |
| `Raise` | `exec_raise` | Formats error, sets `code_context/file/line/col` on instance |
| `FnDef` / `GenDef` | `exec_fn_def` / `exec_gen_def` | Captures closure env at definition time |
| `ClassDef` / `TraitDef` | `exec_class_def` / `exec_trait_def` | Builds `ClassValue` / registers trait |
| `Import` / `FromImport` | `exec_module` | See module execution below |
| `AsyncAssign` | `exec_async_assign` | Submits task to `AsyncManagerData` |
| `BreakPoint` | `exec_breakpoint` | Enters debugger REPL |
| `BlockReturn` | returns `RAISE_SENTINEL` signal | Caught by enclosing block/if/for/while/match expression handler |
| `LoopYield` | appends to `BLOCK_YIELDS` thread-local | Does not interrupt control flow |
| `Break` | returns `ExecResult::Break` or `BREAK_SENTINEL` | Propagates through eval() channels via sentinel string |

### Control-flow signals (thread-locals)

| Signal | Mechanism |
|--------|-----------|
| `return` | `ExecResult::Return(Value)` |
| `break` | `ExecResult::Break` in loops; `BREAK_SENTINEL` error when propagating through `eval()` |
| `block_return` | `RAISE_SENTINEL` with encoded value; caught by expression handlers |
| `loop_yield` | Appends to `BLOCK_YIELDS: Option<Vec<Value>>`; set `Some` inside for/while-expr |
| `raise` | `RAISE_SENTINEL` error string `"\x00__raise__:..."` propagates up call stack |
| `LOOP_DEPTH` | Incremented per for/while (stmt or expr); reset to 0 on function entry |

---

## Expression Evaluation (`eval.rs`)

`eval(expr)` walks `Expr` recursively. Key cases:

- **Identifiers**: `get_val(name)` — lexical scope lookup
- **Literals**: int, float, str, bool, None, f-string (re-lexed inline)
- **Collections**: list `[...]`, dict `{k:v}`, set `{v}`, tuple `(a,b)`
- **BinOp**: delegated to `apply_binop` (ops.rs); handles `in`/`not in` via collection membership
- **UnaryOp**: delegated to `apply_unary`
- **Call**: evaluates callee + args, dispatches to `exec_fn_evaled` / `instantiate` / native / PyO3
- **Attr**: field lookup on `Instance`, `Namespace`, `Class`, or built-in type
- **Subscript / Slice**: list/str/tuple/dict indexing; slice object construction
- **IsType**: runtime `isinstance`-equivalent against class name, trait, primitive type, `function`
- **If/For/While/Match/Block expressions**: run the body with `BLOCK_YIELDS` and `BLOCK_RETURN` machinery; return accumulated list or `block_return` value

---

## Functions (`functions.rs`)

### Call flow (`exec_fn_evaled`)

1. Evaluate default parameter expressions in the caller's scope.
2. `bind_args` — map positional/keyword args onto parameter list; fill defaults; check mutability (`mut` param requires mutable argument).
3. Save and clear non-global scopes (`scopes.truncate(1)`); push a fresh local scope.
4. Bind `self` if present; set `current_class` for access control.
5. Execute function body; catch `Return` signal.
6. Restore saved scopes.
7. If exception propagating, append a `StackFrame` to `call_stack`.

### Closures

- At `FnDef` execution time, the current scope is scanned for free variables referenced by the body.
- `let` vars → deep-cloned into `captured_env: HashMap<String, Value>`.
- `mut` vars → `Rc<RefCell<Value>>` cell cloned into `captured_env`; outer scope and closure share the same cell.
- `static mut` → single cell in `Interpreter::static_cells` keyed by `(name, line, col)`.

### Overload dispatch

`Value::OverloadedFn` holds multiple `FnValue` variants. `dispatch_overload` scores each candidate by matching argument types against `type_ann` on parameters; the best-scoring match wins.

### Auto-cast

If a `let` parameter has a type annotation and the argument is an instance of a different class that defines `__cast__[TypeName]`, the interpreter automatically calls the cast method before binding.

---

## Classes (`classes.rs`)

### Instantiation (`instantiate`)

1. Create `InstanceData { class, fields: HashMap, field_access: HashMap }`.
2. Initialise fields from `ClassDef` field declarations (evaluate defaults).
3. Copy `field_access` map from class + inherited traits (namespaced as `"TraitName::field"`).
4. Call `__init__` if defined.

### Method lookup (`lookup_method_in_class`)

Searches: own class methods → base class methods → trait methods (breadth-first through `bases`).  
`current_class` on `Interpreter` is set before and restored after each method call for access-control checks.

### Access control

`field_access: HashMap<String, Accessibility>` on `InstanceData`. At read/write time, the accessor checks whether the current `current_class` is permitted:

- `Public` → always allowed
- `Private` → only the owning class name matches
- `Protected` → owning class or any class sharing the same trait

Violation raises `AccessError`.

---

## Operators (`ops.rs`)

`apply_binop(op, lhs, rhs)` covers all arithmetic, comparison, bitwise, logical, string, list, set operators. Key points:

- Integer division `//` uses `floor_div` (Python semantics, not truncation).
- `**` promotes `Int` to `Float` when the exponent is negative.
- `+` on lists clones both sides into a new list.
- `|`, `&`, `-`, `^` on `Set` produce new sets.
- `in` / `not in` dispatches per collection type (list linear scan, set dedup scan, dict key scan, str `contains`).

`is_truthy` follows Python rules: `0`, `0.0`, `""`, `None`, empty collections → false.

---

## Exceptions (`exceptions.rs`)

Built-in exception classes (`ValueError`, `TypeError`, `IndexError`, `KeyError`, `RuntimeError`, `OSError`, `StopIteration`, `AccessError`, …) are constructed by `make_error_class` as `ClassValue` objects pre-registered in global scope.

Exception propagation uses the error string `RAISE_SENTINEL = "\x00__raise__:..."` threading through Rust's `Result<_, String>`. `exec_try` catches any error containing the sentinel, matches the class name against `except` handlers via `exc_matches`, runs the matching handler, and always runs `finally`.

`get_context_lines` extracts source lines around the raise site from the `code_context` stored on the exception instance.

---

## Async (`async_mgr.rs`)

`AsyncManagerData` manages a bounded pool of OS threads.

- `exec_async_assign` clones the current scope into `SendableEnv` (deep-copy crossing `Send` boundary).
- `mut` variables are captured by Rc-clone before deep-copy so mutations propagate back to the caller.
- Each task body runs in `std::thread::spawn`; result is returned via `mpsc::channel`.
- `try_schedule` starts threads up to `num_thread`; `poll_completed` harvests `try_recv`.
- `wait_for_finish` loops with a sleep interval; propagates first error if `raise_immediately`.

---

## Native API (`native_api.rs`)

All values crossing the ABI boundary are `i64` handles into a thread-local `VALUE_ARENA: Vec<Value>`.

| Handle | Meaning |
|--------|---------|
| `0` (`TL_NONE`) | `Value::None` |
| `1` (`TL_TRUE`) | `Value::Bool(true)` |
| `2` (`TL_FALSE`) | `Value::Bool(false)` |
| `-1` (`TL_STOP_ITER`) | iteration exhausted |
| `-2` (`TL_EXCEPTION`) | exception raised |
| `>= 3` | index into `VALUE_ARENA` |

`HvCallbacks` is a `#[repr(C)]` struct of function pointers covering `make_int`, `make_str`, `call_fn`, `get_attr`, `set_attr`, `binop`, `iter_from`, `iter_next`, `raise_exc`, etc.  
The struct pointer is passed to each native DLL via `hv_init(cb)`.  
`CURRENT_INTERP` is a thread-local `*mut Interpreter` set before calling into native code.

---

## Key File Locations

| Subject | File | Notes |
|---------|------|-------|
| Value enum + Interpreter struct | `src/interpreter/mod.rs` | All shared types |
| Statement dispatch | `src/interpreter/exec.rs` | `exec()` + all `exec_*` methods |
| Expression evaluation | `src/interpreter/eval.rs` | `eval()` + slice helpers |
| Function call + closure | `src/interpreter/functions.rs` | `exec_fn_evaled`, `bind_args`, closure capture |
| Class instantiation + method dispatch | `src/interpreter/classes.rs` | `instantiate`, `lookup_method_in_class` |
| Operators + display | `src/interpreter/ops.rs` | `apply_binop`, `is_truthy`, `type_name` |
| Lexical scope | `src/interpreter/scope.rs` | `push/pop_scope`, `assign_var` |
| Exception machinery | `src/interpreter/exceptions.rs` | `make_error_class`, sentinel string |
| Async task threads | `src/interpreter/async_mgr.rs` | `AsyncManagerData`, `SendableEnv` |
| Native ABI handle table | `src/interpreter/native_api.rs` | `VALUE_ARENA`, `HvCallbacks` |
| PyO3 interop | `src/interpreter/py_interop.rs` | `load_py_int_module`, value conversion |
| Debugger | `src/interpreter/debugger.rs` | `break_point`, step mode |
