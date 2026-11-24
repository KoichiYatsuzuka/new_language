use super::*;

fn st(name: &str, fields: Vec<CType>, complete: bool) -> CStructDef {
    CStructDef {
        name: name.to_string(),
        fields: fields.into_iter().enumerate().map(|(i, t)| (format!("f{i}"), t)).collect(),
        complete,
    }
}
fn structs() -> Vec<CStructDef> {
    vec![
        st("Rec", vec![I32, CType::Double], true),
        st("Named", vec![I32, CType::CharPtr { wide: false, mutable: false }], true),
        st("Partial", vec![I32; 2], false),
        st("WithCallback", vec![I32, CType::FnPtr], true),
        st("OutBuf", vec![I32, CType::CharPtr { wide: false, mutable: true }], true),
    ]
}
fn sig(params: Vec<CType>, ret: CType) -> CFnSig {
    CFnSig {
        name: "f".to_string(),
        params: params.into_iter().enumerate().map(|(i, t)| (format!("p{i}"), t)).collect(),
        ret,
        namespace: None,
        n_required: 0,
        defaults: vec![],
    }
}
fn st_ptr(n: &str) -> CType { CType::OpaqueStructPtr { type_name: n.to_string(), mutable: false } }
fn by_val(n: &str) -> CType { CType::ByValueStruct { type_name: n.to_string() } }
const CSTR: CType = CType::CharPtr { wide: false, mutable: false };
const I32: CType = CType::Int { width: IntWidth::W32, signed: true };
const I64: CType = CType::Int { width: IntWidth::W64, signed: true };
const U8: CType = CType::Int { width: IntWidth::W8, signed: false };

/// 今の Arrow から正しく呼べるもの（理由なし）
#[test]
fn supported_signatures() {
    let s = structs();
    for f in [
        sig(vec![I32, CType::Double], I64),
        sig(vec![CSTR], I32),
        sig(vec![by_val("Rec")], by_val("Rec")),
        sig(vec![st_ptr("Rec"), CType::Ptr { inner: Box::new(CType::Double), mutable: true }], I32),
        sig(vec![CType::Bool], CType::Void),
        // 4-3: ポインタの戻り値（`char*` / `wchar_t*` → bytes、ほか → Ptr[T]）と、`void*`・構造体と宣言された名前への
        // ポインタ（レイアウトが分からなくても `Ptr[T]` で渡せる）の引数
        sig(vec![], CSTR),
        sig(vec![], CType::CharPtr { wide: true, mutable: false }),
        sig(vec![], CType::Ptr { inner: Box::new(I32), mutable: true }),
        sig(vec![], CType::VoidPtr),
        sig(vec![], st_ptr("Rec")),
        sig(vec![], st_ptr("Partial")),
        sig(vec![CType::VoidPtr], CType::Void),
        sig(vec![st_ptr("Named")], CType::Void),
        sig(vec![st_ptr("Partial"), CSTR], I32),
        // 4-4: 正確なレイアウトの構造体ポインタと C の文字列・bool が混ざっても型つき経路で渡せる
        sig(vec![st_ptr("Rec"), CSTR], CType::Void),
        sig(vec![st_ptr("Rec")], CType::Bool),
        sig(vec![CType::PtrProtocol { by_pointer: true, mutable: true }], I32),
        sig(vec![CType::PtrProtocol { by_pointer: false, mutable: false }, st_ptr("Rec")], CSTR),
        // 4-5: `ArPtrProtocol` の値返し
        sig(vec![I32], CType::PtrProtocol { by_pointer: false, mutable: false }),
        // 4-6: ポインタのフィールドを持つ構造体の値渡し・値返し（D-7 の規則）。`char*` のフィールドは戻り値なら受け取れる
        sig(vec![by_val("Named")], CType::Void),
        sig(vec![], by_val("Named")),
        sig(vec![], by_val("OutBuf")),
        // 4-7: `char*` の出力バッファ（`mut` の bytes へ書き戻す）
        sig(vec![CType::CharPtr { wide: false, mutable: true }, I32], CType::Void),
    ] {
        assert_eq!(f.unsupported_reason(&s), None, "{f:?}");
    }
}

