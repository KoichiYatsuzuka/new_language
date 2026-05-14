// tests.rs — インタープリタ単体テスト

use super::*;
use crate::ast::Stmt;
use crate::lexer::Lexer;
use crate::parser::Parser;

fn run(src: &str) -> Result<(), String> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program()?;
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt)?;
    }
    Ok(())
}

fn eval_expr(src: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    interp.eval(match &stmts[0] {
        Stmt::Expr(e) => e,
        _ => panic!("not an expr"),
    }).unwrap()
}

fn run_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

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

#[test]
fn test_arithmetic() {
    assert!(matches!(eval_expr("2 + 3"), Value::Int(5)));
    assert!(matches!(eval_expr("10 - 4"), Value::Int(6)));
    assert!(matches!(eval_expr("3 * 4"), Value::Int(12)));
    assert!(matches!(eval_expr("7 // 2"), Value::Int(3)));
    assert!(matches!(eval_expr("7 % 3"), Value::Int(1)));
    assert!(matches!(eval_expr("2 ** 10"), Value::Int(1024)));
}

#[test]
fn test_float_arithmetic() {
    if let Value::Float(f) = eval_expr("1.0 + 2.0") {
        assert!((f - 3.0).abs() < f64::EPSILON);
    } else {
        panic!();
    }
}

#[test]
fn test_string_concat() {
    if let Value::Str(s) = eval_expr(r#""hello" + " " + "world""#) {
        assert_eq!(s, "hello world");
    } else {
        panic!();
    }
}

#[test]
fn test_comparison() {
    assert!(matches!(eval_expr("1 < 2"), Value::Bool(true)));
    assert!(matches!(eval_expr("2 > 3"), Value::Bool(false)));
    assert!(matches!(eval_expr("4 == 4"), Value::Bool(true)));
    assert!(matches!(eval_expr("4 != 5"), Value::Bool(true)));
}

#[test]
fn test_logical() {
    assert!(matches!(eval_expr("True and False"), Value::Bool(false)));
    assert!(matches!(eval_expr("True or False"), Value::Bool(true)));
    assert!(matches!(eval_expr("not True"), Value::Bool(false)));
}

#[test]
fn test_let_immutable() {
    assert!(run("let x = 1\nx = 2").is_err());
}

#[test]
fn test_mut_mutable() {
    assert!(run("mut x = 1\nx = 2").is_ok());
}

#[test]
fn test_compound_assign() {
    if let Value::Int(n) = run_get("mut x = 10\nx += 5", "x") {
        assert_eq!(n, 15);
    } else {
        panic!();
    }
}

