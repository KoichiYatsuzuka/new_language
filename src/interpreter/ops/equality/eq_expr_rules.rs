use crate::interpreter::{Interpreter, Value};

/// ⚠ **実際の経路をそのまま呼ぶ**（規則を書き写さない）。`Interpreter::new()` は
/// async のワーカーが 1 スレッドに 1 つ建てているので、テストで建てても軽い。
fn eq(a: Value, b: Value) -> bool {
    let interp = Interpreter::new();
    interp.values_eq_expr(&a, &b).expect("no recursion involved")
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

/// 異型の比較は**実行時エラーにならず `False`** を返す。
#[test]
fn cross_type_is_false_not_an_error() {
    assert!(!eq(Value::Int(1), s("a")));
    assert!(!eq(s("a"), Value::Int(1)));
    assert!(!eq(Value::Int(1), Value::None));
    assert!(!eq(Value::Bool(true), s("x")));
}

/// 数値族（`int` / `uint` / `float`）は**昇格ラティスを通して**比べる。
#[test]
fn numeric_family_is_promoted() {
    assert!(eq(Value::Int(1), Value::Float(1.0)));
    assert!(eq(Value::Float(1.0), Value::Int(1)));
    assert!(eq(Value::Int(1), Value::UInt(1)));
    assert!(eq(Value::UInt(1), Value::Float(1.0)));
    assert!(!eq(Value::Int(1), Value::Float(1.5)));
}

/// ⚠ `bool` は数値の昇格ラティスの**対象外**（`True == 1` は `False`）。
#[test]
fn bool_is_outside_the_numeric_lattice() {
    assert!(!eq(Value::Bool(true), Value::Int(1)));
    assert!(!eq(Value::Int(1), Value::Bool(true)));
}

/// ⚠ 昇格は**比較のオペランドにだけ**掛かり、コンテナの中までは再帰しない。
/// `1 == 1.0` は `True` だが `[1] == [1.0]` は `False`。
#[test]
fn promotion_does_not_recurse_into_containers() {
    use std::{cell::RefCell, rc::Rc};
    let a = Value::List(Rc::new(RefCell::new(vec![Value::Int(1)])));
    let b = Value::List(Rc::new(RefCell::new(vec![Value::Float(1.0)])));
    assert!(eq(Value::Int(1), Value::Float(1.0)));
    assert!(!eq(a, b));
}
