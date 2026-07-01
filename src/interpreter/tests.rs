// tests.rs — インタープリタ単体テスト

use super::*;
use crate::ast::Stmt;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// テストソースを字句解析・構文解析・実行する。エラーがあれば `Err` を返す。
fn run(src: &str) -> Result<(), String> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program()?;
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt)?;
    }
    Ok(())
}

/// 単一の式文を評価して `Value` を返すテストヘルパー。
fn eval_expr(src: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    interp
        .eval(match &stmts[0] {
            Stmt::Expr(e) => e,
            _ => panic!("not an expr"),
        })
        .unwrap()
}

/// テストソースを実行して変数 `var` の値を返すテストヘルパー。
fn run_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// py-int テスト用: examples/ ディレクトリを Python 検索パスに追加して実行する
fn run_py_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    interp.add_python_search_dir(std::path::PathBuf::from("examples"));
    interp.add_python_search_dir(std::path::PathBuf::from("examples/test_modules"));
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// テストソースを実行し、最初の `raise` で発生した例外を返すテストヘルパー。例外がなければ `Ok(None)`。
fn run_exc(src: &str) -> Result<Option<RaisedError>, String> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program()?;
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        match interp.exec(stmt) {
            Ok(ExecResult::Raise(raised)) => return Ok(Some(raised)),
            Ok(_) => {}
            Err(e) if e == RAISE_SENTINEL => return Ok(interp.take_current_exception()),
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// arithmetic のテスト。
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

// --- classes ---

/// class_instantiate のテスト。
#[test]
fn test_class_instantiate() {
    // Fields have defaults → no required args → Point() is the right call.
    let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\nlet p = Point()\n";
    assert!(run(src).is_ok());
}

/// class_instantiate_required_fields のテスト。
#[test]
fn test_class_instantiate_required_fields() {
    // Fields without defaults → auto-init requires args.
    let src = "class Point:\n    mut x: int\n    mut y: int\nlet p = Point(3, 4)\n";
    assert!(run(src).is_ok());
}

/// class_init_sets_field のテスト。
#[test]
fn test_class_init_sets_field() {
    let src = "class Dog:\n    mut name: str = \"\"\n    fn __init__(mut self, name: str) -> None:\n        self.name = name\nlet d = Dog(\"Rex\")\n";
    assert!(run(src).is_ok());
}

/// class_method_call のテスト。
#[test]
fn test_class_method_call() {
    let src = "class Greeter:\n    fn greet(self) -> str:\n        return \"hello\"\nlet g = Greeter()\nlet r = g.greet()\n";
    if let Value::Str(s) = run_get(src, "r") {
        assert_eq!(s, "hello");
    } else {
        panic!();
    }
}

/// class_field_access のテスト。
#[test]
fn test_class_field_access() {
    // Fields have defaults; use defaults when instantiating.
    let src =
        "class Pair:\n    mut x: int = 10\n    mut y: int = 20\nlet p = Pair()\nlet r = p.x\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

/// class_field_access_required のテスト。
#[test]
fn test_class_field_access_required() {
    // Fields without defaults require constructor args.
    let src = "class Pair:\n    mut x: int\n    mut y: int\nlet p = Pair(10, 20)\nlet r = p.x\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

/// access_public_field_ok のテスト。
#[test]
fn test_access_public_field_ok() {
    let src = concat!(
        "class C:\n",
        "    public:\n",
        "    mut x: int = 42\n",
        "let obj = C()\n",
        "let r = obj.x\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

/// access_private_field_from_outside_errors のテスト。
#[test]
fn test_access_private_field_from_outside_errors() {
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut secret: int = 99\n",
        "let obj = C()\n",
        "let r = obj.secret\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
    assert!(msg.contains("private"), "expected 'private', got: {msg}");
}

/// access_private_field_from_method_ok のテスト。
#[test]
fn test_access_private_field_from_method_ok() {
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut secret: int = 99\n",
        "    fn get_secret(self) -> int:\n",
        "        return self.secret\n",
        "let obj = C()\n",
        "let r = obj.get_secret()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!();
    }
}

/// access_protected_field_from_outside_errors のテスト。
#[test]
fn test_access_protected_field_from_outside_errors() {
    let src = concat!(
        "trait Guarding:\n",
        "    protected:\n",
        "    mut guarded: int\n",
        "class C(Guarding):\n",
        "    pass\n",
        "let obj = C(7)\n",
        "let r = obj.guarded\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
    assert!(
        msg.contains("protected"),
        "expected 'protected', got: {msg}"
    );
}

/// access_protected_field_via_method_ok のテスト。
#[test]
fn test_access_protected_field_via_method_ok() {
    let src = concat!(
        "trait Guarding:\n",
        "    protected:\n",
        "    mut guarded: int\n",
        "class C(Guarding):\n",
        "    fn get_guarded(self) -> int:\n",
        "        return self.guarded\n",
        "let obj = C(7)\n",
        "let r = obj.get_guarded()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 7);
    } else {
        panic!();
    }
}

/// access_mixed_sections_in_class のテスト。
#[test]
fn test_access_mixed_sections_in_class() {
    let src = concat!(
        "class C:\n",
        "    public:\n",
        "    mut visible: int = 1\n",
        "    private:\n",
        "    mut hidden: int = 2\n",
        "    fn get_hidden(self) -> int:\n",
        "        return self.hidden\n",
        "let obj = C()\n",
        "let pub_val = obj.visible\n",
        "let priv_val = obj.get_hidden()\n",
    );
    if let (Value::Int(a), Value::Int(b)) = (run_get(src, "pub_val"), run_get(src, "priv_val")) {
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    } else {
        panic!();
    }
}

/// access_private_write_from_outside_errors のテスト。
#[test]
fn test_access_private_write_from_outside_errors() {
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut x: int = 0\n",
        "mut obj = C()\n",
        "obj.x = 5\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
}

/// class_self_field_in_method のテスト。
#[test]
fn test_class_self_field_in_method() {
    let src = concat!(
        "class Box:\n",
        "    mut value: int = 0\n",
        "    fn set(mut self, v: int) -> None:\n",
        "        self.value = v\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "mut b = Box()\n", // mut: instance will be mutated via set()
        "b.set(42)\n",
        "let r = b.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

/// class_inheritance_non_trait_parse_error のテスト。
#[test]
fn test_class_inheritance_non_trait_parse_error() {
    // Class-to-class inheritance is not supported; must use traits instead.
    let src = concat!(
        "class Animal:\n",
        "    fn speak(self) -> str:\n",
        "        return \"...\"\n",
        "class Dog(Animal):\n",
        "    fn speak(self) -> str:\n",
        "        return \"Woof\"\n",
    );
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "expected parse error for class-to-class inheritance"
    );
    assert!(result.unwrap_err().contains("cannot inherit from `Animal`"));
}

/// class_inherit_non_trait_base_parse_error のテスト。
#[test]
fn test_class_inherit_non_trait_base_parse_error() {
    // Class-to-class inheritance is no longer supported; the parser must reject it.
    let src = concat!(
        "class Base:\n",
        "    fn hello(self) -> str:\n",
        "        return \"hi\"\n",
        "class Child(Base):\n",
        "    pass\n",
    );
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "expected parse error for class-to-class inheritance"
    );
    assert!(result.unwrap_err().contains("cannot inherit from `Base`"));
}

// --- trait ---

/// trait_class_instantiate_combined_constructor のテスト。
#[test]
fn test_trait_class_instantiate_combined_constructor() {
    // Class inheriting a trait; combined __init__ takes trait fields then class fields.
    let src = concat!(
        "trait HasValue:\n",
        "    mut value: int\n",
        "class Container(HasValue):\n",
        "    mut tag: str\n",
        "let c = Container(42, \"hello\")\n",
    );
    assert!(run(src).is_ok());
}

/// trait_field_read_via_class_method のテスト。
#[test]
fn test_trait_field_read_via_class_method() {
    // A method defined in the CLASS body reads a trait field via TraitAccess.
    let src = concat!(
        "trait HasValue:\n",
        "    mut value: int\n",
        "class Container(HasValue):\n",
        "    mut tag: str\n",
        "    fn get_value(self) -> int:\n",
        "        return self::HasValue.value\n",
        "    fn get_tag(self) -> str:\n",
        "        return self.tag\n",
        "let c = Container(99, \"hi\")\n",
        "let v = c.get_value()\n",
        "let t = c.get_tag()\n",
    );
    if let Value::Int(n) = run_get(src, "v") {
        assert_eq!(n, 99);
    } else {
        panic!("expected int for v");
    }
    if let Value::Str(s) = run_get(src, "t") {
        assert_eq!(s, "hi");
    } else {
        panic!("expected str for t");
    }
}

/// trait_virtual_override_executes のテスト。
#[test]
fn test_trait_virtual_override_executes() {
    // Virtual method overridden in class; override body actually runs.
    let src = concat!(
        "trait Shape:\n",
        "    fn area(self) -> float:\n",
        "        ...\n",
        "class Square(Shape):\n",
        "    mut side: float\n",
        "    fn area(self) -> float:\n",
        "        return self.side * self.side\n",
        "let s = Square(3.0)\n",
        "let a = s.area()\n",
    );
    if let Value::Float(f) = run_get(src, "a") {
        assert!((f - 9.0).abs() < 1e-9, "expected 9.0, got {f}");
    } else {
        panic!("expected float for a");
    }
}

/// trait_only_required_fields_no_class_fields のテスト。
#[test]
fn test_trait_only_required_fields_no_class_fields() {
    // Class body has no required fields; only the trait's required field.
    let src = concat!(
        "trait Named:\n",
        "    mut name: str\n",
        "class Widget(Named):\n",
        "    fn get_name(self) -> str:\n",
        "        return self::Named.name\n",
        "let w = Widget(\"button\")\n",
        "let n = w.get_name()\n",
    );
    if let Value::Str(s) = run_get(src, "n") {
        assert_eq!(s, "button");
    } else {
        panic!("expected str for n");
    }
}

// --- let-binding immutability for instances ---

/// let_instance_field_is_frozen のテスト。
#[test]
fn test_let_instance_field_is_frozen() {
    // let binding freezes all mut fields — direct field write must fail
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 0\n",
        "let c = Counter()\n",
        "c.value = 1\n",
    );
    assert!(run(src).is_err(), "assigning to a frozen field must fail");
}

/// let_instance_mut_method_forbidden のテスト。
#[test]
fn test_let_instance_mut_method_forbidden() {
    // let binding forbids calling methods with mut self
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 0\n",
        "    fn inc(mut self) -> None:\n",
        "        self.value = self.value + 1\n",
        "let c = Counter()\n",
        "c.inc()\n",
    );
    assert!(
        run(src).is_err(),
        "calling mut self method on let instance must fail"
    );
}

/// let_instance_immutable_method_allowed のテスト。
#[test]
fn test_let_instance_immutable_method_allowed() {
    // let binding still allows calling methods with (non-mut) self
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 5\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "let c = Counter()\n",
        "let r = c.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 5);
    } else {
        panic!("expected int 5");
    }
}

/// mut_instance_mut_method_allowed のテスト。
#[test]
fn test_mut_instance_mut_method_allowed() {
    // mut binding allows calling mut self methods normally
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 0\n",
        "    fn inc(mut self) -> None:\n",
        "        self.value = self.value + 1\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "mut c = Counter()\n",
        "c.inc()\n",
        "c.inc()\n",
        "let r = c.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 2);
    } else {
        panic!("expected int 2");
    }
}

/// let_instance_error_message_names_method のテスト。
#[test]
fn test_let_instance_error_message_names_method() {
    let src = concat!(
        "class Foo:\n",
        "    fn bar(mut self) -> None:\n",
        "        pass\n",
        "let f = Foo()\n",
        "f.bar()\n",
    );
    let err = run(src).expect_err("should fail");
    assert!(
        err.contains("bar"),
        "error should mention method name, got: {err}"
    );
    assert!(
        err.contains("immutable"),
        "error should mention immutable, got: {err}"
    );
}

// --- freeze statement ---

/// freeze_makes_variable_immutable のテスト。
#[test]
fn test_freeze_makes_variable_immutable() {
    // After freeze, reassigning the variable itself must fail
    let src = concat!(
        "class Foo:\n",
        "    mut x: int = 0\n",
        "mut f = Foo()\n",
        "freeze f\n",
        "f = Foo()\n", // reassign the variable
    );
    assert!(run(src).is_err(), "reassigning a frozen variable must fail");
}

/// freeze_freezes_instance_fields のテスト。
#[test]
fn test_freeze_freezes_instance_fields() {
    // After freeze, writing to a mut field must fail
    let src = concat!(
        "class Foo:\n",
        "    mut x: int = 0\n",
        "mut f = Foo()\n",
        "freeze f\n",
        "f.x = 1\n",
    );
    assert!(run(src).is_err(), "writing to a frozen field must fail");
}

/// freeze_forbids_mut_self_methods のテスト。
#[test]
fn test_freeze_forbids_mut_self_methods() {
    // After freeze, calling a mut self method must fail
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 0\n",
        "    fn inc(mut self) -> None:\n",
        "        self.value = self.value + 1\n",
        "mut c = Counter()\n",
        "freeze c\n",
        "c.inc()\n",
    );
    assert!(
        run(src).is_err(),
        "calling mut self method on frozen instance must fail"
    );
}

