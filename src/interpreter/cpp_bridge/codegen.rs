// codegen.rs — Generate the standalone Rust wrapper source that bridges
// tl handles to a C/C++ DLL or static library.
//
// Public API:
//   gen_dll_wrapper — emit the complete Rust source for a dynamic-DLL wrapper

use std::collections::HashMap;

use super::super::native_api::{TL_EXCEPTION, TL_FALSE, TL_NONE, TL_TRUE};
use crate::cpp_header::types::{CStructDef, CFnSig, CType};

// ── Generated Rust source header ─────────────────────────────────────────────

pub(crate) const WRAPPER_HEADER: &str = r#"// Auto-generated cpp bridge — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_unsafe, static_mut_refs)]

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
    // fields 29-37: not used by this wrapper (declared only to reach the fields after them)
    _unused_29_37: [usize; 9],
    to_bool:       unsafe extern "C" fn(i64) -> i32,
    pending_error: unsafe extern "C" fn() -> i32,
    make_bytes:    unsafe extern "C" fn(*const u8, i64) -> i64,
    make_ptr:      unsafe extern "C" fn(u64, *const u8, i32) -> i64,
    bytes_arg:     unsafe extern "C" fn(i64) -> *const u8,
    ptr_arg:       unsafe extern "C" fn(i64, *const u8, i32) -> u64,
    protocol_arg:  unsafe extern "C" fn(i64, *mut u8, i32),
    protocol_done: unsafe extern "C" fn(i64, u64),
}

// ArPtrProtocol (bridge/ptr_protocol/ar_ptr_protocol.h, 4-1). Same layout as Arrow's PtrProtocolRaw; only passed through.
#[repr(C)]
#[derive(Clone, Copy)]
struct _ArPtrProtocol {
    address: u64,
    elem_type: *const u8,
    flags: u32,
    _reserved: u32,
    length: i64,
    release: usize,
    release_ctx: usize,
}

// `char*` / `wchar_t*` return values become bytes (NUL not included; wchar_t as UTF-16LE). NULL is None (4-3).
unsafe fn _cstr_to_handle(p: *const u8) -> i64 {
    if p.is_null() { return 0; }
    let n = std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_bytes().len();
    ((*CB).make_bytes)(p, n as i64)
}
unsafe fn _wstr_to_handle(p: *const u16) -> i64 {
    if p.is_null() { return 0; }
    let mut n = 0usize;
    while *p.add(n) != 0 { n += 1; }
    ((*CB).make_bytes)(p as *const u8, (n * 2) as i64)
}

static mut CB: *const ArCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn ar_init(cb: *const ArCallbacks) { CB = cb; }

// A C unsigned 64-bit value of 2^63 or more does not fit in Arrow's int (ffi_value_exchange_plan.md 1-6).
const _U64_TYPE: &[u8] = b"OverflowError";
const _U64_MSG: &[u8] = b"a C unsigned 64-bit value of 2^63 or more does not fit in int";
unsafe fn _overflow_u64() -> i64 {
    ((*CB).raise_exc)(
        ((*CB).make_str)(_U64_TYPE.as_ptr(), _U64_TYPE.len() as i32),
        ((*CB).make_str)(_U64_MSG.as_ptr(), _U64_MSG.len() as i32),
    )
}
// The typed ABI reports it through the error slot (same layout as native_api::ErrSlot).
#[repr(C)]
struct _ErrSlot { type_ptr: *const u8, type_len: u64, msg_ptr: *const u8, msg_len: u64 }
unsafe fn _typed_overflow_u64(err: *mut u8) -> u32 {
    let e = err as *mut _ErrSlot;
    (*e).type_ptr = _U64_TYPE.as_ptr();
    (*e).type_len = _U64_TYPE.len() as u64;
    (*e).msg_ptr = _U64_MSG.as_ptr();
    (*e).msg_len = _U64_MSG.len() as u64;
    1
}
"#;

