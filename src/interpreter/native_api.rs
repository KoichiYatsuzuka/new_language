// native_api.rs — Interpreter-mediated value table and C callbacks for native compiled modules.
//
// All values crossing the native ABI boundary are represented as `i64` handles:
//   TL_NONE  (0) = None
//   TL_TRUE  (1) = Bool(true)
//   TL_FALSE (2) = Bool(false)
//   TL_STOP_ITER (-1) = iteration exhausted sentinel
//   h >= 3   = index into the thread-local VALUE_ARENA Vec<Value>
//
// The TlCallbacks struct is passed to each native DLL via `tl_init(cb)`.
// All heavy operations (attribute access, function calls, binops) route through
// the interpreter held in CURRENT_INTERP.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{Interpreter, Value, DictData, TupleData};

// ── Handle constants ─────────────────────────────────────────────────────────

pub const TL_NONE: i64 = 0;
pub const TL_TRUE: i64 = 1;
pub const TL_FALSE: i64 = 2;
pub const TL_STOP_ITER: i64 = -1;

// ── BinOp codes (must mirror codegen.rs constants) ──────────────────────────

pub const OP_ADD: i32 = 0;
pub const OP_SUB: i32 = 1;
pub const OP_MUL: i32 = 2;
pub const OP_DIV: i32 = 3;
pub const OP_FLOOR_DIV: i32 = 4;
pub const OP_MOD: i32 = 5;
pub const OP_POW: i32 = 6;
pub const OP_EQ: i32 = 7;
pub const OP_NE: i32 = 8;
pub const OP_LT: i32 = 9;
pub const OP_LE: i32 = 10;
pub const OP_GT: i32 = 11;
pub const OP_GE: i32 = 12;
pub const OP_BIT_AND: i32 = 13;
pub const OP_BIT_OR: i32 = 14;
pub const OP_BIT_XOR: i32 = 15;
pub const OP_LSHIFT: i32 = 16;
pub const OP_RSHIFT: i32 = 17;
pub const OP_IN: i32 = 18;
pub const OP_NOTIN: i32 = 19;

// ── UnaryOp codes ────────────────────────────────────────────────────────────

pub const UOP_NEG: i32 = 0;
pub const UOP_NOT: i32 = 1;
pub const UOP_BIT_NOT: i32 = 2;

// ── Thread-local state ───────────────────────────────────────────────────────

// ── Integer handle cache ─────────────────────────────────────────────────────
//
// Integers 0..INT_CACHE_END are pre-populated at fixed arena indices so that
// cb_make_int(n) for small n returns a permanent handle without any arena push.
//   arena[0]                   = None placeholder  (TL_NONE=0 accessed directly)
//   arena[1]                   = Bool(true)         (TL_TRUE=1 accessed directly)
//   arena[2]                   = Bool(false)        (TL_FALSE=2 accessed directly)
//   arena[3 + n]  (0 ≤ n < 256) = Int(n)           → handle (3 + n)
//
// So INT_CACHE_BASE = 3 + 256 = 259 = the first dynamic slot.

const INT_CACHE_END: i64 = 256;
const INT_CACHE_BASE: usize = 3 + INT_CACHE_END as usize; // 259

#[inline(always)]
fn int_cache_handle(n: i64) -> i64 {
    3 + n
}