/// freeze_allows_immutable_methods のテスト。
#[test]
fn test_freeze_allows_immutable_methods() {
    // After freeze, calling a (non-mut) self method must still work
    let src = concat!(
        "class Counter:\n",
        "    mut value: int = 5\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "mut c = Counter()\n",
        "freeze c\n",
        "let r = c.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 5);
    } else {
        panic!("expected int 5");
    }
}

/// freeze_calls_dunder_freeze のテスト。
#[test]
fn test_freeze_calls_dunder_freeze() {
    // freeze must call __freeze__ on the instance before freezing
    let src = concat!(
        "class Tracked:\n",
        "    mut value: int = 0\n",
        "    mut frozen_at: int = 0\n",
        "    fn __freeze__(mut self) -> None:\n",
        "        self.frozen_at = self.value + 10\n",
        "mut t = Tracked()\n",
        "t.value = 3\n",
        "freeze t\n",
        "let r = t.frozen_at\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 13, "expected frozen_at == value + 10 == 13");
    } else {
        panic!("expected int 13");
    }
}

/// freeze_on_let_variable_errors のテスト。
#[test]
fn test_freeze_on_let_variable_errors() {
    // freeze on an already-immutable variable must fail
    let src = concat!(
        "class Foo:\n",
        "    mut x: int = 0\n",
        "let f = Foo()\n",
        "freeze f\n",
    );
    assert!(run(src).is_err(), "freeze on a let variable must fail");
}

/// freeze_on_undefined_variable_errors のテスト。
#[test]
fn test_freeze_on_undefined_variable_errors() {
    assert!(
        run("freeze x\n").is_err(),
        "freeze on undefined variable must fail"
    );
}

// --- Self type ---

/// self_type_as_constructor_in_method のテスト。
#[test]
fn test_self_type_as_constructor_in_method() {
    // Self(...) inside a method creates a new instance of the same class.
    let src = concat!(
        "class Point:\n",
        "    mut x: int\n",
        "    mut y: int\n",
        "    fn zero(self) -> Self:\n",
        "        return Self(0, 0)\n",
        "mut p = Point(3, 4)\n",
        "mut z = p.zero()\n",
        "let r = z.x\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 0, "Self() should construct a new instance");
    } else {
        panic!("expected int 0");
    }
}

/// self_type_in_return_annotation のテスト。
#[test]
fn test_self_type_in_return_annotation() {
    // `-> Self` is a valid return type annotation inside a class method.
    let src = concat!(
        "class Box:\n",
        "    mut value: int\n",
        "    fn copy(self) -> Self:\n",
        "        return Self(self.value)\n",
        "let b = Box(42)\n",
        "let c = b.copy()\n",
        "let r = c.value\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42, "Self() copy should preserve value");
    } else {
        panic!("expected int 42");
    }
}

/// self_type_in_param_annotation のテスト。
#[test]
fn test_self_type_in_param_annotation() {
    // `other: Self` is a valid parameter type annotation inside a class method.
    let src = concat!(
        "class Pair:\n",
        "    mut value: int\n",
        "    fn add(self, other: Self) -> int:\n",
        "        return self.value + other.value\n",
        "let a = Pair(10)\n",
        "let b = Pair(20)\n",
        "let r = a.add(b)\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 30);
    } else {
        panic!("expected int 30");
    }
}

/// self_type_outside_class_is_parse_error のテスト。
#[test]
fn test_self_type_outside_class_is_parse_error() {
    // `Self` used outside a class or trait must produce a parse error.
    let tokens = crate::lexer::Lexer::new("fn foo() -> Self:\n    pass\n", "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "Self outside class/trait must be a parse error"
    );
    assert!(
        result.unwrap_err().contains("'Self'"),
        "error should mention 'Self'"
    );
}

/// self_type_in_expression_outside_class_is_parse_error のテスト。
#[test]
fn test_self_type_in_expression_outside_class_is_parse_error() {
    // `Self` as an expression outside a class must produce a parse error.
    let tokens = crate::lexer::Lexer::new("Self(42)\n", "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "Self expression outside class/trait must be a parse error"
    );
}

/// trait_field_write_via_method のテスト。
#[test]
fn test_trait_field_write_via_method() {
    // A class method writes to a trait field using TraitAccess assignment.
    let src = concat!(
        "trait HasCount:\n",
        "    mut count: int\n",
        "class Counter(HasCount):\n",
        "    fn increment(mut self) -> None:\n",
        "        self::HasCount.count = self::HasCount.count + 1\n",
        "    fn get(self) -> int:\n",
        "        return self::HasCount.count\n",
        "mut c = Counter(0)\n", // mut: instance will be mutated via increment()
        "c.increment()\n",
        "c.increment()\n",
        "let r = c.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 2);
    } else {
        panic!("expected int 2");
    }
}

// --- new_type ---

/// new_type_class_copy_same_behavior のテスト。
#[test]
fn test_new_type_class_copy_same_behavior() {
    // new_type creates a structurally identical class — field access and methods work.
    let src = concat!(
        "class Meters:\n",
        "    mut value: int\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "new_type Kilometers: Meters\n",
        "let km = Kilometers(42)\n",
        "let r = km.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!("expected 42");
    }
}

/// new_type_instances_are_distinct のテスト。
#[test]
fn test_new_type_instances_are_distinct() {
    // new_type gives a different class name — class name of instance is distinct.
    let src = concat!(
        "class Meters:\n",
        "    mut value: int\n",
        "new_type Kilometers: Meters\n",
        "let m = Meters(1)\n",
        "let km = Kilometers(2)\n",
        "let mv = m.value\n",
        "let kmv = km.value\n",
    );
    if let (Value::Int(mv), Value::Int(kmv)) = (run_get(src, "mv"), run_get(src, "kmv")) {
        assert_eq!(mv, 1);
        assert_eq!(kmv, 2);
    } else {
        panic!("expected ints");
    }
}

/// new_type_primitive_wrapper のテスト。
#[test]
fn test_new_type_primitive_wrapper() {
    // new_type from a primitive type creates a wrapper class with .value field.
    let src = concat!(
        "new_type Meters: int\n",
        "let m = Meters(100)\n",
        "let v = m.value\n",
    );
    if let Value::Int(n) = run_get(src, "v") {
        assert_eq!(n, 100);
    } else {
        panic!("expected 100");
    }
}

/// new_type_self_resolves_to_new_type のテスト。
#[test]
fn test_new_type_self_resolves_to_new_type() {
    // When a method inherited via new_type calls Self(...), it creates an instance
    // of the new_type's class, not the original.
    let src = concat!(
        "class Meters:\n",
        "    mut value: int\n",
        "    fn double(self) -> Self:\n",
        "        return Self(self.value * 2)\n",
        "new_type Kilometers: Meters\n",
        "let km = Kilometers(5)\n",
        "let km2 = km.double()\n",
        "let r = km2.value\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!("expected 10");
    }
}

/// new_type_const_is_parse_error のテスト。
#[test]
fn test_new_type_const_is_parse_error() {
    // Reassigning a new_type binding is a parse error.
    let src = concat!(
        "class Foo:\n",
        "    mut x: int\n",
        "new_type Bar: Foo\n",
        "Bar = Foo\n",
    );
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "expected parse error when reassigning a new_type binding"
    );
}

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
            b.fields.get(idx).and_then(|s| {
                if let Some((Value::Str(s), _)) = s { Some(s.clone()) } else { None }
            })
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

// --- iterator ---

/// list_iter_for_loop のテスト。
#[test]
fn test_list_iter_for_loop() {
    // for loop over a list uses __iter__ internally
    let src = "mut s = 0\nfor x in [1, 2, 3, 4]:\n    s += x\n";
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 10);
    } else {
        panic!("expected int 10");
    }
}

/// list_iter_method_direct のテスト。
#[test]
fn test_list_iter_method_direct() {
    // list.__iter__() returns a Generator; .next() yields each element
    let src = concat!(
        "let lst = [10, 20, 30]\n",
        "let it = lst.__iter__()\n",
        "let a = it.next()\n",
        "let b = it.next()\n",
        "let c = it.next()\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(10)));
    assert!(matches!(run_get(src, "b"), Value::Int(20)));
    assert!(matches!(run_get(src, "c"), Value::Int(30)));
}

/// generator_exhausted_raises_end_of_iteration のテスト。
#[test]
fn test_generator_exhausted_raises_end_of_iteration() {
    // .next() on an exhausted generator must raise EndOfIteration
    let src = concat!(
        "let lst = [1]\n",
        "let it = lst.__iter__()\n",
        "let _ = it.next()\n",
        "it.next()\n",
    );
    assert!(run(src).is_err());
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    let mut err_msg = String::new();
    for stmt in &stmts {
        match interp.exec(stmt) {
            Err(e) => {
                err_msg = e;
                break;
            }
            _ => {}
        }
    }
    assert!(
        err_msg.starts_with("EndOfIteration"),
        "expected EndOfIteration, got: {err_msg}"
    );
}

/// custom_iter_class のテスト。
#[test]
fn test_custom_iter_class() {
    // A class with gen __iter__ can be used in a for loop
    let src = concat!(
        "class Range3:\n",
        "    gen __iter__(self) -> int:\n",
        "        yield 0\n",
        "        yield 1\n",
        "        yield 2\n",
        "let r = Range3()\n",
        "mut s = 0\n",
        "for v in r:\n",
        "    s += v\n",
    );
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 3);
    } else {
        panic!("expected int 3");
    }
}

/// custom_iter_with_fields のテスト。
#[test]
fn test_custom_iter_with_fields() {
    // __iter__ can access self fields to yield instance data
    let src = concat!(
        "class Countdown:\n",
        "    mut start: int\n",
        "    gen __iter__(self) -> int:\n",
        "        mut i = self.start\n",
        "        while i > 0:\n",
        "            yield i\n",
        "            i -= 1\n",
        "let cd = Countdown(3)\n",
        "mut s = 0\n",
        "for v in cd:\n",
        "    s += v\n",
    );
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 6); // 3 + 2 + 1
    } else {
        panic!("expected int 6");
    }
}