// Platform loader inserted into dll-wrapper source.
// RTLD_LAZY is embedded as the literal `1` (standardized value on all POSIX platforms).
pub(crate) const PLATFORM_LOADER: &str = r#"
#[cfg(windows)]
mod _loader {
    extern "system" {
        fn LoadLibraryA(name: *const u8) -> usize;
        fn GetProcAddress(module: usize, name: *const u8) -> usize;
        fn GetModuleHandleExA(flags: u32, addr: *const u8, module: *mut usize) -> i32;
        fn GetModuleFileNameA(module: usize, buf: *mut u8, size: u32) -> u32;
    }
    static ANCHOR: u8 = 0;
    // A relative path is taken from this wrapper DLL's own directory, so the generated
    // files keep working when the directory is moved (cpp-lib's shim sits next to us).
    unsafe fn resolve(path: &str) -> String {
        if std::path::Path::new(path).is_absolute() { return path.to_string(); }
        let mut m: usize = 0;
        // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS (4) | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT (2)
        if GetModuleHandleExA(6, &ANCHOR as *const u8, &mut m) == 0 { return path.to_string(); }
        let mut buf = [0u8; 1024];
        let n = GetModuleFileNameA(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 || n >= buf.len() { return path.to_string(); }
        let me = String::from_utf8_lossy(&buf[..n]).into_owned();
        match std::path::Path::new(&me).parent() {
            Some(dir) => dir.join(path).to_string_lossy().into_owned(),
            None => path.to_string(),
        }
    }
    pub unsafe fn load(path: &str) -> usize {
        let p = format!("{}\0", resolve(path));
        LoadLibraryA(p.as_ptr())
    }
    pub unsafe fn sym(lib: usize, name: &str) -> usize {
        let n = format!("{name}\0");
        GetProcAddress(lib, n.as_ptr())
    }
}
#[cfg(not(windows))]
mod _loader {
    extern "C" {
        fn dlopen(name: *const u8, flags: i32) -> usize;
        fn dlsym(handle: usize, name: *const u8) -> usize;
    }
    pub unsafe fn load(path: &str) -> usize {
        let p = format!("{path}\0");
        dlopen(p.as_ptr(), 1) // RTLD_LAZY
    }
    pub unsafe fn sym(lib: usize, name: &str) -> usize {
        let n = format!("{name}\0");
        dlsym(lib, n.as_ptr())
    }
}
static mut _DLL_HANDLE: usize = 0;
unsafe fn _dll() -> usize {
    if _DLL_HANDLE == 0 { panic!("cpp bridge DLL not loaded"); }
    _DLL_HANDLE
}
"#;

// ── Struct bridge helpers ─────────────────────────────────────────────────────

/// `CType` を `#[repr(C)]` 構造体フィールドで使用する Rust プリミティブ型文字列にマップする。
/// `ByValueStruct` など単純な Rust フィールドで表現できない型は `None` を返す。
pub(crate) fn ctype_to_rust_field(ct: &CType) -> Option<&'static str> {
    match ct {
        // 幅と符号どおり（1-6。以前は int / bool を i32、long を i64 にまとめていた）
        CType::Int { width, signed } => Some(super::types::int_rust_type(*width, *signed)),
        CType::Bool => Some("u8"),
        CType::Float => Some("f32"),
        CType::Double => Some("f64"),
        CType::VoidPtr
        | CType::CharPtr { .. }
        | CType::OpaqueStructPtr { .. }
        | CType::Ptr { .. } => Some("*mut i8"),
        // Nested struct-by-value or function pointer → not representable
        CType::ByValueStruct { .. } | CType::FnPtr => None,
        // `ArPtrProtocol` のフィールドは構造体の展開（4-6）で扱う。それまでは写しを作らない
        CType::PtrProtocol { .. } => None,
        CType::Void => None,
    }
}

/// 構造体の全フィールドが Rust プリミティブにマップできる場合（ネスト構造体なし）に `true` を返す。
pub(crate) fn struct_is_bridgeable(def: &CStructDef) -> bool {
    !def.fields.is_empty() && def.fields.iter().all(|(_, ct)| ctype_to_rust_field(ct).is_some())
}

/// アリーナからハンドルを読み取り、構造体フィールドの Rust 型に変換する Rust 式を生成する。
/// 数値のフィールドは引数・戻り値と同じ変換（`CType::from_handle`）を使う（1-6）。
/// ⚠ ポインタのフィールドを持つ構造体は値で受け渡さない（`has_exact_layout` が偽・1-2 / 1-3）ので、
///   ここへ来るのは数値だけ。
fn field_from_handle(ct: &CType, handle: &str) -> String {
    match ct {
        CType::Int { .. } | CType::Float | CType::Double | CType::Bool => ct.from_handle(handle),
        _ => format!("{handle} as *mut i8"),
    }
}

/// 構造体フィールドの値を tl ハンドルにラップする Rust 式を生成する（戻り値と同じ変換・1-6）。
fn field_to_handle(ct: &CType, val: &str) -> String {
    match ct {
        CType::Int { .. } | CType::Float | CType::Double | CType::Bool => ct.to_handle(val),
        _ => format!("{val} as i64"),
    }
}

// ── Dynamic DLL wrapper generator ────────────────────────────────────────────