thread_local! {
    /// Raw pointer to the currently executing Interpreter.
    /// Set by `enter_native_call` at depth 0; cleared by `exit_native_call` / `abort_native_call`.
    static CURRENT_INTERP: Cell<*mut Interpreter> = Cell::new(std::ptr::null_mut());

    /// Scratch buffers for null-terminated C strings produced by `tl_to_cstr`.
    /// Cleared at the end of the outermost native call so the pointers remain valid
    /// for the duration of any single call chain.
    static CSTR_BUFS: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());

    /// Value arena.
    /// indices 0-2: placeholder None/True/False (accessed via TL_* constants)
    /// indices 3 .. INT_CACHE_BASE-1: pre-cached Int(0)..Int(255) — permanent, never freed
    /// indices INT_CACHE_BASE+: dynamic values pushed at runtime
    static VALUE_ARENA: RefCell<Vec<Value>> = RefCell::new({
        let mut v = Vec::with_capacity(INT_CACHE_BASE + 64);
        v.push(Value::None);          // index 0
        v.push(Value::Bool(true));    // index 1
        v.push(Value::Bool(false));   // index 2
        for i in 0..INT_CACHE_END {   // indices 3..258
            v.push(Value::Int(i));
        }
        v
    });

    /// Iterator table: each entry is (collected_items, current_position).
    /// Iter handles are negative: -(idx+2), so they're all <= -2.
    static ITER_TABLE: RefCell<Vec<(Vec<Value>, usize)>> = RefCell::new(Vec::new());

    /// Error set by a callback on failure; checked after each native call.
    static NATIVE_ERROR: RefCell<Option<String>> = RefCell::new(None);

    // ── Call-frame tracking for arena cleanup ────────────────────────────────
    /// Nesting depth of native calls (0 = not inside any native call).
    static CALL_DEPTH: Cell<usize> = Cell::new(0);
    /// Arena size saved at the outermost native call entry.
    static ARENA_SAVE: Cell<usize> = Cell::new(INT_CACHE_BASE);
    /// Iter table size saved at the outermost native call entry.
    static ITER_SAVE: Cell<usize> = Cell::new(0);
}

// ── Public API for the interpreter ──────────────────────────────────────────

/// Enter a native function call.  Must be paired with `exit_native_call` or `abort_native_call`.
/// Returns `true` for the outermost call (depth was 0); `false` for re-entrant calls.
/// At the outermost level, saves the arena / iter-table savepoints and sets CURRENT_INTERP.
pub fn enter_native_call(interp: *mut Interpreter) -> bool {
    let prev = CALL_DEPTH.with(|c| { let v = c.get(); c.set(v + 1); v });
    if prev == 0 {
        CURRENT_INTERP.with(|c| c.set(interp));
        VALUE_ARENA.with(|a| ARENA_SAVE.with(|s| s.set(a.borrow().len())));
        ITER_TABLE.with(|t| ITER_SAVE.with(|s| s.set(t.borrow().len())));
        true
    } else {
        false
    }
}

/// Exit a native function call (success path).
/// Clones the result value from `result_h`, then — if outermost — truncates the arena/iter
/// table back to the saved positions and clears CURRENT_INTERP.
pub fn exit_native_call(result_h: i64, is_outermost: bool) -> Value {
    let result = clone_value_at(result_h);
    CALL_DEPTH.with(|c| { let v = c.get(); if v > 0 { c.set(v - 1); } });
    if is_outermost {
        CURRENT_INTERP.with(|c| c.set(std::ptr::null_mut()));
        VALUE_ARENA.with(|a| a.borrow_mut().truncate(ARENA_SAVE.with(|s| s.get())));
        ITER_TABLE.with(|t| t.borrow_mut().truncate(ITER_SAVE.with(|s| s.get())));
        CSTR_BUFS.with(|b| b.borrow_mut().clear());
    }
    result
}

/// Exit a native function call (error path).
/// Does the same cleanup as `exit_native_call` but returns nothing.
pub fn abort_native_call(is_outermost: bool) {
    CALL_DEPTH.with(|c| { let v = c.get(); if v > 0 { c.set(v - 1); } });
    if is_outermost {
        CURRENT_INTERP.with(|c| c.set(std::ptr::null_mut()));
        VALUE_ARENA.with(|a| a.borrow_mut().truncate(ARENA_SAVE.with(|s| s.get())));
        ITER_TABLE.with(|t| t.borrow_mut().truncate(ITER_SAVE.with(|s| s.get())));
        CSTR_BUFS.with(|b| b.borrow_mut().clear());
    }
}

/// Push a Value into the arena and return its handle.
/// Small integers (0..INT_CACHE_END) and booleans return pre-cached handles.
pub fn push_handle(v: Value) -> i64 {
    match &v {
        Value::None => TL_NONE,
        Value::Bool(true) => TL_TRUE,
        Value::Bool(false) => TL_FALSE,
        Value::Int(n) if *n >= 0 && *n < INT_CACHE_END => int_cache_handle(*n),
        _ => VALUE_ARENA.with(|a| {
            let mut arena = a.borrow_mut();
            let h = arena.len() as i64;
            arena.push(v);
            h
        }),
    }
}