/// str_iter_for_loop のテスト。
#[test]
fn test_str_iter_for_loop() {
    // for loop over a string iterates over characters
    let src = "mut n = 0\nfor c in \"abc\":\n    n += 1\n";
    if let Value::Int(n) = run_get(src, "n") {
        assert_eq!(n, 3);
    } else {
        panic!("expected int 3");
    }
}

/// for_break_with_iterator のテスト。
#[test]
fn test_for_break_with_iterator() {
    // break still works correctly inside an iterator-based for loop
    let src = "mut s = 0\nfor x in [1, 2, 3, 4, 5]:\n    if x == 3:\n        break\n    s += x\n";
    if let Value::Int(n) = run_get(src, "s") {
        assert_eq!(n, 3); // 1 + 2
    } else {
        panic!("expected int 3");
    }
}

// ---------------------------------------------------------------------------
// Dict tests
// ---------------------------------------------------------------------------

/// dict_literal_empty のテスト。
#[test]
fn test_dict_literal_empty() {
    let src = "let d = {}";
    assert!(run(src).is_ok());
}

/// dict_literal_with_entries のテスト。
#[test]
fn test_dict_literal_with_entries() {
    let src = r#"let d = {"a": 1, "b": 2}"#;
    assert!(run(src).is_ok());
}

/// dict_subscript_read のテスト。
#[test]
fn test_dict_subscript_read() {
    let src = r#"let d = {"x": 42}
let v = d["x"]"#;
    if let Value::Int(n) = run_get(src, "v") {
        assert_eq!(n, 42);
    } else {
        panic!("expected Int(42)");
    }
}

/// dict_subscript_write のテスト。
#[test]
fn test_dict_subscript_write() {
    let src = r#"mut d = {"a": 1}
d["a"] = 99"#;
    run(src).expect("should not fail");
}

/// dict_subscript_add_new_key のテスト。
#[test]
fn test_dict_subscript_add_new_key() {
    let src = r#"mut d = {}
d["k"] = 7
let v = d["k"]"#;
    if let Value::Int(n) = run_get(src, "v") {
        assert_eq!(n, 7);
    } else {
        panic!("expected Int(7)");
    }
}

/// dict_key_not_found_error のテスト。
#[test]
fn test_dict_key_not_found_error() {
    let src = r#"let d = {"a": 1}
let v = d["missing"]"#;
    assert!(run(src).is_err());
}

/// dict_key_method のテスト。
#[test]
fn test_dict_key_method() {
    let src = r#"let d = {1: "one", 2: "two"}
let ks = d.key()"#;
    if let Value::List(ks) = run_get(src, "ks") {
        assert_eq!(ks.borrow().len(), 2);
    } else {
        panic!("expected List");
    }
}

/// dict_item_method のテスト。
#[test]
fn test_dict_item_method() {
    let src = r#"let d = {1: "one", 2: "two"}
let vs = d.item()"#;
    if let Value::List(vs) = run_get(src, "vs") {
        assert_eq!(vs.borrow().len(), 2);
    } else {
        panic!("expected List");
    }
}

/// dict_typed_constructor_empty のテスト。
#[test]
fn test_dict_typed_constructor_empty() {
    let src = "let d = dict[str, int]()";
    assert!(run(src).is_ok());
}

/// dict_typed_constructor_from_literal のテスト。
#[test]
fn test_dict_typed_constructor_from_literal() {
    let src = r#"let d = dict[str, int]({"hello": 1, "world": 2})"#;
    assert!(run(src).is_ok());
}

/// dict_typed_constructor_type_mismatch_key_err のテスト。
#[test]
fn test_dict_typed_constructor_type_mismatch_key_err() {
    let src = r#"let d = dict[int, str]({1: "ok", "bad": "value"})"#;
    let err = run(src).expect_err("should fail with type mismatch");
    assert!(err.contains("StaticTypeError"), "got: {err}");
}

/// dict_typed_constructor_type_mismatch_item_err のテスト。
#[test]
fn test_dict_typed_constructor_type_mismatch_item_err() {
    let src = r#"let d = dict[str, int]({"ok": 1, "bad": "not_an_int"})"#;
    let err = run(src).expect_err("should fail with type mismatch");
    assert!(err.contains("StaticTypeError"), "got: {err}");
}

/// dict_typed_write_type_check のテスト。
#[test]
fn test_dict_typed_write_type_check() {
    let src = r#"mut d = dict[str, int]()
d["key"] = 42"#;
    assert!(run(src).is_ok());
}

/// dict_typed_write_wrong_key_type_err のテスト。
#[test]
fn test_dict_typed_write_wrong_key_type_err() {
    let src = r#"mut d = dict[str, int]()
d[123] = 42"#;
    let err = run(src).expect_err("should fail type check");
    assert!(err.contains("TypeError"), "got: {err}");
}

/// dict_typed_write_wrong_item_type_err のテスト。
#[test]
fn test_dict_typed_write_wrong_item_type_err() {
    let src = r#"mut d = dict[str, int]()
d["key"] = "not_int""#;
    let err = run(src).expect_err("should fail type check");
    assert!(err.contains("TypeError"), "got: {err}");
}

/// dict_multiline_literal のテスト。
#[test]
fn test_dict_multiline_literal() {
    let src = "let d = {\n    \"a\": 1,\n    \"b\": 2\n}";
    assert!(run(src).is_ok());
}

/// dict_int_upcast_to_float_ok のテスト。
#[test]
fn test_dict_int_upcast_to_float_ok() {
    // int value is accepted where float is declared (upcast)
    let src = r#"mut d = dict[str, float]()
d["pi"] = 3"#;
    assert!(run(src).is_ok());
}

/// dict_is_truthy_empty のテスト。
#[test]
fn test_dict_is_truthy_empty() {
    let src = "let d = {}\nlet t = not d";
    if let Value::Bool(b) = run_get(src, "t") {
        assert!(b); // empty dict is falsy
    } else {
        panic!("expected Bool");
    }
}

/// dict_is_truthy_nonempty のテスト。
#[test]
fn test_dict_is_truthy_nonempty() {
    let src = r#"let d = {"x": 1}
let t = not d"#;
    if let Value::Bool(b) = run_get(src, "t") {
        assert!(!b); // non-empty dict is truthy
    } else {
        panic!("expected Bool");
    }
}

// --- tuples ---

/// tuple_empty のテスト。
#[test]
fn test_tuple_empty() {
    let v = eval_expr("()");
    if let Value::Tuple(t) = v {
        assert!(t.is_empty());
    } else {
        panic!("expected Tuple");
    }
}

/// tuple_single のテスト。
#[test]
fn test_tuple_single() {
    let v = eval_expr("(42,)");
    if let Value::Tuple(t) = v {
        assert_eq!(t.len(), 1);
        assert!(matches!(t.get(0), Some(Value::Int(42))));
    } else {
        panic!("expected Tuple");
    }
}

/// tuple_multi のテスト。
#[test]
fn test_tuple_multi() {
    let v = eval_expr(r#"(1, "hello", True)"#);
    if let Value::Tuple(t) = v {
        assert_eq!(t.len(), 3);
        assert!(matches!(t.get(0), Some(Value::Int(1))));
        assert!(matches!(t.get(1), Some(Value::Str(s)) if s == "hello"));
        assert!(matches!(t.get(2), Some(Value::Bool(true))));
    } else {
        panic!("expected Tuple");
    }
}

/// tuple_types のテスト。
#[test]
fn test_tuple_types() {
    let v = eval_expr(r#"(1, "hello", True)"#);
    if let Value::Tuple(t) = v {
        assert_eq!(t.element_type(0), Some("int"));
        assert_eq!(t.element_type(1), Some("str"));
        assert_eq!(t.element_type(2), Some("bool"));
    } else {
        panic!("expected Tuple");
    }
}

/// tuple_grouped_expr_not_tuple のテスト。
#[test]
fn test_tuple_grouped_expr_not_tuple() {
    // (expr) without comma is NOT a tuple
    let v = eval_expr("(42)");
    assert!(matches!(v, Value::Int(42)));
}

/// tuple_display のテスト。
#[test]
fn test_tuple_display() {
    let src = r#"let t = (1, "a", True)"#;
    assert!(run(src).is_ok());
}

/// tuple_equality のテスト。
#[test]
fn test_tuple_equality() {
    let src = "let a = (1, 2)\nlet b = (1, 2)\nlet eq = a == b\n";
    if let Value::Bool(b) = run_get(src, "eq") {
        assert!(b);
    } else {
        panic!("expected Bool");
    }
}

/// tuple_inequality_different_values のテスト。
#[test]
fn test_tuple_inequality_different_values() {
    let src = "let a = (1, 2)\nlet b = (1, 3)\nlet eq = a == b\n";
    if let Value::Bool(b) = run_get(src, "eq") {
        assert!(!b);
    } else {
        panic!("expected Bool");
    }
}

/// tuple_truthy_nonempty のテスト。
#[test]
fn test_tuple_truthy_nonempty() {
    let src = "let t = (1, 2)\nlet r = not t\n";
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(!b); // non-empty tuple is truthy
    } else {
        panic!("expected Bool");
    }
}

/// tuple_falsy_empty のテスト。
#[test]
fn test_tuple_falsy_empty() {
    let src = "let t = ()\nlet r = not t\n";
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(b); // empty tuple is falsy
    } else {
        panic!("expected Bool");
    }
}

/// tuple_multiline のテスト。
#[test]
fn test_tuple_multiline() {
    let src = "let t = (1,\n    2,\n    3)\n";
    if let Value::Tuple(t) = run_get(src, "t") {
        assert_eq!(t.len(), 3);
    } else {
        panic!("expected Tuple");
    }
}

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

// ---------------------------------------------------------------------------
// __getitem__ / __setitem__ — list, str, dict, instance, PyObject
// ---------------------------------------------------------------------------

/// list_getitem のテスト。
#[test]
fn test_list_getitem() {
    // list[int] インデックスアクセス（正・負）
    let src = concat!(
        "let xs = [10, 20, 30]\n",
        "let a = xs[0]\n",
        "let b = xs[2]\n",
        "let c = xs[-1]\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(10)));
    assert!(matches!(run_get(src, "b"), Value::Int(30)));
    assert!(matches!(run_get(src, "c"), Value::Int(30)));
}

/// list_setitem のテスト。
#[test]
fn test_list_setitem() {
    // list[int] = value による要素の書き換え
    let src = concat!("mut xs = [1, 2, 3]\n", "xs[1] = 99\n", "let r = xs[1]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(99)));
}

/// list_setitem_negative のテスト。
#[test]
fn test_list_setitem_negative() {
    // 負インデックスでの書き換え
    let src = concat!("mut xs = [1, 2, 3]\n", "xs[-1] = 77\n", "let r = xs[2]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(77)));
}

/// list_getitem_out_of_range のテスト。
#[test]
fn test_list_getitem_out_of_range() {
    let src = concat!("let xs = [1, 2, 3]\n", "let r = xs[5]\n",);
    assert!(run(src).is_err());
}

/// str_getitem のテスト。
#[test]
fn test_str_getitem() {
    // str[int] インデックスアクセス（正・負）
    let src = concat!("let s = \"hello\"\n", "let a = s[0]\n", "let b = s[-1]\n",);
    if let Value::Str(a) = run_get(src, "a") {
        assert_eq!(a, "h");
    } else {
        panic!("expected Str");
    }
    if let Value::Str(b) = run_get(src, "b") {
        assert_eq!(b, "o");
    } else {
        panic!("expected Str");
    }
}

/// instance_getitem_setitem のテスト。
#[test]
fn test_instance_getitem_setitem() {
    // ユーザー定義クラスの __getitem__ / __setitem__
    let src = concat!(
        "class Box:\n",
        "    mut data: int\n",
        "    fn __init__(mut self) -> None:\n",
        "        self.data = 0\n",
        "    fn __getitem__(self, let key: int) -> int:\n",
        "        return self.data + key\n",
        "    fn __setitem__(mut self, let key: int, let val: int) -> None:\n",
        "        self.data = val\n",
        "mut b = Box()\n",
        "b[10] = 5\n",
        "let r = b[1]\n",
    );
    assert!(matches!(run_get(src, "r"), Value::Int(6)));
}

/// pyobject_getitem のテスト。
#[test]
fn test_pyobject_getitem() {
    // PyObject の subscript read: Container.__getitem__
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([10, 20, 30])\n",
        "let r = c[1]\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 20);
    } else {
        panic!("expected Int(20)");
    }
}

/// pyobject_setitem のテスト。
#[test]
fn test_pyobject_setitem() {
    // PyObject の subscript write: Container.__setitem__
    let src = concat!(
        "import[py-int] py_calculator as calc\n",
        "let c = calc.make_container([1, 2, 3])\n",
        "c[0] = 99\n",
        "let r = c[0]\n",
    );
    if let Value::Int(n) = run_py_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!("expected Int(99)");
    }
}

/// tuple_getitem のテスト。
#[test]
fn test_tuple_getitem() {
    // Python から返ってきた tuple が Value::Tuple に変換された場合の subscript
    let src = concat!("let t = (100, 200, 300)\n", "let r = t[1]\n",);
    assert!(matches!(run_get(src, "r"), Value::Int(200)));
}

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

// ---------------------------------------------------------------------------
// match statement tests
// ---------------------------------------------------------------------------

/// match_case_literal のテスト。
#[test]
fn test_match_case_literal() {
    let src = "
mut x = 0
mut result = 0
match (x):
    case 0:
        result = 1
    case 1:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(1)));
}

