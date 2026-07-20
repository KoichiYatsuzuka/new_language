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

unsafe fn handle_to_string(h: i64) -> String {
    let ptr = cb_to_cstr(h);
    if ptr.is_null() { return String::new(); }
    std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned()
}

"#;

pub(crate) fn lib_rs(fns: &[RsFnSig], structs: &[RsStructSig], crate_ident: &str) -> String {
    let mut out = ABI_HEADER.to_string();

    // Struct arenas (one static arena + counter per struct)
    for st in structs {
        out.push_str(&struct_arena_decl(&st.name, crate_ident));
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

/// Generate the static arena declaration for a struct using OnceLock for lazy init.
pub(crate) fn struct_arena_decl(struct_name: &str, crate_ident: &str) -> String {
    let lock_ident = arena_lock_ident(struct_name);
    let getter = arena_getter_fn(struct_name);
    let counter = counter_ident(struct_name);
    format!(
        "static {lock_ident}: OnceLock<Mutex<HashMap<i64, {crate_ident}::{struct_name}>>> = OnceLock::new();\n\
         fn {getter}() -> &'static Mutex<HashMap<i64, {crate_ident}::{struct_name}>> {{\n    \
         {lock_ident}.get_or_init(|| Mutex::new(HashMap::new()))\n}}\n\
         static {counter}: AtomicI64 = AtomicI64::new(1);\n\n"
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
pub(crate) fn struct_init_wrapper(st: &RsStructSig, crate_ident: &str) -> String {
    let name = &st.name;
    let arena = format!("{}()", arena_getter_fn(name));
    let counter = counter_ident(name);
    // Symbol: method_symbol(name, "__init__") = "{name}____init__", + "_tl"
    let sym = format!("{name}____init___tl");

    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n"
    );
    out.push_str("    let self_h = *args.add(0);\n");

    // Decode constructor args starting at index 1
    for (i, p) in st.ctor_params.iter().enumerate() {
        out.push_str(&param_conversion(i + 1, &p.name, &p.rust_type));
    }

    // Build the struct instance
    if st.use_new_fn {
        let args: Vec<String> = st.ctor_params.iter().map(|p| p.name.clone()).collect();
        out.push_str(&format!(
            "    let _instance = {crate_ident}::{name}::new({});\n",
            args.join(", ")
        ));
    } else {
        let field_inits: Vec<String> = st.ctor_params.iter()
            .map(|p| format!("{}: {}", p.name, p.name))
            .collect();
        out.push_str(&format!(
            "    let _instance = {crate_ident}::{name} {{ {} }};\n",
            field_inits.join(", ")
        ));
    }

    // Store in arena
    out.push_str(&format!(
        "    let _key = {counter}.fetch_add(1, Ordering::SeqCst);\n"
    ));
    out.push_str(&format!(
        "    {arena}.lock().unwrap().insert(_key, _instance);\n"
    ));

    // Store __rs_handle__ in the HV instance
    out.push_str("    let _key_h = cb_make_int(_key);\n");
    out.push_str("    cb_set_attr(self_h, b\"__rs_handle__\", _key_h);\n");

    // Also populate HV field cache from the arena
    for field in &st.fields {
        let fname = &field.name;
        let ftype = &field.rust_type;
        out.push_str(&format!(
            "    {{\n        let _arena = {arena}.lock().unwrap();\n        \
             if let Some(_obj) = _arena.get(&_key) {{\n            \
             let _fh = {};\n            \
             cb_set_attr(self_h, b\"{fname}\", _fh);\n        }}\n    }}\n",
            rust_value_to_handle("_obj", fname, ftype)
        ));
    }

    out.push_str("    TL_NONE\n}\n\n");
    out
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
    let result_expr = rust_value_to_handle("_obj", fname, &field.rust_type);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let _arena = {arena}.lock().unwrap();\n    \
         match _arena.get(&_key) {{\n        \
         Some(_obj) => {result_expr},\n        \
         None => TL_NONE,\n    \
         }}\n}}\n\n"
    )
}

/// `{StructName}__set_{field}_tl` — field setter.
pub(crate) fn struct_setter_wrapper(struct_name: &str, field: &RsField) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let fname = &field.name;
    let sym = format!("{struct_name}__set_{fname}_tl");
    let decode = param_conversion(1, "_val", &field.rust_type);
    let val_handle = rust_value_to_handle_of("_val", &field.rust_type);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n{decode}    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let mut _arena = {arena}.lock().unwrap();\n    \
         if let Some(_obj) = _arena.get_mut(&_key) {{\n        \
         _obj.{fname} = _val;\n    }}\n    \
         drop(_arena);\n    \
         let _fh = {val_handle};\n    \
         cb_set_attr(self_h, b\"{fname}\", _fh);\n    \
         TL_NONE\n}}\n\n"
    )
}