/// **動的** DLL 用のスタンドアロン Rust ラッパーを生成する。
/// ラッパーは `ar_init` 時に `LoadLibraryA`/`dlopen` で DLL をロードし、各呼び出しで
/// `GetProcAddress`/`dlsym` を使ってシンボルを解決する。
/// すべての関数は `{name}_tl(argc, argv) -> i64` 規約に従う。
pub fn gen_dll_wrapper(
    dll_path: &str,
    sigs: &[CFnSig],
    struct_defs: &[CStructDef],
) -> String {
    let mut src = String::new();
    src.push_str(WRAPPER_HEADER);

    // Emit #[repr(C)] struct definitions for every bridgeable C struct.
    for def in struct_defs {
        if !struct_is_bridgeable(def) {
            continue;
        }
        let name = &def.name;
        src.push_str(&format!("#[repr(C)] #[derive(Copy, Clone)] struct _Struct_{name} {{ "));
        for (fname, ftype) in &def.fields {
            src.push_str(&format!("{fname}: {}, ", ctype_to_rust_field(ftype).unwrap()));
        }
        src.push_str("}\n");
    }

    src.push_str(PLATFORM_LOADER);

    // Override ar_init to also load the original DLL
    let escaped = dll_path.replace('\\', "\\\\");
    src.push_str(&format!(
        r#"
const _DLL_PATH: &str = "{escaped}";

#[no_mangle]
pub unsafe extern "C" fn ar_init_cpp(cb: *const ArCallbacks) {{
    CB = cb;
    _DLL_HANDLE = _loader::load(_DLL_PATH);
    if _DLL_HANDLE == 0 {{ panic!("cpp bridge: cannot load DLL: {{}}", _DLL_PATH); }}
}}
"#
    ));

    // Re-export ar_init so the standard ar_init call also works
    src.push_str(
        r#"
// Called by the interpreter at module load — initialise callbacks then load DLL.
#[no_mangle]
pub unsafe extern "C" fn ar_init_bridge(cb: *const ArCallbacks) {
    ar_init(cb);
    ar_init_cpp(cb);
}
"#,
    );

    // Build a name→def map for O(1) lookup inside gen_dll_fn.
    let struct_map: HashMap<String, &CStructDef> =
        struct_defs.iter().filter(|d| struct_is_bridgeable(d)).map(|d| (d.name.clone(), d)).collect();

    for sig in sigs {
        src.push_str(&gen_dll_fn(sig, &struct_map, struct_defs));
    }
    src
}

