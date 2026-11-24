use super::*;
use crate::cpp_header::types::CDefault;

fn sig(d: &str) -> CFnSig {
    parse_fn_decl_ns(d, None, &HashMap::new(), &HashMap::new()).unwrap_or_else(|e| panic!("{d}: {e}"))
}

/// 関数ポインタの引数は `FnPtr` になり（名前は宣言子の中の名前）、関数は残って理由つきで未対応になる。
/// 以前は `void (* cb )` をマクロの呼び出しとして消し、残りが型として読めずに関数ごと捨てていた
#[test]
fn fn_ptr_params_are_kept() {
    let s = sig("int SetRestoreShredPoint( void (* ShredPoint )( void ) )");
    assert_eq!(s.params, vec![("ShredPoint".to_string(), CType::FnPtr)]);
    let why = s.unsupported_reason(&[]).unwrap_or_default();
    assert!(why.contains("`ShredPoint`") && why.contains("callback"), "{why}");
    // 空白の無い形・呼び出し規約つき・名前の無い宣言子
    assert_eq!(sig("void f(void(*cb)(void))").params[0], ("cb".to_string(), CType::FnPtr));
    assert_eq!(sig("void f(int (__cdecl *cmp)(const void*, const void*))").params[0].0, "cmp");
    assert_eq!(sig("void f(void (WINAPI* cb)(int))").params[0].0, "cb");
    assert_eq!(sig("void f(void (*)(int))").params[0], ("_p0".to_string(), CType::FnPtr));
    // 宣言子の後ろの引数も読める（DxLib の MV1LoadModelFromMem の形）
    let s = sig(
        "int MV1LoadModelFromMem( const void *FileImage, int FileSize, \
             int (* FileReadFunc )( const TCHAR *FilePath, void **FileImageAddr, int *FileSize, void *FileReadFuncData ), \
             int (* FileReleaseFunc )( void *MemoryAddr, void *FileReadFuncData ), void *FileReadFuncData DEFAULTPARAM( = NULL ) )",
    );
    let names: Vec<&str> = s.params.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["FileImage", "FileSize", "FileReadFunc", "FileReleaseFunc", "FileReadFuncData"]);
    assert_eq!((s.params[2].1.clone(), s.params[3].1.clone()), (CType::FnPtr, CType::FnPtr));
    assert_eq!((s.n_required, s.defaults[4]), (4, Some(CDefault::Null)));
}

/// 既定値のマクロ（`DEFAULTPARAM( = x )`）と SAL 風の注釈（後ろに型が続く）は従来どおり消える
#[test]
fn parameter_macros_are_still_stripped() {
    let s = sig("int f(int a DEFAULTPARAM( = 0 ))");
    assert_eq!((s.params[0].0.as_str(), s.n_required), ("a", 0));
    let s = sig("int g(_Out_writes_to_(*pcch, *pcch) char* buf, int* pcch)");
    assert_eq!(s.params[0], ("buf".to_string(), CType::CharPtr { wide: false, mutable: true }));
}

/// ヘッダ全体から読んでも関数が残る（以前は `CppBridge: unknown C type` を出して捨てていた）
#[test]
fn header_keeps_callback_functions() {
    let header = "int SetRestoreGraphCallback( void (* Callback )( void ) ) ;\nint Plain( int x ) ;\n";
    let (sigs, _) = parse_header_full(header, &HashMap::new(), &HashMap::new());
    let names: Vec<&str> = sigs.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["SetRestoreGraphCallback", "Plain"]);
}
