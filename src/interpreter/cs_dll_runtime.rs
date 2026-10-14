// cs_dll_runtime.rs — Runtime bridge for import[cs-dll].
//
// This module manages loaded NativeAOT bridge DLLs and provides ABI-adapted call
// routines for constructors, static methods, and instance methods.
//
// ABI contract (NativeAOT bridge side):
//   int/bool args    → i64 direct
//   float args       → i64 bit pattern via f64::to_bits
//   string args      → (*const u8, i32) = (UTF-8 ptr, byte length); null string = (0, -1)
//   handle args      → i64 object id; null object = 0
//   void return      → ignored
//   int return       → i64 direct
//   float return     → i64 bit pattern
//   string return    → (*mut u8, *mut i32) out-params; free with arrow_bridge_free_str; null = NULL / len < 0
//   handle return    → i64 object id; null = 0
//   errors           → a generated bridge (`arrow --gen-cs-exports`・5-1) catches C# exceptions and argument errors and
//                      keeps the message for `arrow_bridge_take_error(byte** out_ptr, int* out_len)`, which Arrow
//                      calls after every call (NULL = no error). Hand-written bridges without it are unchanged.
//
// The slots are laid out from the stub's declared types, and what each declared type may receive is decided
// in `super::cs_values` (shared with cs-proc · ffi_value_exchange_plan.md 2-1).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::cs_values::{self, CsKind};
use super::Value;

// ---------------------------------------------------------------------------
// Thread-local bridge registry
// ---------------------------------------------------------------------------

thread_local! {
    /// Loaded bridge DLLs, keyed by their canonical path.
    static CS_BRIDGES: RefCell<HashMap<PathBuf, Arc<BridgeLib>>> =
        RefCell::new(HashMap::new());
}

// ---------------------------------------------------------------------------
// BridgeLib — thin wrapper around a loaded libloading Library
// ---------------------------------------------------------------------------

pub struct BridgeLib {
    lib: libloading::Library,
    /// 読み込んだパス（戻り値のオブジェクトの `CsObject::bridge_path`・ブリッジの取り違えの検査に使う）
    path: PathBuf,
    /// `arrow_bridge_take_error`（生成した export だけが持つ・5-1）
    take_error: Option<usize>,
    /// `arrow_bridge_free_str`
    free_str: Option<usize>,
}

impl BridgeLib {
    fn load(path: &Path) -> Result<Arc<Self>, String> {
        let lib = unsafe {
            libloading::Library::new(path)
                .map_err(|e| format!("CsDll: cannot load bridge '{}': {e}", path.display()))?
        };
        let mut b = BridgeLib { lib, path: path.to_path_buf(), take_error: None, free_str: None };
        b.take_error = b.sym_ptr("arrow_bridge_take_error");
        b.free_str = b.sym_ptr("arrow_bridge_free_str");
        Ok(Arc::new(b))
    }

    /// C# が確保した文字列を返す（`arrow_bridge_free_str`）。
    fn free_str(&self, p: *mut u8) {
        if let Some(f) = self.free_str {
            let free_fn: unsafe extern "C" fn(*mut u8) = unsafe { std::mem::transmute(f) };
            unsafe { free_fn(p) };
        }
    }

    /// 呼び出しの後で、生成した export が預けたエラー（C# の例外・引数の幅の誤り・5-1）を引き取る。
    /// ⚠ 呼び出しの直後に必ず呼ぶ（スレッドごとに 1 つしか預からない）。
    fn take_error(&self) -> Result<(), String> {
        let Some(f) = self.take_error else { return Ok(()) };
        let take: unsafe extern "C" fn(*mut *mut u8, *mut i32) = unsafe { std::mem::transmute(f) };
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut len: i32 = -1;
        unsafe { take(&mut ptr, &mut len) };
        if ptr.is_null() || len < 0 {
            return Ok(());
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
        let msg = match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(e) => format!("the C# bridge reported an error whose message is not valid UTF-8 ({e})"),
        };
        self.free_str(ptr);
        Err(msg)
    }