/// match_case_no_match のテスト。
#[test]
fn test_match_case_no_match() {
    let src = "
mut x = 5
mut result = 0
match (x):
    case 0:
        result = 1
    case 1:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(0)));
}

/// match_case_wildcard のテスト。
#[test]
fn test_match_case_wildcard() {
    let src = "
mut x = 99
mut result = 0
match (x):
    case 0:
        result = 1
    case _:
        result = 99
";
    assert!(matches!(run_get(src, "result"), Value::Int(99)));
}

/// match_case_string のテスト。
#[test]
fn test_match_case_string() {
    let src = r#"
mut s = "hello"
mut result = 0
match (s):
    case "world":
        result = 1
    case "hello":
        result = 2
    case _:
        result = 3
"#;
    assert!(matches!(run_get(src, "result"), Value::Int(2)));
}

/// match_is_int のテスト。
#[test]
fn test_match_is_int() {
    let src = "
mut x = 42
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(1)));
}

/// match_is_str のテスト。
#[test]
fn test_match_is_str() {
    let src = r#"
mut x = "hello"
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
"#;
    assert!(matches!(run_get(src, "result"), Value::Int(2)));
}

/// match_is_no_match のテスト。
#[test]
fn test_match_is_no_match() {
    let src = "
mut x = 3.14
mut result = 0
match (x):
    is int:
        result = 1
    is str:
        result = 2
";
    assert!(matches!(run_get(src, "result"), Value::Int(0)));
}

/// match_block_return のテスト。
#[test]
fn test_match_block_return() {
    // block_return inside a match arm exits the enclosing block: early
    let src = "
mut x = 2
mut result = 0
block:
    match (x):
        case 1:
            result = 10
            block_return 0
        case 2:
            result = 20
            block_return 0
    result = 999
";
    assert!(matches!(run_get(src, "result"), Value::Int(20)));
}

/// match_return_from_function のテスト。
#[test]
fn test_match_return_from_function() {
    let src = "
fn get(x: int) -> int:
    match (x):
        case 1:
            return 10
        case 2:
            return 20
        case _:
            return 99
    return 0
let result = get(2)
";
    assert!(matches!(run_get(src, "result"), Value::Int(20)));
}

/// match_mixed_arms_parse_error のテスト。
#[test]
fn test_match_mixed_arms_parse_error() {
    let src = "
mut x = 0
match (x):
    case 0:
        pass
    is int:
        pass
";
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("mix"), "expected mix error, got: {msg}");
}

// ---------------------------------------------------------------------------
// 制御フロー式テスト (if/for/while/match as expressions)
// ---------------------------------------------------------------------------

/// `val` が `Str(expected)` であることを表明するテストヘルパー。
fn assert_str(val: Value, expected: &str) {
    if let Value::Str(s) = val {
        assert_eq!(s, expected);
    } else {
        panic!("expected Str({:?}), got {:?}", expected, val);
    }
}

/// `val` が `Int(expected)` であることを表明するテストヘルパー。
fn assert_int(val: Value, expected: i64) {
    if let Value::Int(n) = val {
        assert_eq!(n, expected);
    } else {
        panic!("expected Int({}), got {:?}", expected, val);
    }
}

/// `val` が `List([Int(...)])` であることを表明するテストヘルパー。各要素の整数値を検証する。
fn assert_int_list(val: Value, expected: &[i64]) {
    if let Value::List(rc) = val {
        let list = rc.borrow();
        assert_eq!(list.len(), expected.len(), "list length mismatch");
        for (i, (v, e)) in list.iter().zip(expected.iter()).enumerate() {
            if let Value::Int(n) = v {
                assert_eq!(n, e, "list[{}] mismatch", i);
            } else {
                panic!("list[{}]: expected Int({}), got {:?}", i, e, v);
            }
        }
    } else {
        panic!("expected List, got {:?}", val);
    }
}

/// if_expr_true_branch のテスト。
#[test]
fn test_if_expr_true_branch() {
    let src = "
let x = if True ->str:
    block_return \"yes\"
else:
    block_return \"no\"
";
    assert_str(run_get(src, "x"), "yes");
}

/// if_expr_false_branch のテスト。
#[test]
fn test_if_expr_false_branch() {
    let src = "
let x = if False ->str:
    block_return \"yes\"
else:
    block_return \"no\"
";
    assert_str(run_get(src, "x"), "no");
}

