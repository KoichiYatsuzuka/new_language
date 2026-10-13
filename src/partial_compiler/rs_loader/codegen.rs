// rs_loader/codegen.rs — ラッパー Rust ソース生成: ABI_HEADER 定数、lib_rs、struct/method ラッパー、arena、param/return 変換、fn/digest ラッパー。

use super::*;

// ── Call-through lib.rs generation ───────────────────────────────────────────

const ABI_HEADER: &str = r#"// Auto-generated — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_imports, unused_mut,
         clippy::missing_safety_doc)]

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicI64, Ordering};
use std::collections::HashMap;

const TL_NONE:  i64 = 0;
const TL_TRUE:  i64 = 1;
const TL_FALSE: i64 = 2;
const TL_EXCEPTION: i64 = -2;

#[repr(C)]
struct ArCallbacks {
    make_int:      unsafe extern "C" fn(i64) -> i64,
    make_float:    unsafe extern "C" fn(f64) -> i64,
    make_bool:     unsafe extern "C" fn(i32) -> i64,
    make_str:      unsafe extern "C" fn(*const u8, i32) -> i64,
    make_list:     unsafe extern "C" fn(*const i64, i32) -> i64,
    make_tuple:    unsafe extern "C" fn(*const i64, i32) -> i64,
    make_dict:     unsafe extern "C" fn(*const i64, *const i64, i32) -> i64,
    make_none:     unsafe extern "C" fn() -> i64,
    is_truthy:     unsafe extern "C" fn(i64) -> i32,
    binop:         unsafe extern "C" fn(i32, i64, i64) -> i64,
    unop:          unsafe extern "C" fn(i32, i64) -> i64,
    call_fn:       unsafe extern "C" fn(i64, *const i64, i32) -> i64,
    get_attr:      unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    set_attr:      unsafe extern "C" fn(i64, *const u8, i32, i64),
    subscript:     unsafe extern "C" fn(i64, i64) -> i64,
    get_global:    unsafe extern "C" fn(*const u8, i32) -> i64,
    iter_from:     unsafe extern "C" fn(i64) -> i64,
    iter_next:     unsafe extern "C" fn(i64) -> i64,
    is_type:       unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    arena_save:    unsafe extern "C" fn() -> u64,
    arena_compact: unsafe extern "C" fn(i64, u64) -> i64,
    compact_many:  unsafe extern "C" fn(*const i64, i32, u64, *mut i64),
    to_int:        unsafe extern "C" fn(i64) -> i64,
    to_float:      unsafe extern "C" fn(i64) -> f64,
    deep_copy:     unsafe extern "C" fn(i64) -> i64,
    to_cstr:       unsafe extern "C" fn(i64) -> *const u8,
    write_handle:  unsafe extern "C" fn(i64, i64),
    list_append:   unsafe extern "C" fn(i64, i64) -> i64,
    raise_exc:     unsafe extern "C" fn(i64, i64) -> i64,
    make_cell:     unsafe extern "C" fn(i64) -> i64,
    get_cell:      unsafe extern "C" fn(i64) -> i64,
    set_cell:      unsafe extern "C" fn(i64, i64),
    call_method:   unsafe extern "C" fn(i64, *const u8, i32, *const i64, i32) -> i64,
}

static mut CB: *const ArCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn ar_init(cb: *const ArCallbacks) { CB = cb; }

#[inline(always)] unsafe fn cb_make_int(n: i64) -> i64   { ((*CB).make_int)(n) }
#[inline(always)] unsafe fn cb_make_float(f: f64) -> i64  { ((*CB).make_float)(f) }
#[inline(always)] unsafe fn cb_make_str(p: *const u8, l: i32) -> i64 { ((*CB).make_str)(p, l) }
#[inline(always)] unsafe fn cb_to_int(h: i64) -> i64     { ((*CB).to_int)(h) }
#[inline(always)] unsafe fn cb_to_float(h: i64) -> f64   { ((*CB).to_float)(h) }
#[inline(always)] unsafe fn cb_to_cstr(h: i64) -> *const u8 { ((*CB).to_cstr)(h) }