#[test]
fn test_print_runs() {
    assert!(run(r#"print("hello", "world")"#).is_ok());
}

#[test]
fn test_zero_division() {
    assert!(run("1 // 0").is_err());
}

// --- if ---

#[test]
fn test_if_true_branch() {
    if let Value::Int(n) = run_get("mut x = 0\nif True:\n    x = 1\n", "x") {
        assert_eq!(n, 1);
    } else {
        panic!();
    }
}

#[test]
fn test_if_false_else_branch() {
    if let Value::Int(n) = run_get("mut x = 0\nif False:\n    x = 1\nelse:\n    x = 2\n", "x") {
        assert_eq!(n, 2);
    } else {
        panic!();
    }
}

#[test]
fn test_if_scope_isolation() {
    assert!(run("if True:\n    let x = 1\nprint(x)\n").is_err());
}

// --- while ---

#[test]
fn test_while_loop() {
    if let Value::Int(n) = run_get("mut i = 0\nwhile i < 5:\n    i += 1\n", "i") {
        assert_eq!(n, 5);
    } else {
        panic!();
    }
}

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

#[test]
fn test_while_scope_isolation() {
    assert!(run("mut cond = True\nwhile cond:\n    let x = 1\n    cond = False\nprint(x)\n").is_err());
}

// --- for ---

#[test]
fn test_for_range() {
    if let Value::Int(n) = run_get("mut s = 0\nfor i in range(5):\n    s += i\n", "s") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

#[test]
fn test_for_list() {
    if let Value::Int(n) = run_get("mut s = 0\nfor x in [1, 2, 3]:\n    s += x\n", "s") {
        assert_eq!(n, 6);
    } else {
        panic!();
    }
}

#[test]
fn test_for_loop_var_scope_isolation() {
    assert!(run("for i in range(3):\n    pass\nprint(i)\n").is_err());
}

#[test]
fn test_for_body_scope_isolation() {
    assert!(run("for i in range(1):\n    let x = 99\nprint(x)\n").is_err());
}

// --- block ---

#[test]
fn test_block_scope_isolation() {
    assert!(run("block:\n    let x = 1\nprint(x)\n").is_err());
}

#[test]
fn test_block_reads_outer() {
    assert!(run("let x = 1\nblock:\n    print(x)\n").is_ok());
}

#[test]
fn test_block_modifies_outer() {
    if let Value::Int(n) = run_get("mut x = 0\nblock:\n    x = 42\n", "x") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

// --- builtins ---

#[test]
fn test_range_builtin() {
    if let Value::List(items) = eval_expr("range(3)") {
        assert_eq!(items.len(), 3);
    } else {
        panic!();
    }
}

#[test]
fn test_len_builtin() {
    assert!(matches!(eval_expr("len([1, 2, 3])"), Value::Int(3)));
}

// --- functions ---

#[test]
fn test_fn_call_returns_value() {
    let src = "fn add(a: int, b: int) -> int:\n    return a + b\nlet result = add(3, 4)\n";
    if let Value::Int(n) = run_get(src, "result") {
        assert_eq!(n, 7);
    } else {
        panic!();
    }
}

#[test]
fn test_fn_no_return_gives_none() {
    let src = "fn noop() -> None:\n    pass\nlet r = noop()\n";
    assert!(matches!(run_get(src, "r"), Value::None));
}

#[test]
fn test_fn_recursion() {
    let src = "fn fact(n: int) -> int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\nlet r = fact(5)\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 120);
    } else {
        panic!();
    }
}

#[test]
fn test_fn_kwarg_call() {
    let src = "fn sub(a: int, b: int) -> int:\n    return a - b\nlet r = sub(b=1, a=10)\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 9);
    } else {
        panic!();
    }
}

#[test]
fn test_fn_scope_isolation() {
    let src = "fn f() -> None:\n    let x = 99\nf()\n";
    assert!(run(&format!("{src}print(x)\n")).is_err());
}

// --- overloading ---

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

#[test]
fn test_overload_wrong_count_err() {
    let src = concat!(
        "fn f(x: int) -> None:\n    pass\n",
        "fn f(x: int, y: int) -> None:\n    pass\n",
        "f(1, 2, 3)\n",
    );
    assert!(run(src).is_err());
}

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

#[test]
fn test_class_instantiate() {
    // Fields have defaults → no required args → Point() is the right call.
    let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\nlet p = Point()\n";
    assert!(run(src).is_ok());
}

#[test]
fn test_class_instantiate_required_fields() {
    // Fields without defaults → auto-init requires args.
    let src = "class Point:\n    mut x: int\n    mut y: int\nlet p = Point(3, 4)\n";
    assert!(run(src).is_ok());
}

#[test]
fn test_class_init_sets_field() {
    let src = "class Dog:\n    mut name: str = \"\"\n    fn __init__(mut self, name: str) -> None:\n        self.name = name\nlet d = Dog(\"Rex\")\n";
    assert!(run(src).is_ok());
}

#[test]
fn test_class_method_call() {
    let src = "class Greeter:\n    fn greet(self) -> str:\n        return \"hello\"\nlet g = Greeter()\nlet r = g.greet()\n";
    if let Value::Str(s) = run_get(src, "r") {
        assert_eq!(s, "hello");
    } else {
        panic!();
    }
}

#[test]
fn test_class_field_access() {
    // Fields have defaults; use defaults when instantiating.
    let src = "class Pair:\n    mut x: int = 10\n    mut y: int = 20\nlet p = Pair()\nlet r = p.x\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

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

#[test]
fn test_class_self_field_in_method() {
    let src = concat!(
        "class Box:\n",
        "    mut value: int = 0\n",
        "    fn set(mut self, v: int) -> None:\n",
        "        self.value = v\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "mut b = Box()\n",    // mut: instance will be mutated via set()
        "b.set(42)\n",
        "let r = b.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

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
    assert!(result.is_err(), "expected parse error for class-to-class inheritance");
    assert!(result.unwrap_err().contains("cannot inherit from `Animal`"));
}

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
    assert!(result.is_err(), "expected parse error for class-to-class inheritance");
    assert!(result.unwrap_err().contains("cannot inherit from `Base`"));
}

// --- trait ---

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
    assert!(run(src).is_err(), "calling mut self method on let instance must fail");
}

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
    assert!(err.contains("bar"), "error should mention method name, got: {err}");
    assert!(err.contains("immutable"), "error should mention immutable, got: {err}");
}

// --- freeze statement ---

#[test]
fn test_freeze_makes_variable_immutable() {
    // After freeze, reassigning the variable itself must fail
    let src = concat!(
        "class Foo:\n",
        "    mut x: int = 0\n",
        "mut f = Foo()\n",
        "freeze f\n",
        "f = Foo()\n",   // reassign the variable
    );
    assert!(run(src).is_err(), "reassigning a frozen variable must fail");
}

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
    assert!(run(src).is_err(), "calling mut self method on frozen instance must fail");
}

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

