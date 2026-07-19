// tests/exceptions.rs — 例外処理(try/except/finally/raise)と内部エラーの捕捉可能性のテスト。

use super::*;

// --- Exception handling tests ---

/// raise_uncaught_reaches_caller のテスト。
#[test]
fn test_raise_uncaught_reaches_caller() {
    let raised = run_exc("raise ValueError(\"oops\")\n").unwrap();
    let raised = raised.expect("expected a raised exception");
    if let Value::Instance(inst) = &raised.exception {
        assert_eq!(inst.borrow().class.name, "ValueError");
        let b = inst.borrow();
        let msg = b.class.field_index.get("message").and_then(|&idx| {
            match b.field_value(idx) {
                Some(Value::Str(s)) => Some(s),
                _ => None,
            }
        }).expect("message field missing or wrong type");
        drop(b);
        assert_eq!(msg, "oops");
    } else {
        panic!("expected Instance");
    }
}

/// try_except_catches_matching_type のテスト。
#[test]
fn test_try_except_catches_matching_type() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    raise ValueError(\"bad\")\n",
        "except ValueError as e:\n",
        "    x = 1\n",
    );
    assert!(run(src).is_ok());
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(1)));
}

/// try_except_does_not_catch_different_type のテスト。
#[test]
fn test_try_except_does_not_catch_different_type() {
    let src = concat!(
        "try:\n",
        "    raise TypeError(\"t\")\n",
        "except ValueError as e:\n",
        "    pass\n",
    );
    let raised = run_exc(src).unwrap();
    assert!(
        raised.is_some(),
        "TypeError should not be caught by ValueError handler"
    );
    if let Some(r) = raised {
        if let Value::Instance(inst) = &r.exception {
            assert_eq!(inst.borrow().class.name, "TypeError");
        }
    }
}

/// try_finally_runs_always のテスト。
#[test]
fn test_try_finally_runs_always() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    raise RuntimeError(\"r\")\n",
        "except RuntimeError:\n",
        "    x = 1\n",
        "finally:\n",
        "    x = 2\n",
    );
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(2)));
}

/// try_no_raise_skips_except のテスト。
#[test]
fn test_try_no_raise_skips_except() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    x = 5\n",
        "except ValueError:\n",
        "    x = 99\n",
    );
    let x = run_get(src, "x");
    assert!(matches!(x, Value::Int(5)));
}

/// try_bare_except_catches_all のテスト。
#[test]
fn test_try_bare_except_catches_all() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    raise KeyError(\"k\")\n",
        "except:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// user_defined_error_class のテスト。
#[test]
fn test_user_defined_error_class() {
    let src = concat!(
        "class MyError(Error):\n",
        "    pass\n",
        "mut caught = 0\n",
        "try:\n",
        "    raise MyError(\"custom\")\n",
        "except MyError as e:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// exception_message_accessible のテスト。
#[test]
fn test_exception_message_accessible() {
    let src = concat!(
        "mut msg = \"\"\n",
        "try:\n",
        "    raise ValueError(\"hello world\")\n",
        "except ValueError as e:\n",
        "    msg = e.message\n",
    );
    let v = run_get(src, "msg");
    match v {
        Value::Str(s) => assert_eq!(s, "hello world"),
        _ => panic!("expected Str"),
    }
}

/// bare_raise_reraises のテスト。
#[test]
fn test_bare_raise_reraises() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    try:\n",
        "        raise ValueError(\"v\")\n",
        "    except ValueError as e:\n",
        "        x = 1\n",
        "        raise\n",
        "except ValueError:\n",
        "    x = 2\n",
    );
    let v = run_get(src, "x");
    assert!(matches!(v, Value::Int(2)));
}

/// exception_propagates_through_function のテスト。
#[test]
fn test_exception_propagates_through_function() {
    let src = concat!(
        "fn thrower() -> None:\n",
        "    raise RuntimeError(\"from fn\")\n",
        "mut caught = 0\n",
        "try:\n",
        "    thrower()\n",
        "except RuntimeError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

// --- internal error catchability ---

/// catch_internal_type_error のテスト。
#[test]
fn test_catch_internal_type_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x: int = 1 + \"bad\"\n",
        "except TypeError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_index_error のテスト。
#[test]
fn test_catch_internal_index_error() {
    let src = concat!(
        "mut caught = 0\n",
        "let lst = [1, 2, 3]\n",
        "try:\n",
        "    let x = lst[10]\n",
        "except IndexError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_key_error のテスト。
#[test]
fn test_catch_internal_key_error() {
    let src = concat!(
        "mut caught = 0\n",
        "let d = {\"a\": 1}\n",
        "try:\n",
        "    let x = d[\"missing\"]\n",
        "except KeyError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_zero_division のテスト。
#[test]
fn test_catch_internal_zero_division() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = 10 / 0\n",
        "except ZeroDivisionError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// catch_internal_name_error のテスト。
#[test]
fn test_catch_internal_name_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = undefined_variable\n",
        "except NameError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// internal_error_message_accessible のテスト。
#[test]
fn test_internal_error_message_accessible() {
    let src = concat!(
        "mut msg = \"\"\n",
        "let lst = [1, 2]\n",
        "try:\n",
        "    let x = lst[99]\n",
        "except IndexError as e:\n",
        "    msg = e.message\n",
    );
    let v = run_get(src, "msg");
    match v {
        Value::Str(s) => assert!(!s.is_empty(), "message should be non-empty"),
        _ => panic!("expected Str"),
    }
}

/// bare_except_catches_internal_error のテスト。
#[test]
fn test_bare_except_catches_internal_error() {
    let src = concat!(
        "mut caught = 0\n",
        "try:\n",
        "    let x = 1 / 0\n",
        "except:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// internal_error_not_caught_by_wrong_type のテスト。
#[test]
fn test_internal_error_not_caught_by_wrong_type() {
    let src = concat!(
        "try:\n",
        "    let x = 1 / 0\n",
        "except ValueError:\n",
        "    pass\n",
    );
    let raised = run_exc(src).unwrap();
    assert!(
        raised.is_some(),
        "ZeroDivisionError should not be caught by ValueError handler"
    );
}

/// internal_error_in_function_is_catchable のテスト。
#[test]
fn test_internal_error_in_function_is_catchable() {
    let src = concat!(
        "fn divide(a: int, b: int) -> int:\n",
        "    return a / b\n",
        "mut caught = 0\n",
        "try:\n",
        "    divide(10, 0)\n",
        "except ZeroDivisionError:\n",
        "    caught = 1\n",
    );
    let v = run_get(src, "caught");
    assert!(matches!(v, Value::Int(1)));
}

/// finally_runs_after_internal_error のテスト。
#[test]
fn test_finally_runs_after_internal_error() {
    let src = concat!(
        "mut x = 0\n",
        "try:\n",
        "    let y = 1 / 0\n",
        "except ZeroDivisionError:\n",
        "    x = 1\n",
        "finally:\n",
        "    x = 2\n",
    );
    let v = run_get(src, "x");
    assert!(matches!(v, Value::Int(2)));
}

