// tests/pyobject.rs — PyObject を対象とした for ループ反復と二項演算子のテスト。

use super::*;
use crate::interpreter::*;

// ---------------------------------------------------------------------------
// for ループ: PyObject の反復
// ---------------------------------------------------------------------------

/// pyobject_for_loop のテスト。
#[test]
fn test_pyobject_for_loop() {
    // Container は Python iterable; for ループで各要素を取得できる
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([10, 20, 30])\n",
        "mut total = 0\n",
        "for x in c:\n",
        "    total += x\n",
    );
    assert!(matches!(run_py_get(src, "total"), Value::Int(60)));
}

// ---------------------------------------------------------------------------
// 二項演算子: PyObject オペランド
// ---------------------------------------------------------------------------

/// pyobject_binop_mul_lhs のテスト。
#[test]
fn test_pyobject_binop_mul_lhs() {
    // lhs = PyObject: c * 3 → Container.__mul__(3) → 要素数 6 の Container
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([1, 2])\n",
        "let c2 = c * 3\n",
        "let r = len(c2)\n",
    );
    assert!(matches!(run_py_get(src, "r"), Value::Int(6)));
}

/// pyobject_binop_mul_rhs のテスト。
#[test]
fn test_pyobject_binop_mul_rhs() {
    // rhs = PyObject: 3 * c → Container.__rmul__(3) → 要素数 6 の Container
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([1, 2])\n",
        "let c2 = 3 * c\n",
        "let r = len(c2)\n",
    );
    assert!(matches!(run_py_get(src, "r"), Value::Int(6)));
}

/// pyobject_binop_add のテスト。
#[test]
fn test_pyobject_binop_add() {
    // c1 + c2 → Container.__add__ → 要素が結合された Container
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let a = calc.make_container([1, 2])\n",
        "let b = calc.make_container([3, 4, 5])\n",
        "let c = a + b\n",
        "let r = len(c)\n",
    );
    assert!(matches!(run_py_get(src, "r"), Value::Int(5)));
}

// ---------------------------------------------------------------------------
// block expression tests (block_return / block_yield)
// ---------------------------------------------------------------------------

/// block_expr_block_return のテスト。
#[test]
fn test_block_expr_block_return() {
    let src = "
let x = block:
    block_return 42
";
    assert!(matches!(run_get(src, "x"), Value::Int(42)));
}

/// block_expr_block_return_early_exit のテスト。
#[test]
fn test_block_expr_block_return_early_exit() {
    let src = "
let x = block:
    block_return 1
    block_return 2
";
    // first block_return wins
    assert!(matches!(run_get(src, "x"), Value::Int(1)));
}

/// block_expr_no_block_return_gives_none のテスト。
#[test]
fn test_block_expr_no_block_return_gives_none() {
    let src = "
let x = block:
    mut a = 10
    mut b = 20
";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// block_expr_computed_value のテスト。
#[test]
fn test_block_expr_computed_value() {
    let src = "
let n = 6
let result = block:
    let doubled = n * 2
    block_return doubled + 1
";
    assert!(matches!(run_get(src, "result"), Value::Int(13)));
}

/// block_expr_conditional_return のテスト。
#[test]
fn test_block_expr_conditional_return() {
    let src = "
fn classify(x: int) -> str:
    return block:
        if x > 0:
            block_return \"positive\"
        elif x < 0:
            block_return \"negative\"
        else:
            block_return \"zero\"
let a = classify(5)
let b = classify(-3)
let c = classify(0)
";
    assert!(matches!(run_get(src, "a"), Value::Str(ref s) if s == "positive"));
    assert!(matches!(run_get(src, "b"), Value::Str(ref s) if s == "negative"));
    assert!(matches!(run_get(src, "c"), Value::Str(ref s) if s == "zero"));
}

/// loop_yield_list_from_for_expr のテスト。
#[test]
fn test_loop_yield_list_from_for_expr() {
    // loop_yield accumulates values from a for expression
    let src = "
let items = for i in range(1, 4) ->list[int]:
    loop_yield i
";
    if let Value::List(lst) = run_get(src, "items") {
        let borrow = lst.borrow();
        assert_eq!(borrow.len(), 3);
        assert!(matches!(borrow[0], Value::Int(1)));
        assert!(matches!(borrow[1], Value::Int(2)));
        assert!(matches!(borrow[2], Value::Int(3)));
    } else {
        panic!("expected list");
    }
}

/// loop_yield_in_nested_if_inside_for_expr のテスト。
#[test]
fn test_loop_yield_in_nested_if_inside_for_expr() {
    // loop_yield inside an if that's inside a for expression
    let src = "
let evens = for i in range(6) ->list[int]:
    if i % 2 == 0:
        loop_yield i
";
    if let Value::List(lst) = run_get(src, "evens") {
        let borrow = lst.borrow();
        assert_eq!(borrow.len(), 3);
        assert!(matches!(borrow[0], Value::Int(0)));
        assert!(matches!(borrow[1], Value::Int(2)));
        assert!(matches!(borrow[2], Value::Int(4)));
    } else {
        panic!("expected list");
    }
}

/// loop_yield_outside_for_while_expr_is_error のテスト。
#[test]
fn test_loop_yield_outside_for_while_expr_is_error() {
    // loop_yield inside block: (not a for/while expression) is a runtime error
    let src = "
mut x = 0
block:
    loop_yield 999
    x = 1
";
    assert!(run(src).is_err());
}

/// break_outside_loop_is_error のテスト。
#[test]
fn test_break_outside_loop_is_error() {
    // break outside any for/while loop is a runtime error
    let src = "
fn bad():
    break
bad()
";
    assert!(run(src).is_err());
}

/// block_return_outside_block_expr_is_error のテスト。
#[test]
fn test_block_return_outside_block_expr_is_error() {
    // block_return outside any block: expression should be a SyntaxError
    let src = "
fn bad() -> int:
    block_return 42
    return 0
bad()
";
    assert!(run(src).is_err());
}

