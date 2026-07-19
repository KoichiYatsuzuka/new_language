// tests/instances.rs — インスタンスの不変性(let束縛)、freeze文、Self型、new_type のテスト。

use super::*;

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