/// if_expr_no_else_returns_none のテスト。
#[test]
fn test_if_expr_no_else_returns_none() {
    let src = "
let x = if False ->str:
    block_return \"yes\"
";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// if_expr_elif のテスト。
#[test]
fn test_if_expr_elif() {
    let src = "
let n = 7
let s = if n < 5 ->str:
    block_return \"small\"
elif n < 10:
    block_return \"medium\"
else:
    block_return \"large\"
";
    assert_str(run_get(src, "s"), "medium");
}

/// for_expr_block_yield のテスト。
#[test]
fn test_for_expr_block_yield() {
    let src = "
let evens = for i in range(5) ->list[int]:
    if i % 2 == 0:
        loop_yield i
";
    assert_int_list(run_get(src, "evens"), &[0, 2, 4]);
}

/// for_expr_block_return_single_value のテスト。
#[test]
fn test_for_expr_block_return_single_value() {
    let src = "
let first = for i in range(1, 10) ->int:
    if i % 2 == 0:
        block_return i
";
    assert_int(run_get(src, "first"), 2);
}

/// for_expr_no_yields_returns_none のテスト。
#[test]
fn test_for_expr_no_yields_returns_none() {
    let src = "
let x = for i in range(0) ->list[int]:
    loop_yield i
";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// for_expr_break_returns_partial_list のテスト。
#[test]
fn test_for_expr_break_returns_partial_list() {
    let src = "
let partial = for i in range(10) ->list[int]:
    if i == 3:
        break
    loop_yield i
";
    assert_int_list(run_get(src, "partial"), &[0, 1, 2]);
}

/// while_expr_block_yield のテスト。
#[test]
fn test_while_expr_block_yield() {
    let src = "
mut n = 0
let vals = while n < 3 ->list[int]:
    loop_yield n
    n += 1
";
    assert_int_list(run_get(src, "vals"), &[0, 1, 2]);
}

/// while_expr_block_return のテスト。
#[test]
fn test_while_expr_block_return() {
    let src = "
mut n = 0
let found = while n < 100 ->int:
    n += 1
    if n * n > 50:
        block_return n
";
    assert_int(run_get(src, "found"), 8);
}

/// match_expr_block_return のテスト。
#[test]
fn test_match_expr_block_return() {
    let src = "
let v = 2
let s = match (v) ->str:
    case 1:
        block_return \"one\"
    case 2:
        block_return \"two\"
    case _:
        block_return \"other\"
";
    assert_str(run_get(src, "s"), "two");
}

/// match_expr_no_match_returns_none のテスト。
#[test]
fn test_match_expr_no_match_returns_none() {
    let src = "
let v = 99
let s = match (v) ->str:
    case 1:
        block_return \"one\"
";
    assert!(matches!(run_get(src, "s"), Value::None));
}

/// break_exits_regular_for_loop のテスト。
#[test]
fn test_break_exits_regular_for_loop() {
    let src = "
mut found = -1
for i in range(10):
    if i == 5:
        found = i
        break
";
    assert_int(run_get(src, "found"), 5);
}

// --- break propagation through nested control-flow expressions ---

/// break_inside_if_expr_exits_for_loop のテスト。
#[test]
fn test_break_inside_if_expr_exits_for_loop() {
    // break inside an if expression body should exit the enclosing for loop
    let src = "
mut found = -1
for i in range(10):
    let _ = if i == 4 ->int:
        found = i
        break
    else:
        0
";
    assert_int(run_get(src, "found"), 4);
    // loop must have stopped: next iteration would set found to 5+
    let src2 = "
mut count = 0
for i in range(10):
    let _ = if i == 3 ->int:
        break
    else:
        0
    count += 1
";
    assert_int(run_get(src2, "count"), 3); // iterations 0, 1, 2 complete
}

/// break_inside_if_expr_exits_while_loop のテスト。
#[test]
fn test_break_inside_if_expr_exits_while_loop() {
    let src = "
mut i = 0
mut stopped_at = -1
while i < 20:
    let _ = if i == 7 ->int:
        stopped_at = i
        break
    else:
        0
    i += 1
";
    assert_int(run_get(src, "stopped_at"), 7);
}

/// break_inside_block_expr_exits_loop のテスト。
#[test]
fn test_break_inside_block_expr_exits_loop() {
    // break inside a block: expression should exit the enclosing loop
    let src = "
mut found = -1
for i in range(10):
    let _ = block ->int:
        if i == 5:
            found = i
            break
        block_return i
";
    assert_int(run_get(src, "found"), 5);
}

/// for_expr_break_inside_if_expr_returns_yields のテスト。
#[test]
fn test_for_expr_break_inside_if_expr_returns_yields() {
    // break inside an if expression in a for expression should return accumulated yields
    let src = "
let result = for i in range(10) ->list[int]:
    let _ = if i == 3 ->int:
        break
    else:
        0
    loop_yield i
";
    assert_int_list(run_get(src, "result"), &[0, 1, 2]);
}

/// while_expr_break_inside_if_expr_returns_yields のテスト。
#[test]
fn test_while_expr_break_inside_if_expr_returns_yields() {
    let src = "
mut n = 0
let result = while True ->list[int]:
    let _ = if n == 4 ->int:
        break
    else:
        0
    loop_yield n
    n += 1
";
    assert_int_list(run_get(src, "result"), &[0, 1, 2, 3]);
}

/// break_does_not_cross_function_boundary のテスト。
#[test]
fn test_break_does_not_cross_function_boundary() {
    // break inside a function that has no loop should be an error
    let src = "
fn bad():
    let _ = if True ->int:
        break
    else:
        0
bad()
";
    assert!(run(src).is_err());
}

/// break_inside_function_loop_does_not_exit_outer_loop のテスト。
#[test]
fn test_break_inside_function_loop_does_not_exit_outer_loop() {
    // break inside an inner function's loop must not affect the outer loop
    let src = "
mut outer_count = 0
fn inner() -> int:
    for i in range(5):
        if i == 2:
            break
    return 42
for _ in range(4):
    inner()
    outer_count += 1
";
    assert_int(run_get(src, "outer_count"), 4);
}

/// continue_in_while_loop のテスト。
#[test]
fn test_continue_in_while_loop() {
    let src = "
mut evens = 0
mut i = 0
while i < 10:
    i += 1
    if i % 2 != 0:
        continue
    evens += i
";
    assert_int(run_get(src, "evens"), 30); // 2+4+6+8+10
}

/// continue_in_for_loop のテスト。
#[test]
fn test_continue_in_for_loop() {
    let src = "
mut s = 0
for n in range(1, 11):
    if n % 3 == 0:
        continue
    s += n
";
    assert_int(run_get(src, "s"), 37); // 1+2+4+5+7+8+10
}

/// continue_skips_rest_of_body のテスト。
#[test]
fn test_continue_skips_rest_of_body() {
    // continue skips the remaining statements in the body
    let src = "
mut touched = 0
for i in range(5):
    continue
    touched += 1
";
    assert_int(run_get(src, "touched"), 0);
}

/// continue_in_nested_loop のテスト。
#[test]
fn test_continue_in_nested_loop() {
    // continue only skips the innermost loop iteration
    let src = "
mut s = 0
for i in range(1, 4):
    for j in range(1, 4):
        if j == 2:
            continue
        s += j
";
    // j=1 and j=3 contribute per outer iteration: (1+3)*3 = 12
    assert_int(run_get(src, "s"), 12);
}

/// continue_outside_loop_is_error のテスト。
#[test]
fn test_continue_outside_loop_is_error() {
    let src = "
fn bad():
    continue
bad()
";
    assert!(run(src).is_err());
}

/// continue_outside_loop_toplevel_is_error のテスト。
#[test]
fn test_continue_outside_loop_toplevel_is_error() {
    assert!(run("continue").is_err());
}

/// block_return_propagates_through_nested_if_to_for_expr のテスト。
#[test]
fn test_block_return_propagates_through_nested_if_to_for_expr() {
    let src = "
let result = for i in range(10) ->int:
    if i > 4:
        block_return i
";
    assert_int(run_get(src, "result"), 5);
}

/// block_expr_with_return_type_annotation のテスト。
#[test]
fn test_block_expr_with_return_type_annotation() {
    let src = "
let x = block ->int:
    block_return 42
";
    assert_int(run_get(src, "x"), 42);
}

/// if_expr_without_annotation_still_works のテスト。
#[test]
fn test_if_expr_without_annotation_still_works() {
    let src = "
let x = if True:
    block_return 100
else:
    block_return 0
";
    assert_int(run_get(src, "x"), 100);
}

/// block_return_type_check_ok のテスト。
#[test]
fn test_block_return_type_check_ok() {
    let src = "let x = block ->int:\n    block_return 42\n";
    assert_int(run_get(src, "x"), 42);
}

/// block_return_type_check_error のテスト。
#[test]
fn test_block_return_type_check_error() {
    let src = "let x = block ->int:\n    block_return \"hello\"\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
    assert!(err.contains("'int'"), "expected annotation in error: {err}");
}

/// if_expr_block_return_type_check_ok のテスト。
#[test]
fn test_if_expr_block_return_type_check_ok() {
    let src = "let x = if True ->str:\n    block_return \"ok\"\nelse:\n    block_return \"no\"\n";
    assert_str(run_get(src, "x"), "ok");
}

/// if_expr_block_return_type_check_error のテスト。
#[test]
fn test_if_expr_block_return_type_check_error() {
    let src = "let x = if True ->str:\n    block_return 42\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// for_expr_block_return_type_check_ok のテスト。
#[test]
fn test_for_expr_block_return_type_check_ok() {
    let src = "let x = for i in range(5) ->int:\n    if i == 3:\n        block_return i\n";
    assert_int(run_get(src, "x"), 3);
}

/// for_expr_block_return_type_check_error のテスト。
#[test]
fn test_for_expr_block_return_type_check_error() {
    let src = "let x = for i in range(5) ->int:\n    if i == 3:\n        block_return \"three\"\n";
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// while_expr_block_return_type_check_ok のテスト。
#[test]
fn test_while_expr_block_return_type_check_ok() {
    let src = concat!(
        "mut n = 0\n",
        "let x = while n < 10 ->int:\n",
        "    n += 1\n",
        "    if n == 5:\n",
        "        block_return n\n",
    );
    assert_int(run_get(src, "x"), 5);
}

/// while_expr_block_return_type_check_error のテスト。
#[test]
fn test_while_expr_block_return_type_check_error() {
    let src = concat!(
        "mut n = 0\n",
        "let x = while n < 10 ->int:\n",
        "    n += 1\n",
        "    if n == 5:\n",
        "        block_return \"five\"\n",
    );
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// match_expr_block_return_type_check_ok のテスト。
#[test]
fn test_match_expr_block_return_type_check_ok() {
    let src = concat!(
        "let x = match (1) ->str:\n",
        "    case 1:\n",
        "        block_return \"one\"\n",
        "    case _:\n",
        "        block_return \"other\"\n",
    );
    assert_str(run_get(src, "x"), "one");
}

/// match_expr_block_return_type_check_error のテスト。
#[test]
fn test_match_expr_block_return_type_check_error() {
    let src = concat!(
        "let x = match (1) ->str:\n",
        "    case 1:\n",
        "        block_return 1\n",
        "    case _:\n",
        "        block_return 0\n",
    );
    let err = run(src).unwrap_err();
    assert!(err.contains("TypeError"), "expected TypeError, got: {err}");
}

/// block_return_option_type_check_ok のテスト。
#[test]
fn test_block_return_option_type_check_ok() {
    let src = "let x = block ->Option[int]:\n    block_return None\n";
    assert!(matches!(run_get(src, "x"), Value::None));
}

/// block_return_no_annotation_no_check のテスト。
#[test]
fn test_block_return_no_annotation_no_check() {
    let src = "let x = block:\n    block_return \"anything\"\n";
    assert_str(run_get(src, "x"), "anything");
}

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
        let (v, _) = inst.fields[idx].as_ref().unwrap();
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
            let (v, _) = inst.fields[idx].as_ref().unwrap();
            if let Value::Int(n) = v {
                assert_eq!(*n, expected);
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
        let (v, _) = inst.fields[idx].as_ref().unwrap();
        if let Value::Int(n) = v {
            assert_eq!(*n, 5);
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
        let (v, _) = inst.fields[idx].as_ref().unwrap();
        if let Value::Int(n) = v {
            assert_eq!(*n, 6);
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

// ---------------------------------------------------------------------------
// ファイル I/O テスト
// ---------------------------------------------------------------------------

/// テスト用の一時ファイルパスを生成する（ユニークなサフィックス付き）。
fn temp_path(suffix: &str) -> String {
    format!("target/test_tmp_{suffix}.txt")
}

/// テスト後に一時ファイルを削除する。
fn cleanup(path: &str) {
    let _ = std::fs::remove_file(path);
}

/// file_open_mode_enum のテスト。
#[test]
fn test_file_open_mode_enum() {
    // FileOpenMode enum が正しくグローバルスコープに登録されているかを確認する
    let src = "let m = FileOpenMode.read\nlet v = m.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Int(2)),
        "FileOpenMode.read.value should be 2"
    );
}

/// file_start_point_enum のテスト。
#[test]
fn test_file_start_point_enum() {
    let src = "let s = StartPoint.end\nlet v = s.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Int(1)),
        "StartPoint.end.value should be 1"
    );
}

/// file_path_type のテスト。
#[test]
fn test_file_path_type() {
    // path 型のインスタンスを生成できることを確認する
    let src = "let p = path(\"foo.txt\")\nlet v = p.value\n";
    let val = run_get(src, "v");
    assert!(
        matches!(val, Value::Str(s) if s == "foo.txt"),
        "path.value should be 'foo.txt'"
    );
}

/// file_rewrite_and_read のテスト。
#[test]
fn test_file_rewrite_and_read() {
    let p = temp_path("rewrite_read");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite)\nf.write(\"hello\")\nclose(f)\n\
         let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n",
    );
    run(&src).expect("file rewrite + read should succeed");
    let src2 = format!("let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n");
    let val = run_get(&src2, "r");
    cleanup(&p);
    assert!(
        matches!(val, Value::Str(s) if s == "hello"),
        "read() should return written text"
    );
}

/// file_write_line のテスト。
#[test]
fn test_file_write_line() {
    let p = temp_path("write_line");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite)\n\
         f.write_line(\"line1\")\nf.write_line(\"line2\")\nclose(f)\n"
    );
    run(&src).expect("write_line should succeed");
    let src2 = format!("let g = open(\"{p}\", FileOpenMode.read)\nlet r = g.read()\nclose(g)\n");
    let val = run_get(&src2, "r");
    cleanup(&p);
    assert!(
        matches!(val, Value::Str(s) if s == "line1\nline2\n"),
        "write_line should append newline"
    );
}

/// file_read_line_forward のテスト。
#[test]
fn test_file_read_line_forward() {
    let p = temp_path("read_line_fwd");
    cleanup(&p);
    std::fs::write(&p, "alpha\nbeta\n").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\n\
         let a = f.read_line()\nlet b = f.read_line()\nclose(f)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let a = interp.get_val("a").unwrap();
    let b = interp.get_val("b").unwrap();
    cleanup(&p);
    assert!(
        matches!(a, Value::Str(s) if s == "alpha\n"),
        "first read_line should be 'alpha\\n'"
    );
    assert!(
        matches!(b, Value::Str(s) if s == "beta\n"),
        "second read_line should be 'beta\\n'"
    );
}

/// file_read_letter のテスト。
#[test]
fn test_file_read_letter() {
    let p = temp_path("read_letter");
    cleanup(&p);
    std::fs::write(&p, "AB").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\n\
         let a = f.read_letter()\nlet b = f.read_letter()\nclose(f)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let a = interp.get_val("a").unwrap();
    let b = interp.get_val("b").unwrap();
    cleanup(&p);
    assert!(
        matches!(a, Value::Str(s) if s == "A"),
        "first letter should be 'A'"
    );
    assert!(
        matches!(b, Value::Str(s) if s == "B"),
        "second letter should be 'B'"
    );
}

/// file_eof_error のテスト。
#[test]
fn test_file_eof_error() {
    let p = temp_path("eof");
    cleanup(&p);
    std::fs::write(&p, "x").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read, StartPoint.end)\nlet r = f.read()\nclose(f)\n"
    );
    assert!(run(&src).is_err(), "read at EOF should raise EOFError");
    cleanup(&p);
}

/// file_bof_error のテスト。
#[test]
fn test_file_bof_error() {
    let p = temp_path("bof");
    cleanup(&p);
    std::fs::write(&p, "x").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.read)\nlet r = f.read(backward = True)\nclose(f)\n"
    );
    assert!(run(&src).is_err(), "read at BOF should raise BOFError");
    cleanup(&p);
}

/// file_make_and_write_existing_error のテスト。
#[test]
fn test_file_make_and_write_existing_error() {
    let p = temp_path("maw_exist");
    std::fs::write(&p, "existing").unwrap();
    let src = format!("let f = open(\"{p}\", FileOpenMode.make_and_write)\nclose(f)\n");
    assert!(
        run(&src).is_err(),
        "make_and_write on existing file should error"
    );
    cleanup(&p);
}

