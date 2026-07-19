// tests/async_tests.rs — 非同期(async)処理のテスト。

use super::*;

// ---------------------------------------------------------------------------
// Async tests
// ---------------------------------------------------------------------------

/// async_manager_constructor のテスト。
#[test]
fn test_async_manager_constructor() {
    let val = run_get("mut mng = AsyncManager(num_thread=2)\n", "mng");
    assert!(matches!(val, Value::AsyncManager(_)));
}

/// async_manager_num_thread_attr のテスト。
#[test]
fn test_async_manager_num_thread_attr() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=4)\nlet n = mng.num_thread\n",
        "n",
    );
    assert!(matches!(val, Value::UInt(4)));
}

/// async_manager_raise_immediately_default_false のテスト。
#[test]
fn test_async_manager_raise_immediately_default_false() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=1)\nlet r = mng.raise_immediately\n",
        "r",
    );
    assert!(matches!(val, Value::Bool(false)));
}

/// async_manager_raise_immediately_set のテスト。
#[test]
fn test_async_manager_raise_immediately_set() {
    let val = run_get("mut mng = AsyncManager(num_thread=1, raise_immediately=True)\nlet r = mng.raise_immediately\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_single_task_result のテスト。
#[test]
fn test_async_single_task_result() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=1)\nmng <- async->int:\n    block_return 42\nmng.wait_for_finish()\nlet r = mng.results\n",
        "r",
    );
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::Int(42)));
    } else {
        panic!("expected list");
    }
}

/// async_multiple_tasks_all_done のテスト。
#[test]
fn test_async_multiple_tasks_all_done() {
    let src = "
mut mng = AsyncManager(num_thread=2)
mng <- async->int:
    block_return 1
mng <- async->int:
    block_return 2
mng.wait_for_finish()
let done = mng.all_done()
";
    let val = run_get(src, "done");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_task_captures_env のテスト。
#[test]
fn test_async_task_captures_env() {
    let src = "
let x = 100
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    block_return x
mng.wait_for_finish()
let r = mng.results
";
    let val = run_get(src, "r");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::Int(100)));
    } else {
        panic!("expected list");
    }
}

/// async_error_stored_in_error_list のテスト。
#[test]
fn test_async_error_stored_in_error_list() {
    let src = "
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    raise RuntimeError(\"TestError\")
mng.wait_for_finish()
let errs = mng.error_list
";
    let val = run_get(src, "errs");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(!matches!(items[0], Value::None));
    } else {
        panic!("expected list");
    }
}

/// async_no_error_gives_none_in_error_list のテスト。
#[test]
fn test_async_no_error_gives_none_in_error_list() {
    let src = "
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    block_return 7
mng.wait_for_finish()
let errs = mng.error_list
";
    let val = run_get(src, "errs");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::None));
    } else {
        panic!("expected list");
    }
}

/// async_raise_immediately_propagates_to_try_except のテスト。
#[test]
fn test_async_raise_immediately_propagates_to_try_except() {
    let src = "
mut mng = AsyncManager(num_thread=1, raise_immediately=True)
mng <- async->int:
    raise RuntimeError(\"AsyncFail\")
mut caught = False
try:
    mng.wait_for_finish()
except:
    caught = True
";
    let val = run_get(src, "caught");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_status_enum_values のテスト。
#[test]
fn test_async_status_enum_values() {
    let w = run_get("let w = Async.Waiting\n", "w");
    let r = run_get("let r = Async.Running\n", "r");
    let d = run_get("let d = Async.Done\n", "d");
    assert!(matches!(
        w,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Waiting)
    ));
    assert!(matches!(
        r,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Running)
    ));
    assert!(matches!(
        d,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Done)
    ));
}

/// async_wrong_target_type_error のテスト。
#[test]
fn test_async_wrong_target_type_error() {
    let src = "
let x = 5
x <- async->int:
    block_return 1
";
    assert!(run(src).is_err());
}

