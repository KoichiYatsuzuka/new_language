use super::*;

fn probe_src() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|d| d.join("examples/interop/rs_crates/ffi_probe_rs/src/lib.rs"))
        .find(|p| p.exists())
        .expect("ffi_probe_rs/src/lib.rs");
    std::fs::read_to_string(p).unwrap()
}

/// `new()` があればそれで、無ければ構造体リテラルで、どちらも無理なら構築を未対応にする（2-5）。
#[test]
fn construction_follows_new_or_literal() {
    let structs = parse_struct_sigs(&probe_src());
    let get = |n: &str| structs.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("{n}"));
    assert!(get("Account").use_new_fn && get("Account").ctor_unsupported.is_none());
    assert!(get("Person").use_new_fn);
    assert!(!get("Counter").use_new_fn && get("Counter").ctor_unsupported.is_none());
    // `pub` フィールドが無くても型として拾う
    assert!(get("Secret").use_new_fn && get("Secret").fields.is_empty());
    assert!(get("Sealed").ctor_unsupported.as_deref().is_some_and(|w| w.contains("private fields")));
}

/// `-> Self` は構造体自身。`self` の無い関連関数と `self` を消費するメソッドは理由つきで未対応（2-5）。
#[test]
fn methods_are_classified() {
    let structs = parse_struct_sigs(&probe_src());
    let counter = structs.iter().find(|s| s.name == "Counter").unwrap();
    let doubled = counter.methods.iter().find(|m| m.name == "doubled").expect("doubled");
    assert_eq!(doubled.return_struct.as_deref(), Some("Counter"));
    let seal = counter.methods.iter().find(|m| m.name == "seal").expect("seal");
    assert_eq!(seal.return_struct.as_deref(), Some("Sealed"));
    let why = |n: &str| counter.unsupported_methods.iter().find(|u| u.name == n).map(|u| u.reason.clone());
    assert!(why("zero").is_some_and(|w| w.contains("associated function")));
    assert!(why("into_n").is_some_and(|w| w.contains("by value")));
}

/// 複数行のシグネチャも読む。呼べない関数は理由つきで返す（以前は黙って消えた）。
#[test]
fn free_functions_are_classified() {
    let (ok, bad) = parse_fn_sigs(&probe_src());
    assert!(ok.iter().any(|f| f.name == "add_three" && f.params.len() == 3));
    let why = |n: &str| bad.iter().find(|u| u.name == n).map(|u| u.reason.clone()).unwrap_or_default();
    // 5-4: Vec<String> / &[i64] / Vec<u8> / Option<String> は受け渡せる
    for n in ["words", "sum", "count_bytes", "maybe"] {
        assert!(ok.iter().any(|f| f.name == n), "{n}");
    }
    assert!(why("pair").starts_with("it returns `(i32, String)`"));
    assert!(why("bump").contains("mutable reference"));
    assert_eq!(why("ident"), "it is generic");
    assert_eq!(bad.iter().find(|u| u.name == "ident").unwrap().params.len(), 1);
    assert!(ok.iter().all(|f| !["pair", "bump", "ident"].contains(&f.name.as_str())));
}

#[test]
fn types_are_accepted_by_position() {
    assert!(is_param_type("&[u8]") && !is_return_type("&[u8]"));
    assert!(is_return_type("Vec<u8>") && is_param_type("Vec<u8>"));
    assert!(is_return_type("[u8; 32]") && !is_param_type("[u8; 32]"));
    assert!(is_field_type("String") && !is_field_type("&str") && !is_field_type("Vec<u8>"));
    // 5-4: 数値の Vec / slice・Vec<String>・Option<T>
    assert!(is_param_type("&[i32]") && is_param_type("Vec<f64>") && is_return_type("Vec<u32>"));
    assert!(!is_return_type("&[i32]") && !is_param_type("Vec<i128>") && !is_param_type("Vec<bool>"));
    assert!(is_param_type("&[&str]") && is_return_type("Vec<String>") && is_return_type("Vec<bool>"));
    assert!(is_param_type("Option<String>") && is_return_type("Option<i64>") && !is_param_type("Option<&str>"));
}

/// 型スタブの型（5-4・D-16）。数値の Vec は `Ptr[T]`、引数は `Ptr[T]` か list を受ける。
#[test]
fn arrow_types_of_collections() {
    assert_eq!(rust_type_to_ar("Vec<u8>"), "bytes");
    assert_eq!(rust_type_to_ar("[u8; 4]"), "bytes");
    assert_eq!(rust_type_to_ar("Vec<i32>"), "Ptr[int32]");
    assert_eq!(rust_type_to_ar("Vec<String>"), "list[str]");
    assert_eq!(rust_type_to_ar("Option<String>"), "Option[str]");
    assert_eq!(rust_param_type_to_ar("&[f64]"), "Union[Ptr[float64], list[float]]");
    assert_eq!(rust_param_type_to_ar("Vec<usize>"), "Union[Ptr[uint64], list[int]]");
    assert_eq!(rust_param_type_to_ar("&[u8]"), "bytes");
}