/// file_write_read_only_error のテスト。
#[test]
fn test_file_write_read_only_error() {
    let p = temp_path("write_ro");
    cleanup(&p);
    std::fs::write(&p, "hello").unwrap();
    let src = format!("let f = open(\"{p}\", FileOpenMode.read)\nf.write(\"x\")\nclose(f)\n");
    assert!(run(&src).is_err(), "write on read-only file should error");
    cleanup(&p);
}

/// file_insert_midpoint のテスト。
#[test]
fn test_file_insert_midpoint() {
    let p = temp_path("insert_mid");
    cleanup(&p);
    // Write "helo", then open and insert "l" at position 3 → "hello"
    std::fs::write(&p, "helo").unwrap();
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.write)\n\
         let _ = f.read_letter()\nlet _ = f.read_letter()\nlet _ = f.read_letter()\n\
         f.write(\"l\")\nclose(f)\n"
    );
    run(&src).expect("insert mid should succeed");
    let content = std::fs::read_to_string(&p).unwrap();
    cleanup(&p);
    assert_eq!(
        content, "hello",
        "inserting 'l' at position 3 should give 'hello'"
    );
}

/// file_byte_mode_write_read のテスト。
#[test]
fn test_file_byte_mode_write_read() {
    let p = temp_path("byte_mode");
    cleanup(&p);
    let src = format!(
        "let f = open(\"{p}\", FileOpenMode.rewrite, StartPoint.top, ByteRecognizingMode.byte)\n\
         f.write([72, 105])\nclose(f)\n\
         let g = open(\"{p}\", FileOpenMode.read, StartPoint.top, ByteRecognizingMode.byte)\n\
         let r = g.read()\nclose(g)\n"
    );
    let tokens = crate::lexer::Lexer::new(&src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    let val = interp.get_val("r").unwrap();
    cleanup(&p);
    // r should be [72, 105]
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], Value::Int(72)));
        assert!(matches!(items[1], Value::Int(105)));
    } else {
        panic!("expected list of bytes");
    }
}

// ---------------------------------------------------------------------------
// uint primitive type
// ---------------------------------------------------------------------------

/// uint_literal_via_cast のテスト。
#[test]
fn test_uint_literal_via_cast() {
    // uint(42) returns a UInt value
    let val = run_get("let u = uint(42)\n", "u");
    assert!(matches!(val, Value::UInt(42)));
}

/// uint_zero のテスト。
#[test]
fn test_uint_zero() {
    let val = run_get("let u = uint()\n", "u");
    assert!(matches!(val, Value::UInt(0)));
}

/// uint_from_int のテスト。
#[test]
fn test_uint_from_int() {
    let val = run_get("let u = uint(100)\n", "u");
    assert!(matches!(val, Value::UInt(100)));
}

/// uint_from_bool のテスト。
#[test]
fn test_uint_from_bool() {
    let val = run_get("let u = uint(True)\n", "u");
    assert!(matches!(val, Value::UInt(1)));
}

/// uint_arithmetic のテスト。
#[test]
fn test_uint_arithmetic() {
    assert!(matches!(
        run_get("let r = uint(10) + uint(5)\n", "r"),
        Value::UInt(15)
    ));
    assert!(matches!(
        run_get("let r = uint(10) - uint(3)\n", "r"),
        Value::UInt(7)
    ));
    assert!(matches!(
        run_get("let r = uint(3) * uint(4)\n", "r"),
        Value::UInt(12)
    ));
}

/// uint_comparison のテスト。
#[test]
fn test_uint_comparison() {
    assert!(matches!(
        run_get("let r = uint(5) < uint(10)\n", "r"),
        Value::Bool(true)
    ));
    assert!(matches!(
        run_get("let r = uint(5) == uint(5)\n", "r"),
        Value::Bool(true)
    ));
    assert!(matches!(
        run_get("let r = uint(5) > uint(10)\n", "r"),
        Value::Bool(false)
    ));
}

/// uint_is_type のテスト。
#[test]
fn test_uint_is_type() {
    let val = run_get("let r = uint(7) is uint\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// uint_str_display のテスト。
#[test]
fn test_uint_str_display() {
    // uint should display as its decimal value
    let val = run_get("let r = str(uint(255))\n", "r");
    assert!(matches!(val, Value::Str(s) if s == "255"));
}

// ---------------------------------------------------------------------------
// id() built-in function
// ---------------------------------------------------------------------------

/// id_returns_pointer_instance のテスト。
#[test]
fn test_id_returns_pointer_instance() {
    // id(x) should return a pointer instance with a .value field of type uint
    let src = "let x = 0\nlet p = id(x)\nlet v = p.value\n";
    let val = run_get(src, "v");
    assert!(matches!(val, Value::UInt(_)));
}

/// id_same_instance_same_pointer のテスト。
#[test]
fn test_id_same_instance_same_pointer() {
    // The same instance should have the same id (same Rc allocation)
    let src = concat!(
        "class Box:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "let b = Box(42)\n",
        "let p1 = id(b)\n",
        "let p2 = id(b)\n",
        "let same = p1.value == p2.value\n",
    );
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
}

/// id_different_instances_different_pointers のテスト。
#[test]
fn test_id_different_instances_different_pointers() {
    // Two separate instances should have different ids
    let src = concat!(
        "class Box:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "let a = Box(1)\n",
        "let b = Box(2)\n",
        "let pa = id(a)\n",
        "let pb = id(b)\n",
        "let diff = pa.value != pb.value\n",
    );
    assert!(matches!(run_get(src, "diff"), Value::Bool(true)));
}

/// id_mut_copy_different_from_original のテスト。
#[test]
fn test_id_mut_copy_different_from_original() {
    // mut z = x creates a deep copy, so id(z) != id(x) for reference types
    let src = concat!(
        "class Box:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "let x = Box(10)\n",
        "mut z = x\n",
        "let px = id(x)\n",
        "let pz = id(z)\n",
        "let diff = px.value != pz.value\n",
    );
    assert!(matches!(run_get(src, "diff"), Value::Bool(true)));
}

/// id_let_alias_same_as_original のテスト。
#[test]
fn test_id_let_alias_same_as_original() {
    // let y = x (both let) shares the same Rc, so id(x) == id(y)
    let src = concat!(
        "class Box:\n",
        "    mut n: int\n",
        "    fn __init__(mut self, let n: int) -> None:\n",
        "        self.n = n\n",
        "let x = Box(5)\n",
        "let y = x\n",
        "let px = id(x)\n",
        "let py = id(y)\n",
        "let same = px.value == py.value\n",
    );
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
}

/// id_value_types_equal_int のテスト。
#[test]
fn test_id_value_types_equal_int() {
    // Equal integers should have equal ids (value-based identity)
    let src =
        "let a = 5\nlet b = 5\nlet pa = id(a)\nlet pb = id(b)\nlet same = pa.value == pb.value\n";
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
}

/// id_wrong_arg_count_error のテスト。
#[test]
fn test_id_wrong_arg_count_error() {
    assert!(run("let r = id()\n").is_err());
    assert!(run("let r = id(1, 2)\n").is_err());
}

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

// ---------------------------------------------------------------------------
// Async tests
// ---------------------------------------------------------------------------

/// async_manager_constructor のテスト。
#[test]
fn test_async_manager_constructor() {
    let val = run_get("mut mng = AsyncManager(num_thread=2)\n", "mng");
    assert!(matches!(val, Value::AsyncManager(_)));
}

/// async_manager_num_thread_attr のテスト。
#[test]
fn test_async_manager_num_thread_attr() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=4)\nlet n = mng.num_thread\n",
        "n",
    );
    assert!(matches!(val, Value::UInt(4)));
}

/// async_manager_raise_immediately_default_false のテスト。
#[test]
fn test_async_manager_raise_immediately_default_false() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=1)\nlet r = mng.raise_immediately\n",
        "r",
    );
    assert!(matches!(val, Value::Bool(false)));
}

/// async_manager_raise_immediately_set のテスト。
#[test]
fn test_async_manager_raise_immediately_set() {
    let val = run_get("mut mng = AsyncManager(num_thread=1, raise_immediately=True)\nlet r = mng.raise_immediately\n", "r");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_single_task_result のテスト。
#[test]
fn test_async_single_task_result() {
    let val = run_get(
        "mut mng = AsyncManager(num_thread=1)\nmng <- async->int:\n    block_return 42\nmng.wait_for_finish()\nlet r = mng.results\n",
        "r",
    );
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::Int(42)));
    } else {
        panic!("expected list");
    }
}

/// async_multiple_tasks_all_done のテスト。
#[test]
fn test_async_multiple_tasks_all_done() {
    let src = "
mut mng = AsyncManager(num_thread=2)
mng <- async->int:
    block_return 1
mng <- async->int:
    block_return 2
mng.wait_for_finish()
let done = mng.all_done()
";
    let val = run_get(src, "done");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_task_captures_env のテスト。
#[test]
fn test_async_task_captures_env() {
    let src = "
let x = 100
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    block_return x
mng.wait_for_finish()
let r = mng.results
";
    let val = run_get(src, "r");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::Int(100)));
    } else {
        panic!("expected list");
    }
}

/// async_error_stored_in_error_list のテスト。
#[test]
fn test_async_error_stored_in_error_list() {
    let src = "
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    raise RuntimeError(\"TestError\")
mng.wait_for_finish()
let errs = mng.error_list
";
    let val = run_get(src, "errs");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(!matches!(items[0], Value::None));
    } else {
        panic!("expected list");
    }
}

/// async_no_error_gives_none_in_error_list のテスト。
#[test]
fn test_async_no_error_gives_none_in_error_list() {
    let src = "
mut mng = AsyncManager(num_thread=1)
mng <- async->int:
    block_return 7
mng.wait_for_finish()
let errs = mng.error_list
";
    let val = run_get(src, "errs");
    if let Value::List(items) = val {
        let items = items.borrow();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Value::None));
    } else {
        panic!("expected list");
    }
}

/// async_raise_immediately_propagates_to_try_except のテスト。
#[test]
fn test_async_raise_immediately_propagates_to_try_except() {
    let src = "
mut mng = AsyncManager(num_thread=1, raise_immediately=True)
mng <- async->int:
    raise RuntimeError(\"AsyncFail\")
mut caught = False
try:
    mng.wait_for_finish()
except:
    caught = True
";
    let val = run_get(src, "caught");
    assert!(matches!(val, Value::Bool(true)));
}

/// async_status_enum_values のテスト。
#[test]
fn test_async_status_enum_values() {
    let w = run_get("let w = Async.Waiting\n", "w");
    let r = run_get("let r = Async.Running\n", "r");
    let d = run_get("let d = Async.Done\n", "d");
    assert!(matches!(
        w,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Waiting)
    ));
    assert!(matches!(
        r,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Running)
    ));
    assert!(matches!(
        d,
        Value::AsyncStatusVal(async_mgr::AsyncStatus::Done)
    ));
}

/// async_wrong_target_type_error のテスト。
#[test]
fn test_async_wrong_target_type_error() {
    let src = "
let x = 5
x <- async->int:
    block_return 1
";
    assert!(run(src).is_err());
}

// ---------------------------------------------------------------------------
// Tuple unpack tests
// ---------------------------------------------------------------------------

/// tuple_unpack_basic のテスト。
#[test]
fn test_tuple_unpack_basic() {
    let src = "
let a = (1, 2)
let x, mut y = a
";
    assert!(matches!(run_get(src, "x"), Value::Int(1)));
    assert!(matches!(run_get(src, "y"), Value::Int(2)));
}

/// tuple_unpack_immutable のテスト。
#[test]
fn test_tuple_unpack_immutable() {
    let src = "
let x, let y = (10, 20)
";
    assert!(matches!(run_get(src, "x"), Value::Int(10)));
    assert!(matches!(run_get(src, "y"), Value::Int(20)));
}

