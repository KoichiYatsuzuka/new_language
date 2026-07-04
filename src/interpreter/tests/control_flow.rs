// tests/control_flow.rs — if/while/for/block 文の制御フローとスコープ隔離のテスト。

use super::*;
use crate::interpreter::*;

// --- if ---

/// if_true_branch のテスト。
#[test]
fn test_if_true_branch() {
    if let Value::Int(n) = run_get("mut x = 0\nif True:\n    x = 1\n", "x") {
        assert_eq!(n, 1);
    } else {
        panic!();
    }
}

/// if_false_else_branch のテスト。
#[test]
fn test_if_false_else_branch() {
    if let Value::Int(n) = run_get("mut x = 0\nif False:\n    x = 1\nelse:\n    x = 2\n", "x") {
        assert_eq!(n, 2);
    } else {
        panic!();
    }
}

/// if_scope_isolation のテスト。
#[test]
fn test_if_scope_isolation() {
    assert!(run("if True:\n    let x = 1\nprint(x)\n").is_err());
}

// --- while ---

/// while_loop のテスト。
#[test]
fn test_while_loop() {
    if let Value::Int(n) = run_get("mut i = 0\nwhile i < 5:\n    i += 1\n", "i") {
        assert_eq!(n, 5);
    } else {
        panic!();
    }
}

/// while_break のテスト。
#[test]
fn test_while_break() {
    if let Value::Int(n) = run_get(
        "mut i = 0\nwhile True:\n    i += 1\n    if i == 3:\n        break\n",
        "i",
    ) {
        assert_eq!(n, 3);
    } else {
        panic!();
    }
}

/// while_scope_isolation のテスト。
#[test]
fn test_while_scope_isolation() {
    assert!(
        run("mut cond = True\nwhile cond:\n    let x = 1\n    cond = False\nprint(x)\n").is_err()
    );
}

// --- for ---

/// for_range のテスト。
#[test]
fn test_for_range() {
    if let Value::Int(n) = run_get("mut s = 0\nfor i in range(5):\n    s += i\n", "s") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

/// for_list のテスト。
#[test]
fn test_for_list() {
    if let Value::Int(n) = run_get("mut s = 0\nfor x in [1, 2, 3]:\n    s += x\n", "s") {
        assert_eq!(n, 6);
    } else {
        panic!();
    }
}

/// for_loop_var_scope_isolation のテスト。
#[test]
fn test_for_loop_var_scope_isolation() {
    assert!(run("for i in range(3):\n    pass\nprint(i)\n").is_err());
}

/// for_body_scope_isolation のテスト。
#[test]
fn test_for_body_scope_isolation() {
    assert!(run("for i in range(1):\n    let x = 99\nprint(x)\n").is_err());
}

// --- block ---

/// block_scope_isolation のテスト。
#[test]
fn test_block_scope_isolation() {
    assert!(run("block:\n    let x = 1\nprint(x)\n").is_err());
}

/// block_reads_outer のテスト。
#[test]
fn test_block_reads_outer() {
    assert!(run("let x = 1\nblock:\n    print(x)\n").is_ok());
}

/// block_modifies_outer のテスト。
#[test]
fn test_block_modifies_outer() {
    if let Value::Int(n) = run_get("mut x = 0\nblock:\n    x = 42\n", "x") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