#[test]
fn test_freeze_on_undefined_variable_errors() {
    assert!(run("freeze x\n").is_err(), "freeze on undefined variable must fail");
}

// --- Self type ---

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

#[test]
fn test_self_type_outside_class_is_parse_error() {
    // `Self` used outside a class or trait must produce a parse error.
    let tokens = crate::lexer::Lexer::new("fn foo() -> Self:\n    pass\n", "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(result.is_err(), "Self outside class/trait must be a parse error");
    assert!(result.unwrap_err().contains("'Self'"), "error should mention 'Self'");
}

#[test]
fn test_self_type_in_expression_outside_class_is_parse_error() {
    // `Self` as an expression outside a class must produce a parse error.
    let tokens = crate::lexer::Lexer::new("Self(42)\n", "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(result.is_err(), "Self expression outside class/trait must be a parse error");
}

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
        "mut c = Counter(0)\n",   // mut: instance will be mutated via increment()
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
    assert!(result.is_err(), "expected parse error when reassigning a new_type binding");
}

// --- Exception handling tests ---

#[test]
fn test_raise_uncaught_reaches_caller() {
    let raised = run_exc("raise ValueError(\"oops\")\n").unwrap();
    let raised = raised.expect("expected a raised exception");
    if let Value::Instance(inst) = &raised.exception {
        assert_eq!(inst.borrow().class.name, "ValueError");
        let msg = match inst.borrow().fields.get("message") {
            Some((Value::Str(s), _)) => s.clone(),
            _ => panic!("message field missing or wrong type"),
        };
        assert_eq!(msg, "oops");
    } else {
        panic!("expected Instance");
    }
}

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

#[test]
fn test_try_except_does_not_catch_different_type() {
    let src = concat!(
        "try:\n",
        "    raise TypeError(\"t\")\n",
        "except ValueError as e:\n",
        "    pass\n",
    );
    let raised = run_exc(src).unwrap();
    assert!(raised.is_some(), "TypeError should not be caught by ValueError handler");
    if let Some(r) = raised {
        if let Value::Instance(inst) = &r.exception {
            assert_eq!(inst.borrow().class.name, "TypeError");
        }
    }
}

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

// --- iterator ---

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
    let stmts = crate::parser::Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    let mut err_msg = String::new();
    for stmt in &stmts {
        match interp.exec(stmt) {
            Err(e) => { err_msg = e; break; }
            _ => {}
        }
    }
    assert!(err_msg.starts_with("EndOfIteration"), "expected EndOfIteration, got: {err_msg}");
}

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

