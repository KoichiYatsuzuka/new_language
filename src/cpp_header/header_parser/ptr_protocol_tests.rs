use super::*;

/// `ArPtrProtocol` を引数・戻り値・構造体のフィールドで認識する（ffi_value_exchange_plan.md 4-2）。
/// 引数は 4-4、値返しは 4-5 から使える（ポインタを返すものは使えない）。共有ヘッダの定義を写してあってもクラスにしない。
#[test]
fn recognises_ptr_protocol() {
    let header = r#"
            #include "ar_ptr_protocol.h"
            typedef struct ArPtrProtocol { uint64_t address; const char* elem_type; } ArPtrProtocol;
            ArPtrProtocol make_squares(int n);
            void free_it(ArPtrProtocol* p);
            int count(const struct ArPtrProtocol* p);
            int pass(ArPtrProtocol p);
            ArPtrProtocol* bad(void);
            typedef struct { int n; ArPtrProtocol data; } Holder;
        "#;
    let (sigs, structs) = parse_header_full(header, &HashMap::new(), &HashMap::new());
    let get = |name: &str| sigs.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
    assert_eq!(get("make_squares").ret, CType::PtrProtocol { by_pointer: false, mutable: false });
    assert_eq!(get("free_it").params[0].1, CType::PtrProtocol { by_pointer: true, mutable: true });
    assert_eq!(get("count").params[0].1, CType::PtrProtocol { by_pointer: true, mutable: false });
    assert_eq!(get("pass").params[0].1, CType::PtrProtocol { by_pointer: false, mutable: false });
    assert_eq!(get("bad").ret, CType::PtrProtocol { by_pointer: true, mutable: true });
    // ポインタを返すものは使えない。引数（4-4）と値返し（4-5）は使える
    let why = get("bad").unsupported_reason(&structs).unwrap_or_default();
    assert!(why.contains("ArPtrProtocol"), "bad: {why}");
    for name in ["make_squares", "free_it", "count", "pass"] {
        assert_eq!(get(name).unsupported_reason(&structs), None, "{name}");
    }
    assert!(structs.iter().all(|d| d.name != "ArPtrProtocol"), "the protocol itself is not a class");
    let holder = structs.iter().find(|d| d.name == "Holder").expect("Holder");
    assert_eq!(holder.fields[1].1, CType::PtrProtocol { by_pointer: false, mutable: false });
    assert!(!holder.has_exact_layout(), "a struct with an ArPtrProtocol field is expanded in 4-6");
}

/// 本体の無い構造体の宣言を「構造体の名前」として持つ（4-3）。解決できない typedef（`TCHAR`）へのポインタは
/// 構造体ではないので使えないまま。
#[test]
fn forward_declared_structs_are_known() {
    let header = r#"
            typedef struct Db Db;
            struct Window;
            typedef struct { int x; } Point;
            Db* open_db(const char* name);
            int count(Db* db);
            void show(struct Window* w);
            int draw(const TCHAR* s);
        "#;
    let (sigs, structs) = parse_header_full(header, &HashMap::new(), &HashMap::new());
    for name in ["Db", "Window"] {
        let d = structs.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("{name}"));
        assert!(d.fields.is_empty() && !d.complete);
    }
    assert_eq!(structs.iter().filter(|d| d.name == "Point").count(), 1, "a struct with a body is not added twice");
    let get = |name: &str| sigs.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
    assert_eq!(get("open_db").unsupported_reason(&structs), None);
    assert_eq!(get("count").unsupported_reason(&structs), None);
    assert_eq!(get("show").params[0].1, CType::OpaqueStructPtr { type_name: "Window".to_string(), mutable: true });
    assert_eq!(get("show").unsupported_reason(&structs), None);
    let why = get("draw").unsupported_reason(&structs).unwrap_or_default();
    assert!(why.contains("TCHAR") && why.contains("does not declare as a struct"), "{why}");
}
