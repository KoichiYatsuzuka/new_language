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

/// タスクへ送る C# のオブジェクト（ffi_value_exchange_plan.md 6-2）。ハンドル・ブリッジ・クラス名はそのまま、`Rc`
/// （外側とスタブのクラス）は別に作る。⚠ 以前は `deep_clone` の `other => other.clone()` に落ち、`Rc` を共有したまま
/// ワーカーへ送っていた（参照カウントは非アトミックなので、スレッドをまたぐと壊れうる）。
#[test]
fn test_deep_clone_of_cs_object_keeps_the_handle_and_shares_no_rc() {
    use std::rc::Rc;
    let interp = run_interp("class Person:
    pass
");
    let class = match interp.get_val("Person") {
        Some(Value::Class(c)) => c,
        other => panic!("expected the class Person, got {other:?}"),
    };
    let original = Value::CsObject(Rc::new(crate::interpreter::value::CsObjectData {
        class_name: "Person".to_string(),
        handle: 42,
        bridge_path: std::path::PathBuf::from("bridge.dll"),
        class,
        is_proc: true,
    }));
    let cloned = original.deep_clone();
    let (a, b) = match (&original, &cloned) {
        (Value::CsObject(a), Value::CsObject(b)) => (a, b),
        _ => panic!("expected Value::CsObject on both sides"),
    };
    assert_eq!((b.handle, b.class_name.as_str(), b.is_proc), (42, "Person", true));
    assert_eq!(b.bridge_path, a.bridge_path);
    assert_eq!(b.class.name, a.class.name);
    assert!(!Rc::ptr_eq(a, b), "deep_clone shared the CsObject Rc");
    assert!(!Rc::ptr_eq(&a.class, &b.class), "deep_clone shared the stub class Rc");
}

/// タスクへ送る、スレッドに縛られる値（ffi_value_exchange_plan.md 7-19）。`Rc` を別に作った「切り離した複製」になり、
/// 使うとエラー（元の値はそのまま使える）。⚠ 以前は `deep_clone` の `other => other.clone()` に落ち、`Rc` を共有した
/// まま送っていた（タスクの中の `mng.results` はメインの表を読み、`ticks.emit(..)` はメインのハンドラをワーカーで呼んだ）。
#[test]
fn test_deep_clone_detaches_thread_bound_values() {
    use crate::interpreter::async_mgr::ensure_attached;
    use std::rc::Rc;
    let interp = run_interp("let s = Signal[int]()\nmut mng = AsyncManager(num_thread=1)\n");
    for name in ["s", "mng", "EventLoop"] {
        let original = interp.get_val(name).unwrap_or_else(|| panic!("{name} must exist"));
        let cloned = original.deep_clone();
        let shared = match (&original, &cloned) {
            (Value::Signal(a), Value::Signal(b)) => Rc::ptr_eq(a, b),
            (Value::AsyncManager(a), Value::AsyncManager(b)) => Rc::ptr_eq(a, b),
            (Value::EventLoop(a), Value::EventLoop(b)) => Rc::ptr_eq(a, b),
            _ => panic!("{name}: deep_clone changed the kind of value"),
        };
        assert!(!shared, "{name}: deep_clone shared the Rc");
        assert!(ensure_attached(&original).is_ok(), "{name}: the original must stay usable");
        let err = ensure_attached(&cloned).expect_err("the copy must not be usable");
        assert!(err.contains("cannot be used inside an async task"), "{name}: {err}");
    }
}

/// テンプレートの値もタスクへ送るときに `Rc` を別に作る（7-19。以前は明示的に `rc.clone()` で共有していた）。
#[test]
fn test_deep_clone_of_template_fn_shares_no_rc() {
    use std::rc::Rc;
    let interp = run_interp("fn show[T](let x: T) -> str:\n    return str(x)\n");
    let original = interp.get_val("show").expect("show must exist");
    match (&original, &original.deep_clone()) {
        (Value::TemplateFn(a), Value::TemplateFn(b)) => {
            assert_eq!(a.name, b.name);
            assert_eq!(a.template_params.len(), b.template_params.len());
            assert!(!Rc::ptr_eq(a, b), "deep_clone shared the template Rc");
        }
        _ => panic!("expected Value::TemplateFn on both sides"),
    }
}

/// タスクの本体でテンプレートを具体化する（7-19）。見えている大域の名前は大域の複製と同じ値でなければならない。
/// ⚠ 7-19 でテンプレートの `Rc` を作り直したとき、見えている名前と大域を別々に複製していたので、本体の `Box` と
///   具体化のキャッシュの鍵（大域の `Box` の同一性）が別の値になり、`TemplateError` で結果が黙って `None` になった
///   （`examples/typing/template_contexts.ar` は exit code では気づけない）。
#[test]
fn test_template_instance_in_a_task_matches_its_template() {
    let src = concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    fn get(self) -> T:\n",
        "        return self.v\n",
        "mut mng = AsyncManager(num_thread=1)\n",
        "mng <- async->int:\n",
        "    block_return Box[int](7).get()\n",
        "mng.wait_for_finish()\n",
        "let errors = mng.error_list\n",
        "let results = mng.results\n",
    );
    let interp = run_interp(src);
    let errors = match interp.get_val("errors") {
        Some(Value::List(l)) => l.borrow().clone(),
        other => panic!("expected the error list, got {other:?}"),
    };
    assert!(matches!(errors.as_slice(), [Value::None]), "the task failed: {errors:?}");
    let results = match interp.get_val("results") {
        Some(Value::List(l)) => l.borrow().clone(),
        other => panic!("expected the results, got {other:?}"),
    };
    assert!(matches!(results.as_slice(), [Value::Int(7)]), "unexpected results: {results:?}");
}
