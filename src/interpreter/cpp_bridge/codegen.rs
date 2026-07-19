// codegen.rs — Generate the standalone Rust wrapper source that bridges
// tl handles to a C/C++ DLL or static library.
//
// Public API:
//   gen_dll_wrapper — emit the complete Rust source for a dynamic-DLL wrapper

use std::collections::HashMap;

use super::super::native_api::{TL_FALSE, TL_TRUE};
use super::types::{CStructDef, CFnSig, CType};

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
}

static mut CB: *const ArCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn ar_init(cb: *const ArCallbacks) { CB = cb; }
"#;

// Platform loader inserted into dll-wrapper source.
// RTLD_LAZY is embedded as the literal `1` (standardized value on all POSIX platforms).
pub(crate) const PLATFORM_LOADER: &str = r#"
#[cfg(windows)]
mod _loader {
    extern "system" {
        fn LoadLibraryA(name: *const u8) -> usize;
        fn GetProcAddress(module: usize, name: *const u8) -> usize;
    }
    pub unsafe fn load(path: &str) -> usize {
        let p = format!("{path}\0");
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
        CType::Int | CType::Bool => Some("i32"),
        CType::Long => Some("i64"),
        CType::Float => Some("f32"),
        CType::Double => Some("f64"),
        CType::VoidPtr
        | CType::CharPtr
        | CType::OpaqueStructPtr { .. }
        | CType::Ptr { .. } => Some("*mut i8"),
        // Nested struct-by-value or function pointer → not representable
        CType::ByValueStruct { .. } | CType::FnPtr => None,
        CType::Void => None,
    }
}

/// 構造体の全フィールドが Rust プリミティブにマップできる場合（ネスト構造体なし）に `true` を返す。
pub(crate) fn struct_is_bridgeable(def: &CStructDef) -> bool {
    !def.fields.is_empty() && def.fields.iter().all(|(_, ct)| ctype_to_rust_field(ct).is_some())
}

/// アリーナからハンドルを読み取り、構造体フィールドの Rust 型に変換する Rust 式を生成する。
fn field_from_handle(ct: &CType, handle: &str) -> String {
    match ct {
        CType::Int => format!("((*CB).to_int)({handle}) as i32"),
        CType::Long => format!("((*CB).to_int)({handle})"),
        CType::Float => format!("((*CB).to_float)({handle}) as f32"),
        CType::Double => format!("((*CB).to_float)({handle})"),
        CType::Bool => format!("if {handle} == {TL_TRUE}i64 {{ 1i32 }} else {{ 0i32 }}"),
        _ => format!("{handle} as *mut i8"),
    }
}

/// 構造体フィールドの値を tl ハンドルにラップする Rust 式を生成する。
fn field_to_handle(ct: &CType, val: &str) -> String {
    match ct {
        CType::Int => format!("((*CB).make_int)({val} as i64)"),
        CType::Long => format!("((*CB).make_int)({val})"),
        CType::Float => format!("((*CB).make_float)({val} as f64)"),
        CType::Double => format!("((*CB).make_float)({val})"),
        CType::Bool => {
            format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}")
        }
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
        src.push_str(&gen_dll_fn(sig, &struct_map));
    }
    src
}

