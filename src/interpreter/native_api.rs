// native_api.rs — Interpreter-mediated value table and C callbacks for native compiled modules.
//
// All values crossing the native ABI boundary are represented as `i64` handles:
//   TL_NONE  (0) = None
//   TL_TRUE  (1) = Bool(true)
//   TL_FALSE (2) = Bool(false)
//   TL_STOP_ITER (-1) = iteration exhausted sentinel
//   h >= 3   = index into the thread-local VALUE_ARENA Vec<Value>
//
// The ArCallbacks struct is passed to each native DLL via `ar_init(cb)`.
// All heavy operations (attribute access, function calls, binops) route through
// the interpreter held in CURRENT_INTERP.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::{DictData, Interpreter, TupleData, Value};

// ── Handle constants ─────────────────────────────────────────────────────────

pub const TL_NONE: i64 = 0;
pub const TL_TRUE: i64 = 1;
pub const TL_FALSE: i64 = 2;
pub const TL_STOP_ITER: i64 = -1;
/// Sentinel returned by CB_RAISE: native code raised an exception stored in PENDING_RAISE.
pub const TL_EXCEPTION: i64 = -2;

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

// ── Integer handle cache ─────────────────────────────────────────────────────
//
// arena[0]                    = None placeholder  (TL_NONE=0 accessed directly)
// arena[1]                    = Bool(true)         (TL_TRUE=1 accessed directly)
// arena[2]                    = Bool(false)        (TL_FALSE=2 accessed directly)
// arena[3 + n]  (0 ≤ n < 256) = Int(n)            → handle (3 + n)
// INT_CACHE_BASE = 3 + 256 = 259 = the first dynamic slot.

const INT_CACHE_END: i64 = 256;
const INT_CACHE_BASE: usize = 3 + INT_CACHE_END as usize; // 259

#[inline(always)]
fn int_cache_handle(n: i64) -> i64 {
    3 + n
}

// ── Consolidated thread-local state ─────────────────────────────────────────
//
// All previously separate thread_local! variables are combined here so that
// functions needing multiple TLS fields can open a single borrow instead of
// doing one TLS lookup per field.
//
// Borrow discipline: never hold a borrow on STATE across a call that might
// also borrow STATE.  The pattern used throughout this file is:
//   1. Extract needed values in one borrow — then release.
//   2. Call interpreter (which is not STATE-aware).
//   3. Store result in a new, non-overlapping borrow.

struct NativeCallState {
    /// Raw pointer to the currently executing Interpreter.
    interp_ptr: *mut Interpreter,
    /// Value arena (see handle encoding above).
    arena: Vec<Value>,
    /// Iterator table: (collected_items, current_position).
    /// Iter handles are encoded as -(idx+2) so they are all ≤ -2.
    iter_table: Vec<(Vec<Value>, usize)>,
    /// Nesting depth of native calls (0 = not inside any native call).
    call_depth: usize,
    /// Arena size saved at the outermost native call entry.
    arena_save: usize,
    /// Iter table size saved at the outermost native call entry.
    iter_save: usize,
    /// Error set by a callback on failure; checked after each native call.
    error: Option<String>,
    /// Exception raised by CB_RAISE; (type_name, message) pair.
    pending_raise: Option<(String, String)>,
    /// Scratch C-string buffers produced by `ar_to_cstr`; cleared at outermost exit.
    cstr_bufs: Vec<Vec<u8>>,
    /// Native method dispatch table: (class_name, method_name) → fn_ptr.
    native_methods: HashMap<(String, String), usize>,
}

impl NativeCallState {
    /// Clone a value out of the arena by handle.
    #[inline]
    fn clone_value(&self, h: i64) -> Value {
        match h {
            TL_NONE => Value::None,
            TL_TRUE => Value::Bool(true),
            TL_FALSE => Value::Bool(false),
            n if n >= 3 => self.arena.get(n as usize).cloned().unwrap_or(Value::None),
            _ => Value::None,
        }
    }

    /// Push a value into the arena and return its handle.
    /// Small integers (0..256), booleans and None return cached handles without arena push.
    #[inline]
    fn push_value(&mut self, v: Value) -> i64 {
        match &v {
            Value::None => TL_NONE,
            Value::Bool(true) => TL_TRUE,
            Value::Bool(false) => TL_FALSE,
            Value::Int(n) if *n >= 0 && *n < INT_CACHE_END => int_cache_handle(*n),
            _ => {
                let h = self.arena.len() as i64;
                self.arena.push(v);
                h
            }
        }
    }

