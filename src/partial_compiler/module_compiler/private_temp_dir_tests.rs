use super::{private_temp_dir, sweep_stale_private_dirs};

/// 呼ぶたびに別のディレクトリ（同じプロセスの中でも取り合わない）
#[test]
fn each_call_gets_its_own_dir() {
    let a = private_temp_dir("ar_ptd_test").unwrap();
    let b = private_temp_dir("ar_ptd_test").unwrap();
    assert_ne!(a, b);
    assert!(a.is_dir() && b.is_dir());
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// 後始末は `<prefix>_<数字>_<数字>` の形で古いものだけを消す（新しいもの・別の名前は残す）
#[test]
fn sweep_removes_only_stale_dirs_of_the_same_shape() {
    let root = std::env::temp_dir().join(format!("ar_ptd_sweep_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let fresh = root.join("ar_x_1_0");
    let other = root.join("ar_x_foo");
    std::fs::create_dir_all(&fresh).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    sweep_stale_private_dirs(&root, "ar_x");
    // 作ったばかりなので古くない
    assert!(fresh.is_dir());
    assert!(other.is_dir());
    let _ = std::fs::remove_dir_all(&root);
}
