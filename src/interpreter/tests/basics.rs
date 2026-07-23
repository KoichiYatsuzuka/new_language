// tests/basics.rs — 算術・比較・論理演算、変数宣言(let/mut)、複合代入、print、ゼロ除算の基本テスト。

use super::*;

#[test]
fn test_arithmetic() {
    assert!(matches!(eval_expr("2 + 3"), Value::Int(5)));
    assert!(matches!(eval_expr("10 - 4"), Value::Int(6)));
    assert!(matches!(eval_expr("3 * 4"), Value::Int(12)));
    assert!(matches!(eval_expr("7 // 2"), Value::Int(3)));
    assert!(matches!(eval_expr("7 % 3"), Value::Int(1)));
    assert!(matches!(eval_expr("2 ** 10"), Value::Int(1024)));
}

/// float_arithmetic のテスト。
#[test]
fn test_float_arithmetic() {
    if let Value::Float(f) = eval_expr("1.0 + 2.0") {
        assert!((f - 3.0).abs() < f64::EPSILON);
    } else {
        panic!();
    }
}

/// string_concat のテスト。
#[test]
fn test_string_concat() {
    if let Value::Str(s) = eval_expr(r#""hello" + " " + "world""#) {
        assert_eq!(s, "hello world");
    } else {
        panic!();
    }
}

/// comparison のテスト。
#[test]
fn test_comparison() {
    assert!(matches!(eval_expr("1 < 2"), Value::Bool(true)));
    assert!(matches!(eval_expr("2 > 3"), Value::Bool(false)));
    assert!(matches!(eval_expr("4 == 4"), Value::Bool(true)));
    assert!(matches!(eval_expr("4 != 5"), Value::Bool(true)));
}

/// logical のテスト。
#[test]
fn test_logical() {
    assert!(matches!(eval_expr("True and False"), Value::Bool(false)));
    assert!(matches!(eval_expr("True or False"), Value::Bool(true)));
    assert!(matches!(eval_expr("not True"), Value::Bool(false)));
}

/// let_immutable のテスト。
#[test]
fn test_let_immutable() {
    assert!(run("let x = 1\nx = 2").is_err());
}

/// let_redeclaration_same_scope のテスト。
#[test]
fn test_let_redeclaration_same_scope() {
    let err = run("let a = 5\nlet a = 6\n").expect_err("redeclaration should error");
    assert!(err.contains("already declared"), "got: {err}");
}

/// mut_redeclaration_same_scope のテスト。
#[test]
fn test_mut_redeclaration_same_scope() {
    assert!(run("mut a = 5\nmut a = 6\n").is_err());
}

/// let_then_mut_redeclaration のテスト。
#[test]
fn test_let_then_mut_redeclaration() {
    assert!(run("let a = 5\nmut a = 6\n").is_err());
}

/// redeclaration_in_inner_scope のテスト（外側スコープの変数と同名）。
#[test]
fn test_redeclaration_in_inner_scope() {
    assert!(run("let x = 1\nif True:\n    let x = 2\n").is_err());
}

/// underscore_redeclaration_allowed のテスト（_ は再宣言を許可）。
#[test]
fn test_underscore_redeclaration_allowed() {
    assert!(run("let _ = 1\nlet _ = 2\n").is_ok());
}

/// redeclaration_error_message のテスト（エラーメッセージに変数名が含まれる）。
#[test]
fn test_redeclaration_error_message() {
    let err = run("let foo = 1\nlet foo = 2\n").expect_err("should error");
    assert!(err.contains("foo"), "error should mention variable name, got: {err}");
}

/// mut_mutable のテスト。
#[test]
fn test_mut_mutable() {
    assert!(run("mut x = 1\nx = 2").is_ok());
}

/// compound_assign のテスト。
#[test]
fn test_compound_assign() {
    if let Value::Int(n) = run_get("mut x = 10\nx += 5", "x") {
        assert_eq!(n, 15);
    } else {
        panic!();
    }
}

/// print_runs のテスト。
#[test]
fn test_print_runs() {
    assert!(run(r#"print("hello", "world")"#).is_ok());
}

/// zero_division のテスト。
#[test]
fn test_zero_division() {
    assert!(run("1 // 0").is_err());
}