/// 1 つの C 関数シグネチャに対して `{name}_tl(argc, argv) -> i64` 形式の Rust ラッパー関数ソースを生成する。
fn gen_dll_fn(sig: &CFnSig, struct_defs: &HashMap<String, &CStructDef>, all_defs: &[CStructDef]) -> String {
    let mut s = String::new();
    let n = &sig.name;

    // 今の Arrow から正しく呼べない関数は生成しない（型スタブ側も同じ判定で静的エラーにする・1-2）。
    // 値渡し・値返しの構造体は、C と**同じレイアウトの** `_Struct_{name}` を**値のまま**受け渡す
    // （ffi_value_exchange_plan.md 1-3）。呼び出し規約（8 バイトならレジスタ、それ以外は隠しポインタ…）は
    // Rust の `extern "C"` が合わせる。cpp-lib のシムも構造体を値で受け渡す（`gen_cpp_shim_source`）。
    // ⚠ 以前は「アドレスを渡す・戻り値をポインタとして読む」形で、cpp-dll では呼び出し規約が合わず落ちていた。
    // レイアウトが正確に分からない構造体（`has_exact_layout` が偽）は値で受け渡せないので、これも未対応。
    if sig.unsupported_reason(all_defs).is_some() {
        return String::new();
    }
    let extern_type = |ct: &CType| match ct {
        CType::ByValueStruct { type_name } => format!("_Struct_{type_name}"),
        other => other.rust_extern_type(),
    };

    // Build the C function-pointer type — pointers use their Rust extern type
    let param_types: Vec<String> = sig.params.iter().map(|(_, t)| extern_type(t)).collect();
    let fn_type = if sig.ret == CType::Void {
        format!("unsafe extern \"C\" fn({})", param_types.join(", "))
    } else {
        format!(
            "unsafe extern \"C\" fn({}) -> {}",
            param_types.join(", "),
            extern_type(&sig.ret)
        )
    };

    s.push_str(&format!(
        r#"
#[no_mangle]
pub unsafe extern "C" fn {n}_tl(argv: *const i64, _argc: i32) -> i64 {{
    type _F = {fn_type};
    let _fp: _F = std::mem::transmute(_loader::sym(_dll(), "{n}"));
"#
    ));

    // Marshal each argument
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        let h = format!("*argv.offset({i})");
        match ptype {
            CType::Ptr { inner, mutable: true } => {
                s.push_str(&format!(
                    "    let mut _ptr_{pname}: {} = {};\n",
                    inner.rust_extern_type(),
                    inner.from_handle(&h)
                ));
            }
            CType::Ptr { inner, mutable: false } => {
                s.push_str(&format!(
                    "    let _ptr_{pname}: {} = {};\n",
                    inner.rust_extern_type(),
                    inner.from_handle(&h)
                ));
            }
            CType::CharPtr { .. } => {
                // `const char*` は `bytes` か `None`（NULL・4-3・D-13）。`str` や NUL を含む `bytes` は `bytes_arg` が
                // エラーを設定し、下の `pending_error` で C 関数を呼ばずに戻る（呼び出しの後でエラーが例外になる）。
                // ⚠ 以前は `str` を UTF-8 で渡していた（Shift-JIS のライブラリで黙って文字化けする・D-13）
                s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
            }
            // `ArPtrProtocol`（4-4）: `Ptr` か `None` から構造体を組み立てる。書き込み用ポインタは呼び出しの後で、C が
            // `address` を 0 に書き戻したかを伝える（下の `protocol_done`）
            CType::PtrProtocol { by_pointer, mutable } => {
                let writable = *by_pointer && *mutable;
                s.push_str(&format!(
                    "    let mut _{pname}: _ArPtrProtocol = std::mem::zeroed();\n    ((*CB).protocol_arg)({h}, &mut _{pname} as *mut _ArPtrProtocol as *mut u8, {});\n",
                    writable as i32
                ));
            }
            CType::ByValueStruct { type_name } => {
                // 冒頭の `CFnSig::unsupported_reason` で、C と同じレイアウトの分かる構造体だけが来る
                let def = struct_defs[type_name.as_str()];
                // Read each field from the Arrow instance via get_attr and build the C struct (by value)
                for (fname, ftype) in &def.fields {
                    let attr_h = format!("((*CB).get_attr)({h}, b\"{fname}\\0\".as_ptr(), {})", fname.len());
                    s.push_str(&format!(
                        "    let _{pname}_{fname}: {} = {};\n",
                        ctype_to_rust_field(ftype).unwrap(),
                        field_from_handle(ftype, &attr_h)
                    ));
                }
                let field_inits: String = def.fields.iter()
                    .map(|(f, _)| format!("{f}: _{pname}_{f}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                s.push_str(&format!(
                    "    let _{pname} = _Struct_{type_name} {{ {field_inits} }};\n"
                ));
            }
            _ => {
                s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
            }
        }
    }

    // 引数の変換（`to_int` / `to_float` / `to_bool` / 構造体のフィールド）でエラーが出たら、C 関数を呼ばずに戻る
    // （呼び出しの後でエラーが例外になる・7-12）。以前は数値でない値を黙って 0 にして呼んでいた。
    if !sig.params.is_empty() {
        s.push_str(&format!("    if ((*CB).pending_error)() != 0 {{ return {TL_NONE}i64; }}\n"));
    }

    // Build call argument list
    let args: Vec<String> = sig
        .params
        .iter()
        .map(|(pname, ptype)| match ptype {
            CType::Ptr { mutable: true, .. } => format!("&mut _ptr_{pname}"),
            CType::Ptr { mutable: false, .. } => format!("&_ptr_{pname}"),
            CType::PtrProtocol { by_pointer: true, mutable: true } => format!("&mut _{pname} as *mut _ArPtrProtocol"),
            CType::PtrProtocol { by_pointer: true, mutable: false } => format!("&_{pname} as *const _ArPtrProtocol"),
            _ => format!("_{pname}"),
        })
        .collect();

    // Invoke and convert return value
    if sig.ret == CType::Void {
        s.push_str(&format!("    _fp({});\n", args.join(", ")));
    } else if let CType::ByValueStruct { type_name } = &sig.ret {
        // 戻り値も値のまま受け取る（冒頭の `CFnSig::unsupported_reason` で、C と同じレイアウトの分かる構造体だけが来る）
        let def = struct_defs[type_name.as_str()];
        s.push_str(&format!(
            "    let _raw_r: _Struct_{type_name} = _fp({});\n",
            args.join(", ")
        ));
        // Read the fields and construct an Arrow instance of the struct class
        for (fname, ftype) in &def.fields {
            s.push_str(&format!(
                "    let _{fname}_h = {};\n",
                field_to_handle(ftype, &format!("_raw_r.{fname}"))
            ));
            // 符号なし 64 ビットの 2^63 以上は OverflowError（1-6）。例外のハンドルをフィールドにしない
            if ftype.to_handle_may_raise() {
                s.push_str(&format!(
                    "    if _{fname}_h == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}\n"
                ));
            }
        }
        let n_fields = def.fields.len();
        let args_items: String = def.fields.iter()
            .map(|(f, _)| format!("_{f}_h"))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("    let _ctor_args = [{args_items}];\n"));
        let nlen = type_name.len();
        s.push_str(&format!(
            "    let _cls = ((*CB).get_global)(b\"{type_name}\\0\".as_ptr(), {nlen});\n"
        ));
        s.push_str(&format!(
            "    let _ret = if _cls != 0 {{ ((*CB).call_fn)(_cls, _ctor_args.as_ptr(), {n_fields}) }} else {{ 0i64 }};\n"
        ));
    } else {
        s.push_str(&format!("    let _r = _fp({});\n", args.join(", ")));
        s.push_str(&format!("    let _ret = {};\n", sig.ret.to_handle("_r")));
    }

    // `ArPtrProtocol*`（C の側で解放できる）: `address` が 0 に書き戻されていたら Arrow の `Ptr` を使えなくする（4-4）
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        if let CType::PtrProtocol { by_pointer: true, mutable: true } = ptype {
            s.push_str(&format!("    ((*CB).protocol_done)(*argv.offset({i}), _{pname}.address);\n"));
        }
    }

    // Write-back for mutable pointer params (after call, before return)
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        if let CType::Ptr { inner, mutable: true } = ptype {
            let h = format!("*argv.offset({i})");
            let new_h = inner.to_handle(&format!("_ptr_{pname}"));
            if inner.to_handle_may_raise() {
                // 符号なし 64 ビットの 2^63 以上は OverflowError（1-6）。例外のハンドルを書き戻さない
                s.push_str(&format!(
                    "    let _wb_{pname} = {new_h};\n    if _wb_{pname} == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}\n    ((*CB).write_handle)({h}, _wb_{pname});\n"
                ));
            } else {
                s.push_str(&format!("    ((*CB).write_handle)({h}, {new_h});\n"));
            }
        }
    }

    if sig.ret == CType::Void {
        s.push_str("    0i64\n");
    } else {
        s.push_str("    _ret\n");
    }
    s.push_str("}\n");

    // 統一 typed ABI 変種を追加生成（全プリミティブ + raw レイアウト既知の構造体ポインタ／
    // by-value 構造体のシグネチャの場合のみ）
    if cpp_typed_eligible(sig, all_defs) {
        s.push_str(&gen_dll_fn_typed(sig, struct_defs));
    }
    s
}