#[inline(always)]
unsafe fn cb_get_attr(obj_h: i64, name: &[u8]) -> i64 {
    ((*CB).get_attr)(obj_h, name.as_ptr(), name.len() as i32)
}

#[inline(always)]
unsafe fn cb_set_attr(obj_h: i64, name: &[u8], val_h: i64) {
    ((*CB).set_attr)(obj_h, name.as_ptr(), name.len() as i32, val_h)
}

// Raise an Arrow exception from the wrapper (returns TL_EXCEPTION; the caller returns it) — 2-5.
unsafe fn _raise(ty: &[u8], msg: &str) -> i64 {
    ((*CB).raise_exc)(cb_make_str(ty.as_ptr(), ty.len() as i32), cb_make_str(msg.as_ptr(), msg.len() as i32))
}
unsafe fn _overflow_in(name: &str, ty: &str) -> i64 {
    _raise(b"OverflowError", &format!("argument `{name}` does not fit in the Rust type `{ty}`"))
}
unsafe fn _overflow_out(what: &str) -> i64 {
    _raise(b"OverflowError", &format!("{what} is 2^63 or more (or below -2^63) and does not fit in int"))
}
unsafe fn _dropped(ty: &[u8]) -> i64 {
    _raise(b"RuntimeError", &format!("the Rust `{}` object has been dropped", String::from_utf8_lossy(ty)))
}

unsafe fn handle_to_string(h: i64) -> String {
    let ptr = cb_to_cstr(h);
    if ptr.is_null() { return String::new(); }
    std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned()
}

"#;

pub(crate) fn lib_rs(fns: &[RsFnSig], structs: &[RsStructSig], crate_ident: &str) -> String {
    let mut out = ABI_HEADER.to_string();

    // Struct arenas (one static arena + counter + adoption slot + field sync per struct)
    for st in structs {
        out.push_str(&struct_arena_decl(st, crate_ident));
    }

    // Free function wrappers
    for sig in fns {
        out.push_str(&fn_wrapper(sig, crate_ident));
    }

    // Struct wrappers
    for st in structs {
        out.push_str(&struct_wrappers(st, crate_ident, structs));
    }

    out
}

/// Generate the static arena declaration for a struct using OnceLock for lazy init, the adoption slot and the
/// field-sync helper (ffi_value_exchange_plan.md 2-5).
pub(crate) fn struct_arena_decl(st: &RsStructSig, crate_ident: &str) -> String {
    let struct_name = &st.name;
    let lock_ident = arena_lock_ident(struct_name);
    let getter = arena_getter_fn(struct_name);
    let counter = counter_ident(struct_name);
    let adopt = adopt_ident(struct_name);
    let sync = sync_fn(struct_name);
    // Field sync: copy every field of the Rust value into the Arrow instance's fields. Returns TL_EXCEPTION when
    // a value does not fit (an Arrow `int` from a u64 field ≥ 2^63).
    let mut sync_body = String::new();
    for field in &st.fields {
        let fname = &field.name;
        sync_body.push_str(&format!(
            "    let _fh = {};\n    if _fh == TL_EXCEPTION {{ return TL_EXCEPTION; }}\n    cb_set_attr(self_h, b\"{fname}\", _fh);\n",
            rust_value_to_handle_of(&format!("obj.{fname}"), &field.rust_type, fname)
        ));
    }
    format!(
        "static {lock_ident}: OnceLock<Mutex<HashMap<i64, {crate_ident}::{struct_name}>>> = OnceLock::new();\n\
         fn {getter}() -> &'static Mutex<HashMap<i64, {crate_ident}::{struct_name}>> {{\n    \
         {lock_ident}.get_or_init(|| Mutex::new(HashMap::new()))\n}}\n\
         static {counter}: AtomicI64 = AtomicI64::new(1);\n\
         // A value returned by a method, waiting to be adopted by the next `__init__` (instead of constructing).\n\
         thread_local! {{ static {adopt}: std::cell::RefCell<Option<{crate_ident}::{struct_name}>> = std::cell::RefCell::new(None); }}\n\
         unsafe fn {sync}(self_h: i64, obj: &{crate_ident}::{struct_name}) -> i64 {{\n{sync_body}    TL_NONE\n}}\n\n"
    )
}

