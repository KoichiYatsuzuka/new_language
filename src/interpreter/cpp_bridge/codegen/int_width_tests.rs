use super::*;
use crate::cpp_header::types::IntWidth;

fn f(params: Vec<CType>, ret: CType) -> CFnSig {
    CFnSig {
        name: "f".to_string(),
        params: params.into_iter().enumerate().map(|(i, t)| (format!("p{i}"), t)).collect(),
        ret,
        namespace: None,
        n_required: 0,
    defaults: vec![],
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
    // OS は `ar_config.json` の `"os"`（7-32）。どちらの OS の中継コードも作れる
    crate::cpp_header::target::set_target_os(crate::cpp_header::TargetOs::Windows);
    let s = gen_dll_fn(&f(vec![], int(IntWidth::CLong, true)), &HashMap::new(), &[]);
    assert!(s.contains("-> i32"), "{s}");
    crate::cpp_header::target::set_target_os(crate::cpp_header::TargetOs::Linux);
    let s = gen_dll_fn(&f(vec![], int(IntWidth::CLong, true)), &HashMap::new(), &[]);
    assert!(s.contains("-> i64"), "{s}");
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
    // 正確なレイアウトの構造体は C のバイト列を写す（4-6）。2^63 以上は `InstanceData::from_c_bytes` が OverflowError
    let named = CStructDef {
        name: "BigName".to_string(),
        fields: vec![
            ("v".to_string(), u64t.clone()),
            ("name".to_string(), CType::CharPtr { wide: false, mutable: false }),
        ],
        complete: true,
    };
    let defs = [def, named];
    let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
    let s = gen_dll_fn(&f(vec![], CType::ByValueStruct { type_name: "Big".to_string() }), &map, &defs);
    assert!(s.contains("((*CB).make_raw_instance)(_cls, &_raw_r as *const _Struct_Big as *const u8"), "{s}");
    assert!(!s.contains("call_fn"), "no constructor call: {s}");
    // ポインタのフィールドを持つ構造体はフィールドごとに変換する（D-7）。2^63 以上は例外のまま返す
    let s = gen_dll_fn(&f(vec![], CType::ByValueStruct { type_name: "BigName".to_string() }), &map, &defs);
    assert!(s.contains(&format!("if _v_h == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}")), "{s}");
    assert!(s.contains("let _name_h = _cstr_to_handle(_raw_r.name as *const u8);"), "{s}");
    assert!(s.contains("((*CB).make_instance)(_cls, _fields.as_ptr(), 2)"), "{s}");
    // 値渡しの引数: `const char*` のフィールドは `bytes_arg`
    let s = gen_dll_fn(&f(vec![CType::ByValueStruct { type_name: "BigName".to_string() }], CType::Void), &map, &defs);
    assert!(s.contains("let _p0_name: *mut i8 = ((*CB).bytes_arg)("), "{s}");

    let out = CType::Ptr { inner: Box::new(u64t), mutable: true };
    let s = gen_dll_fn(&f(vec![out], CType::Void), &HashMap::new(), &[]);
    assert!(s.contains(&format!("if _wb_p0 == {TL_EXCEPTION}i64 {{ return {TL_EXCEPTION}i64; }}")), "{s}");
    assert!(s.contains("((*CB).write_handle)(*argv.offset(0), _wb_p0);"), "{s}");
}

/// `wchar_t*` の補助と型は `ar_config.json` の `"os"` で変わる（7-32）: Windows は UTF-16 をそのまま写し、Linux / macOS は
/// UTF-32 を UTF-16LE に符号化し直す。どちらの OS の中継コードも作れる
#[test]
fn wchar_helper_follows_the_os() {
    use crate::cpp_header::{target::set_target_os, TargetOs};
    let wide_ret = || gen_dll_fn(&f(vec![], CType::CharPtr { wide: true, mutable: false }), &HashMap::new(), &[]);
    set_target_os(TargetOs::Windows);
    assert!(wrapper_header().contains("unsafe fn _wstr_to_handle(p: *const u16)"));
    let s = wide_ret();
    assert!(s.contains("*const u16") && !s.contains("*const u32"), "{s}");
    set_target_os(TargetOs::Linux);
    let h = wrapper_header();
    assert!(h.contains("unsafe fn _wstr_to_handle(p: *const u32)") && h.contains("char::from_u32"), "{h}");
    let s = wide_ret();
    assert!(s.contains("*const u32"), "{s}");
}

/// 中継 DLL の文言と、typed 経路の書き戻し（`decode_out_ptr`）の文言を揃える
#[test]
fn u64_overflow_message_matches_the_interpreter() {
    let msg = crate::interpreter::value::U64_OVERFLOW_MSG;
    assert!(wrapper_header().contains(&format!("b\"{msg}\"")), "{msg}");
}
