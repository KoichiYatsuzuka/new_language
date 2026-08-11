// tests/unpacking.rs — タプルアンパックと enumerate / zip のテスト。

use super::*;

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
    assert!(matches!(run_get(src, "s"), Value::Str(ref s) if &**s == "Hello, Alice! Age: 30"));
}

/// fstring_expr のテスト。
#[test]
fn test_fstring_expr() {
    let src = r#"
let x = 5
let y = 7
let s = f"sum = {x + y}"
"#;
    assert!(matches!(run_get(src, "s"), Value::Str(ref s) if &**s == "sum = 12"));
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
    assert!(matches!(val, Value::Str(ref s) if &**s == r"\n\t"));
}

/// math_string_superscript のテスト。
#[test]
fn test_math_string_superscript() {
    let val = eval_expr(r#"m"x^2""#);
    assert!(matches!(val, Value::Str(ref s) if &**s == "x²"));
}

/// math_string_subscript のテスト。
#[test]
fn test_math_string_subscript() {
    let val = eval_expr(r#"m"x_0""#);
    assert!(matches!(val, Value::Str(ref s) if &**s == "x₀"));
}

/// math_string_greek のテスト。
#[test]
fn test_math_string_greek() {
    let val = eval_expr(r#"m"\alpha + \beta""#);
    assert!(matches!(val, Value::Str(ref s) if &**s == "α + β"));
}

/// dollar_math_string のテスト。
#[test]
fn test_dollar_math_string() {
    let val = eval_expr("$x^2 + y^2$");
    assert!(matches!(val, Value::Str(ref s) if &**s == "x² + y²"));
}

/// str_upper_lower のテスト。
#[test]
fn test_str_upper_lower() {
    assert!(matches!(eval_expr(r#""hello".upper()"#), Value::Str(ref s) if &**s == "HELLO"));
    assert!(matches!(eval_expr(r#""WORLD".lower()"#), Value::Str(ref s) if &**s == "world"));
}

/// str_strip のテスト。
#[test]
fn test_str_strip() {
    assert!(matches!(eval_expr(r#""  hi  ".strip()"#), Value::Str(ref s) if &**s == "hi"));
    assert!(matches!(eval_expr(r#""  hi  ".lstrip()"#), Value::Str(ref s) if &**s == "hi  "));
    assert!(matches!(eval_expr(r#""  hi  ".rstrip()"#), Value::Str(ref s) if &**s == "  hi"));
}

/// str_split_join のテスト。
#[test]
fn test_str_split_join() {
    let src = r#"let parts = "a,b,c".split(",")"#;
    let val = run_get(src, "parts");
    if let Value::List(lst) = val {
        let items = lst.borrow();
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Value::Str(s) if &**s == "a"));
        assert!(matches!(&items[1], Value::Str(s) if &**s == "b"));
        assert!(matches!(&items[2], Value::Str(s) if &**s == "c"));
    } else {
        panic!("expected list");
    }
    assert!(matches!(eval_expr(r#"",".join(["x", "y", "z"])"#), Value::Str(ref s) if &**s == "x,y,z"));
}

/// str_replace のテスト。
#[test]
fn test_str_replace() {
    assert!(
        matches!(eval_expr(r#""hello world".replace("world", "Rust")"#), Value::Str(ref s) if &**s == "hello Rust")
    );
    assert!(matches!(eval_expr(r#""aaa".replace("a", "b", 2)"#), Value::Str(ref s) if &**s == "bba"));
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
        matches!(eval_expr(r#""Hello, {}!".format("World")"#), Value::Str(ref s) if &**s == "Hello, World!")
    );
    assert!(matches!(eval_expr(r#""{:.2f}".format(3.14159)"#), Value::Str(ref s) if &**s == "3.14"));
    assert!(matches!(eval_expr(r#""{0} + {1}".format(1, 2)"#), Value::Str(ref s) if &**s == "1 + 2"));
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
    assert!(matches!(eval_expr(r#""42".zfill(5)"#), Value::Str(ref s) if &**s == "00042"));
    assert!(matches!(eval_expr(r#""hi".ljust(6)"#), Value::Str(ref s) if &**s == "hi    "));
    assert!(matches!(eval_expr(r#""hi".rjust(6)"#), Value::Str(ref s) if &**s == "    hi"));
    assert!(matches!(eval_expr(r#""hi".center(6)"#), Value::Str(ref s) if &**s == "  hi  "));
}

/// str_partition のテスト。
#[test]
fn test_str_partition() {
    let src = r#"let t = "one:two:three".partition(":")"#;
    let val = run_get(src, "t");
    if let Value::Tuple(t) = val {
        let vals = t.all_values();
        assert!(matches!(&vals[0], Value::Str(s) if &**s == "one"));
        assert!(matches!(&vals[1], Value::Str(s) if &**s == ":"));
        assert!(matches!(&vals[2], Value::Str(s) if &**s == "two:three"));
    } else {
        panic!("expected tuple");
    }
}

/// str_removeprefix_removesuffix のテスト。
#[test]
fn test_str_removeprefix_removesuffix() {
    assert!(
        matches!(eval_expr(r#""Hello, World!".removeprefix("Hello, ")"#), Value::Str(ref s) if &**s == "World!")
    );
    assert!(
        matches!(eval_expr(r#""Hello, World!".removesuffix(", World!")"#), Value::Str(ref s) if &**s == "Hello")
    );
}

/// str_title_capitalize_swapcase のテスト。
#[test]
fn test_str_title_capitalize_swapcase() {
    assert!(
        matches!(eval_expr(r#""hello world".title()"#), Value::Str(ref s) if &**s == "Hello World")
    );
    assert!(matches!(eval_expr(r#""hello".capitalize()"#), Value::Str(ref s) if &**s == "Hello"));
    assert!(
        matches!(eval_expr(r#""Hello World".swapcase()"#), Value::Str(ref s) if &**s == "hELLO wORLD")
    );
}

/// percent_format_int のテスト。
#[test]
fn test_percent_format_int() {
    assert!(matches!(eval_expr(r#""%d" % 42"#), Value::Str(ref s) if &**s == "42"));
    assert!(matches!(eval_expr(r#""%05d" % 42"#), Value::Str(ref s) if &**s == "00042"));
    assert!(matches!(eval_expr(r#""%x" % 255"#), Value::Str(ref s) if &**s == "ff"));
}

/// percent_format_float のテスト。
#[test]
fn test_percent_format_float() {
    assert!(matches!(eval_expr(r#""%.2f" % 3.14159"#), Value::Str(ref s) if &**s == "3.14"));
}

/// percent_format_str のテスト。
#[test]
fn test_percent_format_str() {
    assert!(
        matches!(eval_expr(r#""%s world" % "hello""#), Value::Str(ref s) if &**s == "hello world")
    );
}

/// percent_format_tuple のテスト。
#[test]
fn test_percent_format_tuple() {
    assert!(
        matches!(eval_expr(r#""%s is %d" % ("Alice", 30)"#), Value::Str(ref s) if &**s == "Alice is 30")
    );
}

/// str_repeat のテスト。
#[test]
fn test_str_repeat() {
    assert!(matches!(eval_expr(r#""ha" * 3"#), Value::Str(ref s) if &**s == "hahaha"));
    assert!(matches!(eval_expr(r#"3 * "na""#), Value::Str(ref s) if &**s == "nanana"));
}

/// str_regex_findall のテスト。
#[test]
fn test_str_regex_findall() {
    let src = r#"let ms = "abc 123 def 456".findall(r"\d+")"#;
    let val = run_get(src, "ms");
    if let Value::List(lst) = val {
        let items = lst.borrow();
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Value::Str(s) if &**s == "123"));
        assert!(matches!(&items[1], Value::Str(s) if &**s == "456"));
    } else {
        panic!("expected list");
    }
}

/// str_regex_sub のテスト。
#[test]
fn test_str_regex_sub() {
    assert!(matches!(
        eval_expr(r#""foo123bar".sub(r"\d+", "NUM")"#),
        Value::Str(ref s) if &**s == "fooNUMbar"
    ));
}

/// str_regex_search のテスト。
#[test]
fn test_str_regex_search() {
    assert!(matches!(
        eval_expr(r#""hello 42 world".search(r"\d+")"#),
        Value::Str(ref s) if &**s == "42"
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
        Value::Str(ref s) if &**s == "hello"
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
            b.field_value(idx)
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
            b.field_value(idx)
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
            b.field_value(idx)
        });
        assert!(
            matches!(inner, Some(Value::Int(9))),
            "inner value should be 9, not a nested instance"
        );
    } else {
        panic!("expected Instance");
    }
}