    /// Like push_value but always allocates a new arena slot (no cached handle).
    /// Used for write-back (MutPtr) parameters that must be addressable.
    #[inline]
    fn push_value_writeback(&mut self, v: Value) -> i64 {
        let h = self.arena.len() as i64;
        self.arena.push(v);
        h
    }
}

thread_local! {
    static STATE: RefCell<NativeCallState> = RefCell::new({
        let mut arena = Vec::with_capacity(INT_CACHE_BASE + 64);
        arena.push(Value::None);         // index 0
        arena.push(Value::Bool(true));   // index 1
        arena.push(Value::Bool(false));  // index 2
        for i in 0..INT_CACHE_END {     // indices 3..258
            arena.push(Value::Int(i));
        }
        NativeCallState {
            interp_ptr: std::ptr::null_mut(),
            arena,
            iter_table: Vec::new(),
            call_depth: 0,
            arena_save: INT_CACHE_BASE,
            iter_save: 0,
            error: None,
            pending_raise: None,
            cstr_bufs: Vec::new(),
            native_methods: HashMap::new(),
        }
    });
}

// ── Public API for the interpreter ──────────────────────────────────────────

/// ネイティブ関数呼び出しを開始する。最外呼び出し（深さ0）のとき `true` を返す。
/// 最外レベルではアリーナとイテレータテーブルの保存点を記録し `interp_ptr` をセットする。
pub fn enter_native_call(interp: *mut Interpreter) -> bool {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let prev = st.call_depth;
        st.call_depth = prev + 1;
        if prev == 0 {
            st.interp_ptr = interp;
            st.arena_save = st.arena.len();
            st.iter_save = st.iter_table.len();
            true
        } else {
            false
        }
    })
}

/// ネイティブ関数呼び出しを正常終了する。
/// `result_h` から結果値をクローンし、最外呼び出しであればアリーナ等を保存点まで巻き戻す。
pub fn exit_native_call(result_h: i64, is_outermost: bool) -> Value {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let result = st.clone_value(result_h);
        if st.call_depth > 0 {
            st.call_depth -= 1;
        }
        if is_outermost {
            st.interp_ptr = std::ptr::null_mut();
            let arena_save = st.arena_save;
            let iter_save = st.iter_save;
            st.arena.truncate(arena_save);
            st.iter_table.truncate(iter_save);
            st.cstr_bufs.clear();
        }
        result
    })
}

/// ネイティブ関数呼び出しをエラー終了する（値を返さずクリーンアップのみ）。
pub fn abort_native_call(is_outermost: bool) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.call_depth > 0 {
            st.call_depth -= 1;
        }
        if is_outermost {
            st.interp_ptr = std::ptr::null_mut();
            let arena_save = st.arena_save;
            let iter_save = st.iter_save;
            st.arena.truncate(arena_save);
            st.iter_table.truncate(iter_save);
            st.cstr_bufs.clear();
        }
    })
}

/// 値をアリーナにプッシュしてハンドルを返す（小整数・真偽値はキャッシュ済みハンドルを返す）。
pub fn push_handle(v: Value) -> i64 {
    match &v {
        Value::None => TL_NONE,
        Value::Bool(true) => TL_TRUE,
        Value::Bool(false) => TL_FALSE,
        Value::Int(n) if *n >= 0 && *n < INT_CACHE_END => int_cache_handle(*n),
        _ => STATE.with(|s| {
            let mut st = s.borrow_mut();
            let h = st.arena.len() as i64;
            st.arena.push(v);
            h
        }),
    }
}

/// 値をアリーナに**常に新規スロット**としてプッシュしてハンドルを返す（cpp-bridge write-back 用）。
pub fn push_handle_writeback(v: Value) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let h = st.arena.len() as i64;
        st.arena.push(v);
        h
    })
}

/// アリーナのハンドル `h` が指す値をクローンして返す。
pub fn clone_value_at(h: i64) -> Value {
    match h {
        TL_NONE => Value::None,
        TL_TRUE => Value::Bool(true),
        TL_FALSE => Value::Bool(false),
        n if n >= 3 => {
            STATE.with(|s| s.borrow().arena.get(n as usize).cloned().unwrap_or(Value::None))
        }
        _ => Value::None,
    }
}