/// The static OnceLock identifier for the arena.
pub(crate) fn arena_lock_ident(struct_name: &str) -> String {
    format!("ARENA_LOCK_{}", struct_name.to_uppercase())
}

/// The getter function name that lazily initialises the arena.
pub(crate) fn arena_getter_fn(struct_name: &str) -> String {
    format!("get_arena_{}", struct_name.to_lowercase())
}

pub(crate) fn counter_ident(struct_name: &str) -> String {
    format!("COUNTER_{}", struct_name.to_uppercase())
}

/// The thread-local slot holding a returned value to adopt (2-5).
pub(crate) fn adopt_ident(struct_name: &str) -> String {
    format!("ADOPT_{}", struct_name.to_uppercase())
}

/// The helper that copies the Rust value's fields into the Arrow instance (2-5).
pub(crate) fn sync_fn(struct_name: &str) -> String {
    format!("_sync_fields_{}", struct_name.to_lowercase())
}

/// Generate all wrappers for a struct: __init__, drop, field getters/setters, methods.
pub(crate) fn struct_wrappers(st: &RsStructSig, crate_ident: &str, all_structs: &[RsStructSig]) -> String {
    let mut out = String::new();
    out.push_str(&struct_init_wrapper(st, crate_ident));
    out.push_str(&struct_drop_wrapper(&st.name));
    for field in &st.fields {
        out.push_str(&struct_getter_wrapper(&st.name, field));
        out.push_str(&struct_setter_wrapper(&st.name, field));
    }
    for m in &st.methods {
        out.push_str(&struct_method_wrapper(&st.name, m, crate_ident, all_structs));
    }
    out
}

/// `{StructName}____init___tl` — constructor wrapper.
///
/// A value returned by a method is waiting in the adoption slot when the method constructs the Arrow instance
/// through the class (`struct_method_wrapper`): take it instead of constructing a new one. ⚠ Before task 2-5 the
/// returned struct was rebuilt from its fields through the constructor — wrong when `new()` is used (its
/// parameters are not the fields) and a build error with private fields.
pub(crate) fn struct_init_wrapper(st: &RsStructSig, crate_ident: &str) -> String {
    let name = &st.name;
    let arena = format!("{}()", arena_getter_fn(name));
    let counter = counter_ident(name);
    let adopt = adopt_ident(name);
    let sync = sync_fn(name);
    // Symbol: method_symbol(name, "__init__") = "{name}____init__", + "_tl"
    let sym = format!("{name}____init___tl");

    let mut construct = String::new();
    match &st.ctor_unsupported {
        // Construction from Arrow is a static error (the stub's `__init__` is marked); this is the runtime guard.
        Some(why) => {
            let msg = format!("Rust struct `{name}` cannot be constructed from Arrow: {why}").replace('"', "'");
            construct.push_str(&format!("            return _raise(b\"TypeError\", \"{msg}\");\n"));
        }
        None => {
            for (i, p) in st.ctor_params.iter().enumerate() {
                construct.push_str(&indent(&param_conversion(i + 1, &p.name, &p.rust_type), 8));
            }
            let args: Vec<String> = st.ctor_params.iter().map(|p| p.name.clone()).collect();
            if st.use_new_fn {
                construct.push_str(&format!("            {crate_ident}::{name}::new({})\n", args.join(", ")));
            } else {
                let field_inits: Vec<String> = args.iter().map(|a| format!("{a}: {a}")).collect();
                construct.push_str(&format!("            {crate_ident}::{name} {{ {} }}\n", field_inits.join(", ")));
            }
        }
    }

    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _instance = match {adopt}.with(|a| a.borrow_mut().take()) {{\n        \
         Some(_v) => _v,\n        \
         None => {{\n{construct}        }}\n    \
         }};\n    \
         let _key = {counter}.fetch_add(1, Ordering::SeqCst);\n    \
         let _key_h = cb_make_int(_key);\n    \
         cb_set_attr(self_h, b\"__rs_handle__\", _key_h);\n    \
         let _synced = {sync}(self_h, &_instance);\n    \
         {arena}.lock().unwrap().insert(_key, _instance);\n    \
         if _synced == TL_EXCEPTION {{ return TL_EXCEPTION; }}\n    \
         TL_NONE\n}}\n\n"
    )
}

