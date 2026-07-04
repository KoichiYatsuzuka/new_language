// tests/callables.rs — 関数型(function type)、クロージャ、デコレータのテスト。

use super::*;
use crate::interpreter::*;
use crate::lexer::Lexer;
use crate::parser::Parser;

// --- function type ---

/// function_type_call_positional のテスト。
#[test]
fn test_function_type_call_positional() {
    let src = concat!(
        "fn make() -> function[let int]->int:\n",
        "    fn inner(let x: int) -> int:\n",
        "        return x\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f(42)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// function_type_call_named_param のテスト。
#[test]
fn test_function_type_call_named_param() {
    let src = concat!(
        "fn make() -> function{let value:int}->int:\n",
        "    fn inner(let value: int) -> int:\n",
        "        return value\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f(value = 99)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// function_type_chained_call のテスト。
#[test]
fn test_function_type_chained_call() {
    let src = concat!(
        "fn make() -> function[let int]->int:\n",
        "    fn inner(let x: int) -> int:\n",
        "        return x\n",
        "    return inner\n",
        "mut r = make()(7)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 7);
    } else {
        panic!("expected Int(7)");
    }
}

/// function_type_bare_any_call のテスト。
#[test]
fn test_function_type_bare_any_call() {
    // bare `function` type parameter should work with any call.
    let src = concat!(
        "fn apply(let f: function, let x: int) -> int:\n",
        "    return f(x)\n",
        "fn double(let n: int) -> int:\n",
        "    return n * 2\n",
        "let r = apply(double, 5)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!("expected Int(10)");
    }
}

/// function_type_zero_params のテスト。
#[test]
fn test_function_type_zero_params() {
    let src = concat!(
        "fn make() -> function[]->int:\n",
        "    fn inner() -> int:\n",
        "        return 100\n",
        "    return inner\n",
        "let f = make()\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 100);
    } else {
        panic!("expected Int(100)");
    }
}

/// function_type_is_guard のテスト。
#[test]
fn test_function_type_is_guard() {
    // `f is function` should be True for a function value.
    let src = concat!(
        "fn add(let x: int) -> int:\n",
        "    return x + 1\n",
        "let r = add is function\n",
    );
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(b);
    } else {
        panic!("expected Bool(true)");
    }
}

// --- closures ---

/// closure_captures_immutable のテスト。
#[test]
fn test_closure_captures_immutable() {
    // 不変変数のキャプチャ: 定義時の値が内側関数に保持される
    let src = concat!(
        "fn make(let n: int) -> function[]->int:\n",
        "    fn inner() -> int:\n",
        "        return n\n",
        "    return inner\n",
        "let f = make(42)\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// closure_captures_mutable_shared のテスト。
#[test]
fn test_closure_captures_mutable_shared() {
    // 可変変数のキャプチャ: 内側関数が外側スコープの変数を変更できる
    let src = concat!(
        "fn make_counter() -> function[]->int:\n",
        "    mut count = 0\n",
        "    fn inc() -> int:\n",
        "        count += 1\n",
        "        return count\n",
        "    return inc\n",
        "let counter = make_counter()\n",
        "let r1 = counter()\n",
        "let r2 = counter()\n",
        "let r3 = counter()\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

/// closure_each_call_new_env のテスト。
#[test]
fn test_closure_each_call_new_env() {
    // 呼び出しごとに独立したクロージャ環境が生成される
    let src = concat!(
        "fn make(let start: int) -> function[]->int:\n",
        "    mut n = start\n",
        "    fn inc() -> int:\n",
        "        n += 1\n",
        "        return n\n",
        "    return inc\n",
        "let a = make(0)\n",
        "let b = make(100)\n",
        "let r_a = a()\n",
        "let r_b = b()\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    assert!(matches!(interp.get_val("r_a").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r_b").unwrap(), Value::Int(101)));
}

/// closure_inner_called_from_outer のテスト。
#[test]
fn test_closure_inner_called_from_outer() {
    // 内側関数が外側関数の実行中に呼ばれ、変更が外側に反映される
    let src = concat!(
        "fn outer() -> int:\n",
        "    mut x = 0\n",
        "    fn inc() -> int:\n",
        "        x += 1\n",
        "        return x\n",
        "    inc()\n",
        "    inc()\n",
        "    return x\n",
        "let r = outer()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 2);
    } else {
        panic!("expected Int(2)");
    }
}

/// closure_static_shared_across_calls のテスト。
#[test]
fn test_closure_static_shared_across_calls() {
    // static mut 変数: 複数の呼び出しで同じセルを共有する
    let src = concat!(
        "fn make_counter() -> function[]->int:\n",
        "    static mut count = 0\n",
        "    fn inc() -> int:\n",
        "        count += 1\n",
        "        return count\n",
        "    return inc\n",
        // make_counter() を2回呼ぶ → 両方とも同じ count セルを共有する
        "let a = make_counter()\n",
        "let b = make_counter()\n",
        "let r1 = a()\n",
        "let r2 = b()\n",
        "let r3 = a()\n",
    );
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

/// closure_freeze_captured_var_error のテスト。
#[test]
fn test_closure_freeze_captured_var_error() {
    // クロージャにキャプチャされた可変変数は freeze できない
    let src = concat!(
        "fn outer() -> None:\n",
        "    mut x = 0\n",
        "    fn inner() -> None:\n",
        "        x += 1\n",
        "    freeze x\n",
        "outer()\n",
    );
    assert!(run(src).is_err());
}

/// closure_nested のテスト。
#[test]
fn test_closure_nested() {
    // 二重ネストしたクロージャ
    let src = concat!(
        "fn outer(let a: int) -> function[]->function[]->int:\n",
        "    fn middle(let b: int) -> function[]->int:\n",
        "        fn inner() -> int:\n",
        "            return a + b\n",
        "        return inner\n",
        "    return middle\n",
        "let f = outer(10)(20)\n",
        "let r = f()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 30);
    } else {
        panic!("expected Int(30)");
    }
}

// --- Decorator ---

/// decorator_fn_basic のテスト。
#[test]
fn test_decorator_fn_basic() {
    // 関数デコレータ: @log で包まれた関数を呼ぶと wrapper が実行される
    let src = concat!(
        "fn log(let f: function) -> function:\n",
        "    fn wrapper() -> int:\n",
        "        return 99\n",
        "    return wrapper\n",
        "@log\n",
        "fn original() -> int:\n",
        "    return 1\n",
        "let r = original()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// decorator_fn_passes_original のテスト。
#[test]
fn test_decorator_fn_passes_original() {
    // デコレータは元の関数を受け取ってラップできる
    let src = concat!(
        "fn identity(let f: function) -> function:\n",
        "    return f\n",
        "@identity\n",
        "fn add(let x: int) -> int:\n",
        "    return x + 10\n",
        "let r = add(5)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 15);
    } else {
        panic!("expected Int(15)");
    }
}

/// decorator_stacked のテスト。
#[test]
fn test_decorator_stacked() {
    // スタックされたデコレータは下から順に適用される
    let src = concat!(
        "fn add1(let f: function) -> function:\n",
        "    fn wrapper() -> int:\n",
        "        return f() + 1\n",
        "    return wrapper\n",
        "@add1\n",
        "@add1\n",
        "fn base() -> int:\n",
        "    return 10\n",
        "let r = base()\n",
    );
    // add1 applied to base first → 11, then add1 again → 12
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 12);
    } else {
        panic!("expected Int(12)");
    }
}

/// decorator_class_as_decorator_for_fn のテスト。
#[test]
fn test_decorator_class_as_decorator_for_fn() {
    // クラスデコレータ（関数に適用）
    let src = concat!(
        "class Wrap:\n",
        "    mut inner: function\n",
        "    fn __init__(mut self, let f: function) -> None:\n",
        "        self.inner = f\n",
        "    fn __call__(self) -> function:\n",
        "        let fn_copy = self.inner\n",
        "        fn wrapper() -> int:\n",
        "            return fn_copy() + 100\n",
        "        return wrapper\n",
        "@Wrap\n",
        "fn base() -> int:\n",
        "    return 7\n",
        "let wrapped = base()\n",
        "let r = wrapped()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 107);
    } else {
        panic!("expected Int(107)");
    }
}

/// decorator_instance_callable のテスト。
#[test]
fn test_decorator_instance_callable() {
    // Value::Instance が __call__ を持つ場合に関数として呼び出せる
    let src = concat!(
        "class Adder:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "    fn __call__(self) -> int:\n",
        "        return self.n + 1\n",
        "let a = Adder(41)\n",
        "let r = a()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// ar_to_py_dict のテスト。
#[test]
fn test_ar_to_py_dict() {
    // Value::Dict を Python に渡せることを確認する (sum_dict はすべての int 値を合計する)
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let d = {\"x\": 10, \"y\": 20, \"z\": 12}\n",
        "let r = calc.sum_dict(d)\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// ar_to_py_tuple のテスト。
#[test]
fn test_ar_to_py_tuple() {
    // Value::Tuple を Python に渡せることを確認する (first_of_tuple は先頭要素を返す)
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let t = (99, 1, 2)\n",
        "let r = calc.first_of_tuple(t)\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

