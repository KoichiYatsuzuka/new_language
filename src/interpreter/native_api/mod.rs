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
/// `===`（同一性・`BinOp::RefEq`）。生成コードの `binop_code` と同じ番号（1-12）。
/// ⚠ 以前は読み取りに無く、コンパイル済みコードの `===` が必ず `NativeError: invalid binop code 20` になっていた。
pub const OP_REF_EQ: i32 = 20;

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

/// ネイティブコードの `for` の反復状態（`ar_iter_from` / `ar_iter_next`）。
pub(crate) enum NativeIter {
    /// 実体化した要素と次の位置（`list` / `tuple` / `str` / `dict` のキー / `fixed_list`）。
    Items(Vec<Value>, usize),
    /// インタープリタの反復プロトコル: `make_for_iterator` が作ったジェネレータを `gen_next` で進める
    /// （VM の `GetIter` / `ForIter` と同じ。`set`・`bytes`・`enumerate` / `zip`・ジェネレータ・`__iter__` を持つ
    /// インスタンスなど・native_object_access_plan.md 1-3）。⚠ 以前はこれらが「not iterable in native context」だった。
    Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>),
}

struct NativeCallState {
    /// Raw pointer to the currently executing Interpreter.
    interp_ptr: *mut Interpreter,
    /// Value arena (see handle encoding above).
    arena: Vec<Value>,
    /// Iterator table. Iter handles are encoded as -(idx+2) so they are all ≤ -2.
    iter_table: Vec<NativeIter>,
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
    /// `ArPtrProtocol` arguments built by `ar_protocol_arg` (their element-type strings); cleared at outermost exit (4-4).
    protocol_args: Vec<(i64, crate::interpreter::PtrProtocolArg)>,
    /// Native method dispatch table: class_name → method_name → (fn_ptr, クラスを定義したモジュールの大域の番号)。
    /// 大域の番号は呼ぶ間だけ差し替える先（`NativeFnRef::globals` と同じ・1-7）。
    /// ⚠ 2 段にしてあるのは `&str` のまま引くため（10-1）。以前は `(String, String)` をキーにしていて、引くたびに
    ///   `String` を 2 個作っていた（生成のたびに・ネイティブのクラスでなくても・1 回 約 50〜100 ns）。
    /// ⚠ キーは**クラスの番号**（`class_id`・11-4）。以前は素のクラス名で、本体（や別のモジュール）の同じ名前のクラスの
    ///   生成・メソッド呼び出しでも、コンパイル済みのモジュールのメソッドが呼ばれていた（型はモジュールごとに別・10-8）。
    native_methods: HashMap<u32, HashMap<String, (usize, u32)>>,
    /// 文字列のリテラルの表（11-2・D-12）: 呼び出し箇所のセルに入れた番号 − 1 → 1 度だけ作った文字列。VM の定数表と同じく、
    /// 評価のたびに作り直さず参照数の加算で使う。番号はプロセスで一意（`ar_make_str_ic`）、表はスレッドごと
    /// （`Rc<str>` はスレッドをまたげないので、非同期タスクのワーカーは自分の表に作る）。いちばん外の呼び出しを抜けても捨てない。
    str_consts: Vec<Option<Value>>,
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
            protocol_args: Vec::new(),
            native_methods: HashMap::new(),
            str_consts: Vec::new(),
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
            st.protocol_args.clear();
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
            st.protocol_args.clear();
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
/// `class_id` はモジュールが定義したクラスの番号（同じ名前の別のクラスと混ざらない・11-4）。
/// `globals` はクラスを定義したモジュールの大域の番号（呼ぶ間だけそこへ差し替える・1-7）。
pub fn register_native_method(class_id: u32, method_name: &str, fn_ptr: usize, globals: u32) {
    STATE.with(|s| {
        s.borrow_mut()
            .native_methods
            .entry(class_id)
            .or_default()
            .insert(method_name.to_string(), (fn_ptr, globals));
    });
}

/// Look up the raw function pointer for a native class method.
pub fn lookup_native_method_ptr(class_id: u32, method_name: &str) -> Option<usize> {
    lookup_native_method(class_id, method_name).map(|(ptr, _)| ptr)
}

/// ネイティブのメソッドの関数ポインタと、走らせる大域の番号（1-7）。
pub(crate) fn lookup_native_method(class_id: u32, method_name: &str) -> Option<(usize, u32)> {
    STATE.with(|s| {
        s.borrow()
            .native_methods
            .get(&class_id)
            .and_then(|m| m.get(method_name))
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
    let class = match &obj {
        Value::Instance(rc) => rc.borrow().class.clone(),
        _ => return None,
    };
    let (fn_ptr, globals) = lookup_native_method(class.class_id, method_name)?;
    Some(call_native_method_at(interp, fn_ptr, globals, class, obj, &arg_vals))
}

/// 引いてあるネイティブのメソッド（`fn_ptr` / `globals`）を、受け手 `obj` と宣言の順の引数で呼ぶ（6-3）。
fn call_native_method_at(
    interp: &mut Interpreter,
    fn_ptr: usize,
    globals: u32,
    class: std::rc::Rc<crate::interpreter::ClassValue>,
    obj: Value,
    arg_vals: &[Value],
) -> Result<Value, String> {
    let is_outermost = enter_native_call(interp as *mut Interpreter);

    let obj_h = push_handle(obj);
    let mut all_args: Vec<i64> = Vec::with_capacity(1 + arg_vals.len());
    all_args.push(obj_h);
    for v in arg_vals {
        all_args.push(push_handle(v.clone()));
    }

    // メソッドはクラスを定義したモジュールの大域で、受け手のクラスの文脈で走る（1-7・1-9）
    let prev = interp.enter_native_method(globals, class);
    let result_h = unsafe {
        let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(fn_ptr);
        func(all_args.as_ptr(), all_args.len() as i32)
    };
    interp.leave_native_method(prev);

    if let Some(err) = take_error() {
        abort_native_call(is_outermost);
        return Err(err);
    }

    if let Some((exc_type, exc_msg)) = take_pending_raise() {
        abort_native_call(is_outermost);
        return Err(format!("{exc_type}: {exc_msg}"));
    }

    Ok(exit_native_call(result_h, is_outermost))
}

/// インタープリタからコンパイル済みのメソッドを呼ぶ入口（native_object_access_plan.md 6-3）。引数はメソッドの宣言
/// どおりに束縛してから（[`Interpreter::bind_native_args`]: 名前つき引数・既定値・`let self` / `let` 仮引数の複製を
/// インタープリタの呼び出しと同じ規則で行う）、宣言の順に渡す（表は 1 回だけ引く）。
/// `recv_mut` は受け手が `mut` の値か（6-1）。ネイティブの表に無ければ `None`（呼ぶ側は先に表を引かなくてよい）。
/// ⚠ 以前は評価済みの引数をそのまま渡し、`mut` の受け手・引数を `let` で受けても複製せず（§0.3 ⑬）、名前つき引数を
///   呼び出しの順に並べ（⑭）、既定値を省くと引数の配列の外を読んでいた（⑮）。
pub fn try_dispatch_native_method_evaled(
    interp: &mut Interpreter,
    obj: Value,
    method_name: &str,
    evaled: &[(Option<String>, Value, bool)],
    recv_mut: bool,
) -> Option<Result<Value, String>> {
    let class = match &obj {
        Value::Instance(rc) => rc.borrow().class.clone(),
        _ => return None,
    };
    let (fn_ptr, globals) = lookup_native_method(class.class_id, method_name)?;
    // 呼ばれた側の宣言（`call_method_common` と同じ引き方）。ネイティブの表に載るのはモジュールのクラスの本体に
    // 書かれた（自動生成の `__init__` を含む）メソッドだけで、オーバーロードはコンパイルしない（1-8）ので、1 つに
    // 決まらなければ表と宣言がずれている。黙って素通しせずエラーにする。
    let decl = match class.methods.get(method_name) {
        Some(ov) if ov.len() == 1 => ov[0].clone(),
        _ => {
            return Some(Err(format!(
                "NativeError: compiled method '{}.{method_name}' has no single declaration to bind its arguments",
                class.name
            )))
        }
    };
    // `self` を取らない宣言（静的メソッド）はインスタンスからは呼べない。振り分けの続き（静的メソッドのエラー）へ戻す。
    if decl.params.first().is_none_or(|p| p.name != "self") {
        return None;
    }
    let mut vals = match interp.bind_native_args(&decl.params, decl.globals, evaled, Some(obj), recv_mut) {
        Ok(v) => v,
        Err(e) => return Some(Err(e)),
    };
    let receiver = vals.remove(0);
    Some(call_native_method_at(interp, fn_ptr, globals, class, receiver, &vals))
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
        OP_REF_EQ => Some(BinOp::RefEq),
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

    // ── Strict argument conversion (fields 38-39・ffi_value_exchange_plan.md 7-12) ──
    // ⚠ 末尾に足すこと。中継コード（cpp / rs）は**この構造体の先頭部分だけ**を宣言しているので、途中に足すと
    //   既存の生成コードの添字がずれる（LLVM の生成コードは添字で引く・`llvm_codegen` の `CB_*`）。
    /// `bool` の引数（C の `bool`・Rust の `bool`）を 0 / 1 にする。`True` / `False` 以外はエラーを設定して 0。
    pub to_bool: extern "C" fn(i64) -> i32,
    /// 引数の変換でエラーが設定されたか（1 = 設定された）。中継コードは引数を全部変換した後にこれを見て、
    /// 1 なら C / Rust の関数を**呼ばずに**戻る（呼び出しの後でエラーが例外になる）。
    pub pending_error: extern "C" fn() -> i32,

    // ── Raw pointers (fields 40-43・ffi_value_exchange_plan.md 4-3・D-7・D-8・D-13) ──
    /// `char*` / `wchar_t*` の戻り値を `bytes` にする（`len` バイトをコピーする。NUL 終端の長さは中継コードが数える）。
    pub make_bytes: extern "C" fn(*const u8, i64) -> i64,
    /// ポインタの戻り値を借用の `Ptr[elem]` にする（Arrow は解放しない・D-7）。アドレス 0（NULL）は `None`（D-8）。
    pub make_ptr: extern "C" fn(u64, *const u8, i32) -> i64,
    /// `const char*` の引数: `bytes`（NUL を付けたコピー・最外のネイティブ呼び出しが終わるまで有効）か `None`（NULL）。
    /// `str` はエラー（文字コードを書かせる・D-13）。
    pub bytes_arg: extern "C" fn(i64) -> *const u8,
    /// `void*` / 構造体ポインタの引数: `Ptr`（要素型が `elem` と合うもの。`"void"` はどれでも）か `None`（NULL）。
    pub ptr_arg: extern "C" fn(i64, *const u8, i32) -> u64,

    // ── ArPtrProtocol arguments on the handle path (fields 44-45・4-4) ──
    /// `ArPtrProtocol` の引数: `Ptr`（か `None`）から構造体を組み立てて `out`（中継コードの局所変数）へ書く。
    /// `writable` は `ArPtrProtocol*`（C の側で解放できる）。要素の型名の文字列は最外の呼び出しが終わるまで生きる。
    pub protocol_arg: extern "C" fn(i64, *mut u8, i32),
    /// `ArPtrProtocol*` の引数の呼び出し後: C が `address` を 0 に書き戻していたら、その `Ptr` を使えなくする。
    pub protocol_done: extern "C" fn(i64, u64),

    // ── ArPtrProtocol return values on the handle path (field 46・4-5) ──
    /// `ArPtrProtocol` の戻り値（中継コードの局所変数を指す）を `Ptr`（所有・STATIC）にする。`structs` はそのヘッダの
    /// 構造体の名前を `,` でつないだもの（要素の型名の照合）。誤りはエラーを設定して `None`。
    pub protocol_ret: extern "C" fn(*const u8, *const u8, i32) -> i64,

    // ── Struct values (fields 47-48・4-6) ──
    /// 値返しの構造体（正確なレイアウト）: C のバイト列をクラス `cls` のインスタンスへ写す（コンストラクタは呼ばない）。
    pub make_raw_instance: extern "C" fn(i64, *const u8, i64) -> i64,
    /// 値返しの構造体（ポインタのフィールドあり）: 変換済みのフィールドの値を宣言順に入れたインスタンスを作る（同上）。
    pub make_instance: extern "C" fn(i64, *const i64, i32) -> i64,

    // ── Output buffers (field 49・4-7) ──
    /// `char*`（書き込み用）の引数: `bytes` のコピー（NUL を含んでよい・末尾に NUL を 1 つ足す）を作り、そのアドレスを返して
    /// 長さを `len` に書く。`None` は NULL。呼び出しの後で中継コードが `make_bytes` で読み直して `write_handle` で書き戻す。
    pub bytes_buf_arg: extern "C" fn(i64, *mut i64) -> *mut u8,

    // ── Builtins by name (field 50・native_object_access_plan.md 1-3) ──
    /// 組み込み関数（`vm::compiler::VM_BUILTIN_NAMES`）を VM の `CallBuiltin` と同じ入口
    /// （`Interpreter::call_builtin_by_name_evaled`）で呼ぶ。グローバルから値として引かない。
    pub call_builtin: extern "C" fn(*const u8, i32, *const i64, i32) -> i64,

    // ── Attribute assignment of compiled Arrow code (field 51・native_object_access_plan.md 1-4) ──
    /// `obj.attr = v`: VM の `SetAttr` と同じ `Interpreter::attr_assign_evaled`（`static mut`・不変インスタンスの
    /// 判定を含む）。⚠ `set_attr`（field 13）は `import[rs]` の中継コードが Arrow 側の写しを更新する経路
    /// （`set_attr_val`・setter へ再入させない）なので、意味を変えずに残してある。
    pub assign_attr: extern "C" fn(i64, *const u8, i32, i64),

    // ── Trait-qualified fields of compiled Arrow code (fields 52-53・1-4) ──
    /// `obj::Trait.attr`: VM の `GetTraitAttr` と同じ `trait_access_evaled`。引数は (obj, trait 名, 属性名)。
    pub get_trait_attr: extern "C" fn(i64, *const u8, i32, *const u8, i32) -> i64,
    /// `obj::Trait.attr = v`: VM の `SetTraitAttr` と同じ `trait_assign_evaled`。⚠ 以前の生成コードはこの代入を
    /// 黙って出さず、trait を実装したクラスの `__init__` が trait のフィールドを設定しなかった。
    pub assign_trait_attr: extern "C" fn(i64, *const u8, i32, *const u8, i32, i64),

    // ── Value passing of compiled Arrow code (fields 54-55・1-5・D-8) ──
    /// `let` 仮引数へ `mut` の値を渡すときの複製（VM の束縛と同じ `copy_value`）。
    pub copy_arg: extern "C" fn(i64) -> i64,
    /// 名前によるメソッド呼び出し（`call_method` と同じ）＋ `mut_mask`（bit 0 = 受け手・bit i+1 = 引数 i が
    /// `mut` の値か）で、呼ばれた側の `let self` / `let` 仮引数の分を複製してから呼ぶ。
    pub call_method_m: extern "C" fn(i64, *const u8, i32, *const i64, i32, u32) -> i64,

    // ── Declarations of compiled Arrow code (fields 56-57・1-6) ──
    /// `mut x = e`（mode 0）/ `let x = <mut の名前・式>`（mode 1）の複製。VM の store 命令と同じ `bind_copy`。
    pub bind_value: extern "C" fn(i32, i64) -> i64,
    /// `let x = <グローバル等の名前>`: ソースの可変性を実行時に引いて決める（VM の `StoreLocalFromIdent` と同じ
    /// `vm_let_value_from_ident`）。引数は (名前, 値)。
    pub let_from_name: extern "C" fn(*const u8, i32, i64) -> i64,

    // ── Attribute reads with per-site inline caches (fields 58-60・2-2・D-1) ──
    /// `obj.attr`（IC 付き）。引数は (obj, 名前, 箇所ごとの IC のセル)。命中経路・ミス経路とも VM の `GetAttr` と同じ。
    pub get_attr_ic: extern "C" fn(i64, *const u8, i32, *mut u64) -> i64,
    /// `float` と分かっているフィールドの読み出し（IC 付き）。変換規則は `get_float_field` と同じ。
    pub get_float_field_ic: extern "C" fn(i64, *const u8, i32, *mut u64) -> f64,
    /// `int` と分かっているフィールドの読み出し（IC 付き）。変換規則は `get_int_field` と同じ。
    pub get_int_field_ic: extern "C" fn(i64, *const u8, i32, *mut u64) -> i64,
    /// `obj.attr = v`（IC 付き・field 61・2-3）。引数は (obj, 名前, 値, 箇所ごとの書き込みの IC のセル)。
    /// VM の `SetAttr` と同じ `attr_assign_ic`。
    pub assign_attr_ic: extern "C" fn(i64, *const u8, i32, i64, *mut u64),

    // ── Class context of direct method calls (fields 62-63・1-9) ──
    /// 生成コードがほかのクラスのメソッドを直接呼ぶ前に、`current_class` を受け手のクラスにする。戻り値は前の文脈の印。
    pub enter_method_ctx: extern "C" fn(i64) -> i64,
    /// `enter_method_ctx` の印を受け取り、前の文脈に戻す。
    pub leave_method_ctx: extern "C" fn(i64),

    // ── Guard of annotation-based direct calls (field 64・3-2・D-2) ──
    /// 受け手が名前のクラスのインスタンス（class_id が一致）で、`mut self` のメソッドなら不変インスタンスでないときだけ 1。
    /// 引数は (受け手, クラス名, 箇所ごとのセル（class_id を 1 回だけ引いて覚える）, `mut self` か)。
    pub method_guard: extern "C" fn(i64, *const u8, i32, *mut u64, i32) -> i32,

    // ── Typed attribute writes (fields 65-66・4-2) ──
    /// `obj.attr = <int>`（IC 付き）。値を箱に入れずに渡す。中身は `assign_attr_ic`（field 61）と同じ。
    pub assign_int_field_ic: extern "C" fn(i64, *const u8, i32, i64, *mut u64),
    /// `obj.attr = <float>`（IC 付き）。
    pub assign_float_field_ic: extern "C" fn(i64, *const u8, i32, f64, *mut u64),

    // ── Construction of module classes (field 67・5-1・D-4) ──
    /// `Class(args)`: VM の `Call` と同じ `instantiate_evaled` を直接呼ぶ。引数は (クラス名, 引数, 個数, `mut_mask`)。
    /// `mut_mask` の bit i+1 = 引数 i が `mut` の値か（`call_method_m` と同じ並び・bit 0 は使わない）。
    pub construct: extern "C" fn(*const u8, i32, *const i64, i32, u32) -> i64,

    // ── Built-ins with known argument types (fields 68-69・5-2) ──
    /// `len(x)`: 組み込みの `len` と同じ `len_of` を呼び、生の int で返す。
    pub len: extern "C" fn(i64) -> i64,
    /// `str(<int>)`: 組み込みの `str` と同じ `call_type_by_name_evaled("str", ..)`（値を箱に入れずに渡す）。
    pub str_of_int: extern "C" fn(i64) -> i64,

    // ── Construction with a per-site class cache (field 70・10-2・D-11) ──
    /// `Class(args)` の呼び出し箇所ごとのセルつき版。引数は (セル, クラス名, 引数, 個数, `mut_mask`)。セルには前回
    /// 名前で引いたクラスの `class_id + 1` が入り（0 = まだ）、当たれば名前で引かずにインタープリタの表から引く。
    pub construct_ic: extern "C" fn(*mut u64, *const u8, i32, *const i64, i32, u32) -> i64,

    // ── Releasing a `for` iterator when its loop exits (field 71・11-1・D-12) ──
    /// `for` のループを抜けた（終わり・`break`・ループの中の `return` / `block_return`）ときに、そのループの
    /// イテレータと、それより後に作ったイテレータを表から捨てる。VM はイテレータを関数のローカルの欄に置くので、
    /// 関数を抜ければ消える。⚠ 以前は、いちばん外のネイティブ呼び出しが終わるまで表に残り、リストのイテレータが
    /// 持つ要素の写しも解放されなかった（§0.6 要因 5）。
    pub iter_drop: extern "C" fn(i64),

    // ── String literals built once per site (field 72・11-2・D-12) ──
    /// 文字列のリテラル `"..."` の呼び出し箇所ごとのセルつき版。引数は (セル, バイト列, 長さ)。セルにはプロセスで一意の
    /// 番号が入り（0 = まだ）、文字列はスレッドの状態の表から引いてアリーナへ積む（VM の `Const` と同じく作り直さない）。
    /// ⚠ 以前は評価のたびに `make_str`（UTF-8 の検査・確保）を呼び、ループの中のリテラル 1 個に約 90 ns かかっていた（§0.6 要因 6）。
    pub make_str_ic: extern "C" fn(*mut u64, *const u8, i32) -> i64,

    // ── Method call by name with a per-site cell and the FFI call info (field 73・11-3・D-12) ──
    /// 名前によるメソッド呼び出し（`call_method_m` と同じ引数）に、箇所ごとのセルと FFI の検査の情報を足したもの。引数は
    /// (セル, 受け手, 名前, 長さ, 引数, 個数, `mut_mask`, 情報, 長さ)。セルが空のときだけ名前を検査する。情報は
    /// `表示名 \x1f 宣言型 \x1f 位置`（型検査がこの呼び出しに付けた戻り値の型が無ければ null）で、受け手が外部言語の
    /// ものなら VM の `vm_method_call_other` と同じ境界の検査（`check_ffi_return`）をする。
    /// ⚠ 以前の生成コードの道（field 55）は検査をせず、Python が宣言と違う型を返しても黙って通していた（§0.6 要因 7）。
    pub call_method_ic: extern "C" fn(*mut u64, i64, *const u8, i32, *const i64, i32, u32, *const u8, i32) -> i64,

    // ── Assignment through an attribute path in one call (field 74・12-3・D-13) ──
    /// `a.b.c = v` を 1 回で行う。引数は (根のハンドル, 段の表, 段の数, 値の種類, 値)。段の表は段ごとに
    /// (名前のポインタ, 長さ, IC のセル) の 3 つの `i64` で、最後の段が書き込む属性。途中の段は VM の `GetAttr` と同じ
    /// IC で読み（アリーナに積まない）、最後は `attr_assign_ic` で書く。値の種類は 0 = ハンドル・1 = `int`・
    /// 2 = `float`（ビット列）。⚠ 生成コードは値を先に評価するので、値の評価が何も起こさない形（リテラル・変数・
    /// 数の演算）のときだけ使う（途中の読み出しの順が VM と入れ替わっても見分けられない）。
    pub assign_path_ic: extern "C" fn(i64, *const i64, i32, i32, i64),
}

// ── Callback implementations ─────────────────────────────────────────────────


mod callbacks;

#[cfg(test)]
mod tests {
    /// 生成コードが渡す演算の番号（`binop_code`）を、コールバックがすべて同じ演算に読み戻す（1-12・§0.3 ⑯）。
    /// ⚠ 以前は `===`（20）が読み取りに無く、コンパイル済みコードの `===` が必ずエラーになっていた。
    #[test]
    fn every_binop_code_is_decoded_by_the_callbacks() {
        use crate::ast::BinOp;
        // ⚠ `BinOp` に種類を足すとこの `match` が通らなくなる。足した種類を下の `all` にも入れること。
        fn listed(op: &BinOp) {
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::FloorDiv | BinOp::Mod
                | BinOp::Pow | BinOp::Eq | BinOp::RefEq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq
                | BinOp::Gt | BinOp::GtEq | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
                | BinOp::LShift | BinOp::RShift | BinOp::In | BinOp::NotIn => {}
                // 短絡評価なのでコールバックへは渡さない（`gen_bool_logic`）
                BinOp::And | BinOp::Or => {}
            }
        }
        let all = [
            BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::FloorDiv, BinOp::Mod, BinOp::Pow,
            BinOp::Eq, BinOp::RefEq, BinOp::NotEq, BinOp::Lt, BinOp::LtEq, BinOp::Gt, BinOp::GtEq,
            BinOp::BitAnd, BinOp::BitOr, BinOp::BitXor, BinOp::LShift, BinOp::RShift, BinOp::In,
            BinOp::NotIn,
        ];
        for op in &all {
            listed(op);
            let code = crate::partial_compiler::llvm_codegen::binop_code(op);
            assert_eq!(
                super::i32_to_binop(code).as_ref(),
                Some(op),
                "binop code {code} of {op:?} is not decoded back by the callbacks"
            );
        }
    }
}
