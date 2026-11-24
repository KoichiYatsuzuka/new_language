use super::*;
use crate::cpp_header::types::IntWidth;

const I32: CType = CType::Int { width: IntWidth::W32, signed: true };

fn sig(name: &str, ret: CType) -> CFnSig {
    CFnSig {
        name: name.to_string(),
        params: vec![("x".to_string(), I32)],
        ret,
        namespace: None,
        n_required: 1,
    defaults: vec![],
    }
}

fn rec() -> CStructDef {
    CStructDef {
        name: "Rec".to_string(),
        fields: vec![("id".to_string(), I32), ("score".to_string(), CType::Double)],
        complete: true,
    }
}

/// 刻印は同じ入力なら同じ値（置き場所に依存しない・ヘッダもシムもファイル名で入る）。
#[test]
fn cache_stamp_is_deterministic() {
    let cfg = CppBuildConfig::default();
    let sigs = vec![sig("f", I32)];
    let a = cache_stamp(&sigs, &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
    let b = cache_stamp(&sigs, &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
    assert_eq!(a, b);
    assert_eq!(a.len(), 16);
}

/// 関数・構造体・設定のどれが変わっても刻印が変わる（古い生成物を使い続けない）。
#[test]
fn cache_stamp_changes_with_inputs() {
    let cfg = CppBuildConfig::default();
    let base = cache_stamp(&[sig("f", I32)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
    let more_fns =
        cache_stamp(&[sig("f", I32), sig("g", I32)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
    let other_ret = cache_stamp(&[sig("f", CType::Double)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
    let no_struct = cache_stamp(&[sig("f", I32)], &[], "lib.h", "ar_lib_shim.dll", &cfg);
    let mut cfg2 = CppBuildConfig::default();
    cfg2.cl_extra_flags.push("/DFOO".to_string());
    let other_cfg = cache_stamp(&[sig("f", I32)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg2);
    for other in [&more_fns, &other_ret, &no_struct, &other_cfg] {
        assert_ne!(&base, other);
    }
}

/// `.syms` の 1 行目の刻印を読み、残りを関数名として扱う。刻印の無い（1-5 より前の）ファイルは刻印 `None`。
#[test]
fn read_syms_file_parses_stamp() {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let with_stamp = dir.join(format!("ar_test_syms_stamp_{pid}.syms"));
    let legacy = dir.join(format!("ar_test_syms_legacy_{pid}.syms"));
    std::fs::write(&with_stamp, "#stamp 0123456789abcdef\nf\ng").unwrap();
    std::fs::write(&legacy, "f\ng").unwrap();
    let all = vec![sig("f", I32), sig("g", I32), sig("h", I32)];

    let (stamp, eff) = read_syms_file(&with_stamp, &all).unwrap();
    assert_eq!(stamp.as_deref(), Some("0123456789abcdef"));
    assert_eq!(eff.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["f", "g"]);

    let (stamp, eff) = read_syms_file(&legacy, &all).unwrap();
    assert_eq!(stamp, None);
    assert_eq!(eff.len(), 2);

    assert!(read_syms_file(&dir.join(format!("ar_test_syms_missing_{pid}.syms")), &all).is_none());
    let _ = std::fs::remove_file(&with_stamp);
    let _ = std::fs::remove_file(&legacy);
}

/// シムのエラーから集めた名前のうち、シムに入れた関数だけを外す（型名・名前空間は外す対象にもログにも出さない・7-29）
#[test]
fn failed_fn_names_keeps_only_functions() {
    let sigs = vec![sig("SetWindowText", I32), sig("Plain", I32), sig("GetWindowHandle", I32)];
    let quoted: std::collections::HashSet<String> =
        ["HWND", "DxLib", "GetWindowHandle", "SetWindowText", "HICON"].iter().map(|s| s.to_string()).collect();
    assert_eq!(failed_fn_names(&quoted, &sigs), vec!["SetWindowText", "GetWindowHandle"]);
    // 関数名が 1 つも無いエラーは「外せるものが無い」（再試行せずにエラーを返す）
    let types_only: std::collections::HashSet<String> = ["HWND", "DxLib"].iter().map(|s| s.to_string()).collect();
    assert!(failed_fn_names(&types_only, &sigs).is_empty());
}