/// tuple_unpack_mut_is_mutable のテスト。
#[test]
fn test_tuple_unpack_mut_is_mutable() {
    let src = "
let x, mut y = (1, 2)
y = 99
";
    assert!(matches!(run_get(src, "y"), Value::Int(99)));
}

/// tuple_unpack_wildcard のテスト。
#[test]
fn test_tuple_unpack_wildcard() {
    let src = "
let a = (10, 20, 30, 40)
let p, mut q, _ = a
";
    assert!(matches!(run_get(src, "p"), Value::Int(10)));
    assert!(matches!(run_get(src, "q"), Value::Int(20)));
}

/// tuple_unpack_wildcard_two_remaining のテスト。
#[test]
fn test_tuple_unpack_wildcard_two_remaining() {
    let src = "
let p, _ = (5, 6, 7, 8)
";
    assert!(matches!(run_get(src, "p"), Value::Int(5)));
}

/// tuple_unpack_arity_mismatch_runtime のテスト。
#[test]
fn test_tuple_unpack_arity_mismatch_runtime() {
    // Static check catches tuple literals; for dynamic RHS the runtime catches it
    let src = "
fn get() -> tuple[int, int, int]:
    return (1, 2, 3)
let x, mut y = get()
";
    assert!(run(src).is_err());
}

/// tuple_unpack_non_tuple_error のテスト。
#[test]
fn test_tuple_unpack_non_tuple_error() {
    let src = "
let x, mut y = 42
";
    assert!(run(src).is_err());
}

/// tuple_unpack_static_missing_qualifier のテスト。
#[test]
fn test_tuple_unpack_static_missing_qualifier() {
    let src = "let x, y = (1, 2)";
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let errors = crate::type_check::TypeChecker::check(&stmts);
    assert!(
        !errors.is_empty(),
        "expected a StaticTypeError for missing qualifier"
    );
}

/// tuple_unpack_static_arity_mismatch のテスト。
#[test]
fn test_tuple_unpack_static_arity_mismatch() {
    let src = "let x, mut y = (1, 2, 3)";
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let stmts = crate::parser::Parser::new(tokens, None)
        .parse_program()
        .unwrap();
    let errors = crate::type_check::TypeChecker::check(&stmts);
    assert!(
        !errors.is_empty(),
        "expected a StaticTypeError for arity mismatch"
    );
}

// ---------------------------------------------------------------------------
// enumerate / zip tests
// ---------------------------------------------------------------------------

/// enumerate_basic のテスト。
#[test]
fn test_enumerate_basic() {
    // Check index and value sum: sum of (i + val) for [10,20,30] → (0+10)+(1+20)+(2+30) = 63
    let src = "
mut total = 0
for i, v in enumerate([10, 20, 30]):
    total = total + i + v
";
    assert!(matches!(run_get(src, "total"), Value::Int(63)));
}

/// enumerate_start のテスト。
#[test]
fn test_enumerate_start() {
    let src = "
mut first_idx = 0
for i, v in enumerate([10, 20], start=5):
    first_idx = i
    break
";
    assert!(matches!(run_get(src, "first_idx"), Value::Int(5)));
}

/// enumerate_for_unpack のテスト。
#[test]
fn test_enumerate_for_unpack() {
    // sum of idx + val for enumerate([100,200,300]) = (0+100)+(1+200)+(2+300) = 603
    let src = "
mut sum = 0
for idx, val in enumerate([100, 200, 300]):
    sum = sum + idx + val
";
    assert!(matches!(run_get(src, "sum"), Value::Int(603)));
}

/// zip_basic のテスト。
#[test]
fn test_zip_basic() {
    // sum of a + b for zip([1,2,3],[10,20,30]) = 11+22+33 = 66
    let src = "
mut total = 0
for a, b in zip([1, 2, 3], [10, 20, 30]):
    total = total + a + b
";
    assert!(matches!(run_get(src, "total"), Value::Int(66)));
}

/// zip_stops_at_shortest のテスト。
#[test]
fn test_zip_stops_at_shortest() {
    let src = "
mut count = 0
for a, b in zip([1, 2, 3, 4], [10, 20]):
    count = count + 1
";
    assert!(matches!(run_get(src, "count"), Value::Int(2)));
}

/// zip_three のテスト。
#[test]
fn test_zip_three() {
    let src = "
mut last_sum = 0
for x, y, z in zip([1, 2], [10, 20], [100, 200]):
    last_sum = x + y + z
";
    assert!(matches!(run_get(src, "last_sum"), Value::Int(222)));
}

/// zip_empty のテスト。
#[test]
fn test_zip_empty() {
    assert!(run("for a, b in zip():\n    pass\n").is_ok());
}

/// for_tuple_target_mismatch_error のテスト。
#[test]
fn test_for_tuple_target_mismatch_error() {
    let src = "
for a, b in [(1, 2, 3)]:
    pass
";
    assert!(run(src).is_err());
}

/// for_single_target_still_works のテスト。
#[test]
fn test_for_single_target_still_works() {
    let src = "
mut s = 0
for x in [1, 2, 3, 4]:
    s = s + x
";
    assert!(matches!(run_get(src, "s"), Value::Int(10)));
}

/// tuple_iteration_in_for のテスト。
#[test]
fn test_tuple_iteration_in_for() {
    let src = "
mut s = 0
for x in (1, 2, 3):
    s = s + x
";
    assert!(matches!(run_get(src, "s"), Value::Int(6)));
}

// ─── String features ─────────────────────────────────────────────────────────

/// fstring_basic のテスト。
#[test]
fn test_fstring_basic() {
    let src = r#"
let name = "Alice"
let age = 30
let s = f"Hello, {name}! Age: {age}"
"#;
    assert!(matches!(run_get(src, "s"), Value::Str(ref s) if s == "Hello, Alice! Age: 30"));
}

/// fstring_expr のテスト。
#[test]
fn test_fstring_expr() {
    let src = r#"
let x = 5
let y = 7
let s = f"sum = {x + y}"
"#;
    assert!(matches!(run_get(src, "s"), Value::Str(ref s) if s == "sum = 12"));
}