fn indent(code: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    code.lines().map(|l| format!("{pad}{l}\n")).collect()
}

/// `{StructName}__drop_tl` — destructor wrapper.
pub(crate) fn struct_drop_wrapper(struct_name: &str) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let sym = format!("{struct_name}__drop_tl");
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         {arena}.lock().unwrap().remove(&_key);\n    \
         TL_NONE\n}}\n\n"
    )
}

/// `{StructName}__get_{field}_tl` — field getter.
pub(crate) fn struct_getter_wrapper(struct_name: &str, field: &RsField) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let fname = &field.name;
    let sym = format!("{struct_name}__get_{fname}_tl");
    let result_expr = rust_value_to_handle_of(&format!("_obj.{fname}"), &field.rust_type, fname);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let _arena = {arena}.lock().unwrap();\n    \
         match _arena.get(&_key) {{\n        \
         Some(_obj) => {result_expr},\n        \
         None => _dropped(b\"{struct_name}\"),\n    \
         }}\n}}\n\n"
    )
}

/// `{StructName}__set_{field}_tl` — field setter (also called for `obj.field = v` from Arrow — 2-5).
/// ⚠ The Arrow-side copy is computed **before** the value moves into the struct (it used to read `_val` after
/// moving it, which did not compile for `String` fields and made the whole crate import fail).
pub(crate) fn struct_setter_wrapper(struct_name: &str, field: &RsField) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let fname = &field.name;
    let sym = format!("{struct_name}__set_{fname}_tl");
    let decode = param_conversion(1, "_val", &field.rust_type);
    let val_handle = rust_value_to_handle_of("_val", &field.rust_type, fname);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n{decode}    \
         let _fh = {val_handle};\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let mut _arena = {arena}.lock().unwrap();\n    \
         match _arena.get_mut(&_key) {{\n        \
         Some(_obj) => _obj.{fname} = _val,\n        \
         None => return _dropped(b\"{struct_name}\"),\n    \
         }}\n    \
         drop(_arena);\n    \
         cb_set_attr(self_h, b\"{fname}\", _fh);\n    \
         TL_NONE\n}}\n\n"
    )
}

/// `{StructName}__{method_name}_tl` — method wrapper.
/// `&mut self` methods copy the fields back into the Arrow instance afterwards (2-5: `obj.field` used to keep the
/// value from construction time). A returned struct is handed to the class through the adoption slot.
pub(crate) fn struct_method_wrapper(struct_name: &str, m: &RsMethodSig, crate_ident: &str, all_structs: &[RsStructSig]) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let sync = sync_fn(struct_name);
    let mname = &m.name;
    let sym = format!("{struct_name}__{mname}_tl");
    let _ = crate_ident;

    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n"
    );
    out.push_str("    let self_h = *args.add(0);\n");
    out.push_str("    let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n");
    out.push_str("    let _key = cb_to_int(_key_h);\n");

    for (i, p) in m.params.iter().enumerate() {
        out.push_str(&param_conversion(i + 1, &p.name, &p.rust_type));
    }

    let args_call: Vec<String> = m.params.iter().map(|p| p.name.clone()).collect();
    let call_expr = format!("_obj.{mname}({})", args_call.join(", "));
    let (lock_kw, borrow) = if m.self_mutable { ("mut ", "get_mut(&_key)") } else { ("", "get(&_key)") };
    out.push_str(&format!("    let {lock_kw}_arena = {arena}.lock().unwrap();\n"));
    out.push_str(&format!(
        "    let _obj = match _arena.{borrow} {{ Some(_o) => _o, None => return _dropped(b\"{struct_name}\") }};\n"
    ));
    let sync_after = if m.self_mutable {
        format!("    if {sync}(self_h, _obj) == TL_EXCEPTION {{ return TL_EXCEPTION; }}\n")
    } else {
        String::new()
    };

    if let Some(ret_struct_name) = &m.return_struct {
        let adopt = adopt_ident(ret_struct_name);
        // Arrow's `__init__` of the returned class takes this many arguments; they are ignored while adopting.
        let n_ctor = all_structs
            .iter()
            .find(|s| &s.name == ret_struct_name)
            .map_or(0, |s| s.ctor_params.len());
        out.push_str(&format!("    let _rval = {call_expr};\n"));
        out.push_str(&sync_after);
        out.push_str("    drop(_arena); // release the lock before calling back into Arrow\n");
        out.push_str(&format!("    {adopt}.with(|a| *a.borrow_mut() = Some(_rval));\n"));
        out.push_str(&format!(
            "    let _cls_h = ((*CB).get_global)(\"{ret_struct_name}\".as_ptr(), {n});\n",
            n = ret_struct_name.len()
        ));
        out.push_str(&format!("    let _ctor: [i64; {n_ctor}] = [TL_NONE; {n_ctor}];\n"));
        out.push_str("    let _r = ((*CB).call_fn)(_cls_h, _ctor.as_ptr(), _ctor.len() as i32);\n");
        // If `__init__` never ran (the class could not be found), do not leave the value for the next construction.
        out.push_str(&format!("    {adopt}.with(|a| a.borrow_mut().take());\n"));
        out.push_str("    _r\n}\n\n");
    } else {
        out.push_str(&format!(
            "    let _ret: i64 = {{ {} }};\n",
            return_conversion_expr(&call_expr, m.return_type.as_deref())
        ));
        out.push_str(&sync_after);
        out.push_str("    _ret\n}\n\n");
    }
    out
}