/// 1 つの C 関数シグネチャに対して `{name}_tl(argc, argv) -> i64` 形式の Rust ラッパー関数ソースを生成する。
fn gen_dll_fn(sig: &CFnSig, struct_defs: &HashMap<String, &CStructDef>) -> String {
    let mut s = String::new();
    let n = &sig.name;

    // Build the C function-pointer type — pointers use their Rust extern type
    let param_types: Vec<String> = sig
        .params
        .iter()
        .map(|(_, t)| t.rust_extern_type())
        .collect();
    let fn_type = if sig.ret == CType::Void {
        format!("unsafe extern \"C\" fn({})", param_types.join(", "))
    } else {
        format!(
            "unsafe extern \"C\" fn({}) -> {}",
            param_types.join(", "),
            sig.ret.rust_extern_type()
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
            CType::CharPtr => {
                s.push_str(&format!("    let _{pname} = ((*CB).to_cstr)({h});\n"));
            }
            CType::ByValueStruct { type_name } => {
                if let Some(def) = struct_defs.get(type_name.as_str()) {
                    // Read each field from the tl instance via get_attr, build C struct
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
                        "    let _buf_{pname} = _Struct_{type_name} {{ {field_inits} }};\n"
                    ));
                    s.push_str(&format!(
                        "    let _{pname} = &_buf_{pname} as *const _Struct_{type_name} as *mut i8;\n"
                    ));
                } else {
                    s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
                }
            }
            _ => {
                s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
            }
        }
    }

    // Build call argument list
    let args: Vec<String> = sig
        .params
        .iter()
        .map(|(pname, ptype)| match ptype {
            CType::Ptr { mutable: true, .. } => format!("&mut _ptr_{pname}"),
            CType::Ptr { mutable: false, .. } => format!("&_ptr_{pname}"),
            _ => format!("_{pname}"),
        })
        .collect();

    // Invoke and convert return value
    if sig.ret == CType::Void {
        s.push_str(&format!("    _fp({});\n", args.join(", ")));
    } else if let CType::ByValueStruct { type_name } = &sig.ret {
        s.push_str(&format!("    let _raw_r = _fp({});\n", args.join(", ")));
        if let Some(def) = struct_defs.get(type_name.as_str()) {
            // Read fields from the C buffer and construct a tl class instance
            s.push_str(&format!("    let _rp = _raw_r as *const _Struct_{type_name};\n"));
            for (fname, ftype) in &def.fields {
                s.push_str(&format!(
                    "    let _{fname}_h = {};\n",
                    field_to_handle(ftype, &format!("(*_rp).{fname}"))
                ));
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
            s.push_str(&format!("    let _ret = {};\n", sig.ret.to_handle("_raw_r")));
        }
    } else {
        s.push_str(&format!("    let _r = _fp({});\n", args.join(", ")));
        s.push_str(&format!("    let _ret = {};\n", sig.ret.to_handle("_r")));
    }

    // Write-back for mutable pointer params (after call, before return)
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        if let CType::Ptr { inner, mutable: true } = ptype {
            let h = format!("*argv.offset({i})");
            let new_h = inner.to_handle(&format!("_ptr_{pname}"));
            s.push_str(&format!("    ((*CB).write_handle)({h}, {new_h});\n"));
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
    if cpp_typed_eligible(sig, struct_defs) {
        s.push_str(&gen_dll_fn_typed(sig, struct_defs));
    }
    s
}

/// `CType` が「raw レイアウト既知の構造体」を指す（ポインタ or by-value）場合、
/// その構造体名を返す。typed ABI 適格性判定・引数コード生成の両方で使う。
fn typed_struct_name(ct: &CType) -> Option<&str> {
    match ct {
        CType::OpaqueStructPtr { type_name, .. } | CType::ByValueStruct { type_name } => {
            Some(type_name.as_str())
        }
        _ => None,
    }
}

/// C 関数が typed ABI（`{name}_typed`）でエクスポート可能か。
///
/// - 戻り値: void/int/long/float/double のみ（構造体戻り値は非対応 — ハンドル経路のまま）
/// - パラメータ: int/long/float/double に加え、`raw_layout()` が既知の構造体への
///   ポインタ（`OpaqueStructPtr`）・by-value 構造体（`ByValueStruct`）、および
///   プリミティブ書き込みポインタ（`int*` / `double*` 等 — `AbiTy::OutPtr`）も可
/// exec.rs 側の `build_cpp_typed_sig` と条件を一致させること。
pub(crate) fn cpp_typed_eligible(sig: &CFnSig, struct_defs: &HashMap<String, &CStructDef>) -> bool {
    matches!(
        sig.ret,
        CType::Void | CType::Int | CType::Long | CType::Float | CType::Double
    ) && sig.params.iter().all(|(_, t)| {
        matches!(t, CType::Int | CType::Long | CType::Float | CType::Double)
            || matches!(t, CType::Ptr { inner, mutable: true }
                if matches!(**inner, CType::Int | CType::Long | CType::Float | CType::Double))
            || typed_struct_name(t)
                .and_then(|name| struct_defs.get(name))
                .is_some_and(|d| d.raw_layout().is_some())
    })
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

    let param_types: Vec<String> = sig.params.iter().map(|(_, t)| match t {
        CType::OpaqueStructPtr { type_name, mutable } => {
            if *mutable { format!("*mut _Struct_{type_name}") } else { format!("*const _Struct_{type_name}") }
        }
        CType::ByValueStruct { type_name } => format!("_Struct_{type_name}"),
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
            CType::Int => {
                s.push_str(&format!("    let _{pname}: i32 = (*_args.offset({i})) as i64 as i32;\n"));
            }
            CType::Long => {
                s.push_str(&format!("    let _{pname}: i64 = (*_args.offset({i})) as i64;\n"));
            }
            CType::Float => {
                s.push_str(&format!("    let _{pname}: f32 = f64::from_bits(*_args.offset({i})) as f32;\n"));
            }
            CType::Double => {
                s.push_str(&format!("    let _{pname}: f64 = f64::from_bits(*_args.offset({i}));\n"));
            }
            CType::OpaqueStructPtr { type_name, mutable } => {
                let kw = if *mutable { "mut" } else { "const" };
                s.push_str(&format!(
                    "    let _{pname} = (*_args.offset({i})) as usize as *{kw} _Struct_{type_name};\n"
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
        CType::Int | CType::Long => {
            s.push_str(&format!("    *_ret = ({call} as i64) as u64;\n"));
        }
        CType::Float | CType::Double => {
            s.push_str(&format!("    *_ret = ({call} as f64).to_bits();\n"));
        }
        _ => unreachable!(),
    }
    s.push_str("    0\n}\n");
    s
}