/// Push a Value into the arena as a **fresh, writable slot** and return its handle.
///
/// Unlike `push_handle`, this never returns a pre-cached constant handle — it always
/// appends a new slot that the native DLL can later overwrite via `tl_write_handle`.
/// Used for `MutPtr` (write-back) arguments in the cpp-bridge.
pub fn push_handle_writeback(v: Value) -> i64 {
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(v);
        h
    })
}

/// Clone the Value at `h` from the arena.
pub fn clone_value_at(h: i64) -> Value {
    match h {
        TL_NONE => Value::None,
        TL_TRUE => Value::Bool(true),
        TL_FALSE => Value::Bool(false),
        n if n >= 3 => VALUE_ARENA.with(|a| {
            a.borrow().get(n as usize).cloned().unwrap_or(Value::None)
        }),
        _ => Value::None, // TL_STOP_ITER or invalid
    }
}

/// Take and return any error set by the last native call chain.
pub fn take_error() -> Option<String> {
    NATIVE_ERROR.with(|e| e.borrow_mut().take())
}

/// Get a `*const TlCallbacks` pointing to the static callbacks instance.
pub fn get_callbacks() -> *const TlCallbacks {
    &CALLBACKS as *const TlCallbacks
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn get_interp_ptr() -> *mut Interpreter {
    CURRENT_INTERP.with(|c| c.get())
}

fn has_error() -> bool {
    NATIVE_ERROR.with(|e| e.borrow().is_some())
}

fn set_error(msg: String) {
    NATIVE_ERROR.with(|e| *e.borrow_mut() = Some(msg));
}

fn i32_to_binop(op: i32) -> Option<crate::ast::BinOp> {
    use crate::ast::BinOp;
    match op {
        OP_ADD => Some(BinOp::Add),
        OP_SUB => Some(BinOp::Sub),
        OP_MUL => Some(BinOp::Mul),
        OP_DIV => Some(BinOp::Div),
        OP_FLOOR_DIV => Some(BinOp::FloorDiv),
        OP_MOD => Some(BinOp::Mod),
        OP_POW => Some(BinOp::Pow),
        OP_EQ => Some(BinOp::Eq),
        OP_NE => Some(BinOp::NotEq),
        OP_LT => Some(BinOp::Lt),
        OP_LE => Some(BinOp::LtEq),
        OP_GT => Some(BinOp::Gt),
        OP_GE => Some(BinOp::GtEq),
        OP_BIT_AND => Some(BinOp::BitAnd),
        OP_BIT_OR => Some(BinOp::BitOr),
        OP_BIT_XOR => Some(BinOp::BitXor),
        OP_LSHIFT => Some(BinOp::LShift),
        OP_RSHIFT => Some(BinOp::RShift),
        OP_IN     => Some(BinOp::In),
        OP_NOTIN  => Some(BinOp::NotIn),
        _ => None,
    }
}

// ── TlCallbacks struct ───────────────────────────────────────────────────────

/// C-compatible struct of function pointers passed to native DLLs via `tl_init`.
/// Layout must exactly match the `TlCallbacks` struct emitted by codegen.rs.
#[repr(C)]
pub struct TlCallbacks {
    pub make_int:   extern "C" fn(i64) -> i64,
    pub make_float: extern "C" fn(f64) -> i64,
    pub make_bool:  extern "C" fn(i32) -> i64,
    pub make_str:   extern "C" fn(*const u8, i32) -> i64,
    pub make_list:  extern "C" fn(*const i64, i32) -> i64,
    pub make_tuple: extern "C" fn(*const i64, i32) -> i64,
    pub make_dict:  extern "C" fn(*const i64, *const i64, i32) -> i64,
    pub make_none:  extern "C" fn() -> i64,
    pub is_truthy:  extern "C" fn(i64) -> i32,
    pub binop:      extern "C" fn(i32, i64, i64) -> i64,
    pub unop:       extern "C" fn(i32, i64) -> i64,
    pub call_fn:    extern "C" fn(i64, *const i64, i32) -> i64,
    pub get_attr:   extern "C" fn(i64, *const u8, i32) -> i64,
    pub set_attr:   extern "C" fn(i64, *const u8, i32, i64),
    pub subscript:  extern "C" fn(i64, i64) -> i64,
    pub get_global: extern "C" fn(*const u8, i32) -> i64,
    pub iter_from:    extern "C" fn(i64) -> i64,
    pub iter_next:    extern "C" fn(i64) -> i64,
    pub is_type:      extern "C" fn(i64, *const u8, i32) -> i64,
    pub arena_save:    extern "C" fn() -> u64,
    pub arena_compact: extern "C" fn(i64, u64) -> i64,
    pub compact_many:  extern "C" fn(*const i64, i32, u64, *mut i64),
    pub to_int:        extern "C" fn(i64) -> i64,
    pub to_float:      extern "C" fn(i64) -> f64,
    pub deep_copy:     extern "C" fn(i64) -> i64,
    /// Convert a tl string handle to a null-terminated C string pointer.
    /// The pointer is valid until the end of the outermost native call.
    pub to_cstr:       extern "C" fn(i64) -> *const u8,
    /// Overwrite arena[target_h] with a clone of the value at new_val_h.
    /// Used by cpp-bridge wrappers for T* write-back parameters.
    pub write_handle:  extern "C" fn(i64, i64),
}

// ── Callback implementations ─────────────────────────────────────────────────

extern "C" fn tl_make_int(n: i64) -> i64 {
    push_handle(Value::Int(n))
}

extern "C" fn tl_make_float(f: f64) -> i64 {
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(Value::Float(f));
        h
    })
}