/// Generate a Rust expression that converts a value expression to an i64 handle.
/// Integers that do not fit in Arrow's `int` (i64) raise OverflowError (2-5: `u64` / `usize` / `u128` / `i128` used
/// to wrap around with `as i64`). `what` names the value in the message.
pub(crate) fn rust_value_to_handle_of(expr: &str, rust_type: &str, what: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("cb_make_int({expr} as i64)"),
        "i32" | "i16" | "i8" | "u32" | "u16" | "u8" => format!("cb_make_int({expr} as i64)"),
        "u64" | "usize" | "u128" | "i128" => format!(
            "match i64::try_from({expr}) {{ Ok(_n) => cb_make_int(_n), Err(_) => _overflow_out(\"the Rust field `{what}`\") }}"
        ),
        "f64" => format!("cb_make_float({expr})"),
        "f32" => format!("cb_make_float({expr} as f64)"),
        "bool" => format!("if {expr} {{ TL_TRUE }} else {{ TL_FALSE }}"),
        "String" | "&str" | "&String" => format!("cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"),
        other => format!("_raise(b\"TypeError\", \"cannot convert a Rust `{other}` to an Arrow value\")"),
    }
}

/// Decode argument `i` into a Rust local `name`. Integers that do not fit the Rust type raise OverflowError
/// (2-5: they used to wrap around with `as`, e.g. 300 for a `u8` became 44, -1 for a `u64` became 2^64-1).
pub(crate) fn param_conversion(i: usize, name: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n", t = rust_type.trim()),
        t @ ("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") => format!(
            "    let {name}: {t} = match {t}::try_from(cb_to_int(*args.add({i}))) {{ Ok(_v) => _v, Err(_) => return _overflow_in(\"{name}\", \"{t}\") }};\n"
        ),
        "f64" => format!("    let {name}: f64 = cb_to_float(*args.add({i}));\n"),
        "f32" => format!("    let {name}: f32 = cb_to_float(*args.add({i})) as f32;\n"),
        "bool" => format!("    let {name}: bool = *args.add({i}) == TL_TRUE;\n"),
        "String" =>
            format!("    let {name}: String = handle_to_string(*args.add({i}));\n"),
        "&str" | "&String" =>
            format!(
                "    let _owned_{name}: String = handle_to_string(*args.add({i}));\n    let {name}: &str = &_owned_{name};\n"
            ),
        // &[u8]: decode str handle → String → take bytes slice
        "&[u8]" =>
            format!(
                "    let _bytes_{name}: String = handle_to_string(*args.add({i}));\n    let {name}: &[u8] = _bytes_{name}.as_bytes();\n"
            ),
        // The scanner only lets the types above through (`is_param_type`).
        other => format!("    return _raise(b\"TypeError\", \"cannot pass an Arrow value as a Rust `{other}`\");\n"),
    }
}

