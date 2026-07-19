// tests/enum_defaults.rs — enum とデフォルト引数のテスト。

use super::*;

// --- enum ---

/// enum_basic のテスト。
#[test]
fn test_enum_basic() {
    let src = "enum Color:\n    Red\n    Green\n    Blue\n";
    run(src).unwrap();
}

/// enum_member_access_value のテスト。
#[test]
fn test_enum_member_access_value() {
    let src = "enum Color:\n    Red\n    Green\n    Blue\nlet x = Color.Red\n";
    let val = run_get(src, "x");
    if let Value::Instance(inst_rc) = val {
        let inst = inst_rc.borrow();
        assert_eq!(inst.class.name, "enum_item_Color");
        let &idx = inst.class.field_index.get("value").unwrap();
        let v = inst.field_value(idx).unwrap();
        assert!(matches!(v, Value::Int(0)));
    } else {
        panic!("expected Instance");
    }
}

/// enum_auto_numbering のテスト。
#[test]
fn test_enum_auto_numbering() {
    // Red=0, Green=1, Blue=2 の順で自動採番される
    let src = "enum Color:\n    Red\n    Green\n    Blue\nlet r = Color.Red\nlet g = Color.Green\nlet b = Color.Blue\n";
    for (var, expected) in [("r", 0i64), ("g", 1), ("b", 2)] {
        let val = run_get(src, var);
        if let Value::Instance(inst_rc) = val {
            let inst = inst_rc.borrow();
            let &idx = inst.class.field_index.get("value").unwrap();
            let v = inst.field_value(idx).unwrap();
            if let Value::Int(n) = v {
                assert_eq!(n, expected);
            } else {
                panic!("expected Int");
            }
        } else {
            panic!("expected Instance for {var}");
        }
    }
}

/// enum_explicit_value のテスト。
#[test]
fn test_enum_explicit_value() {
    let src = "enum MyEnum:\n    a\n    b = 5\n    c\nlet xb = MyEnum.b\nlet xc = MyEnum.c\n";
    let b = run_get(src, "xb");
    let c = run_get(src, "xc");
    if let Value::Instance(inst_rc) = b {
        let inst = inst_rc.borrow();
        let &idx = inst.class.field_index.get("value").unwrap();
        let v = inst.field_value(idx).unwrap();
        if let Value::Int(n) = v {
            assert_eq!(n, 5);
        } else {
            panic!("expected Int 5");
        }
    } else {
        panic!("expected Instance for b");
    }
    // c は b=5 の次なので 6
    if let Value::Instance(inst_rc) = c {
        let inst = inst_rc.borrow();
        let &idx = inst.class.field_index.get("value").unwrap();
        let v = inst.field_value(idx).unwrap();
        if let Value::Int(n) = v {
            assert_eq!(n, 6);
        } else {
            panic!("expected Int 6");
        }
    } else {
        panic!("expected Instance for c");
    }
}

/// enum_equality のテスト。
#[test]
fn test_enum_equality() {
    // 同じバリアントに2回アクセスしたとき等値になること（Rc::ptr_eq）
    let src = "enum Color:\n    Red\n    Green\nlet a = Color.Red\nlet b = Color.Red\nlet c = Color.Green\nmut same = False\nmut diff = False\nif a == b:\n    same = True\nif a != c:\n    diff = True\n";
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
    assert!(matches!(run_get(src, "diff"), Value::Bool(true)));
}

/// enum_match のテスト。
#[test]
fn test_enum_match() {
    let src = r#"
enum Color:
    Red
    Green
    Blue
let x = Color.Green
mut result = 0
match (x):
    case Color.Red:
        result = 1
    case Color.Green:
        result = 2
    case Color.Blue:
        result = 3
"#;
    assert_int(run_get(src, "result"), 2);
}

/// enum_item_type_name のテスト。
#[test]
fn test_enum_item_type_name() {
    // enum_item_Color 型が登録されていること
    let src = "enum Color:\n    Red\nlet x = Color.Red\n";
    let val = run_get(src, "x");
    if let Value::Instance(inst_rc) = val {
        assert_eq!(inst_rc.borrow().class.name, "enum_item_Color");
    } else {
        panic!("expected Instance");
    }
}

// --- default parameters ---

/// default_param_uses_default_when_omitted のテスト。
#[test]
fn test_default_param_uses_default_when_omitted() {
    let src = "fn greet(let name: str = \"world\") -> str:\n    return name\nlet a = greet()\nlet b = greet(\"Alice\")\n";
    assert!(matches!(run_get(src, "a"), Value::Str(s) if s == "world"));
    assert!(matches!(run_get(src, "b"), Value::Str(s) if s == "Alice"));
}

/// default_param_multiple_defaults のテスト。
#[test]
fn test_default_param_multiple_defaults() {
    let src = "fn add(let a: int = 1, let b: int = 2) -> int:\n    return a + b\nlet r1 = add()\nlet r2 = add(10)\nlet r3 = add(10, 20)\n";
    assert_int(run_get(src, "r1"), 3);
    assert_int(run_get(src, "r2"), 12);
    assert_int(run_get(src, "r3"), 30);
}

/// default_param_mixed_required_and_default のテスト。
#[test]
fn test_default_param_mixed_required_and_default() {
    let src = "fn f(let x: int, let y: int = 99) -> int:\n    return x + y\nlet a = f(1)\nlet b = f(1, 2)\n";
    assert_int(run_get(src, "a"), 100);
    assert_int(run_get(src, "b"), 3);
}

/// default_param_via_keyword_arg のテスト。
#[test]
fn test_default_param_via_keyword_arg() {
    let src =
        "fn f(let a: int = 0, let b: int = 0) -> int:\n    return a * 10 + b\nlet r = f(b=5)\n";
    assert_int(run_get(src, "r"), 5);
}

/// default_param_ordering_error のテスト。
#[test]
fn test_default_param_ordering_error() {
    let src = "fn f(let a: int = 0, let b: int) -> int:\n    return 0\n";
    assert!(
        run(src).is_err(),
        "expected ParseError for non-default after default"
    );
}

/// default_param_too_many_args_error のテスト。
#[test]
fn test_default_param_too_many_args_error() {
    let src = "fn f(let x: int = 0) -> int:\n    return x\nlet r = f(1, 2)\n";
    assert!(run(src).is_err(), "expected TypeError for too many args");
}

