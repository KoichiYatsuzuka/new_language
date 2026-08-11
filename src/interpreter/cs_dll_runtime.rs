// cs_dll_runtime.rs — Runtime bridge for import[cs-dll].
//
// This module manages loaded NativeAOT bridge DLLs and provides ABI-adapted call
// routines for constructors, static methods, and instance methods.
//
// ABI contract (NativeAOT bridge side):
//   int/bool args    → i64 direct
//   float args       → i64 bit pattern via f64::to_bits
//   string args      → (*const u8, i32) = (UTF-8 ptr, byte length)
//   handle args      → i64 object id
//   void return      → ignored
//   int return       → i64 direct
//   float return     → i64 bit pattern
//   string return    → (*mut u8, *mut i32) out-params; free with arrow_bridge_free_str
//   handle return    → i64 object id

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
}

impl BridgeLib {
    fn load(path: &Path) -> Result<Arc<Self>, String> {
        let lib = unsafe {
            libloading::Library::new(path)
                .map_err(|e| format!("CsDll: cannot load bridge '{}': {e}", path.display()))?
        };
        Ok(Arc::new(BridgeLib { lib }))
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
pub fn call_constructor(
    bridge: &BridgeLib,
    class_name: &str,
    args: &[Value],
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

    let handle = unsafe { call_bridge_fn(ptr, args, None)? };
    Ok(handle)
}

// ---------------------------------------------------------------------------
// Static method call
// ---------------------------------------------------------------------------

/// Call a static bridge method: `{ClassName}_{method}(args...)`.
/// `ret_type` is the Arrow type annotation string (e.g. "int", "float", "str", class name).
pub fn call_static(
    bridge: &BridgeLib,
    class_name: &str,
    method: &str,
    args: &[Value],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let sym_name = format!("{class_name}_{method}");
    let ptr = bridge.sym_ptr(&sym_name).ok_or_else(|| {
        format!("CsDll: bridge has no export '{sym_name}'")
    })?;

    if matches_str_ret(ret_type) {
        return unsafe { call_returning_str(bridge, ptr, args, None) };
    }

    let raw = unsafe { call_bridge_fn(ptr, args, None)? };
    Ok(raw_to_value(raw, ret_type))
}

// ---------------------------------------------------------------------------
// Instance method call
// ---------------------------------------------------------------------------

/// Call an instance bridge method: `{ClassName}_inst_{method}(handle, args...)`.
pub fn call_instance(
    bridge: &BridgeLib,
    class_name: &str,
    handle: i64,
    method: &str,
    args: &[Value],
    ret_type: Option<&str>,
) -> Result<Value, String> {
    let sym_name = format!("{class_name}_inst_{method}");
    let ptr = bridge.sym_ptr(&sym_name).ok_or_else(|| {
        format!("CsDll: bridge has no export '{sym_name}'")
    })?;

    if matches_str_ret(ret_type) {
        return unsafe { call_returning_str(bridge, ptr, args, Some(handle)) };
    }

    let raw = unsafe { call_bridge_fn(ptr, args, Some(handle))? };

    // Void returns (fn returns void in C#) → result is undefined; return None
    if ret_type.map(|t| t == "None" || t == "void").unwrap_or(false) {
        return Ok(Value::None);
    }
    Ok(raw_to_value(raw, ret_type))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert an Arrow Value to the i64 representation expected by the bridge.
fn value_to_i64(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n,
        Value::UInt(n) => *n as i64,
        Value::Bool(b) => if *b { 1 } else { 0 },
        Value::Float(f) => f.to_bits() as i64,
        Value::CsObject(o) => o.handle,
        Value::None => 0,
        _ => 0,
    }
}

/// Convert a raw i64 result to an Arrow Value based on the return type annotation.
fn raw_to_value(raw: i64, ret_type: Option<&str>) -> Value {
    match ret_type {
        Some("float") => Value::Float(f64::from_bits(raw as u64)),
        Some("bool") => Value::Bool(raw != 0),
        Some("None") | Some("void") => Value::None,
        _ => Value::Int(raw),
    }
}

fn matches_str_ret(ret_type: Option<&str>) -> bool {
    matches!(ret_type, Some("str"))
}

/// Call a bridge function that returns a UTF-8 string via out-params
/// `(byte** out_ptr, int* out_len)` appended after the normal args.
/// Calls `arrow_bridge_free_str` to free the returned buffer.
unsafe fn call_returning_str(
    bridge: &BridgeLib,
    ptr: usize,
    args: &[Value],
    self_handle: Option<i64>,
) -> Result<Value, String> {
    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: i32 = 0;

    // Build arg i64 array
    let mut i64_args: Vec<i64> = Vec::new();
    if let Some(h) = self_handle {
        i64_args.push(h);
    }
    for v in args {
        match v {
            Value::Str(s) => {
                // String arg — push ptr and len separately
                let bytes = s.as_bytes();
                i64_args.push(bytes.as_ptr() as i64);
                i64_args.push(bytes.len() as i64);
            }
            other => i64_args.push(value_to_i64(other)),
        }
    }
    // Append out-param pointers
    i64_args.push(&mut out_ptr as *mut *mut u8 as i64);
    i64_args.push(&mut out_len as *mut i32 as i64);

    // We call as a variadic-style function.  Since we pre-built the arg list,
    // dispatch through a trampoline indexed on argc.
    call_variadic(ptr, &i64_args);

    if out_ptr.is_null() || out_len < 0 {
        return Ok(Value::None);
    }
    let bytes = std::slice::from_raw_parts(out_ptr, out_len as usize);
    let s = String::from_utf8_lossy(bytes).into_owned();

    // Release the C# heap allocation
    if let Some(free_ptr) = bridge.sym_ptr("arrow_bridge_free_str") {
        let free_fn: unsafe extern "C" fn(*mut u8) = std::mem::transmute(free_ptr);
        free_fn(out_ptr);
    }
    Ok(Value::str(s))
}

/// Low-level bridge call: passes i64 args (possibly with a leading self handle)
/// through a cdecl/stdcall function and returns a raw i64 result.
unsafe fn call_bridge_fn(
    ptr: usize,
    args: &[Value],
    self_handle: Option<i64>,
) -> Result<i64, String> {
    let mut i64_args: Vec<i64> = Vec::new();
    if let Some(h) = self_handle {
        i64_args.push(h);
    }
    for v in args {
        match v {
            Value::Str(s) => {
                let bytes = s.as_bytes();
                i64_args.push(bytes.as_ptr() as i64);
                i64_args.push(bytes.len() as i64);
            }
            other => i64_args.push(value_to_i64(other)),
        }
    }
    Ok(call_variadic(ptr, &i64_args))
}

/// Call an arbitrary C function pointer with up to 8 i64 arguments.
/// Uses the x86-64 Windows ABI (RCX, RDX, R8, R9 then stack).
/// Returns the i64 result (for void functions the result is undefined/zero).
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