extern "C" fn tl_make_bool(b: i32) -> i64 {
    if b != 0 { TL_TRUE } else { TL_FALSE }
}

extern "C" fn tl_make_str(ptr: *const u8, len: i32) -> i64 {
    let s = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize))
    }.to_owned();
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(Value::Str(s));
        h
    })
}

extern "C" fn tl_make_list(items_ptr: *const i64, n: i32) -> i64 {
    let items: Vec<Value> = (0..n as usize)
        .map(|i| clone_value_at(unsafe { *items_ptr.add(i) }))
        .collect();
    let list = Rc::new(RefCell::new(items));
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(Value::List(list));
        h
    })
}

extern "C" fn tl_make_tuple(items_ptr: *const i64, n: i32) -> i64 {
    let elements: Vec<Value> = (0..n as usize)
        .map(|i| clone_value_at(unsafe { *items_ptr.add(i) }))
        .collect();
    let tuple = Rc::new(TupleData::new(elements, vec![]));
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(Value::Tuple(tuple));
        h
    })
}

extern "C" fn tl_make_dict(keys_ptr: *const i64, vals_ptr: *const i64, n: i32) -> i64 {
    let mut dict = DictData::new("Any".to_string(), "Any".to_string());
    for i in 0..n as usize {
        let k = clone_value_at(unsafe { *keys_ptr.add(i) });
        let v = clone_value_at(unsafe { *vals_ptr.add(i) });
        dict.set(k, v);
    }
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        let h = arena.len() as i64;
        arena.push(Value::Dict(Rc::new(RefCell::new(dict))));
        h
    })
}

extern "C" fn tl_make_none() -> i64 {
    TL_NONE
}

extern "C" fn tl_is_truthy(h: i64) -> i32 {
    let v = clone_value_at(h);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        // Fallback without interpreter
        match &v {
            Value::None => 0,
            Value::Bool(b) => i32::from(*b),
            Value::Int(n) => i32::from(*n != 0),
            Value::Float(f) => i32::from(*f != 0.0),
            Value::Str(s) => i32::from(!s.is_empty()),
            _ => 1,
        }
    } else {
        let interp = unsafe { &mut *ptr };
        i32::from(interp.is_truthy(&v))
    }
}

extern "C" fn tl_binop(op: i32, a: i64, b: i64) -> i64 {
    if has_error() { return TL_NONE; }
    let ast_op = match i32_to_binop(op) {
        Some(o) => o,
        None => {
            set_error(format!("NativeError: invalid binop code {op}"));
            return TL_NONE;
        }
    };
    let lhs = clone_value_at(a);
    let rhs = clone_value_at(b);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for binop".to_string());
        return TL_NONE;
    }
    let interp = unsafe { &*ptr };
    match interp.apply_binop(&ast_op, lhs, rhs) {
        Ok(v) => push_handle(v),
        Err(e) => { set_error(e); TL_NONE }
    }
}

