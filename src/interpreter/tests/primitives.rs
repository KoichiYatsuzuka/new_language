// tests/primitives.rs — uint プリミティブ型と id() 組み込み関数のテスト。

use super::*;

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
    assert!(matches!(val, Value::Str(s) if &*s == "255"));
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