    // Look up an exported symbol by name.
    // Returns a raw usize (function pointer) or None.
    fn sym_ptr(&self, name: &str) -> Option<usize> {
        let name_c = std::ffi::CString::new(name).ok()?;
        let sym: Result<libloading::Symbol<*const ()>, _> =
            unsafe { self.lib.get(name_c.as_bytes_with_nul()) };
        sym.ok().map(|s| *s as usize)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load (and cache) the NativeAOT bridge DLL.
pub fn load_bridge(path: &Path) -> Result<Arc<BridgeLib>, String> {
    let canon = path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    CS_BRIDGES.with(|m| {
        let mut map = m.borrow_mut();
        if let Some(existing) = map.get(&canon) {
            return Ok(Arc::clone(existing));
        }
        let bridge = BridgeLib::load(path)?;
        // ar_event_fire は arrow.exe 内のシンボルで DLL 側からは解決できないため、
        // ブリッジが `arrow_bridge_set_event_fire` をエクスポートしていれば
        // 関数ポインタを注入する (シンボルが無い旧 DLL では何もしない)。
        if let Some(p) = bridge.sym_ptr("arrow_bridge_set_event_fire") {
            let setter: unsafe extern "C" fn(usize) = unsafe { std::mem::transmute(p) };
            unsafe { setter(crate::interpreter::event_loop::ar_event_fire as *const () as usize) };
        }
        map.insert(canon.clone(), Arc::clone(&bridge));
        Ok(bridge)
    })
}

/// Retrieve a previously loaded bridge by path.
pub fn get_bridge(path: &Path) -> Option<Arc<BridgeLib>> {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    CS_BRIDGES.with(|m| m.borrow().get(&canon).cloned())
}

// ---------------------------------------------------------------------------
// Constructor
// ---------------------------------------------------------------------------

/// Call a NativeAOT bridge constructor.
/// Tries `{ClassName}_new_{argc}` first, then `{ClassName}_new` as a fallback.
/// Returns the i64 object handle on success.
/// `param_types` は引数の個数で選んだコンストラクタの型（[`cs_values::ctor_types`]・5-1）。
pub fn call_constructor(
    bridge: &BridgeLib,
    class_name: &str,
    args: &[Value],
    param_types: &[Option<String>],
) -> Result<i64, String> {
    // Prefer the numbered variant first (Calculator_new_0, Calculator_new_1, …)
    let numbered = format!("{class_name}_new_{}", args.len());
    let sym_name_fallback = format!("{class_name}_new");
    let ptr = bridge.sym_ptr(&numbered)
        .or_else(|| bridge.sym_ptr(&sym_name_fallback))
        .ok_or_else(|| {
            format!(
                "CsDll: bridge has no constructor export for '{class_name}' \
                 (tried '{numbered}' and '{sym_name_fallback}')"
            )
        })?;

    // 引数は、型スタブの `__init__`（公開のコンストラクタごとに 1 つ）の型で並べる（5-1）。
    // ⚠ 以前はスタブに型が無く、値の種類で並べていた。`double` を受けるコンストラクタへ `int` を渡すと整数のビット列が
    //   `double` として読まれ、`float` を渡しても手書きの export が `double` の引数をレジスタ（XMM0）で受けて 0 になった
    //   （`cs_interop_test.ar` の `Calculator(100.0)` が黙って 0 で始まっていた）。
    let what = format!("{class_name}.__init__");
    let slots = encode_args(bridge, args, param_types, None, &what)?;
    check_slot_count(slots.len(), &what)?;
    let handle = unsafe { call_variadic(ptr, &slots) };
    bridge.take_error()?;
    // ⚠ 以前は 0（無いオブジェクト）もそのまま `CsObject` にしていた。オブジェクト表は 1 より大きい番号を返す。
    if handle == 0 {
        return Err(format!("CsDll: the constructor of '{class_name}' returned no object"));
    }
    Ok(handle)
}

// ---------------------------------------------------------------------------
// Static method call
// ---------------------------------------------------------------------------

/// Call a static bridge method: `{ClassName}_{method}(args...)`.
/// `param_types` / `ret_type` are the Arrow type annotations of the stub (e.g. "int", "Option[str]", class name).
pub fn call_static(
    bridge: &BridgeLib,
    class_name: &str,
    method: &str,
    args: &[Value],
    param_types: &[Option<String>],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let sym_name = format!("{class_name}_{method}");
    let ptr = bridge.sym_ptr(&sym_name).ok_or_else(|| {
        format!("CsDll: bridge has no export '{sym_name}'")
    })?;
    let what = format!("{class_name}.{method}");
    call_typed(bridge, ptr, args, param_types, ret_type, None, &what)
}

// ---------------------------------------------------------------------------
// Instance method call
// ---------------------------------------------------------------------------

/// Call an instance bridge method: `{ClassName}_inst_{method}(handle, args...)`.
#[allow(clippy::too_many_arguments)]
pub fn call_instance(
    bridge: &BridgeLib,
    class_name: &str,
    handle: i64,
    method: &str,
    args: &[Value],
    param_types: &[Option<String>],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let sym_name = format!("{class_name}_inst_{method}");
    let ptr = bridge.sym_ptr(&sym_name).ok_or_else(|| {
        format!("CsDll: bridge has no export '{sym_name}'")
    })?;
    let what = format!("{class_name}.{method}");
    call_typed(bridge, ptr, args, param_types, ret_type, Some(handle), &what)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// 引数を宣言された型で並べ、戻り値を宣言された型で受け取る（2-1）。
///
/// ⚠ 呼ぶ前に全部確かめる（受け取れない戻り値の型・多すぎる引数）。呼んでからエラーにすると C# 側の
///   副作用だけが起きる。
fn call_typed(
    bridge: &BridgeLib,
    ptr: usize,
    args: &[Value],
    param_types: &[Option<String>],
    ret_type: Option<&str>,
    self_handle: Option<i64>,
    what: &str,
) -> Result<Value, String> {
    let decl = cs_values::decl_of(ret_type)?;
    match (&decl.kind, decl.optional) {
        // 数値・bool の null を表す取り決めがブリッジに無い（手書きの export は値をそのまま返す）
        (CsKind::Int | CsKind::Float | CsKind::Bool, true) => {
            return Err(format!(
                "TypeError: C# method '{what}' returns `{}`, but the cs-dll bridge has no way to return null for it yet",
                decl.shown
            ));
        }
        // ⚠ 以前はハンドルの番号をそのまま整数にしていた
        (CsKind::Any, _) => {
            return Err(format!(
                "TypeError: C# method '{what}' returns `object`, which the cs-dll bridge cannot read \
                 (return a class of this assembly or a primitive instead)"
            ));
        }
        _ => {}
    }
    let mut slots = encode_args(bridge, args, param_types, self_handle, what)?;

    if decl.kind == CsKind::Str {
        // 文字列の戻り値は末尾の出力引数 `(byte** out_ptr, int* out_len)` で受け取る
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: i32 = 0;
        slots.push(&mut out_ptr as *mut *mut u8 as i64);
        slots.push(&mut out_len as *mut i32 as i64);
        check_slot_count(slots.len(), what)?;
        unsafe { call_variadic(ptr, &slots) };
        if let Err(e) = bridge.take_error() {
            if !out_ptr.is_null() {
                bridge.free_str(out_ptr);
            }
            return Err(e);
        }
        if out_ptr.is_null() || out_len < 0 {
            return cs_values::null_result(&decl, what);
        }
        let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len as usize) };
        // ⚠ 以前は `from_utf8_lossy`（壊れたバイトを黙って置き換えていた）
        let s = std::str::from_utf8(bytes).map(str::to_string).map_err(|e| {
            format!("ValueError: C# method '{what}' returned a string that is not valid UTF-8 ({e})")
        });
        // Release the C# heap allocation
        bridge.free_str(out_ptr);
        return Ok(Value::str(s?));
    }

    check_slot_count(slots.len(), what)?;
    let raw = unsafe { call_variadic(ptr, &slots) };
    bridge.take_error()?;
    match &decl.kind {
        CsKind::Void => Ok(Value::None),
        CsKind::Int => Ok(Value::Int(raw)),
        CsKind::Float => Ok(Value::Float(f64::from_bits(raw as u64))),
        CsKind::Bool => Ok(Value::Bool(raw != 0)),
        CsKind::Class(_) => cs_values::object_result(&decl, &bridge.path, false, raw, what),
        CsKind::Str | CsKind::Any => unreachable!("handled above"),
    }
}

/// 引数を i64 のスロットに並べる（文字列は `(ptr, len)` の 2 つ・null の文字列は `(0, -1)`）。
///
/// 並べ方は**宣言された型**で決める（2-1）。⚠ 以前は値の種類で決めていて、`string?` の引数に `None` を渡すと
/// 1 スロットしか積まず、後ろの引数が 1 つずつずれた。運べない値（list など）は 0 にしていた。
/// 宣言の無い引数（コンストラクタ）と `object` の引数は、値の種類で並べる。
fn encode_args(
    bridge: &BridgeLib,
    args: &[Value],
    param_types: &[Option<String>],
    self_handle: Option<i64>,
    what: &str,
) -> Result<Vec<i64>, String> {
    let mut slots: Vec<i64> = Vec::new();
    if let Some(h) = self_handle {
        slots.push(h);
    }
    for (i, v) in args.iter().enumerate() {
        let decl = cs_values::decl_of(param_types.get(i).and_then(|t| t.as_deref()))?;
        let any = decl.kind == CsKind::Any;
        match (&decl.kind, v) {
            (CsKind::Str, Value::Str(s)) | (CsKind::Any, Value::Str(s)) => {
                let bytes = s.as_bytes();
                slots.push(bytes.as_ptr() as i64);
                slots.push(bytes.len() as i64);
            }
            (CsKind::Str, Value::None) if decl.optional => {
                slots.push(0);
                slots.push(-1);
            }
            (CsKind::Int, Value::Int(n)) => slots.push(*n),
            (CsKind::Int, Value::UInt(n)) => slots.push(*n as i64),
            (CsKind::Float, Value::Float(f)) => slots.push(f.to_bits() as i64),
            (CsKind::Bool, Value::Bool(b)) => slots.push(i64::from(*b)),
            (CsKind::Class(_), Value::CsObject(o)) => {
                cs_values::check_same_bridge(o, &bridge.path, false, i, what)?;
                slots.push(o.handle);
            }
            (CsKind::Class(_), Value::None) if decl.optional => slots.push(0),
            (CsKind::Any, Value::Int(n)) => slots.push(*n),
            (CsKind::Any, Value::UInt(n)) => slots.push(*n as i64),
            (CsKind::Any, Value::Float(f)) => slots.push(f.to_bits() as i64),
            (CsKind::Any, Value::Bool(b)) => slots.push(i64::from(*b)),
            (CsKind::Any, Value::CsObject(o)) => {
                cs_values::check_same_bridge(o, &bridge.path, false, i, what)?;
                slots.push(o.handle);
            }
            (CsKind::Any, Value::None) => slots.push(0),
            (CsKind::Int | CsKind::Float | CsKind::Bool, Value::None) if decl.optional => {
                return Err(format!(
                    "TypeError: argument {} of C# method '{what}' is None, but the cs-dll bridge has no way to pass \
                     null for `{}` yet",
                    i + 1,
                    decl.shown
                ));
            }
            (_, Value::Int(_) | Value::UInt(_) | Value::Float(_) | Value::Bool(_) | Value::Str(_)
                | Value::CsObject(_) | Value::None) if !any => {
                return Err(format!(
                    "TypeError: argument {} of C# method '{what}' must be `{}`, not '{}'",
                    i + 1,
                    decl.shown,
                    crate::interpreter::ops::typecheck::runtime_type_name(v)
                ));
            }
            (_, other) => return Err(cs_values::unsupported_arg(other, i, what)),
        }
    }
    Ok(slots)
}

/// `call_variadic` が渡せるのは 8 スロットまで（⚠ 以前は 9 個目から黙って捨てていた）。
fn check_slot_count(n: usize, what: &str) -> Result<(), String> {
    if n > 8 {
        Err(format!(
            "TypeError: C# method '{what}' needs {n} argument slots, but the cs-dll bridge passes at most 8 \
             (a string takes 2, a string return value 2 more)"
        ))
    } else {
        Ok(())
    }
}

/// Call an arbitrary C function pointer with up to 8 i64 arguments.
/// Uses the x86-64 Windows ABI (RCX, RDX, R8, R9 then stack).
/// Returns the i64 result (for void functions the result is undefined/zero).
/// ⚠ 呼ぶ側が `check_slot_count` で 8 以下を確かめておくこと。
unsafe fn call_variadic(ptr: usize, args: &[i64]) -> i64 {
    // Pad to at least 4 args (Windows ABI requires shadow space for 4 regs).
    let a0 = args.first().copied().unwrap_or(0);
    let a1 = args.get(1).copied().unwrap_or(0);
    let a2 = args.get(2).copied().unwrap_or(0);
    let a3 = args.get(3).copied().unwrap_or(0);
    let a4 = args.get(4).copied().unwrap_or(0);
    let a5 = args.get(5).copied().unwrap_or(0);
    let a6 = args.get(6).copied().unwrap_or(0);
    let a7 = args.get(7).copied().unwrap_or(0);

    match args.len() {
        0 => {
            let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(ptr);
            f()
        }
        1 => {
            let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(ptr);
            f(a0)
        }
        2 => {
            let f: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1)
        }
        3 => {
            let f: unsafe extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2)
        }
        4 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2, a3)
        }
        5 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2, a3, a4)
        }
        6 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2, a3, a4, a5)
        }
        7 => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2, a3, a4, a5, a6)
        }
        _ => {
            let f: unsafe extern "C" fn(i64, i64, i64, i64, i64, i64, i64, i64) -> i64 = std::mem::transmute(ptr);
            f(a0, a1, a2, a3, a4, a5, a6, a7)
        }
    }
}
