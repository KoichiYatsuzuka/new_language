use super::*;

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ar_native_lib_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// version 3 はコールバックの表の大きさを書き、読み込みはこの exe の表より大きければ誤りにする（known_bugs.md #29）。
/// 表と同じ大きさまでは読む。大きさを持たない version 1 は照らさない。
#[test]
fn load_tlc_refuses_a_dll_needing_more_callbacks() {
    let d = scratch("arc_v3");
    let have = crate::arc_format::CALLBACK_TABLE_LEN;
    let exports = vec![codegen::FnExport { name: "f".into(), n_params: 1 }];
    let ok = d.join("ok.arc");
    write_tlc_native("fn f(let x: int) -> int:\n    return x\n", "ok", &exports, b"dll", have, &ok).unwrap();
    let arc = crate::arc_format::read(&ok).unwrap();
    assert_eq!(arc.native.as_ref().map(|(e, dll)| (e.len(), dll.as_slice())), Some((1, &b"dll"[..])));
    assert_eq!(load_tlc(&ok).unwrap().0, "ok");
    let _ = take_native_bytes("ok");

    let newer = d.join("newer.arc");
    write_tlc_native("", "newer", &exports, b"dll", have + 1, &newer).unwrap();
    let err = load_tlc(&newer).unwrap_err().to_string();
    assert!(err.contains("compiled by a newer arrow") && err.contains("--compile"), "{err}");
    assert!(take_native_bytes("newer").is_none(), "a refused DLL must not be registered");

    // version 1（大きさを持たない）: 書いていた形をそのまま作って読む
    let mut v1 = Vec::new();
    v1.extend_from_slice(MAGIC);
    v1.extend_from_slice(&crate::arc_format::VERSION_V1.to_le_bytes());
    write_len_prefixed(&mut v1, b"old");
    write_len_prefixed(&mut v1, b"");
    v1.extend_from_slice(&0u32.to_le_bytes());
    write_len_prefixed(&mut v1, b"dll");
    let arc = crate::arc_format::parse(&v1).unwrap();
    assert_eq!(arc.native.map(|(e, dll)| (e.len(), dll)), Some((0, b"dll".to_vec())));

    // version 2（取り除いた JIT のビットコードの形）は今も読まない（新しい形と読み違えない）
    let mut v2 = v1.clone();
    v2[4..8].copy_from_slice(&2u32.to_le_bytes());
    let err = crate::arc_format::parse(&v2).err().expect("version 2 must be refused");
    assert!(err.contains("unsupported .arc version 2"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// 生成した IR が使う欄の最大の番号 + 1 を数え、表の型（`%ArCallbacks`）の欄の数はこの exe の表と同じ。
#[test]
fn callbacks_required_counts_the_highest_field() {
    let ir = "  %a = getelementptr inbounds %ArCallbacks, ptr %cb, i32 0, i32 3\n  %b = load ptr, ptr %a\n  \
                  %c = getelementptr inbounds %ArCallbacks, ptr %cb, i32 0, i32 41\n";
    assert_eq!(codegen::callbacks_required(ir), Ok(42));
    assert_eq!(codegen::callbacks_required("define void @f() {\n  ret void\n}\n"), Ok(0));
    assert!(codegen::callbacks_required("  %a = getelementptr inbounds %ArCallbacks, ptr %cb, i32 0, i32 %n\n").is_err());
    let src = "fn f(let s: str) -> str:\n    return s + \"!\"\n";
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None).parse_program().expect("parse error");
    let (ir, _) = codegen::generate_llvm_module(&stmts, &AstAnnotations::default()).expect("eligible functions");
    let ty = ir.lines().find(|l| l.starts_with("%ArCallbacks = type")).expect("callback table type");
    assert_eq!(ty.matches("ptr").count() as u32, crate::arc_format::CALLBACK_TABLE_LEN);
    let need = codegen::callbacks_required(&ir).unwrap();
    assert!(need > 0 && need <= crate::arc_format::CALLBACK_TABLE_LEN, "{need}");
}

/// 同じ中身なら書き直さない・違う中身なら書き直す（7-8）
#[test]
fn place_native_lib_rewrites_only_changed_contents() {
    let d = scratch("place");
    assert!(place_native_lib(&d, "m_tl_x.dll", b"abc").unwrap(), "first placement writes");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(d.join("m_tl_x.dll")).unwrap().set_modified(old).unwrap();
    assert!(!place_native_lib(&d, "m_tl_x.dll", b"abc").unwrap(), "same contents are not rewritten");
    let touched = std::fs::metadata(d.join("m_tl_x.dll")).unwrap().modified().unwrap();
    assert!(touched > old, "reuse refreshes the modification time");
    assert!(place_native_lib(&d, "m_tl_x.dll", b"abcd").unwrap(), "changed contents are rewritten");
    assert_eq!(std::fs::read(d.join("m_tl_x.dll")).unwrap(), b"abcd");
    let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 1, "no temporary file is left: {left:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// 古いものだけ・同じ prefix の 16 桁のハッシュの名前だけを消す
#[test]
fn remove_stale_native_libs_keeps_recent_and_foreign_files() {
    let d = scratch("stale");
    let ext = native_lib_ext();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60);
    let names = [
        format!("m_tl_{:016x}.{ext}", 1),       // 古い → 消す
        format!("m_tl_{:016x}.{ext}", 2),       // 新しい → 残す
        format!("m_tl_{:016x}.{ext}", 3),       // keep（今書いたもの）→ 残す
        format!("m_tl_b_tl_{:016x}.{ext}", 4),  // 別のモジュール（`m_tl_b`）→ 残す
        format!("n_tl_{:016x}.{ext}", 5),       // 別の prefix → 残す
    ];
    for (i, n) in names.iter().enumerate() {
        std::fs::write(d.join(n), b"x").unwrap();
        if i != 1 {
            std::fs::File::options().write(true).open(d.join(n)).unwrap().set_modified(old).unwrap();
        }
    }
    remove_stale_native_libs(&d, "m_tl", &names[2]);
    for (i, n) in names.iter().enumerate() {
        assert_eq!(d.join(n).exists(), i != 0, "{n}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// 1 つのプロセスで同じ中身を 2 回置くと別のパスになる（DLL の実体を共有しない）
#[test]
fn materialize_native_lib_never_hands_out_a_path_twice() {
    let prefix = format!("ar_test_materialize_{}", std::process::id());
    let bytes = b"not really a dll".to_vec();
    let a = materialize_native_lib(&prefix, &bytes).unwrap();
    let b = materialize_native_lib(&prefix, &bytes).unwrap();
    assert_ne!(a, b);
    assert_eq!(std::fs::read(&a).unwrap(), bytes);
    assert_eq!(std::fs::read(&b).unwrap(), bytes);
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}