/// C 関数が typed ABI（`{name}_typed`）でエクスポート可能か。
///
/// - 戻り値: void/int/long/float/double のみ（構造体戻り値は非対応 — ハンドル経路のまま）
/// - パラメータ: int/long/float/double に加え、`raw_layout()` が既知の構造体への
///   ポインタ（`OpaqueStructPtr`）・by-value 構造体（`ByValueStruct`）、および
///   プリミティブ書き込みポインタ（`int*` / `double*` 等 — `AbiTy::OutPtr`）も可
/// exec.rs 側の `build_cpp_typed_sig` と条件を一致させること。
pub(crate) fn cpp_typed_eligible(sig: &CFnSig, all_defs: &[CStructDef]) -> bool {
    // 判定は `cpp_header` の 1 か所（未対応の判定 `unsupported_reason` も同じものを使う・1-2）
    sig.typed_abi_eligible(all_defs)
}

/// `{name}_typed(args: *const u64, ret: *mut u64, err: *mut ErrSlot) -> u32` を生成する。
///
/// - シンボルは初回呼び出し時に1度だけ解決して static にキャッシュする
/// - スカラー引数は u64 スロットから直接変換（int はキャスト、float はビット再解釈）
/// - 構造体ポインタ引数（`OpaqueStructPtr`）は u64 スロットの値をそのまま
///   `*mut/*const _Struct_{name}` にキャストする（インタープリタ側が
///   `InstanceData.raw` のフィールド領域アドレスを渡す — ゼロコピー）
/// - by-value 構造体引数（`ByValueStruct`）は同じアドレスから `*_Struct_{name}` として
///   デリファレンス・コピーしてから値渡しする（Rust の Copy セマンティクスにより
///   呼び出し先はこの関数のスタック上のコピーのみを見る — Arrow 側の原本は不変）
/// - CB コールバックは一切使わない。C 関数は Arrow 例外を投げないため、
///   シンボル欠落時（status 1）以外は常に status 0
fn gen_dll_fn_typed(sig: &CFnSig, struct_defs: &HashMap<String, &CStructDef>) -> String {
    let n = &sig.name;

    // 正確なレイアウトの構造体（`_Struct_` の写しがある）か。レイアウトの分からない構造体ポインタは `Ptr` のアドレスを
    // そのまま渡す（4-4）。⚠ `build_cpp_typed_sig` の `raw_layouts` と同じ判定
    let exact = |name: &str| struct_defs.get(name).is_some_and(|d| d.has_exact_layout());
    let param_types: Vec<String> = sig.params.iter().map(|(_, t)| match t {
        CType::OpaqueStructPtr { type_name, mutable } if exact(type_name) => {
            if *mutable { format!("*mut _Struct_{type_name}") } else { format!("*const _Struct_{type_name}") }
        }
        CType::ByValueStruct { type_name } => format!("_Struct_{type_name}"),
        CType::PtrProtocol { by_pointer: false, .. } => "_ArPtrProtocol".to_string(),
        CType::PtrProtocol { by_pointer: true, mutable: true } => "*mut _ArPtrProtocol".to_string(),
        CType::PtrProtocol { by_pointer: true, mutable: false } => "*const _ArPtrProtocol".to_string(),
        other => other.rust_extern_type(),
    }).collect();
    let fn_type = if sig.ret == CType::Void {
        format!("unsafe extern \"C\" fn({})", param_types.join(", "))
    } else {
        format!(
            "unsafe extern \"C\" fn({}) -> {}",
            param_types.join(", "),
            sig.ret.rust_extern_type()
        )
    };

    let mut s = format!(
        r#"
static mut _FPT_{n}: usize = 0;
#[no_mangle]
pub unsafe extern "C" fn {n}_typed(_args: *const u64, _ret: *mut u64, _err: *mut u8) -> u32 {{
    if _FPT_{n} == 0 {{
        let p = _loader::sym(_dll(), "{n}");
        _FPT_{n} = if p == 0 {{ usize::MAX }} else {{ p }};
    }}
    if _FPT_{n} == usize::MAX {{ return 1; }}
    type _F = {fn_type};
    let _fp: _F = std::mem::transmute(_FPT_{n});
"#
    );

    // 引数変換: u64 スロット → C 型（コールバックなしの純キャスト／ポインタキャスト）
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        match ptype {
            // C の幅と符号に切り詰める（1-6。以前は int を i32、long を i64 にまとめていた）
            CType::Int { width, signed } => {
                let rt = super::types::int_rust_type(*width, *signed);
                s.push_str(&format!("    let _{pname}: {rt} = (*_args.offset({i})) as i64 as {rt};\n"));
            }
            CType::Float => {
                s.push_str(&format!("    let _{pname}: f32 = f64::from_bits(*_args.offset({i})) as f32;\n"));
            }
            CType::Double => {
                s.push_str(&format!("    let _{pname}: f64 = f64::from_bits(*_args.offset({i}));\n"));
            }
            CType::OpaqueStructPtr { type_name, mutable } if exact(type_name) => {
                let kw = if *mutable { "mut" } else { "const" };
                s.push_str(&format!(
                    "    let _{pname} = (*_args.offset({i})) as usize as *{kw} _Struct_{type_name};\n"
                ));
            }
            // 4-4: スロットはアドレス（`Ptr` / `bytes` のコピー / 組み立てた `ArPtrProtocol`）か 0/1（`bool`）
            CType::OpaqueStructPtr { .. } | CType::VoidPtr => {
                s.push_str(&format!("    let _{pname} = (*_args.offset({i})) as usize as *mut i8;\n"));
            }
            CType::CharPtr { wide: false, mutable: false } => {
                s.push_str(&format!("    let _{pname} = (*_args.offset({i})) as usize as *const u8;\n"));
            }
            CType::Bool => {
                s.push_str(&format!("    let _{pname}: u8 = (*_args.offset({i}) != 0) as u8;\n"));
            }
            CType::PtrProtocol { by_pointer: false, .. } => {
                s.push_str(&format!(
                    "    let _{pname} = *((*_args.offset({i})) as usize as *const _ArPtrProtocol);\n"
                ));
            }
            CType::PtrProtocol { by_pointer: true, mutable } => {
                let kw = if *mutable { "mut" } else { "const" };
                s.push_str(&format!(
                    "    let _{pname} = (*_args.offset({i})) as usize as *{kw} _ArPtrProtocol;\n"
                ));
            }
            CType::ByValueStruct { type_name } => {
                // アドレスから直接コピー（呼び出し元の raw メモリには一切触れない）
                s.push_str(&format!(
                    "    let _{pname} = *((*_args.offset({i})) as usize as *const _Struct_{type_name});\n"
                ));
            }
            // プリミティブ書き込みポインタ（AbiTy::OutPtr）: スロット値は
            // インタープリタが用意したローカル u64 のアドレス — C 幅のポインタにキャスト。
            CType::Ptr { inner, mutable: true } => {
                let rt = inner.rust_extern_type();
                s.push_str(&format!(
                    "    let _{pname} = (*_args.offset({i})) as usize as *mut {rt};\n"
                ));
            }
            _ => unreachable!("cpp_typed_eligible guarantees supported params"),
        }
    }
    let _ = struct_defs; // 適格性は呼び出し元で確認済み（この関数は生成のみ）

    let call_args: Vec<String> = sig.params.iter().map(|(p, _)| format!("_{p}")).collect();
    let call = format!("_fp({})", call_args.join(", "));

    match sig.ret {
        CType::Void => {
            s.push_str(&format!("    {call};\n    *_ret = 0;\n"));
        }
        // 符号付きは符号拡張・符号なしはゼロ拡張（`as i64`）。符号なし 64 ビットの 2^63 以上は `int` に収まらない
        // ので OverflowError（1-6）
        CType::Int { width, signed } => {
            if super::types::int_rust_type(width, signed) == "u64" {
                s.push_str(&format!(
                    "    let _u: u64 = {call};\n    if _u > i64::MAX as u64 {{ return _typed_overflow_u64(_err); }}\n    *_ret = _u;\n"
                ));
            } else {
                s.push_str(&format!("    *_ret = ({call} as i64) as u64;\n"));
            }
        }
        CType::Float | CType::Double => {
            s.push_str(&format!("    *_ret = ({call} as f64).to_bits();\n"));
        }
        // 4-4: `bool` は 0/1、ポインタはアドレス（Arrow 側が bytes / 借用の Ptr にする・`decode_typed_ret`）
        CType::Bool => {
            s.push_str(&format!("    *_ret = ({call} != 0) as u64;\n"));
        }
        CType::CharPtr { .. } | CType::VoidPtr | CType::OpaqueStructPtr { .. } | CType::Ptr { .. } => {
            s.push_str(&format!("    *_ret = {call} as usize as u64;\n"));
        }
        _ => unreachable!(),
    }
    s.push_str("    0\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpp_header::types::IntWidth;

    const I32: CType = CType::Int { width: IntWidth::W32, signed: true };

    fn rec() -> CStructDef {
        CStructDef {
            name: "Rec".to_string(),
            fields: vec![("id".to_string(), I32), ("score".to_string(), CType::Double)],
            complete: true,
        }
    }

    /// 不完全な構造体（配列などのフィールドを読み飛ばした）。値で受け渡せない
    /// 配列のフィールドを読み飛ばした構造体（`float m[4]` の `Mat2` など）。フィールドの並びが C と同じでない
    fn incomplete() -> CStructDef {
        CStructDef { name: "Mat2".to_string(), fields: vec![], complete: false }
    }

    fn by_value(name: &str, ty: &str) -> CFnSig {
        let st = CType::ByValueStruct { type_name: ty.to_string() };
        CFnSig { name: name.to_string(), params: vec![("v".to_string(), st.clone())], ret: st, namespace: None, n_required: 1 }
    }

    /// 値渡し・値返しの構造体は `_Struct_` のまま受け渡す（アドレスを渡さない・戻り値をポインタとして読まない）。
    #[test]
    fn by_value_struct_is_passed_and_returned_by_value() {
        let defs = [rec()];
        let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
        let s = gen_dll_fn(&by_value("f", "Rec"), &map, &defs);
        assert!(s.contains("unsafe extern \"C\" fn(_Struct_Rec) -> _Struct_Rec"), "{s}");
        assert!(s.contains("let _v = _Struct_Rec {"), "{s}");
        assert!(s.contains("let _raw_r: _Struct_Rec = _fp(_v);"), "{s}");
        assert!(!s.contains("as *mut i8"), "{s}");
    }

    /// 生のポインタ（4-3）: `const char*` の引数は `bytes_arg`、`void*` の引数は `ptr_arg`、`char*` の戻り値は bytes、
    /// ほかのポインタの戻り値は借用の `Ptr`。
    #[test]
    fn raw_pointer_parameters_and_returns() {
        let cstr = CType::CharPtr { wide: false, mutable: false };
        let s = gen_dll_fn(
            &CFnSig {
                name: "f".to_string(),
                params: vec![("s".to_string(), cstr.clone()), ("h".to_string(), CType::VoidPtr)],
                ret: cstr,
                namespace: None,
                n_required: 2,
            },
            &HashMap::new(),
            &[],
        );
        assert!(s.contains("((*CB).bytes_arg)(*argv.offset(0))"), "{s}");
        assert!(s.contains("((*CB).ptr_arg)(*argv.offset(1), b\"void\".as_ptr(), 4)"), "{s}");
        assert!(s.contains("_cstr_to_handle("), "{s}");
        let s = gen_dll_fn(
            &CFnSig { name: "g".to_string(), params: vec![], ret: CType::VoidPtr, namespace: None, n_required: 0 },
            &HashMap::new(),
            &[],
        );
        assert!(s.contains("((*CB).make_ptr)("), "{s}");
    }

    /// 型つき経路（4-4）: `bool`・`const char*`・`void*`・`ArPtrProtocol*` の引数と、`char*` の戻り値を純粋なキャストで渡す。
    /// ハンドル経路の `ArPtrProtocol*` は `protocol_arg` で組み立て、呼び出しの後で `protocol_done` を呼ぶ。
    #[test]
    fn typed_wrapper_for_pointers_and_protocols() {
        let sig = CFnSig {
            name: "f".to_string(),
            params: vec![
                ("b".to_string(), CType::Bool),
                ("s".to_string(), CType::CharPtr { wide: false, mutable: false }),
                ("v".to_string(), CType::VoidPtr),
                ("p".to_string(), CType::PtrProtocol { by_pointer: true, mutable: true }),
            ],
            ret: CType::CharPtr { wide: false, mutable: false },
            namespace: None,
            n_required: 4,
        };
        assert!(cpp_typed_eligible(&sig, &[]));
        let s = gen_dll_fn(&sig, &HashMap::new(), &[]);
        assert!(s.contains("pub unsafe extern \"C\" fn f_typed("), "{s}");
        assert!(s.contains("let _b: u8 = (*_args.offset(0) != 0) as u8;"), "{s}");
        assert!(s.contains("let _s = (*_args.offset(1)) as usize as *const u8;"), "{s}");
        assert!(s.contains("let _v = (*_args.offset(2)) as usize as *mut i8;"), "{s}");
        assert!(s.contains("let _p = (*_args.offset(3)) as usize as *mut _ArPtrProtocol;"), "{s}");
        assert!(s.contains("*_ret = _fp(_b, _s, _v, _p) as usize as u64;"), "{s}");
        // ハンドル経路
        assert!(s.contains("((*CB).protocol_arg)(*argv.offset(3), &mut _p as *mut _ArPtrProtocol as *mut u8, 1);"), "{s}");
        assert!(s.contains("((*CB).protocol_done)(*argv.offset(3), _p.address);"), "{s}");
    }

    /// 引数を変換した後、エラーが設定されていれば C の関数を呼ばずに戻る。`bool` の引数は `to_bool`（7-12）。
    #[test]
    fn wrapper_returns_before_the_call_when_an_argument_fails() {
        let sig = CFnSig {
            name: "f".to_string(),
            params: vec![("n".to_string(), CType::Int { width: IntWidth::W32, signed: true }), ("b".to_string(), CType::Bool)],
            ret: CType::Int { width: IntWidth::W32, signed: true },
            namespace: None,
            n_required: 2,
        };
        let s = gen_dll_fn(&sig, &HashMap::new(), &[]);
        let check = s.find("((*CB).pending_error)() != 0").expect(&s);
        let call = s.find("_fp(").expect(&s);
        assert!(check < call, "{s}");
        assert!(s.contains("((*CB).to_bool)(*argv.offset(1)) as u8"), "{s}");
    }

    /// レイアウトが正確に分からない構造体（不完全・定義が無い）を値で受け渡す関数は生成しない。
    #[test]
    fn by_value_struct_without_exact_layout_is_not_generated() {
        let defs = [incomplete()];
        let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
        assert_eq!(gen_dll_fn(&by_value("f", "Mat2"), &map, &defs), "");
        assert_eq!(gen_dll_fn(&by_value("g", "Unknown"), &map, &defs), "");
    }
}

#[cfg(test)]
mod int_width_tests {
    use super::*;
    use crate::cpp_header::types::IntWidth;

    fn f(params: Vec<CType>, ret: CType) -> CFnSig {
        CFnSig {
            name: "f".to_string(),
            params: params.into_iter().enumerate().map(|(i, t)| (format!("p{i}"), t)).collect(),
            ret,
            namespace: None,
            n_required: 0,
        }
    }
    fn int(width: IntWidth, signed: bool) -> CType { CType::Int { width, signed } }

    /// 引数・戻り値を C の幅と符号どおりに宣言する（上位のゴミを読まない・符号なしはゼロ拡張・1-6）
    #[test]
    fn ints_use_their_c_width_and_sign() {
        let s = gen_dll_fn(&f(vec![int(IntWidth::W8, false), int(IntWidth::W16, true)], int(IntWidth::W32, false)), &HashMap::new(), &[]);
        assert!(s.contains("unsafe extern \"C\" fn(u8, i16) -> u32"), "{s}");
        assert!(s.contains("as u8;") && s.contains("as i16;"), "{s}");
        assert!(s.contains("((*CB).make_int)(_r as i64)"), "{s}");
        // typed 経路も同じ幅
        assert!(s.contains("let _p0: u8 = (*_args.offset(0)) as i64 as u8;"), "{s}");
    }

    /// Windows の `long` は 4 バイト（LLP64）
    #[test]
    fn c_long_follows_the_platform() {
        let s = gen_dll_fn(&f(vec![], int(IntWidth::CLong, true)), &HashMap::new(), &[]);
        let want = if cfg!(windows) { "-> i32" } else { "-> i64" };
        assert!(s.contains(want), "{s}");
    }

    /// 符号なし 64 ビットの 2^63 以上は `int` に収まらないので例外（ハンドル経路も typed 経路も）
    #[test]
    fn u64_return_reports_overflow() {
        let s = gen_dll_fn(&f(vec![], int(IntWidth::W64, false)), &HashMap::new(), &[]);
        assert!(s.contains("_overflow_u64()"), "{s}");
        assert!(s.contains("_typed_overflow_u64(_err)"), "{s}");
    }

    /// C++ の bool は 1 バイトで受け取る
    #[test]
    fn bool_is_one_byte() {
        let s = gen_dll_fn(&f(vec![CType::Bool], CType::Bool), &HashMap::new(), &[]);
        assert!(s.contains("unsafe extern \"C\" fn(u8) -> u8"), "{s}");
    }

    /// 戻り値以外（値返しの構造体のフィールド・書き込みポインタの書き戻し）でも、2^63 以上なら例外のまま返す。
    /// 例外のハンドルを値として使うと、呼び出しは正常に終わり、例外は次の呼び出しまで残ってしまう。
    #[test]
    fn u64_overflow_in_fields_and_writebacks_returns_the_exception() {
        let u64t = int(IntWidth::W64, false);
        let def = CStructDef {
            name: "Big".to_string(),
            fields: vec![("v".to_string(), u64t.clone()), ("tag".to_string(), int(IntWidth::W32, true))],
            complete: true,
        };
        let defs = [def];
        let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
        let s = gen_dll_fn(&f(vec![], CType::ByValueStruct { type_name: "Big".to_string() }), &map, &defs);
        assert!(s.contains(&format!("if _v_h == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}")), "{s}");
        assert!(!s.contains("if _tag_h =="), "{s}");

        let out = CType::Ptr { inner: Box::new(u64t), mutable: true };
        let s = gen_dll_fn(&f(vec![out], CType::Void), &HashMap::new(), &[]);
        assert!(s.contains(&format!("if _wb_p0 == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}")), "{s}");
        assert!(s.contains("((*CB).write_handle)(*argv.offset(0), _wb_p0);"), "{s}");
    }

    /// 中継 DLL の文言と、typed 経路の書き戻し（`decode_out_ptr`）の文言を揃える
    #[test]
    fn u64_overflow_message_matches_the_interpreter() {
        let msg = crate::interpreter::value::U64_OVERFLOW_MSG;
        assert!(WRAPPER_HEADER.contains(&format!("b\"{msg}\"")), "{msg}");
    }
}
