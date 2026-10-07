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
        assert_eq!(inst.class.name, "Color");
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

/// メンバーのクラスは enum と同じ名前で、`enum_of` が enum を指す。`x is Color` が真になる。
/// ⚠ 以前は `enum_item_Color` という別の名前で、`x is Color` が偽だった
///   （`implementation_plans/enum_member_type_plan.md`）。
#[test]
fn test_enum_member_class_is_the_enum() {
    let src = "enum Color:\n    Red\nlet x = Color.Red\nlet t = x is Color\n";
    let val = run_get(src, "x");
    if let Value::Instance(inst_rc) = val {
        let inst = inst_rc.borrow();
        assert_eq!(inst.class.name, "Color");
        assert_eq!(inst.class.enum_of.as_deref(), Some("Color"));
    } else {
        panic!("expected Instance");
    }
    assert!(matches!(run_get(src, "t"), Value::Bool(true)));
}

/// enum のメンバーは**値**で、暗黙に `const`。メンバーへの代入はクラスの `const` と同じ
/// `AssignToConst`、メンバーの中身（`Color.Red.value`）の書き換えは `ModifyConst`、
/// 変数に入れた写しの `value` の書き換えは `AssignToImmutableField`。
/// ⚠ 以前は型検査を通り、`Color.Red = ..` は実行時の `TypeError`、`Color.Red.value = 5` は
///   **共有のメンバーそのものを書き換えていた**。
#[test]
fn test_enum_member_is_a_value_statically() {
    use crate::type_check::TypeErrorKind as K;
    let head = "enum Color:\n    Red\n    Green\n";
    for (body, what) in [
        ("Color.Red = Color.Green\n", "assign"),
        ("Color.Red += 1\n", "compound assign"),
    ] {
        let errs = static_errors(&format!("{head}{body}"));
        assert!(
            errs.iter().any(|e| matches!(&e.kind,
                K::AssignToConst { owner, member } if owner == "Color" && member == "Red")),
            "{what}: expected AssignToConst, got: {errs:?}"
        );
    }
    // メンバー（`const`）の中身の書き換えは `ModifyConst`（5-3）。
    let errs = static_errors(&format!("{head}Color.Red.value = 5\n"));
    assert!(
        errs.iter().any(|e| matches!(&e.kind,
            K::ModifyConst { owner, member } if owner == "Color" && member == "Red")),
        "Color.Red.value: expected ModifyConst, got: {errs:?}"
    );
    // 変数に入れた写しの `value` は不変のフィールド（`AssignToImmutableField`）。
    for body in ["mut m = Color.Red\nm.value = 5\n", "mut m = Color.Red\nm.value += 1\n"] {
        let errs = static_errors(&format!("{head}{body}"));
        assert!(
            errs.iter().any(|e| matches!(&e.kind,
                K::AssignToImmutableField { field_name, class_name } if field_name == "value" && class_name == "Color")),
            "{body:?}: expected AssignToImmutableField, got: {errs:?}"
        );
    }
    // 変数の付け替えは値の書き換えではないので通る。
    let errs = static_errors(&format!("{head}mut c = Color.Red\nc = Color.Green\n"));
    assert!(errs.is_empty(), "rebinding a variable must pass: {errs:?}");
}

/// 実行時もメンバーの `value` は書き換えられない（型検査を通らない経路の最後の砦）。
/// ⚠ `run` は型エラーを無視して実行する。
#[test]
fn test_enum_member_value_is_immutable_at_runtime() {
    let head = "enum Color:\n    Red\n    Green\n";
    for body in ["Color.Red.value = 5\n", "mut m = Color.Red\nm.value = 5\n"] {
        assert!(run(&format!("{head}{body}")).is_err(), "{body:?}: the runtime must reject it");
    }
    // 組み込みの enum も同じ。
    assert!(run("FileOpenMode.read.value = 9\n").is_err(), "built-in enum member must be immutable");
    // 負の対照: 読むだけなら通る。
    assert_int(run_get(&format!("{head}let v = Color.Green.value\n"), "v"), 1);
}

// --- default parameters ---

/// default_param_uses_default_when_omitted のテスト。
#[test]
fn test_default_param_uses_default_when_omitted() {
    let src = "fn greet(let name: str = \"world\") -> str:\n    return name\nlet a = greet()\nlet b = greet(\"Alice\")\n";
    assert!(matches!(run_get(src, "a"), Value::Str(s) if &*s == "world"));
    assert!(matches!(run_get(src, "b"), Value::Str(s) if &*s == "Alice"));
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