extern "C" fn tl_unop(op: i32, a: i64) -> i64 {
    if has_error() { return TL_NONE; }
    use crate::ast::UnaryOp;
    let ast_op = match op {
        UOP_NEG => UnaryOp::Neg,
        UOP_NOT => UnaryOp::Not,
        UOP_BIT_NOT => UnaryOp::BitNot,
        _ => {
            set_error(format!("NativeError: invalid unop code {op}"));
            return TL_NONE;
        }
    };
    let operand = clone_value_at(a);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for unop".to_string());
        return TL_NONE;
    }
    let interp = unsafe { &*ptr };
    match interp.apply_unary(&ast_op, operand) {
        Ok(v) => push_handle(v),
        Err(e) => { set_error(e); TL_NONE }
    }
}

extern "C" fn tl_call_fn(fn_h: i64, args_ptr: *const i64, n_args: i32) -> i64 {
    if has_error() { return TL_NONE; }
    let fn_val = clone_value_at(fn_h);
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for call_fn".to_string());
        return TL_NONE;
    }
    let interp = unsafe { &mut *ptr };
    match interp.call_value_with_args(fn_val, args) {
        Ok(v) => push_handle(v),
        Err(e) => { set_error(e); TL_NONE }
    }
}

extern "C" fn tl_get_attr(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    if has_error() { return TL_NONE; }
    let name = unsafe {
        std::str::from_utf8_unchecked(
            std::slice::from_raw_parts(name_ptr, name_len as usize)
        )
    }.to_owned();
    let obj = clone_value_at(obj_h);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for get_attr".to_string());
        return TL_NONE;
    }
    let interp = unsafe { &mut *ptr };
    match interp.get_attr_val(obj, &name) {
        Ok(v) => push_handle(v),
        Err(e) => { set_error(e); TL_NONE }
    }
}

extern "C" fn tl_set_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
    if has_error() { return; }
    let name = unsafe {
        std::str::from_utf8_unchecked(
            std::slice::from_raw_parts(name_ptr, name_len as usize)
        )
    }.to_owned();
    let obj = clone_value_at(obj_h);
    let val = clone_value_at(val_h);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for set_attr".to_string());
        return;
    }
    let interp = unsafe { &mut *ptr };
    if let Err(e) = interp.set_attr_val(obj, &name, val) {
        set_error(e);
    }
}

extern "C" fn tl_subscript(obj_h: i64, key_h: i64) -> i64 {
    if has_error() { return TL_NONE; }
    let obj = clone_value_at(obj_h);
    let key = clone_value_at(key_h);
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error("NativeError: interpreter not set for subscript".to_string());
        return TL_NONE;
    }
    let interp = unsafe { &mut *ptr };
    match interp.eval_subscript(obj, key) {
        Ok(v) => push_handle(v),
        Err(e) => { set_error(e); TL_NONE }
    }
}

extern "C" fn tl_get_global(name_ptr: *const u8, name_len: i32) -> i64 {
    if has_error() { return TL_NONE; }
    let name = unsafe {
        std::str::from_utf8_unchecked(
            std::slice::from_raw_parts(name_ptr, name_len as usize)
        )
    }.to_owned();
    let ptr = get_interp_ptr();
    if ptr.is_null() {
        set_error(format!("NativeError: interpreter not set (looking up '{name}')"));
        return TL_NONE;
    }
    let interp = unsafe { &mut *ptr };
    match interp.get_val(&name) {
        Some(v) => push_handle(v),
        None => {
            set_error(format!("NameError: '{name}' is not defined"));
            TL_NONE
        }
    }
}

extern "C" fn tl_iter_from(obj_h: i64) -> i64 {
    if has_error() { return TL_NONE; }
    let obj = clone_value_at(obj_h);
    let items: Vec<Value> = match obj {
        Value::List(l) => l.borrow().clone(),
        Value::Tuple(t) => t.all_values().to_vec(),
        Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
        Value::Dict(d) => d.borrow().all_keys(),
        other => {
            set_error(format!(
                "TypeError: value of type '{}' is not iterable in native context",
                match &other {
                    Value::Int(_) => "int",
                    Value::Float(_) => "float",
                    Value::Bool(_) => "bool",
                    Value::None => "NoneType",
                    _ => "object",
                }
            ));
            return TL_NONE;
        }
    };
    ITER_TABLE.with(|t| {
        let mut tbl = t.borrow_mut();
        let idx = tbl.len();
        tbl.push((items, 0));
        // Encode as negative: -(idx+2), so ≤ -2 (distinguishable from TL_STOP_ITER = -1)
        -((idx as i64) + 2)
    })
}