/// fstring_empty のテスト。
#[test]
fn test_fstring_empty() {
    let val = eval_expr(r#"f"""#);
    // empty fstring — lexer produces FStr([]) which desugars to ""
    assert!(matches!(val, Value::Str(ref s) if s.is_empty()));
}

/// raw_string のテスト。
#[test]
fn test_raw_string() {
    // r"" should not process escape sequences
    let val = eval_expr(r#"r"\n\t""#);
    assert!(matches!(val, Value::Str(ref s) if s == r"\n\t"));
}

/// math_string_superscript のテスト。
#[test]
fn test_math_string_superscript() {
    let val = eval_expr(r#"m"x^2""#);
    assert!(matches!(val, Value::Str(ref s) if s == "x²"));
}

/// math_string_subscript のテスト。
#[test]
fn test_math_string_subscript() {
    let val = eval_expr(r#"m"x_0""#);
    assert!(matches!(val, Value::Str(ref s) if s == "x₀"));
}

/// math_string_greek のテスト。
#[test]
fn test_math_string_greek() {
    let val = eval_expr(r#"m"\alpha + \beta""#);
    assert!(matches!(val, Value::Str(ref s) if s == "α + β"));
}

/// dollar_math_string のテスト。
#[test]
fn test_dollar_math_string() {
    let val = eval_expr("$x^2 + y^2$");
    assert!(matches!(val, Value::Str(ref s) if s == "x² + y²"));
}

/// str_upper_lower のテスト。
#[test]
fn test_str_upper_lower() {
    assert!(matches!(eval_expr(r#""hello".upper()"#), Value::Str(ref s) if s == "HELLO"));
    assert!(matches!(eval_expr(r#""WORLD".lower()"#), Value::Str(ref s) if s == "world"));
}

/// str_strip のテスト。
#[test]
fn test_str_strip() {
    assert!(matches!(eval_expr(r#""  hi  ".strip()"#), Value::Str(ref s) if s == "hi"));
    assert!(matches!(eval_expr(r#""  hi  ".lstrip()"#), Value::Str(ref s) if s == "hi  "));
    assert!(matches!(eval_expr(r#""  hi  ".rstrip()"#), Value::Str(ref s) if s == "  hi"));
}

/// str_split_join のテスト。
#[test]
fn test_str_split_join() {
    let src = r#"let parts = "a,b,c".split(",")"#;
    let val = run_get(src, "parts");
    if let Value::List(lst) = val {
        let items = lst.borrow();
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Value::Str(s) if s == "a"));
        assert!(matches!(&items[1], Value::Str(s) if s == "b"));
        assert!(matches!(&items[2], Value::Str(s) if s == "c"));
    } else {
        panic!("expected list");
    }
    assert!(matches!(eval_expr(r#"",".join(["x", "y", "z"])"#), Value::Str(ref s) if s == "x,y,z"));
}

/// str_replace のテスト。
#[test]
fn test_str_replace() {
    assert!(
        matches!(eval_expr(r#""hello world".replace("world", "Rust")"#), Value::Str(ref s) if s == "hello Rust")
    );
    assert!(matches!(eval_expr(r#""aaa".replace("a", "b", 2)"#), Value::Str(ref s) if s == "bba"));
}

/// str_find のテスト。
#[test]
fn test_str_find() {
    assert!(matches!(eval_expr(r#""hello".find("ll")"#), Value::Int(2)));
    assert!(matches!(
        eval_expr(r#""hello".find("xyz")"#),
        Value::Int(-1)
    ));
}

/// str_startswith_endswith のテスト。
#[test]
fn test_str_startswith_endswith() {
    assert!(matches!(
        eval_expr(r#""hello".startswith("he")"#),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval_expr(r#""hello".endswith("lo")"#),
        Value::Bool(true)
    ));
    assert!(matches!(
        eval_expr(r#""hello".startswith("lo")"#),
        Value::Bool(false)
    ));
}

/// str_count のテスト。
#[test]
fn test_str_count() {
    assert!(matches!(
        eval_expr(r#""banana".count("an")"#),
        Value::Int(2)
    ));
}

/// str_format のテスト。
#[test]
fn test_str_format() {
    assert!(
        matches!(eval_expr(r#""Hello, {}!".format("World")"#), Value::Str(ref s) if s == "Hello, World!")
    );
    assert!(matches!(eval_expr(r#""{:.2f}".format(3.14159)"#), Value::Str(ref s) if s == "3.14"));
    assert!(matches!(eval_expr(r#""{0} + {1}".format(1, 2)"#), Value::Str(ref s) if s == "1 + 2"));
}

/// str_is_checks のテスト。
#[test]
fn test_str_is_checks() {
    assert!(matches!(eval_expr(r#""123".isdigit()"#), Value::Bool(true)));
    assert!(matches!(eval_expr(r#""abc".isalpha()"#), Value::Bool(true)));
    assert!(matches!(
        eval_expr(r#""abc123".isalnum()"#),
        Value::Bool(true)
    ));
    assert!(matches!(eval_expr(r#""   ".isspace()"#), Value::Bool(true)));
    assert!(matches!(eval_expr(r#""ABC".isupper()"#), Value::Bool(true)));
    assert!(matches!(eval_expr(r#""abc".islower()"#), Value::Bool(true)));
}

/// str_zfill_ljust_rjust_center のテスト。
#[test]
fn test_str_zfill_ljust_rjust_center() {
    assert!(matches!(eval_expr(r#""42".zfill(5)"#), Value::Str(ref s) if s == "00042"));
    assert!(matches!(eval_expr(r#""hi".ljust(6)"#), Value::Str(ref s) if s == "hi    "));
    assert!(matches!(eval_expr(r#""hi".rjust(6)"#), Value::Str(ref s) if s == "    hi"));
    assert!(matches!(eval_expr(r#""hi".center(6)"#), Value::Str(ref s) if s == "  hi  "));
}

/// str_partition のテスト。
#[test]
fn test_str_partition() {
    let src = r#"let t = "one:two:three".partition(":")"#;
    let val = run_get(src, "t");
    if let Value::Tuple(t) = val {
        let vals = t.all_values();
        assert!(matches!(&vals[0], Value::Str(s) if s == "one"));
        assert!(matches!(&vals[1], Value::Str(s) if s == ":"));
        assert!(matches!(&vals[2], Value::Str(s) if s == "two:three"));
    } else {
        panic!("expected tuple");
    }
}

/// str_removeprefix_removesuffix のテスト。
#[test]
fn test_str_removeprefix_removesuffix() {
    assert!(
        matches!(eval_expr(r#""Hello, World!".removeprefix("Hello, ")"#), Value::Str(ref s) if s == "World!")
    );
    assert!(
        matches!(eval_expr(r#""Hello, World!".removesuffix(", World!")"#), Value::Str(ref s) if s == "Hello")
    );
}

/// str_title_capitalize_swapcase のテスト。
#[test]
fn test_str_title_capitalize_swapcase() {
    assert!(
        matches!(eval_expr(r#""hello world".title()"#), Value::Str(ref s) if s == "Hello World")
    );
    assert!(matches!(eval_expr(r#""hello".capitalize()"#), Value::Str(ref s) if s == "Hello"));
    assert!(
        matches!(eval_expr(r#""Hello World".swapcase()"#), Value::Str(ref s) if s == "hELLO wORLD")
    );
}

/// percent_format_int のテスト。
#[test]
fn test_percent_format_int() {
    assert!(matches!(eval_expr(r#""%d" % 42"#), Value::Str(ref s) if s == "42"));
    assert!(matches!(eval_expr(r#""%05d" % 42"#), Value::Str(ref s) if s == "00042"));
    assert!(matches!(eval_expr(r#""%x" % 255"#), Value::Str(ref s) if s == "ff"));
}

/// percent_format_float のテスト。
#[test]
fn test_percent_format_float() {
    assert!(matches!(eval_expr(r#""%.2f" % 3.14159"#), Value::Str(ref s) if s == "3.14"));
}

/// percent_format_str のテスト。
#[test]
fn test_percent_format_str() {
    assert!(
        matches!(eval_expr(r#""%s world" % "hello""#), Value::Str(ref s) if s == "hello world")
    );
}

/// percent_format_tuple のテスト。
#[test]
fn test_percent_format_tuple() {
    assert!(
        matches!(eval_expr(r#""%s is %d" % ("Alice", 30)"#), Value::Str(ref s) if s == "Alice is 30")
    );
}

/// str_repeat のテスト。
#[test]
fn test_str_repeat() {
    assert!(matches!(eval_expr(r#""ha" * 3"#), Value::Str(ref s) if s == "hahaha"));
    assert!(matches!(eval_expr(r#"3 * "na""#), Value::Str(ref s) if s == "nanana"));
}

/// str_regex_findall のテスト。
#[test]
fn test_str_regex_findall() {
    let src = r#"let ms = "abc 123 def 456".findall(r"\d+")"#;
    let val = run_get(src, "ms");
    if let Value::List(lst) = val {
        let items = lst.borrow();
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Value::Str(s) if s == "123"));
        assert!(matches!(&items[1], Value::Str(s) if s == "456"));
    } else {
        panic!("expected list");
    }
}

/// str_regex_sub のテスト。
#[test]
fn test_str_regex_sub() {
    assert!(matches!(
        eval_expr(r#""foo123bar".sub(r"\d+", "NUM")"#),
        Value::Str(ref s) if s == "fooNUMbar"
    ));
}

/// str_regex_search のテスト。
#[test]
fn test_str_regex_search() {
    assert!(matches!(
        eval_expr(r#""hello 42 world".search(r"\d+")"#),
        Value::Str(ref s) if s == "42"
    ));
    assert!(matches!(
        eval_expr(r#""no digits".search(r"\d+")"#),
        Value::None
    ));
}

/// str_match のテスト。
#[test]
fn test_str_match() {
    assert!(matches!(
        eval_expr(r#""hello world".match(r"hello")"#),
        Value::Str(ref s) if s == "hello"
    ));
    // match anchors to start
    assert!(matches!(
        eval_expr(r#""hello world".match(r"world")"#),
        Value::None
    ));
}

// ── cast operator: new_type ──────────────────────────────────────────────────

/// cast_primitive_to_new_type_int のテスト。
#[test]
fn test_cast_primitive_to_new_type_int() {
    // 4 => MyInt should produce a MyInt instance wrapping 4
    let src = "new_type MyInt: int\nlet x = 4=>MyInt\n";
    let val = run_get(src, "x");
    if let Value::Instance(rc) = val {
        let b = rc.borrow();
        let inner = b.class.field_index.get("value").and_then(|&idx| {
            b.fields.get(idx).and_then(|s| s.as_ref().map(|(v, _)| v.clone()))
        });
        assert!(matches!(inner, Some(Value::Int(4))));
        assert_eq!(b.class.name, "MyInt");
    } else {
        panic!("expected Instance, got {:?}", val);
    }
}

/// cast_primitive_to_new_type_float のテスト。
#[test]
fn test_cast_primitive_to_new_type_float() {
    let src = "new_type Meters: float\nlet m = 2.5=>Meters\n";
    let val = run_get(src, "m");
    if let Value::Instance(rc) = val {
        let b = rc.borrow();
        let inner = b.class.field_index.get("value").and_then(|&idx| {
            b.fields.get(idx).and_then(|s| s.as_ref().map(|(v, _)| v.clone()))
        });
        assert!(matches!(inner, Some(Value::Float(f)) if (f - 2.5).abs() < 1e-10));
        assert_eq!(b.class.name, "Meters");
    } else {
        panic!("expected Instance");
    }
}

/// cast_new_type_instance_to_base_int のテスト。
#[test]
fn test_cast_new_type_instance_to_base_int() {
    // MyInt(7) => int should return the inner int value 7
    let src = "new_type MyInt: int\nlet inst = MyInt(7)\nlet x = inst=>int\n";
    let val = run_get(src, "x");
    assert!(matches!(val, Value::Int(7)));
}

/// cast_new_type_instance_to_base_float のテスト。
#[test]
fn test_cast_new_type_instance_to_base_float() {
    let src = "new_type Meters: float\nlet m = Meters(3.0)\nlet f = m=>float\n";
    let val = run_get(src, "f");
    assert!(matches!(val, Value::Float(f) if (f - 3.0).abs() < 1e-10));
}

/// cast_cross_new_type_same_base のテスト。
#[test]
fn test_cast_cross_new_type_same_base() {
    // MyInt(9) => YourInt should produce YourInt(9), not YourInt(MyInt(9))
    let src = "new_type MyInt: int\nnew_type YourInt: int\nlet a = MyInt(9)=>YourInt\n";
    let val = run_get(src, "a");
    if let Value::Instance(rc) = val {
        let b = rc.borrow();
        assert_eq!(b.class.name, "YourInt");
        let inner = b.class.field_index.get("value").and_then(|&idx| {
            b.fields.get(idx).and_then(|s| s.as_ref().map(|(v, _)| v.clone()))
        });
        assert!(
            matches!(inner, Some(Value::Int(9))),
            "inner value should be 9, not a nested instance"
        );
    } else {
        panic!("expected Instance");
    }
}

// ============================================================================
// mustbe テスト
// ============================================================================

/// mustbe: プリミティブ型の成功ケース
#[test]
fn test_mustbe_primitive_pass() {
    let val = run_get("let a = 42 mustbe int\n", "a");
    assert!(matches!(val, Value::Int(42)));
}

/// mustbe: プリミティブ型の失敗ケース (TypeError が raise される)
#[test]
fn test_mustbe_primitive_fail() {
    let exc = run_exc("let a = \"hello\" mustbe int\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
            assert_eq!(rc.borrow().class.name, "TypeError");
        } else { panic!("expected TypeError instance"); }
}

/// mustbe: list の外側型チェック (成功)
#[test]
fn test_mustbe_list_pass() {
    let val = run_get("let a = [1, 2, 3] mustbe list\n", "a");
    assert!(matches!(val, Value::List(_)));
}

/// mustbe: list[int] の外側型チェック (list であれば合格・要素型は無視)
#[test]
fn test_mustbe_list_typed_pass() {
    let val = run_get("let a = [1, 2, 3] mustbe list[int]\n", "a");
    assert!(matches!(val, Value::List(_)));
}

/// mustbe: list の失敗ケース
#[test]
fn test_mustbe_list_fail() {
    let exc = run_exc("let a = 42 mustbe list\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: function の呼び出し可能チェック (成功)
#[test]
fn test_mustbe_function_pass() {
    let src = "fn double(let x: int) -> int:\n    return x * 2\nlet f = double mustbe function\n";
    let val = run_get(src, "f");
    assert!(matches!(val, Value::Function(_)));
}

/// mustbe: function の失敗ケース
#[test]
fn test_mustbe_function_fail() {
    let exc = run_exc("let a = 42 mustbe function\n").unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: __call__ を持つクラスは function チェックに合格する
#[test]
fn test_mustbe_class_with_call_passes_function() {
    let src = r#"
class Callable:
    fn __call__(self) -> int:
        return 1
let c = Callable
let f = c mustbe function
"#;
    let val = run_get(src, "f");
    assert!(matches!(val, Value::Class(_)));
}

/// mustbe: __call__ を持たないクラスは function チェックに失敗する
#[test]
fn test_mustbe_class_without_call_fails_function() {
    let src = "class Plain:\n    let x: int\nlet c = Plain\nlet f = c mustbe function\n";
    let exc = run_exc(src).unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}

/// mustbe: Any 型の値を int に絞り込む
#[test]
fn test_mustbe_any_to_int() {
    // Any として返す関数を模擬
    let src = r#"
fn get_val() -> int:
    return 99
let x: Any = get_val()
let a = x mustbe int
"#;
    let val = run_get(src, "a");
    assert!(matches!(val, Value::Int(99)));
}

/// mustbe: 添字アクセスでの型推論連携
#[test]
fn test_mustbe_list_subscript_inference() {
    // ランタイム: list[int] のアサーションと添字アクセス
    let src = "let xs = [10, 20, 30] mustbe list[int]\nlet b = xs[1]\n";
    let val = run_get(src, "b");
    assert!(matches!(val, Value::Int(20)));
}

/// mustbe: Protocol のフィールドチェック (成功)
#[test]
fn test_mustbe_protocol_pass() {
    let src = r#"
protocol Greetable:
    fn greet(self) -> str:
        ...

class Person:
    let name: str
    fn __init__(mut self, let n: str) -> None:
        self.name = n
    fn greet(self) -> str:
        return "Hello"

let p = Person("Alice")
let g = p mustbe Greetable
"#;
    let val = run_get(src, "g");
    assert!(matches!(val, Value::Instance(_)));
}

/// mustbe: Protocol のフィールドチェック (失敗: greet がない)
#[test]
fn test_mustbe_protocol_fail() {
    let src = r#"
protocol Greetable:
    fn greet(self) -> str:
        ...

class Robot:
    let id: int
    fn __init__(mut self, let i: int) -> None:
        self.id = i

let r = Robot(1)
let g = r mustbe Greetable
"#;
    let exc = run_exc(src).unwrap();
    assert!(exc.is_some());
    let err = exc.unwrap();
    if let Value::Instance(rc) = &err.exception {
        assert_eq!(rc.borrow().class.name, "TypeError");
    } else { panic!("expected TypeError instance"); }
}