#[test]
fn test_dict_literal_empty() {
    let src = "let d = {}";
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_literal_with_entries() {
    let src = r#"let d = {"a": 1, "b": 2}"#;
    assert!(run(src).is_ok());
}

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

#[test]
fn test_dict_subscript_write() {
    let src = r#"mut d = {"a": 1}
d["a"] = 99"#;
    run(src).expect("should not fail");
}

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

#[test]
fn test_dict_key_not_found_error() {
    let src = r#"let d = {"a": 1}
let v = d["missing"]"#;
    assert!(run(src).is_err());
}

#[test]
fn test_dict_key_method() {
    let src = r#"let d = {1: "one", 2: "two"}
let ks = d.key()"#;
    if let Value::List(ks) = run_get(src, "ks") {
        assert_eq!(ks.len(), 2);
    } else {
        panic!("expected List");
    }
}

#[test]
fn test_dict_item_method() {
    let src = r#"let d = {1: "one", 2: "two"}
let vs = d.item()"#;
    if let Value::List(vs) = run_get(src, "vs") {
        assert_eq!(vs.len(), 2);
    } else {
        panic!("expected List");
    }
}

#[test]
fn test_dict_typed_constructor_empty() {
    let src = "let d = dict[str, int]()";
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_typed_constructor_from_literal() {
    let src = r#"let d = dict[str, int]({"hello": 1, "world": 2})"#;
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_typed_constructor_type_mismatch_key_err() {
    let src = r#"let d = dict[int, str]({1: "ok", "bad": "value"})"#;
    let err = run(src).expect_err("should fail with type mismatch");
    assert!(err.contains("StaticTypeError"), "got: {err}");
}

#[test]
fn test_dict_typed_constructor_type_mismatch_item_err() {
    let src = r#"let d = dict[str, int]({"ok": 1, "bad": "not_an_int"})"#;
    let err = run(src).expect_err("should fail with type mismatch");
    assert!(err.contains("StaticTypeError"), "got: {err}");
}

#[test]
fn test_dict_typed_write_type_check() {
    let src = r#"mut d = dict[str, int]()
d["key"] = 42"#;
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_typed_write_wrong_key_type_err() {
    let src = r#"mut d = dict[str, int]()
d[123] = 42"#;
    let err = run(src).expect_err("should fail type check");
    assert!(err.contains("TypeError"), "got: {err}");
}

#[test]
fn test_dict_typed_write_wrong_item_type_err() {
    let src = r#"mut d = dict[str, int]()
d["key"] = "not_int""#;
    let err = run(src).expect_err("should fail type check");
    assert!(err.contains("TypeError"), "got: {err}");
}

#[test]
fn test_dict_multiline_literal() {
    let src = "let d = {\n    \"a\": 1,\n    \"b\": 2\n}";
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_int_upcast_to_float_ok() {
    // int value is accepted where float is declared (upcast)
    let src = r#"mut d = dict[str, float]()
d["pi"] = 3"#;
    assert!(run(src).is_ok());
}

#[test]
fn test_dict_is_truthy_empty() {
    let src = "let d = {}\nlet t = not d";
    if let Value::Bool(b) = run_get(src, "t") {
        assert!(b); // empty dict is falsy
    } else {
        panic!("expected Bool");
    }
}

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

#[test]
fn test_tuple_empty() {
    let v = eval_expr("()");
    if let Value::Tuple(t) = v {
        assert!(t.is_empty());
    } else {
        panic!("expected Tuple");
    }
}

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

#[test]
fn test_tuple_grouped_expr_not_tuple() {
    // (expr) without comma is NOT a tuple
    let v = eval_expr("(42)");
    assert!(matches!(v, Value::Int(42)));
}

#[test]
fn test_tuple_display() {
    let src = r#"let t = (1, "a", True)"#;
    assert!(run(src).is_ok());
}

#[test]
fn test_tuple_equality() {
    let src = "let a = (1, 2)\nlet b = (1, 2)\nlet eq = a == b\n";
    if let Value::Bool(b) = run_get(src, "eq") {
        assert!(b);
    } else {
        panic!("expected Bool");
    }
}

#[test]
fn test_tuple_inequality_different_values() {
    let src = "let a = (1, 2)\nlet b = (1, 3)\nlet eq = a == b\n";
    if let Value::Bool(b) = run_get(src, "eq") {
        assert!(!b);
    } else {
        panic!("expected Bool");
    }
}

#[test]
fn test_tuple_truthy_nonempty() {
    let src = "let t = (1, 2)\nlet r = not t\n";
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(!b); // non-empty tuple is truthy
    } else {
        panic!("expected Bool");
    }
}

#[test]
fn test_tuple_falsy_empty() {
    let src = "let t = ()\nlet r = not t\n";
    if let Value::Bool(b) = run_get(src, "r") {
        assert!(b); // empty tuple is falsy
    } else {
        panic!("expected Bool");
    }
}

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
    for stmt in &stmts { interp.exec(stmt).unwrap(); }
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

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
    for stmt in &stmts { interp.exec(stmt).unwrap(); }
    assert!(matches!(interp.get_val("r_a").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r_b").unwrap(), Value::Int(101)));
}

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
    for stmt in &stmts { interp.exec(stmt).unwrap(); }
    assert!(matches!(interp.get_val("r1").unwrap(), Value::Int(1)));
    assert!(matches!(interp.get_val("r2").unwrap(), Value::Int(2)));
    assert!(matches!(interp.get_val("r3").unwrap(), Value::Int(3)));
}

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