/// 直前のネイティブ呼び出しチェーンでセットされたエラーを取り出して返す。
pub fn take_error() -> Option<String> {
    STATE.with(|s| s.borrow_mut().error.take())
}

/// Take any exception raised by CB_RAISE. Returns (type_name, message) if set.
pub fn take_pending_raise() -> Option<(String, String)> {
    STATE.with(|s| s.borrow_mut().pending_raise.take())
}

/// Register a natively compiled class method so `ar_call_method` can dispatch to it.
pub fn register_native_method(class_name: &str, method_name: &str, fn_ptr: usize) {
    STATE.with(|s| {
        s.borrow_mut()
            .native_methods
            .insert((class_name.to_string(), method_name.to_string()), fn_ptr);
    });
}

/// Clear all registered native methods (call on unload / hot-reload).
pub fn clear_native_methods() {
    STATE.with(|s| s.borrow_mut().native_methods.clear());
}

/// Look up the raw function pointer for a native class method.
pub fn lookup_native_method_ptr(class_name: &str, method_name: &str) -> Option<usize> {
    STATE.with(|s| {
        s.borrow()
            .native_methods
            .get(&(class_name.to_string(), method_name.to_string()))
            .copied()
    })
}

/// Dispatch a native class method from interpreter code (not from within a native call).
pub fn try_dispatch_native_method(
    interp: &mut Interpreter,
    obj: Value,
    method_name: &str,
    arg_vals: Vec<Value>,
) -> Option<Result<Value, String>> {
    let class_name = match &obj {
        Value::Instance(rc) => rc.borrow().class.name.clone(),
        _ => return None,
    };

    let fn_ptr = lookup_native_method_ptr(&class_name, method_name)?;

    let is_outermost = enter_native_call(interp as *mut Interpreter);

    let obj_h = push_handle(obj);
    let mut all_args: Vec<i64> = Vec::with_capacity(1 + arg_vals.len());
    all_args.push(obj_h);
    for v in &arg_vals {
        all_args.push(push_handle(v.clone()));
    }

    let result_h = unsafe {
        let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(fn_ptr);
        func(all_args.as_ptr(), all_args.len() as i32)
    };

    if let Some(err) = take_error() {
        abort_native_call(is_outermost);
        return Some(Err(err));
    }

    if let Some((exc_type, exc_msg)) = take_pending_raise() {
        abort_native_call(is_outermost);
        return Some(Err(format!("{exc_type}: {exc_msg}")));
    }

    Some(Ok(exit_native_call(result_h, is_outermost)))
}

/// 静的コールバックインスタンスへの `*const ArCallbacks` ポインタを返す。
pub fn get_callbacks() -> *const ArCallbacks {
    &CALLBACKS as *const ArCallbacks
}

// ── Internal helpers ─────────────────────────────────────────────────────────

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
        OP_IN => Some(BinOp::In),
        OP_NOTIN => Some(BinOp::NotIn),
        _ => None,
    }
}

// ── ArCallbacks struct ───────────────────────────────────────────────────────

/// ネイティブ DLL に `ar_init` 経由で渡される C 互換の関数ポインタ構造体。
/// レイアウトは `codegen.rs` が生成する `ArCallbacks` 構造体と完全に一致しなければならない。
#[repr(C)]
pub struct ArCallbacks {
    pub make_int: extern "C" fn(i64) -> i64,
    pub make_float: extern "C" fn(f64) -> i64,
    pub make_bool: extern "C" fn(i32) -> i64,
    pub make_str: extern "C" fn(*const u8, i32) -> i64,
    pub make_list: extern "C" fn(*const i64, i32) -> i64,
    pub make_tuple: extern "C" fn(*const i64, i32) -> i64,
    pub make_dict: extern "C" fn(*const i64, *const i64, i32) -> i64,
    pub make_none: extern "C" fn() -> i64,
    pub is_truthy: extern "C" fn(i64) -> i32,
    pub binop: extern "C" fn(i32, i64, i64) -> i64,
    pub unop: extern "C" fn(i32, i64) -> i64,
    pub call_fn: extern "C" fn(i64, *const i64, i32) -> i64,
    pub get_attr: extern "C" fn(i64, *const u8, i32) -> i64,
    pub set_attr: extern "C" fn(i64, *const u8, i32, i64),
    pub subscript: extern "C" fn(i64, i64) -> i64,
    pub get_global: extern "C" fn(*const u8, i32) -> i64,
    pub iter_from: extern "C" fn(i64) -> i64,
    pub iter_next: extern "C" fn(i64) -> i64,
    pub is_type: extern "C" fn(i64, *const u8, i32) -> i64,
    pub arena_save: extern "C" fn() -> u64,
    pub arena_compact: extern "C" fn(i64, u64) -> i64,
    pub compact_many: extern "C" fn(*const i64, i32, u64, *mut i64),
    pub to_int: extern "C" fn(i64) -> i64,
    pub to_float: extern "C" fn(i64) -> f64,
    pub deep_copy: extern "C" fn(i64) -> i64,
    /// tl 文字列ハンドルをヌル終端 C 文字列ポインタに変換する。ポインタは最外ネイティブ呼び出し終了まで有効。
    pub to_cstr: extern "C" fn(i64) -> *const u8,
    /// `arena[target_h]` を `new_val_h` の値のクローンで上書きする。cpp-bridge の T* 書き戻しパラメータ用。
    pub write_handle: extern "C" fn(i64, i64),