/// ポインタの指す先の要素の型名（D-12・4-3）
#[test]
fn pointer_element_names() {
    assert_eq!(CType::VoidPtr.ptr_elem_name().as_deref(), Some("void"));
    assert_eq!(st_ptr("Rec").ptr_elem_name().as_deref(), Some("Rec"));
    assert_eq!(CSTR.ptr_elem_name().as_deref(), Some("uint8"));
    let p = |t: CType| CType::Ptr { inner: Box::new(t), mutable: false }.ptr_elem_name();
    assert_eq!(p(I32).as_deref(), Some("int32"));
    assert_eq!(p(U8).as_deref(), Some("uint8"));
    assert_eq!(p(I64).as_deref(), Some("int64"));
    assert_eq!(p(CType::Float).as_deref(), Some("float32"));
    assert_eq!(p(CType::Double).as_deref(), Some("float64"));
    assert_eq!(I32.ptr_elem_name(), None);
}

/// 呼べないもの: 理由に何が問題かが入る（1-2）
#[test]
fn unsupported_signatures() {
    let s = structs();
    let cases: Vec<(CFnSig, &str)> = vec![
        (sig(vec![], CType::FnPtr), "function pointer"),
        // 構造体と宣言されていない名前（解決できない typedef など）へのポインタ（4-3）
        (sig(vec![], st_ptr("Unknown")), "does not declare as a struct"),
        (sig(vec![], by_val("Partial")), "struct `Partial` by value"),
        (sig(vec![CType::CharPtr { wide: true, mutable: false }], CType::Void), "wide string"),
        (sig(vec![CType::FnPtr], CType::Void), "callback"),
        (sig(vec![CType::Ptr { inner: Box::new(I32), mutable: false }], CType::Void), "single value or an array"),
        (sig(vec![st_ptr("Unknown")], CType::Void), "does not declare as a struct"),
        // 構造体に関数ポインタのフィールド（4-6 でも渡せない）
        (sig(vec![by_val("WithCallback")], CType::Void), "field `f1` cannot be passed in a struct by value"),
        // 引数では、C が書き込むかもしれない `char*` のフィールドは渡せない（戻り値なら受け取れる）
        (sig(vec![by_val("OutBuf")], CType::Void), "field `f1` cannot be passed in a struct by value"),
        // 型つき経路に載らない形（値返しの構造体）と、インスタンスを渡す構造体ポインタは混ぜられない
        (sig(vec![st_ptr("Rec")], by_val("Rec")), "struct pointer together with a struct returned by value"),
        // `ArPtrProtocol*` を返す関数は誰が持つか決められない（値で返す）
        (sig(vec![], CType::PtrProtocol { by_pointer: true, mutable: false }), "return the struct itself by value"),
    ];
    for (f, want) in cases {
        let got = f.unsupported_reason(&s);
        assert!(got.as_deref().is_some_and(|r| r.contains(want)), "{f:?} -> {got:?} (want {want:?})");
    }
}

/// 正確なレイアウト: 完全で int / float / double だけ
#[test]
fn exact_layout() {
    let s = structs();
    assert!(s[0].has_exact_layout());
    assert!(!s[1].has_exact_layout()); // ポインタのフィールド
    assert!(!s[2].has_exact_layout()); // 不完全（配列などのフィールドを読み飛ばした）
    assert!(st("Rgba", vec![U8; 4], true).has_exact_layout()); // 4 バイト未満の整数も幅が分かる（1-6）
    assert!(!st("Empty", vec![], true).has_exact_layout());
    assert!(!st("L", vec![CType::Bool], true).has_exact_layout());
}

/// `wchar_t` の要素型は OS で幅も符号も決まり、値・`wchar_t*`・数値のポインタのどれから引いても同じ（7-35。以前は
/// Linux で値の要素型が `uint32`、`wchar_t*` が `int32` だった）
#[test]
fn wchar_element_type_follows_the_os() {
    use super::super::target::{set_target_os, TargetOs};
    let wchar = CType::Int { width: IntWidth::WChar, signed: false };
    let wstr = CType::CharPtr { wide: true, mutable: false };
    let wptr = CType::Ptr { inner: Box::new(wchar.clone()), mutable: false };
    for (os, name) in [(TargetOs::Windows, "uint16"), (TargetOs::Linux, "int32"), (TargetOs::MacOs, "int32")] {
        set_target_os(os);
        assert_eq!(wchar.c_elem_name().as_deref(), Some(name), "{os:?}");
        assert_eq!(wstr.ptr_elem_name().as_deref(), Some(name), "{os:?}");
        assert_eq!(wptr.ptr_elem_name().as_deref(), Some(name), "{os:?}");
    }
}