pub(crate) fn return_conversion(call: &str, rust_type: Option<&str>) -> String {
    format!("    {}\n", return_conversion_expr(call, rust_type))
}

pub(crate) fn return_conversion_expr(call: &str, rust_type: Option<&str>) -> String {
    match rust_type.map(str::trim) {
        None | Some("()") =>
            format!("{call};\n            TL_NONE"),
        Some("i64" | "isize") =>
            format!("cb_make_int({call})"),
        Some("i32" | "i16" | "i8" | "u32" | "u16" | "u8") =>
            format!("cb_make_int(({call}) as i64)"),
        // ⚠ 2^63 以上（負の i128）は Arrow の int に入らないので OverflowError（2-5。以前は `as i64` で黙って折り返した）
        Some("u64" | "usize" | "u128" | "i128") =>
            format!("match i64::try_from({call}) {{ Ok(_n) => cb_make_int(_n), Err(_) => _overflow_out(\"the Rust return value\") }}"),
        Some("f64") =>
            format!("cb_make_float({call})"),
        Some("f32") =>
            format!("cb_make_float(({call}) as f64)"),
        Some("bool") =>
            format!("if {call} {{ TL_TRUE }} else {{ TL_FALSE }}"),
        Some("String") => {
            format!("{{ let _r: String = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}")
        }
        Some("&str") => {
            format!("{{ let _r: &str = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}")
        }
        // Byte-array output: hex-encode → return as str handle
        Some("Vec<u8>") => {
            format!("{{ let _r: Vec<u8> = {call};\n            let _hex = _r.iter().map(|b| format!(\"{{:02x}}\", b)).collect::<String>();\n            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}")
        }
        Some(t) if is_fixed_byte_array(t) => {
            format!("{{ let _r = {call};\n            let _hex = _r.iter().map(|b| format!(\"{{:02x}}\", b)).collect::<String>();\n            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}")
        }
        // The scanner only lets the types above through (`is_return_type`).
        Some(other) =>
            format!("{{ let _ = {call}; _raise(b\"TypeError\", \"cannot convert a Rust `{other}` to an Arrow value\") }}"),
    }
}

pub(crate) fn fn_wrapper(sig: &RsFnSig, crate_ident: &str) -> String {
    // Digest-pattern: synthesised one-shot hash wrapper.
    if let Some(type_path) = &sig.digest_type {
        return digest_wrapper(&sig.name, type_path);
    }

    let name = &sig.name;
    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {name}_tl(args: *const i64, _n: i32) -> i64 {{\n"
    );

    for (i, p) in sig.params.iter().enumerate() {
        out.push_str(&param_conversion(i, &p.name, &p.rust_type));
    }

    let args: Vec<String> = sig.params.iter().map(|p| p.name.clone()).collect();
    let call = format!("{crate_ident}::{name}({})", args.join(", "));
    out.push_str(&return_conversion(&call, sig.return_type.as_deref()));
    out.push_str("}\n\n");
    out
}

/// Generate a one-shot hash wrapper for a RustCrypto Digest type alias.
/// Calls `TypePath::digest(input.as_bytes())` and hex-encodes the result.
pub(crate) fn digest_wrapper(fn_name: &str, type_path: &str) -> String {
    format!(
        "#[no_mangle]\n\
         pub unsafe extern \"C\" fn {fn_name}_tl(args: *const i64, _n: i32) -> i64 {{\n    \
         let _s: String = handle_to_string(*args.add(0));\n    \
         use {type_path} as _HvHasher;\n    \
         use digest::Digest as _HvDigest;\n    \
         let _result = _HvHasher::digest(_s.as_bytes());\n    \
         let _hex: String = _result.iter().map(|b| format!(\"{{:02x}}\", b)).collect();\n    \
         cb_make_str(_hex.as_ptr(), _hex.len() as i32)\n\
         }}\n\n"
    )
}