    // ── Feature-extension callbacks (fields 27-31) ──────────────────────────
    pub list_append: extern "C" fn(i64, i64) -> i64,
    pub raise_exc: extern "C" fn(i64, i64) -> i64,
    pub make_cell: extern "C" fn(i64) -> i64,
    pub get_cell: extern "C" fn(i64) -> i64,
    pub set_cell: extern "C" fn(i64, i64),
    pub call_method: extern "C" fn(i64, *const u8, i32, *const i64, i32) -> i64,

    // ── Typed field-read callbacks (fields 33-34) ────────────────────────────
    pub get_float_field: extern "C" fn(i64, *const u8, i32) -> f64,
    pub get_int_field: extern "C" fn(i64, *const u8, i32) -> i64,

    // ── Flat frozen-list access callbacks (fields 35-36) ─────────────────────
    pub flat_data_ptr: extern "C" fn(i64) -> i64,
    pub flat_len: extern "C" fn(i64) -> i64,

    // ── Function-object trampoline (field 37) ─────────────────────────────────
    pub fn_trampoline: extern "C" fn(i64) -> *const (),
}

// ── Callback implementations ─────────────────────────────────────────────────

extern "C" fn ar_make_int(n: i64) -> i64 {
    push_handle(Value::Int(n))
}

extern "C" fn ar_make_float(f: f64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.push_value(Value::Float(f))
    })
}

extern "C" fn ar_make_bool(b: i32) -> i64 {
    if b != 0 { TL_TRUE } else { TL_FALSE }
}

extern "C" fn ar_make_str(ptr: *const u8, len: i32) -> i64 {
    let text = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize))
    }
    .to_owned();
    STATE.with(|s| s.borrow_mut().push_value(Value::Str(text)))
}

// Build list/tuple/dict in one borrow: clone all items then push the container.
extern "C" fn ar_make_list(items_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let items: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let list = Rc::new(RefCell::new(items));
        st.push_value(Value::List(list))
    })
}

extern "C" fn ar_make_tuple(items_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let elements: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let tuple = Rc::new(TupleData::new(elements, vec![]));
        st.push_value(Value::Tuple(tuple))
    })
}

extern "C" fn ar_make_dict(keys_ptr: *const i64, vals_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let mut dict = DictData::new("Any".to_string(), "Any".to_string());
        for i in 0..n as usize {
            let k = st.clone_value(unsafe { *keys_ptr.add(i) });
            let v = st.clone_value(unsafe { *vals_ptr.add(i) });
            dict.set(k, v);
        }
        st.push_value(Value::Dict(Rc::new(RefCell::new(dict))))
    })
}

extern "C" fn ar_make_none() -> i64 {
    TL_NONE
}

extern "C" fn ar_is_truthy(h: i64) -> i32 {
    // Phase 1: extract value and interp_ptr.
    let (v, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.clone_value(h), st.interp_ptr)
    });
    if interp_ptr.is_null() {
        return match &v {
            Value::None => 0,
            Value::Bool(b) => i32::from(*b),
            Value::Int(n) => i32::from(*n != 0),
            Value::Float(f) => i32::from(*f != 0.0),
            Value::Str(s) => i32::from(!s.is_empty()),
            _ => 1,
        };
    }
    let interp = unsafe { &mut *interp_ptr };
    i32::from(interp.is_truthy(&v))
}

