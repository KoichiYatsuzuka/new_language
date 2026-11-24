use super::*;

/// シムも構造体を値のまま受け渡す（実際の関数と同じ呼び出し規約・静的バッファを使わない・1-3）。
#[test]
fn shim_passes_and_returns_structs_by_value() {
    let st = CType::ByValueStruct { type_name: "Rec".to_string() };
    let sig = CFnSig {
        name: "f".to_string(),
        params: vec![("r".to_string(), st.clone())],
        ret: st,
        namespace: None,
        n_required: 1,
    defaults: vec![],
    };
    let src = gen_cpp_shim_source(&[sig], "lib.h", &[], true);
    assert!(src.contains("Rec ar_shim_f(Rec r) { return f(r); }"), "{src}");
    assert!(!src.contains("_ret_buf_"), "{src}");
    assert!(!src.contains("void*"), "{src}");
}
