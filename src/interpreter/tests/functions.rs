// tests/functions.rs — 組み込み関数、関数定義・再帰、および関数オーバーロードのテスト。

use super::*;
use crate::interpreter::*;

// --- builtins ---

/// range_builtin のテスト。
#[test]
fn test_range_builtin() {
    if let Value::List(items) = eval_expr("range(3)") {
        assert_eq!(items.borrow().len(), 3);
    } else {
        panic!();
    }
}

/// len_builtin のテスト。
#[test]
fn test_len_builtin() {
    assert!(matches!(eval_expr("len([1, 2, 3])"), Value::Int(3)));
}

// --- functions ---

/// fn_call_returns_value のテスト。
#[test]
fn test_fn_call_returns_value() {
    let src = "fn add(a: int, b: int) -> int:\n    return a + b\nlet result = add(3, 4)\n";
    if let Value::Int(n) = run_get(src, "result") {
        assert_eq!(n, 7);
    } else {
        panic!();
    }
}

/// fn_no_return_gives_none のテスト。
#[test]
fn test_fn_no_return_gives_none() {
    let src = "fn noop() -> None:\n    pass\nlet r = noop()\n";
    assert!(matches!(run_get(src, "r"), Value::None));
}

/// fn_recursion のテスト。
#[test]
fn test_fn_recursion() {
    let src = "fn fact(n: int) -> int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\nlet r = fact(5)\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 120);
    } else {
        panic!();
    }
}

/// fn_kwarg_call のテスト。
#[test]
fn test_fn_kwarg_call() {
    let src = "fn sub(a: int, b: int) -> int:\n    return a - b\nlet r = sub(b=1, a=10)\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 9);
    } else {
        panic!();
    }
}

/// fn_scope_isolation のテスト。
#[test]
fn test_fn_scope_isolation() {
    let src = "fn f() -> None:\n    let x = 99\nf()\n";
    assert!(run(&format!("{src}print(x)\n")).is_err());
}

// --- overloading ---

/// overload_by_count のテスト。
#[test]
fn test_overload_by_count() {
    // Two overloads differing only in argument count.
    let src = concat!(
        "fn describe(x: int) -> str:\n",
        "    return \"one\"\n",
        "fn describe(x: int, y: int) -> str:\n",
        "    return \"two\"\n",
        "let a = describe(1)\n",
        "let b = describe(1, 2)\n",
    );
    if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
        assert_eq!(a, "one");
        assert_eq!(b, "two");
    } else {
        panic!();
    }
}

/// overload_by_type のテスト。
#[test]
fn test_overload_by_type() {
    // Two overloads with the same argument count but different types.
    let src = concat!(
        "fn process(x: int) -> str:\n",
        "    return \"int\"\n",
        "fn process(x: str) -> str:\n",
        "    return \"str\"\n",
        "let a = process(42)\n",
        "let b = process(\"hello\")\n",
    );
    if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
        assert_eq!(a, "int");
        assert_eq!(b, "str");
    } else {
        panic!();
    }
}

/// overload_three_variants のテスト。
#[test]
fn test_overload_three_variants() {
    let src = concat!(
        "fn show(x: int) -> str:\n",
        "    return \"int\"\n",
        "fn show(x: str) -> str:\n",
        "    return \"str\"\n",
        "fn show(x: bool) -> str:\n",
        "    return \"bool\"\n",
        "let a = show(1)\n",
        "let b = show(\"hi\")\n",
        "let c = show(True)\n",
    );
    if let (Value::Str(a), Value::Str(b), Value::Str(c)) =
        (run_get(src, "a"), run_get(src, "b"), run_get(src, "c"))
    {
        assert_eq!(a, "int");
        assert_eq!(b, "str");
        assert_eq!(c, "bool");
    } else {
        panic!();
    }
}

/// overload_wrong_count_err のテスト。
#[test]
fn test_overload_wrong_count_err() {
    let src = concat!(
        "fn f(x: int) -> None:\n    pass\n",
        "fn f(x: int, y: int) -> None:\n    pass\n",
        "f(1, 2, 3)\n",
    );
    assert!(run(src).is_err());
}

/// overload_method_by_type のテスト。
#[test]
fn test_overload_method_by_type() {
    // Method overloading inside a class.
    let src = concat!(
        "class Printer:\n",
        "    fn print_val(self, x: int) -> str:\n",
        "        return \"int\"\n",
        "    fn print_val(self, x: str) -> str:\n",
        "        return \"str\"\n",
        "let p = Printer()\n",
        "let a = p.print_val(42)\n",
        "let b = p.print_val(\"hi\")\n",
    );
    if let (Value::Str(a), Value::Str(b)) = (run_get(src, "a"), run_get(src, "b")) {
        assert_eq!(a, "int");
        assert_eq!(b, "str");
    } else {
        panic!();
    }
}

/// overload_method_by_count のテスト。
#[test]
fn test_overload_method_by_count() {
    let src = concat!(
        "class Calc:\n",
        "    fn add(self, x: int) -> int:\n",
        "        return x\n",
        "    fn add(self, x: int, y: int) -> int:\n",
        "        return x + y\n",
        "let c = Calc()\n",
        "let a = c.add(5)\n",
        "let b = c.add(3, 4)\n",
    );
    if let (Value::Int(a), Value::Int(b)) = (run_get(src, "a"), run_get(src, "b")) {
        assert_eq!(a, 5);
        assert_eq!(b, 7);
    } else {
        panic!();
    }
}