/// `{StructName}__{method_name}_tl` — method wrapper.
/// `all_structs` is needed to look up the return struct's ctor params.
pub(crate) fn struct_method_wrapper(struct_name: &str, m: &RsMethodSig, _crate_ident: &str, all_structs: &[RsStructSig]) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let mname = &m.name;
    let sym = format!("{struct_name}__{mname}_tl");

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

    if let Some(ret_struct_name) = &m.return_struct {
        // Returns a struct — extract field values before releasing the arena lock,
        // then call cb_call_fn to construct the HV instance.
        let ret_struct = all_structs.iter().find(|s| &s.name == ret_struct_name);
        let ctor_params: &[RsParam] = ret_struct
            .map(|s| s.ctor_params.as_slice())
            .unwrap_or(&[]);

        // Open arena block
        let lock_kw = if m.self_mutable { "mut " } else { "" };
        out.push_str(&format!("    let {lock_kw}_arena = {arena}.lock().unwrap();\n"));
        out.push_str(&format!("    let _found = _arena.{borrow};\n",
            borrow = if m.self_mutable { "get_mut(&_key)" } else { "get(&_key)" }
        ));
        out.push_str("    if _found.is_none() { return TL_NONE; }\n");
        out.push_str("    let _obj = _found.unwrap();\n");
        out.push_str(&format!("    let _rval = {call_expr};\n"));

        // Extract each ctor field from the result before lock is dropped
        for (i, p) in ctor_params.iter().enumerate() {
            let t = &p.rust_type;
            let fname = &p.name;
            out.push_str(&format!("    let _ret_{i}: {t} = _rval.{fname};\n"));
        }
        out.push_str("    drop(_arena);\n"); // explicit drop = release lock before callbacks

        // Build HV instance via cb_get_global + cb_call_fn
        out.push_str(&format!(
            "    let _cls_h = ((*CB).get_global)(\"{ret_struct_name}\".as_ptr(), {n});\n",
            n = ret_struct_name.len()
        ));

        // Build ctor arg handles
        let handle_exprs: Vec<String> = ctor_params.iter().enumerate()
            .map(|(i, p)| rust_value_to_handle_of(&format!("_ret_{i}"), &p.rust_type))
            .collect();
        if handle_exprs.is_empty() {
            out.push_str("    let _ctor: [i64; 0] = [];\n");
        } else {
            out.push_str(&format!("    let _ctor = [{}];\n", handle_exprs.join(", ")));
        }
        out.push_str("    ((*CB).call_fn)(_cls_h, _ctor.as_ptr(), _ctor.len() as i32)\n");
        out.push_str("}\n\n");
    } else {
        // Primitive / void return — original path
        let obj_borrow = if m.self_mutable { "get_mut(&_key)" } else { "get(&_key)" };
        if m.self_mutable {
            out.push_str(&format!("    let mut _arena = {arena}.lock().unwrap();\n"));
        } else {
            out.push_str(&format!("    let _arena = {arena}.lock().unwrap();\n"));
        }
        out.push_str(&format!("    match _arena.{obj_borrow} {{\n"));
        out.push_str("        None => TL_NONE,\n");
        out.push_str(&format!(
            "        Some(_obj) => {{\n{}\n        }},\n    }}\n}}\n\n",
            return_conversion_expr(&call_expr, m.return_type.as_deref())
        ));
    }

    out
}

/// Generate a Rust expression that converts an arena object's field to an i64 handle.
/// `obj_var` is the variable holding a reference to the struct.
pub(crate) fn rust_value_to_handle(obj_var: &str, field: &str, rust_type: &str) -> String {
    let field_expr = format!("{obj_var}.{field}");
    rust_value_to_handle_of(&field_expr, rust_type)
}

/// Generate a Rust expression that converts a value expression to an i64 handle.
pub(crate) fn rust_value_to_handle_of(expr: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("cb_make_int({expr})"),
        "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128" =>
            format!("cb_make_int({expr} as i64)"),
        "f64" => format!("cb_make_float({expr})"),
        "f32" => format!("cb_make_float({expr} as f64)"),
        "bool" => format!("if {expr} {{ TL_TRUE }} else {{ TL_FALSE }}"),
        "String" => format!("cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"),
        "&str" | "&String" => format!("cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"),
        _ => format!("cb_make_int({expr} as i64)"),
    }
}

pub(crate) fn param_conversion(i: usize, name: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("    let {name}: i64 = cb_to_int(*args.add({i}));\n"),
        t @ ("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") =>
            format!("    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n"),
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
        _ => format!("    let {name}: i64 = *args.add({i});\n"),
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
        Some("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") =>
            format!("cb_make_int(({call}) as i64)"),
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
        Some(_) =>
            format!("{{ let _r: i64 = {call} as i64;\n            cb_make_int(_r) }}"),
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