extern "C" fn ar_binop(op: i32, a: i64, b: i64) -> i64 {
    let (has_err, lhs, rhs, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(a), st.clone_value(b), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let ast_op = match i32_to_binop(op) {
        Some(o) => o,
        None => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: invalid binop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for binop".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &*interp_ptr };
    match interp.apply_binop(&ast_op, lhs, rhs) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_unop(op: i32, a: i64) -> i64 {
    let (has_err, operand, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(a), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    use crate::ast::UnaryOp;
    let ast_op = match op {
        UOP_NEG => UnaryOp::Neg,
        UOP_NOT => UnaryOp::Not,
        UOP_BIT_NOT => UnaryOp::BitNot,
        _ => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: invalid unop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for unop".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &*interp_ptr };
    match interp.apply_unary(&ast_op, operand) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 関数ハンドルと引数ハンドル配列を受け取り関数を呼び出す C コールバック。
///
/// Fast path: `Value::NativeFunction` はインタープリタを経由せず直接 DLL 関数を呼び出す。
/// Slow path: クロージャ・ユーザー定義関数などは `call_value_with_args` に委譲する。
extern "C" fn ar_call_fn(fn_h: i64, args_ptr: *const i64, n_args: i32) -> i64 {
    use std::sync::atomic::Ordering;

    // Phase 1: extract fn_val and interp_ptr, check for pending error.
    let (has_err, fn_val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(fn_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // Fast path: NativeFunction (cpp-dll or JIT).
    // Arg handles are already live in the arena — pass them straight through.
    if let Value::NativeFunction(ref fn_ref) = fn_val {
        if !interp_ptr.is_null() {
            let is_outermost = enter_native_call(interp_ptr);

            let result_h = if fn_ref.raw_fn_ptr != 0 {
                // JIT: pointer is embedded directly.
                unsafe {
                    let f: unsafe extern "C" fn(*const i64, i32) -> i64 =
                        std::mem::transmute(fn_ref.raw_fn_ptr);
                    f(args_ptr, n_args)
                }
            } else {
                // cpp-dll: use cached_fn_ptr; resolve on first call.
                let cached = fn_ref.cached_fn_ptr.load(Ordering::Relaxed);
                let fn_ptr = if cached != 0 {
                    cached
                } else {
                    let sym_name = format!("{}_tl\0", fn_ref.fn_name);
                    let interp = unsafe { &*interp_ptr };
                    match interp.native_libs.get(&fn_ref.lib_path) {
                        Some(lib) => {
                            match unsafe {
                                lib.0.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(
                                    sym_name.as_bytes(),
                                )
                            } {
                                Ok(f) => {
                                    let fp = *f as usize;
                                    fn_ref.cached_fn_ptr.store(fp, Ordering::Relaxed);
                                    fp
                                }
                                Err(e) => {
                                    STATE.with(|s| s.borrow_mut().error = Some(format!(
                                        "RuntimeError: symbol '{}' not found: {e}",
                                        fn_ref.fn_name
                                    )));
                                    abort_native_call(is_outermost);
                                    return TL_NONE;
                                }
                            }
                        }
                        None => {
                            STATE.with(|s| s.borrow_mut().error = Some(format!(
                                "RuntimeError: native library not loaded: {}",
                                fn_ref.lib_path.display()
                            )));
                            abort_native_call(is_outermost);
                            return TL_NONE;
                        }
                    }
                };
                unsafe {
                    let f: unsafe extern "C" fn(*const i64, i32) -> i64 =
                        std::mem::transmute(fn_ptr);
                    f(args_ptr, n_args)
                }
            };

            if STATE.with(|s| s.borrow().error.is_some()) {
                abort_native_call(is_outermost);
                return TL_NONE;
            }
            let result_val = exit_native_call(result_h, is_outermost);
            return push_handle(result_val);
        }
    }

    // Slow path: HV functions, closures, overloaded fns, etc.
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(
            "NativeError: interpreter not set for call_fn".to_string()
        ));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_value_with_args(fn_val, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_attr(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for get_attr".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, &name) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_set_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, obj, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for set_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.set_attr_val(obj, &name, val) {
        STATE.with(|s| s.borrow_mut().error = Some(e));
    }
}

extern "C" fn ar_subscript(obj_h: i64, key_h: i64) -> i64 {
    let (has_err, obj, key, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(key_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for subscript".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.eval_subscript(obj, key) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_global(name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (looking up '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_val(&name) {
        Some(v) => push_handle(v),
        None => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NameError: '{name}' is not defined")));
            TL_NONE
        }
    }
}

/// イテラブルなオブジェクトハンドルからイテレータハンドルを生成する。
/// イテレータハンドルは `-(idx+2)` の負値でエンコードされる。
extern "C" fn ar_iter_from(obj_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() { return TL_NONE; }
        let obj = st.clone_value(obj_h);
        let items: Vec<Value> = match obj {
            Value::List(l) => l.borrow().clone(),
            Value::FrozenList { ref state, ref layout } => {
                let fst = state.borrow();
                (0..fst.len).map(|i| layout.reconstruct_item(&fst.data, i)).collect()
            }
            Value::Tuple(t) => t.all_values().to_vec(),
            Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
            Value::Dict(d) => d.borrow().all_keys(),
            other => {
                st.error = Some(format!(
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
        let idx = st.iter_table.len();
        st.iter_table.push((items, 0));
        -((idx as i64) + 2)
    })
}

/// イテレータハンドルから次の要素ハンドルを取得する。
extern "C" fn ar_iter_next(iter_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() { return TL_STOP_ITER; }
        if iter_h > -2 {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
            return TL_STOP_ITER;
        }
        let idx = (-(iter_h + 2)) as usize;
        // Extract the next value from iter_table, then push to arena separately
        // to avoid simultaneous mutable access to two fields via a method call.
        let maybe_v: Option<Value> = st.iter_table.get_mut(idx).and_then(|(items, pos)| {
            if *pos < items.len() {
                let v = items[*pos].clone();
                *pos += 1;
                Some(v)
            } else {
                None
            }
        });
        match maybe_v {
            Some(v) => st.push_value(v),
            None => TL_STOP_ITER,
        }
    })
}

extern "C" fn ar_is_type(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.clone_value(obj_h), st.interp_ptr)
    });
    let result = if !interp_ptr.is_null() {
        let interp = unsafe { &*interp_ptr };
        interp.value_is_type(&obj, name)
    } else {
        match &obj {
            Value::Int(_) => name == "int",
            Value::Float(_) => name == "float",
            Value::Str(_) => name == "str",
            Value::Bool(_) => name == "bool",
            Value::None => name == "None" || name == "NoneType",
            Value::List(_) => name == "list" || name == "list_like",
            Value::FrozenList { .. } => name == "fixed_list" || name == "list_like",
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

extern "C" fn ar_arena_save() -> u64 {
    STATE.with(|s| s.borrow().arena.len() as u64)
}

extern "C" fn ar_arena_compact(h: i64, saved: u64) -> i64 {
    let saved = saved as usize;
    // Special handles (None/True/False): just truncate — no arena slot.
    if h < 3 {
        STATE.with(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    if (h as usize) < saved {
        // Handle lives before the save point — still valid after truncation.
        STATE.with(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    // Handle was allocated inside the callee: clone, truncate, re-push — all in one borrow.
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(h);
        st.arena.truncate(saved);
        st.push_value(val)
    })
}

/// ループ本体終端で複数の生存変数ハンドルをまとめて保存点まで巻き戻す。
/// 全クローン・truncate・再プッシュを1回の STATE borrow 内で完了する。
extern "C" fn ar_compact_many(handles_in: *const i64, n: i32, save: u64, handles_out: *mut i64) {
    let n = n as usize;
    let ins = unsafe { std::slice::from_raw_parts(handles_in, n) };
    let outs = unsafe { std::slice::from_raw_parts_mut(handles_out, n) };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if n == 0 {
            st.arena.truncate(save as usize);
            return;
        }
        let vals: Vec<Value> = ins.iter().map(|&h| st.clone_value(h)).collect();
        st.arena.truncate(save as usize);
        for (i, val) in vals.into_iter().enumerate() {
            outs[i] = st.push_value(val);
        }
    });
}

// ── Typed value extraction ───────────────────────────────────────────────────

extern "C" fn ar_deep_copy(h: i64) -> i64 {
    let val = clone_value_at(h);
    let copied = super::Interpreter::deep_copy_value(val);
    push_handle(copied)
}

extern "C" fn ar_to_int(h: i64) -> i64 {
    match h {
        TL_NONE => 0,
        TL_TRUE => 1,
        TL_FALSE => 0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => n - 3,
        n => STATE.with(|s| match s.borrow().arena.get(n as usize) {
            Some(Value::Int(v)) => *v,
            Some(Value::UInt(v)) => *v as i64,
            Some(Value::Float(f)) => *f as i64,
            Some(Value::Bool(b)) => *b as i64,
            _ => 0,
        }),
    }
}

extern "C" fn ar_to_float(h: i64) -> f64 {
    match h {
        TL_NONE => 0.0,
        TL_TRUE => 1.0,
        TL_FALSE => 0.0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => (n - 3) as f64,
        n => STATE.with(|s| match s.borrow().arena.get(n as usize) {
            Some(Value::Int(v)) => *v as f64,
            Some(Value::UInt(v)) => *v as f64,
            Some(Value::Float(f)) => *f,
            Some(Value::Bool(b)) => *b as u8 as f64,
            _ => 0.0,
        }),
    }
}

// ── cpp-bridge helpers ───────────────────────────────────────────────────────

extern "C" fn ar_to_cstr(h: i64) -> *const u8 {
    let s = match clone_value_at(h) {
        Value::Str(s) => s,
        _ => String::new(),
    };
    STATE.with(|s_tls| {
        let mut st = s_tls.borrow_mut();
        let mut bytes = s.into_bytes();
        bytes.push(0u8);
        st.cstr_bufs.push(bytes);
        st.cstr_bufs.last().unwrap().as_ptr()
    })
}

extern "C" fn ar_write_handle(target_h: i64, new_val_h: i64) {
    if target_h < 3 { return; }
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let new_val = st.clone_value(new_val_h);
        if let Some(slot) = st.arena.get_mut(target_h as usize) {
            *slot = new_val;
        }
    });
}

// ── Feature-extension callbacks ──────────────────────────────────────────────

extern "C" fn ar_list_append(list_h: i64, item_h: i64) -> i64 {
    // Clone the Rc (cheap) while holding the borrow, then mutate outside.
    let (has_err, item, maybe_list) = STATE.with(|s| {
        let st = s.borrow();
        let item = st.clone_value(item_h);
        let maybe_list = if let Some(Value::List(list)) = st.arena.get(list_h as usize) {
            Some(list.clone())
        } else {
            None
        };
        (st.error.is_some(), item, maybe_list)
    });
    if has_err { return list_h; }
    if let Some(list_rc) = maybe_list {
        list_rc.borrow_mut().push(item);
    }
    list_h
}

extern "C" fn ar_raise_exc(type_h: i64, msg_h: i64) -> i64 {
    let (type_name, msg) = STATE.with(|s| {
        let st = s.borrow();
        let tn = match st.clone_value(type_h) {
            Value::Str(s) => s,
            Value::Class(c) => c.name.clone(),
            Value::Instance(inst) => inst.borrow().class.name.clone(),
            _ => "Exception".to_string(),
        };
        let m = match st.clone_value(msg_h) {
            Value::Str(s) => s,
            Value::None => String::new(),
            Value::Instance(inst) => {
                let b = inst.borrow();
                b.class.field_index.get("message").and_then(|&idx| {
                    b.fields.get(idx).and_then(|s| {
                        if let Some((Value::Str(s), _)) = s { Some(s.clone()) } else { None }
                    })
                }).unwrap_or_default()
            }
            other => format!("{other:?}"),
        };
        (tn, m)
    });
    STATE.with(|s| s.borrow_mut().pending_raise = Some((type_name, msg)));
    TL_EXCEPTION
}

extern "C" fn ar_make_cell(init_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(init_h);
        let cell = Rc::new(RefCell::new(vec![val]));
        st.push_value(Value::List(cell))
    })
}

extern "C" fn ar_get_cell(cell_h: i64) -> i64 {
    // Clone the Rc, release borrow, then access cell outside STATE lock.
    let maybe_list = STATE.with(|s| {
        if let Some(Value::List(list)) = s.borrow().arena.get(cell_h as usize) {
            Some(list.clone())
        } else {
            None
        }
    });
    if let Some(list_rc) = maybe_list {
        if let Some(v) = list_rc.borrow().first().cloned() {
            return push_handle(v);
        }
    }
    TL_NONE
}

extern "C" fn ar_set_cell(cell_h: i64, val_h: i64) {
    let (val, maybe_list) = STATE.with(|s| {
        let st = s.borrow();
        let val = st.clone_value(val_h);
        let maybe_list = if let Some(Value::List(list)) = st.arena.get(cell_h as usize) {
            Some(list.clone())
        } else {
            None
        };
        (val, maybe_list)
    });
    if let Some(list_rc) = maybe_list {
        if let Some(slot) = list_rc.borrow_mut().first_mut() {
            *slot = val;
        }
    }
}

extern "C" fn ar_call_method(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();

    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // Native dispatch: check NATIVE_METHODS table first.
    if let Value::Instance(ref inst_rc) = obj {
        let class_name = inst_rc.borrow().class.name.clone();
        let fn_ptr = STATE.with(|s| {
            s.borrow().native_methods.get(&(class_name.clone(), name.clone())).copied()
        });
        if let Some(ptr) = fn_ptr {
            let mut all_args = Vec::with_capacity(1 + n_args as usize);
            all_args.push(obj_h);
            for i in 0..n_args as usize {
                all_args.push(unsafe { *args_ptr.add(i) });
            }
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(ptr);
                return func(all_args.as_ptr(), all_args.len() as i32);
            }
        }
    }

    // Interpreter fallback.
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    let evaled: Vec<(Option<String>, Value, bool)> =
        args.into_iter().map(|v| (None, v, true)).collect();
    match interp.eval_method_call_evaled(obj, &name, evaled) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

// ── Typed field-read callbacks ─────────────────────────────────────────────

extern "C" fn ar_get_float_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> f64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0.0; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_float_field '{name}')")));
        return 0.0;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, name) {
        Ok(Value::Float(f)) => f,
        Ok(Value::Int(n)) => n as f64,
        Ok(Value::Bool(b)) => b as u8 as f64,
        Ok(_) | Err(_) => 0.0,
    }
}

extern "C" fn ar_get_int_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_int_field '{name}')")));
        return 0;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, name) {
        Ok(Value::Int(n)) => n,
        Ok(Value::Float(f)) => f as i64,
        Ok(Value::Bool(b)) => b as i64,
        Ok(_) | Err(_) => 0,
    }
}

