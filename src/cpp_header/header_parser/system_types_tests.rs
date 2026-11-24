use super::*;
use crate::cpp_header::types::{CDefault, IntWidth};

/// `const T*` は T が typedef でも `const` のまま（以前は外れて、`const TCHAR*` が書き込み用の `char*` になっていた）
#[test]
fn const_survives_typedef_resolution() {
    let mut td = HashMap::new();
    td.insert("TCHAR".to_string(), "char".to_string());
    td.insert("PCSTR".to_string(), "const char*".to_string());
    let c = |s: &str| parse_c_type_str(s, &HashMap::new(), &td).unwrap();
    assert_eq!(c("const TCHAR *"), CType::CharPtr { wide: false, mutable: false });
    assert_eq!(c("TCHAR *"), CType::CharPtr { wide: false, mutable: true });
    assert_eq!(c("PCSTR"), CType::CharPtr { wide: false, mutable: false });
    // 組み込みの型は typedef で置き換えない（Windows SDK の C 向けの `typedef unsigned short wchar_t;`）
    td.insert("wchar_t".to_string(), "unsigned short".to_string());
    let c = |s: &str| parse_c_type_str(s, &HashMap::new(), &td).unwrap();
    assert_eq!(c("const wchar_t *"), CType::CharPtr { wide: true, mutable: false });
}

/// 既定値: `DEFAULTPARAM( = x )` と C++ の `= x` を引数の型で読む。読めない既定値の引数は省略できない
#[test]
fn default_arguments() {
    let sig = |d: &str| parse_fn_decl_ns(d, None, &HashMap::new(), &HashMap::new()).unwrap();
    let s = sig("int DrawString(int x, const char* s, unsigned int Edge DEFAULTPARAM( = 0 ), double z DEFAULTPARAM( = 1.5f ), void* p DEFAULTPARAM(=NULL))");
    assert_eq!(s.n_required, 2);
    assert_eq!(
        s.defaults,
        vec![None, None, Some(CDefault::Int(0)), Some(CDefault::Float(1.5)), Some(CDefault::Null)]
    );
    // マクロ名の既定値は読めない: その引数は省略できない（後ろの読める既定値だけ省略できる）
    let s = sig("int f(int a, int mode DEFAULTPARAM( = MODE_X ), int b DEFAULTPARAM( = -1 ))");
    assert_eq!(s.n_required, 2);
    assert_eq!(s.defaults, vec![None, None, Some(CDefault::Int(-1))]);
    // C++ の既定値（名前と型も正しく読む）
    let s = sig("int g(int a, int b = 0x10, bool on = true)");
    assert_eq!((s.n_required, s.params[1].0.as_str(), s.params[2].0.as_str()), (1, "b", "on"));
    assert_eq!(s.defaults, vec![None, Some(CDefault::Int(16)), Some(CDefault::Bool(true))]);
    // Windows の `BOOL` の TRUE / FALSE。ポインタの既定値は NULL だけ
    let int = CType::Int { width: IntWidth::W32, signed: true };
    assert_eq!(int.parse_default("TRUE"), Some(CDefault::Int(1)));
    assert_eq!(int.parse_default("( -2 )"), Some(CDefault::Int(-2)));
    assert_eq!(CType::VoidPtr.parse_default("5"), None);
    assert_eq!(CType::Double.parse_default("SOME_MACRO"), None);
}

/// 型スタブと実行時が使うヘッダの集め方（7-28）: cpp-lib で `precompile_macros` があるときだけ取り込んだヘッダも読む
#[test]
fn included_headers_are_collected_for_cpp_lib() {
    let dir = std::env::temp_dir().join(format!("ar_collect_decls_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extra.h"), "int extra_fn(int x);
").unwrap();
    let main = "#include \"extra.h\"
int main_fn(int x);
";
    let mut cfg = crate::cpp_header::CppBuildConfig::default();
    cfg.precompile_macros = vec!["WINDOWS_DESKTOP_OS".to_string()];
    let names = |lang: &str, cfg: &crate::cpp_header::CppBuildConfig| -> Vec<String> {
        collect_header_decls(main, &dir, lang, cfg).unwrap().sigs.into_iter().map(|s| s.name).collect()
    };
    assert_eq!(names("cpp-lib", &cfg), vec!["main_fn", "extra_fn"]);
    assert_eq!(names("cpp-dll", &cfg), vec!["main_fn"], "cpp-dll does not read included headers");
    assert_eq!(names("cpp-lib", &crate::cpp_header::CppBuildConfig::default()), vec!["main_fn"], "only with precompile_macros");
    let d = collect_header_decls(main, &dir, "cpp-lib", &cfg).unwrap();
    assert_eq!(d.included.len(), 1);
    assert_eq!(d.included[0].1, 1, "one function added from extra.h");
    let _ = std::fs::remove_dir_all(&dir);
}

/// システムヘッダの構造体（`RECT`）を、参照されたものだけ足す
#[test]
fn system_structs_are_added_when_referred() {
    let mut td = HashMap::new();
    td.insert("LONG".to_string(), "long".to_string());
    let mut system = HashMap::new();
    system.insert("RECT".to_string(), "typedef struct { LONG left; LONG top; LONG right; LONG bottom; } RECT;".to_string());
    system.insert("POINT".to_string(), "typedef struct { LONG x; LONG y; } POINT;".to_string());
    let (sigs, mut structs) =
        parse_header_full("int ClearDrawScreen(const RECT *ClearRect DEFAULTPARAM( = NULL ));", &HashMap::new(), &td);
    assert!(sigs[0].unsupported_reason(&structs).is_some(), "RECT is unknown before");
    add_system_structs(&sigs, &mut structs, &system, &HashMap::new(), &td);
    let names: Vec<&str> = structs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["RECT"], "only the referred struct");
    assert!(structs[0].has_exact_layout());
    assert_eq!(sigs[0].unsupported_reason(&structs), None);
    assert_eq!(sigs[0].params[0].1, CType::OpaqueStructPtr { type_name: "RECT".to_string(), mutable: false });
    assert_eq!((sigs[0].n_required, sigs[0].defaults.clone()), (0, vec![Some(CDefault::Null)]));
}