extern "C" fn tl_iter_next(iter_h: i64) -> i64 {
    if has_error() { return TL_STOP_ITER; }
    if iter_h > -2 {
        set_error(format!("NativeError: invalid iter handle {iter_h}"));
        return TL_STOP_ITER;
    }
    let idx = (-(iter_h + 2)) as usize;
    ITER_TABLE.with(|t| {
        let mut tbl = t.borrow_mut();
        if let Some((items, pos)) = tbl.get_mut(idx) {
            if *pos < items.len() {
                let v = items[*pos].clone();
                *pos += 1;
                push_handle(v)
            } else {
                TL_STOP_ITER
            }
        } else {
            TL_STOP_ITER
        }
    })
}

extern "C" fn tl_is_type(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(
            std::slice::from_raw_parts(name_ptr, name_len as usize)
        )
    };
    let obj = clone_value_at(obj_h);
    let ptr = get_interp_ptr();
    let result = if !ptr.is_null() {
        let interp = unsafe { &*ptr };
        interp.value_is_type(&obj, name)
    } else {
        // Fallback: basic type checks only
        match &obj {
            Value::Int(_) => name == "int",
            Value::Float(_) => name == "float",
            Value::Str(_) => name == "str",
            Value::Bool(_) => name == "bool",
            Value::None => name == "None" || name == "NoneType",
            Value::List(_) => name == "list",
            Value::Dict(_) => name == "dict",
            Value::Tuple(_) => name == "tuple",
            _ => false,
        }
    };
    if result { TL_TRUE } else { TL_FALSE }
}

// ── Arena save / compact helpers ────────────────────────────────────────────
//
// Called around every intra-module function call in generated code to prevent
// arena blowup.  Without these, each call leaves ~N×ops intermediate handles
// in the arena for the entire duration of the outermost native call.
//
// Usage in generated code:
//   let _s = cb_arena_save();
//   let _r = some_fn_impl(args...);
//   let _r = cb_arena_compact(_r, _s);  // truncates callee's intermediates

extern "C" fn tl_arena_save() -> u64 {
    VALUE_ARENA.with(|a| a.borrow().len() as u64)
}

extern "C" fn tl_arena_compact(h: i64, saved: u64) -> i64 {
    let saved = saved as usize;
    // Always truncate to `saved` to discard callee's intermediates.
    if h < 3 {
        // Special handle (None/True/False): no arena slot — just truncate.
        VALUE_ARENA.with(|a| a.borrow_mut().truncate(saved));
        return h;
    }
    let h_idx = h as usize;
    if h_idx < saved {
        // Handle lives before the save point — still valid after truncation.
        VALUE_ARENA.with(|a| a.borrow_mut().truncate(saved));
        return h;
    }
    // Handle was allocated inside the callee: clone, truncate, re-push.
    let val = clone_value_at(h);
    VALUE_ARENA.with(|a| a.borrow_mut().truncate(saved));
    push_handle(val)
}

// ── Loop-level arena compaction ─────────────────────────────────────────────
//
// Called at the end of every while/for loop body to keep the arena bounded.
// Clones all live variable handles, truncates to the loop-frame save point,
// then re-pushes — so the arena stays O(n_live_vars) instead of O(n_iters).
//
// Usage in generated code:
//   let _lf: u64 = cb_arena_save();             // once before the loop
//   loop {
//       // body
//       let _cin: [i64; N] = [_v_x, _v_y, ...];
//       let mut _cout: [i64; N] = [0i64; N];
//       cb_compact_many(_cin.as_ptr(), N, _lf, _cout.as_mut_ptr());
//       _v_x = _cout[0]; _v_y = _cout[1]; ...
//   }

extern "C" fn tl_compact_many(
    handles_in: *const i64,
    n: i32,
    save: u64,
    handles_out: *mut i64,
) {
    let n = n as usize;
    if n == 0 {
        VALUE_ARENA.with(|a| a.borrow_mut().truncate(save as usize));
        return;
    }
    let ins = unsafe { std::slice::from_raw_parts(handles_in, n) };
    let outs = unsafe { std::slice::from_raw_parts_mut(handles_out, n) };
    // Clone all values to Rust stack BEFORE truncation.
    let vals: Vec<Value> = ins.iter().map(|&h| clone_value_at(h)).collect();
    // Truncate arena: discards all temporaries from this iteration.
    VALUE_ARENA.with(|a| a.borrow_mut().truncate(save as usize));
    // Re-push; cached handles (small ints, bool, None) return their fixed handle.
    for (i, val) in vals.into_iter().enumerate() {
        outs[i] = push_handle(val);
    }
}