extern "C" fn ar_flat_data_ptr(h: i64) -> i64 {
    if STATE.with(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().data.as_ptr() as i64,
        _ => 0,
    }
}

extern "C" fn ar_flat_len(h: i64) -> i64 {
    if STATE.with(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().len as i64,
        _ => 0,
    }
}

extern "C" fn ar_fn_trampoline(_fn_h: i64) -> *const () {
    ar_call_fn as *const ()
}

// ── Static callbacks instance ─────────────────────────────────────────────────

static CALLBACKS: ArCallbacks = ArCallbacks {
    make_int: ar_make_int,
    make_float: ar_make_float,
    make_bool: ar_make_bool,
    make_str: ar_make_str,
    make_list: ar_make_list,
    make_tuple: ar_make_tuple,
    make_dict: ar_make_dict,
    make_none: ar_make_none,
    is_truthy: ar_is_truthy,
    binop: ar_binop,
    unop: ar_unop,
    call_fn: ar_call_fn,
    get_attr: ar_get_attr,
    set_attr: ar_set_attr,
    subscript: ar_subscript,
    get_global: ar_get_global,
    iter_from: ar_iter_from,
    iter_next: ar_iter_next,
    is_type: ar_is_type,
    arena_save: ar_arena_save,
    arena_compact: ar_arena_compact,
    compact_many: ar_compact_many,
    to_int: ar_to_int,
    to_float: ar_to_float,
    deep_copy: ar_deep_copy,
    to_cstr: ar_to_cstr,
    write_handle: ar_write_handle,
    list_append: ar_list_append,
    raise_exc: ar_raise_exc,
    make_cell: ar_make_cell,
    get_cell: ar_get_cell,
    set_cell: ar_set_cell,
    call_method: ar_call_method,
    get_float_field: ar_get_float_field,
    get_int_field: ar_get_int_field,
    flat_data_ptr: ar_flat_data_ptr,
    flat_len: ar_flat_len,
    fn_trampoline: ar_fn_trampoline,
};
