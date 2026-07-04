// tests/set_type.rs — set 型のテスト。

use super::*;
use crate::interpreter::*;

// ---------------------------------------------------------------------------
// set type
// ---------------------------------------------------------------------------

/// set_literal_basic のテスト。
#[test]
fn test_set_literal_basic() {
    let val = run_get("let s = {1, 2, 3}\n", "s");
    if let Value::Set(items) = val {
        let v = items.borrow();
        assert_eq!(v.len(), 3);
    } else {
        panic!("expected Set");
    }
}

/// set_literal_dedup のテスト。
#[test]
fn test_set_literal_dedup() {
    let val = run_get("let s = {1, 2, 2, 3, 1}\n", "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 3);
    } else {
        panic!("expected Set");
    }
}

/// set_constructor_empty のテスト。
#[test]
fn test_set_constructor_empty() {
    let val = run_get("let s = set()\n", "s");
    assert!(matches!(val, Value::Set(_)));
    if let Value::Set(items) = val {
        assert!(items.borrow().is_empty());
    }
}

/// set_constructor_from_list のテスト。
#[test]
fn test_set_constructor_from_list() {
    let val = run_get("let s = set([1, 2, 2, 3])\n", "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 3);
    } else {
        panic!("expected Set");
    }
}

/// set_constructor_from_str のテスト。
#[test]
fn test_set_constructor_from_str() {
    // "aab" → {'a', 'b'}
    let val = run_get("let s = set(\"aab\")\n", "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_add のテスト。
#[test]
fn test_set_add() {
    let src = "let s = {1, 2}\ns.add(3)\n";
    let val = run_get(src, "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 3);
    } else {
        panic!("expected Set");
    }
}

/// set_add_duplicate のテスト。
#[test]
fn test_set_add_duplicate() {
    let src = "let s = {1, 2}\ns.add(2)\n";
    let val = run_get(src, "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_discard のテスト。
#[test]
fn test_set_discard() {
    let src = "let s = {1, 2, 3}\ns.discard(2)\n";
    let val = run_get(src, "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_discard_missing_no_error のテスト。
#[test]
fn test_set_discard_missing_no_error() {
    assert!(run("let s = {1, 2}\ns.discard(99)\n").is_ok());
}

/// set_remove のテスト。
#[test]
fn test_set_remove() {
    let src = "let s = {1, 2, 3}\ns.remove(2)\n";
    let val = run_get(src, "s");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_remove_missing_error のテスト。
#[test]
fn test_set_remove_missing_error() {
    assert!(run("let s = {1, 2}\ns.remove(99)\n").is_err());
}

/// list_pop のテスト。
#[test]
fn test_list_pop() {
    let src = "mut xs: list[int] = [10, 20, 30]\nlet v = xs.pop()\n";
    assert!(matches!(run_get(src, "v"), Value::Int(30)));
}

/// list_pop_empty_error のテスト。
#[test]
fn test_list_pop_empty_error() {
    assert!(run("mut xs: list[int] = []\nxs.pop()\n").is_err());
}

/// set_pop のテスト。
#[test]
fn test_set_pop() {
    let src = "let s = {1, 2, 3}\nlet v = s.pop()\n";
    let val = run_get(src, "v");
    assert!(matches!(val, Value::Int(_)));
}

/// set_pop_empty_error のテスト。
#[test]
fn test_set_pop_empty_error() {
    assert!(run("let s = set()\ns.pop()\n").is_err());
}

/// set_clear のテスト。
#[test]
fn test_set_clear() {
    let src = "let s = {1, 2, 3}\ns.clear()\n";
    let val = run_get(src, "s");
    if let Value::Set(items) = val {
        assert!(items.borrow().is_empty());
    } else {
        panic!("expected Set");
    }
}

/// set_len のテスト。
#[test]
fn test_set_len() {
    let val = run_get("let n = len({1, 2, 3})\n", "n");
    assert!(matches!(val, Value::Int(3)));
}

/// set_len_empty のテスト。
#[test]
fn test_set_len_empty() {
    let val = run_get("let n = len(set())\n", "n");
    assert!(matches!(val, Value::Int(0)));
}

/// set_in_operator のテスト。
#[test]
fn test_set_in_operator() {
    let val = run_get("let r = 2 in {1, 2, 3}\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_in_operator_false のテスト。
#[test]
fn test_set_in_operator_false() {
    let val = run_get("let r = 99 in {1, 2, 3}\n", "r");
    assert!(matches!(val, Value::Bool(false)));
}

/// set_not_in_operator のテスト。
#[test]
fn test_set_not_in_operator() {
    let val = run_get("let r = 99 not in {1, 2, 3}\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_union のテスト。
#[test]
fn test_set_union() {
    let src = "let a = {1, 2}\nlet b = {2, 3}\nlet c = a | b\n";
    let val = run_get(src, "c");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 3);
    } else {
        panic!("expected Set");
    }
}

/// set_intersection のテスト。
#[test]
fn test_set_intersection() {
    let src = "let a = {1, 2, 3}\nlet b = {2, 3, 4}\nlet c = a & b\n";
    let val = run_get(src, "c");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_difference のテスト。
#[test]
fn test_set_difference() {
    let src = "let a = {1, 2, 3}\nlet b = {2, 3}\nlet c = a - b\n";
    let val = run_get(src, "c");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 1);
        assert!(matches!(items.borrow()[0], Value::Int(1)));
    } else {
        panic!("expected Set");
    }
}

/// set_symmetric_difference のテスト。
#[test]
fn test_set_symmetric_difference() {
    let src = "let a = {1, 2, 3}\nlet b = {2, 3, 4}\nlet c = a ^ b\n";
    let val = run_get(src, "c");
    if let Value::Set(items) = val {
        assert_eq!(items.borrow().len(), 2);
    } else {
        panic!("expected Set");
    }
}

/// set_equality のテスト。
#[test]
fn test_set_equality() {
    let val = run_get("let r = {1, 2, 3} == {3, 1, 2}\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_equality_false のテスト。
#[test]
fn test_set_equality_false() {
    let val = run_get("let r = {1, 2} == {1, 2, 3}\n", "r");
    assert!(matches!(val, Value::Bool(false)));
}

/// set_issubset のテスト。
#[test]
fn test_set_issubset() {
    let src = "let a = {1, 2}\nlet b = {1, 2, 3}\nlet r = a.issubset(b)\n";
    let val = run_get(src, "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_issuperset のテスト。
#[test]
fn test_set_issuperset() {
    let src = "let a = {1, 2, 3}\nlet b = {1, 2}\nlet r = a.issuperset(b)\n";
    let val = run_get(src, "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_iteration のテスト。
#[test]
fn test_set_iteration() {
    let src = concat!(
        "mut total = 0\n",
        "for x in {1, 2, 3}:\n",
        "    total = total + x\n",
    );
    let val = run_get(src, "total");
    assert!(matches!(val, Value::Int(6)));
}

/// set_bool_truthy のテスト。
#[test]
fn test_set_bool_truthy() {
    let val = run_get("let r = bool({1})\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// set_bool_falsy のテスト。
#[test]
fn test_set_bool_falsy() {
    let val = run_get("let r = bool(set())\n", "r");
    assert!(matches!(val, Value::Bool(false)));
}

/// list_in_operator のテスト。
#[test]
fn test_list_in_operator() {
    let val = run_get("let r = 2 in [1, 2, 3]\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// str_in_operator のテスト。
#[test]
fn test_str_in_operator() {
    let val = run_get("let r = \"bc\" in \"abcd\"\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// dict_in_operator のテスト。
#[test]
fn test_dict_in_operator() {
    let val = run_get("let r = \"a\" in {\"a\": 1, \"b\": 2}\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

