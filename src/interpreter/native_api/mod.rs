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

use super::{Interpreter, Value};

// ── Handle constants ─────────────────────────────────────────────────────────

pub const TL_NONE: i64 = 0;
pub const TL_TRUE: i64 = 1;
pub const TL_FALSE: i64 = 2;
pub const TL_STOP_ITER: i64 = -1;
/// Sentinel returned by CB_RAISE: native code raised an exception stored in PENDING_RAISE.
pub const TL_EXCEPTION: i64 = -2;

// ── Typed ABI error slot ─────────────────────────────────────────────────────

/// `{name}_typed` エントリのエラー出力スロット。呼び出し側がスタックに確保して渡す。
/// ネイティブコードは raise 時にここへ書き込み、status 1 を返す。
/// 文字列ポインタは DLL 内の静的領域を指す（DLL がロードされている限り有効）。
///
/// LLVM 側レイアウト（オフセット固定・8 バイト境界）:
///   +0:  type_ptr (ptr)   — 例外クラス名（例: "ValueError"）
///   +8:  type_len (i64)
///   +16: msg_ptr  (ptr)   — メッセージ文字列
///   +24: msg_len  (i64)
#[repr(C)]
pub struct ErrSlot {
    pub type_ptr: *const u8,
    pub type_len: u64,
    pub msg_ptr: *const u8,
    pub msg_len: u64,
}

impl Default for ErrSlot {
    fn default() -> Self {
        ErrSlot {
            type_ptr: std::ptr::null(),
            type_len: 0,
            msg_ptr: std::ptr::null(),
            msg_len: 0,
        }
    }
}

impl ErrSlot {
    /// スロットの内容を `"TypeName: message"` 形式のエラー文字列に変換する。
    /// 既存の raise 経路（`take_pending_raise` 後のフォーマット）と同じ形式。
    pub fn to_error_string(&self) -> String {
        let read = |p: *const u8, n: u64| -> String {
            if p.is_null() || n == 0 {
                return String::new();
            }
            let bytes = unsafe { std::slice::from_raw_parts(p, n as usize) };
            String::from_utf8_lossy(bytes).into_owned()
        };
        let type_name = {
            let t = read(self.type_ptr, self.type_len);
            if t.is_empty() { "RuntimeError".to_string() } else { t }
        };
        let msg = read(self.msg_ptr, self.msg_len);
        format!("{type_name}: {msg}")
    }
}

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
    &callbacks::CALLBACKS as *const ArCallbacks
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


mod callbacks;
