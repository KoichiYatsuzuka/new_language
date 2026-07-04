// tests/classes.rs — クラス定義・継承・メソッド、およびトレイトのテスト。

use super::*;
use crate::interpreter::*;

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