// ── Typed value extraction ───────────────────────────────────────────────────
//
// Used by type-specialized generated functions to unwrap handle parameters to
// raw Rust values at function entry, eliminating per-operation cb_binop calls.

extern "C" fn tl_deep_copy(h: i64) -> i64 {
    let val = clone_value_at(h);
    let copied = super::Interpreter::deep_copy_value(val);
    push_handle(copied)
}

extern "C" fn tl_to_int(h: i64) -> i64 {
    match h {
        TL_NONE  => 0,
        TL_TRUE  => 1,
        TL_FALSE => 0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => n - 3,
        n => VALUE_ARENA.with(|a| match a.borrow().get(n as usize) {
            Some(Value::Int(v))   => *v,
            Some(Value::UInt(v))  => *v as i64,
            Some(Value::Float(f)) => *f as i64,
            Some(Value::Bool(b))  => *b as i64,
            _ => 0,
        }),
    }
}

extern "C" fn tl_to_float(h: i64) -> f64 {
    match h {
        TL_NONE  => 0.0,
        TL_TRUE  => 1.0,
        TL_FALSE => 0.0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => (n - 3) as f64,
        n => VALUE_ARENA.with(|a| match a.borrow().get(n as usize) {
            Some(Value::Int(v))   => *v as f64,
            Some(Value::UInt(v))  => *v as f64,
            Some(Value::Float(f)) => *f,
            Some(Value::Bool(b))  => *b as u8 as f64,
            _ => 0.0,
        }),
    }
}

// ── cpp-bridge helpers ───────────────────────────────────────────────────────

/// Convert the tl string handle `h` to a null-terminated C string.
///
/// The bytes are stored in the thread-local `CSTR_BUFS` scratch buffer so the
/// returned pointer remains valid until `exit_native_call` / `abort_native_call`
/// clears the buffer at the end of the outermost native call.
extern "C" fn tl_to_cstr(h: i64) -> *const u8 {
    let s = match clone_value_at(h) {
        Value::Str(s) => s,
        _ => String::new(),
    };
    CSTR_BUFS.with(|bufs| {
        let mut bufs = bufs.borrow_mut();
        let mut bytes = s.into_bytes();
        bytes.push(0u8); // null terminator
        bufs.push(bytes);
        bufs.last().unwrap().as_ptr()
    })
}

/// Overwrite the arena slot at `target_h` with a clone of the value at `new_val_h`.
///
/// Used by generated cpp-bridge wrappers to write back `T*` output-parameter
/// values after a C call.
extern "C" fn tl_write_handle(target_h: i64, new_val_h: i64) {
    if target_h < 3 { return; } // never overwrite fixed slots
    let new_val = clone_value_at(new_val_h);
    VALUE_ARENA.with(|a| {
        let mut arena = a.borrow_mut();
        if let Some(slot) = arena.get_mut(target_h as usize) {
            *slot = new_val;
        }
    });
}

// ── Static callbacks instance ─────────────────────────────────────────────────

static CALLBACKS: TlCallbacks = TlCallbacks {
    make_int:   tl_make_int,
    make_float: tl_make_float,
    make_bool:  tl_make_bool,
    make_str:   tl_make_str,
    make_list:  tl_make_list,
    make_tuple: tl_make_tuple,
    make_dict:  tl_make_dict,
    make_none:  tl_make_none,
    is_truthy:  tl_is_truthy,
    binop:      tl_binop,
    unop:       tl_unop,
    call_fn:    tl_call_fn,
    get_attr:   tl_get_attr,
    set_attr:   tl_set_attr,
    subscript:  tl_subscript,
    get_global: tl_get_global,
    iter_from:    tl_iter_from,
    iter_next:    tl_iter_next,
    is_type:      tl_is_type,
    arena_save:    tl_arena_save,
    arena_compact: tl_arena_compact,
    compact_many:  tl_compact_many,
    to_int:        tl_to_int,
    to_float:      tl_to_float,
    deep_copy:     tl_deep_copy,
    to_cstr:       tl_to_cstr,
    write_handle:  tl_write_handle,
};
