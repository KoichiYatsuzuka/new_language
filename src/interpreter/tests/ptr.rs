// tests/ptr.rs — 外部のメモリを指す値 `Value::Ptr` のテスト（ffi_value_exchange_plan.md 3-2）。
//
// ⚠ Arrow のコードから `Ptr` を作る手段はまだ無い（作るのはフェーズ 4 のブリッジ）。ここでは値を直接作り、
//   インタプリタの入口（メソッド呼び出し・等値・ハッシュ・表示・`deep_clone`）に通す。
// ⚠ 解放の回数は `ctx` の指す数え先で数える（テストは並列に走るので、テストごとに別の数え先を使う）。

use super::*;
use crate::interpreter::{PtrHandle, PtrOwner};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

unsafe extern "C" fn count_release(ctx: *mut std::ffi::c_void, _address: u64) {
    (*(ctx as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
}

fn owned_ptr(counter: &AtomicUsize) -> Value {
    let ctx = counter as *const AtomicUsize as usize;
    let h = PtrHandle::new(0x1000, "int32", 0, Some(4), PtrOwner::Owned { release: count_release, ctx }, None).unwrap();
    Value::Ptr(Arc::new(h))
}

fn borrowed_ptr() -> Value {
    Value::Ptr(Arc::new(PtrHandle::new(0x2000, "void", 0, None, PtrOwner::Borrowed, None).unwrap()))
}

#[test]
fn release_method_frees_once_and_then_errors() {
    let n = AtomicUsize::new(0);
    let p = owned_ptr(&n);
    let copy = p.clone();
    let mut interp = Interpreter::new();
    let r = interp.eval_method_call_full(p.clone(), "release", vec![], None).unwrap();
    assert!(matches!(r, Value::None));
    assert_eq!(n.load(Ordering::SeqCst), 1);
    // コピーは同じ `Ptr` なので、コピーから解放しようとしても誤り（2 回目は呼ばない）
    let e = interp.eval_method_call_full(copy, "release", vec![], None).unwrap_err();
    assert!(e.starts_with("ValueError:") && e.contains("already released"), "{e}");
    assert!(interp.display(&p).contains("released"));
    drop(p);
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn last_reference_releases_and_deep_clone_shares() {
    let n = AtomicUsize::new(0);
    let p = owned_ptr(&n);
    // 非同期タスクへ渡すときの `deep_clone` も共有（2 回解放しない・別々に解放しない）
    let task_copy = p.deep_clone();
    match (&p, &task_copy) {
        (Value::Ptr(a), Value::Ptr(b)) => assert!(Arc::ptr_eq(a, b)),
        _ => unreachable!(),
    }
    drop(p);
    assert_eq!(n.load(Ordering::SeqCst), 0, "the task's copy is still alive");
    drop(task_copy);
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn borrowed_pointer_cannot_be_released() {
    let mut interp = Interpreter::new();
    let e = interp.eval_method_call_full(borrowed_ptr(), "release", vec![], None).unwrap_err();
    assert!(e.contains("borrowed"), "{e}");
    let e = interp.eval_method_call_full(borrowed_ptr(), "address", vec![], None).unwrap_err();
    assert!(e.starts_with("AttributeError:"), "the address is never exposed: {e}");
}

#[test]
fn equality_and_hash_are_identity() {
    let interp = Interpreter::new();
    let p = borrowed_ptr();
    let same = p.clone();
    // 同じアドレスを指す別の `Ptr`
    let other = Value::Ptr(Arc::new(PtrHandle::new(0x2000, "void", 0, None, PtrOwner::Borrowed, None).unwrap()));
    assert!(interp.values_eq(&p, &same).unwrap());
    assert!(!interp.values_eq(&p, &other).unwrap());
    assert!(!interp.values_eq(&p, &Value::Int(0x2000)).unwrap());
    // 等値なら同じハッシュ（辞書のキー・集合の要素に使える）
    assert_eq!(Interpreter::default_hash(&p).ok(), Interpreter::default_hash(&same).ok());
    assert!(Interpreter::default_hash(&p).is_ok());
}

#[test]
fn display_and_type_name_hide_the_address() {
    let interp = Interpreter::new();
    let p = borrowed_ptr();
    assert_eq!(interp.display(&p), "<Ptr[void]>");
    assert_eq!(interp.type_name(&p), "Ptr");
    assert_eq!(interp.type_name_of(&p), "Ptr[void]");
    assert!(interp.is_truthy(&p), "a Ptr is never NULL (NULL is None)");
}
