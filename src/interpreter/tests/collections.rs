// tests/collections.rs — 辞書(dict)とタプルのテスト。

use super::*;

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
        assert!(matches!(t.get(1), Some(Value::Str(s)) if &**s == "hello"));
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

// ---------------------------------------------------------------------------
// list 同士・tuple 同士の大小比較（python_builtins_plan.md のタスク 1-3）
// ---------------------------------------------------------------------------

/// CPython の `list_richcompare` / `tuplerichcompare` と同じ結果か（最初に違う要素で決まり、無ければ長さ）。
#[test]
fn test_seq_ordering_follows_cpython() {
    let src = concat!(
        "let a = [1, 2] < [1, 3]\n",
        "let b = (1, 2) < (1, 2, 0)\n",
        "let c = (2, 1) > (1, 9)\n",
        "let d = [[1, 2], [3]] < [[1, 2], [4]]\n",
        "let e = (1, 2) <= (1, 2)\n",
        "let f = [1.5, 2] >= [1.5]\n",
        "fn lt(x: tuple[int, str], y: tuple[int, str]) -> bool:\n",
        "    return x < y\n",
        "let g = lt((1, \"b\"), (1, \"a\"))\n",
    );
    for (var, want) in [("a", true), ("b", true), ("c", true), ("d", true), ("e", true), ("f", true), ("g", false)] {
        assert!(matches!(run_get(src, var), Value::Bool(b) if b == want), "{var}");
    }
}

/// 比べられない要素に届いたときだけ `TypeError`（届かなければ決まる）。list と tuple は比べない。
/// ⚠ `native` 限定（変換器 `python_converter` を使う）。
#[cfg(feature = "native")]
#[test]
fn test_seq_ordering_type_errors() {
    use crate::ast::Stmt;
    // 静的検査を通らない形なので、Python から変換した関数（引数の型が無い）で確かめる。
    let py = "def lt(a, b):\n    return a < b\n";
    let body = crate::python_converter::convert_python_source(py, "cmp.py").expect("convert");
    let run = |main: &str| -> Result<Interpreter, String> {
        let mut stmts = vec![Stmt::Import {
            lang: "py".to_string(),
            module: vec!["cmp".to_string()],
            source_module: None,
            alias: Some("c".to_string()),
            body: body.clone(),
        }];
        stmts.extend(Parser::new(Lexer::new(main, "").tokenize(), None).parse_program()?);
        let (_e, _w, annotations) = crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
        let mut interp = Interpreter::new();
        interp.wire_resolution(
            annotations,
            crate::interpreter::resolver::toplevel_declared_globals(&stmts),
            crate::interpreter::GlobalsMode::Replace,
        );
        for st in &stmts {
            if let Err(e) = interp.exec(st) {
                return Err(match interp.take_current_exception() {
                    Some(r) if e == crate::interpreter::RAISE_SENTINEL => Interpreter::format_error_report(&r),
                    _ => e,
                });
            }
        }
        Ok(interp)
    };
    let i = run("let ok = c.lt((1, \"a\"), (2, 3))\n").expect("decided at the first element");
    assert!(matches!(i.get_val("ok"), Some(Value::Bool(true))));
    let e = run("let bad = c.lt((1, \"a\"), (1, 3))\n").err().expect("str vs int");
    assert!(e.contains("TypeError"), "{e}");
    let e = run("let bad = c.lt([1], (1,))\n").err().expect("list vs tuple");
    assert!(e.contains("TypeError"), "{e}");
}

// ---------------------------------------------------------------------------
// 辞書・ジェネレータの `__iter__`（python_builtins_plan.md のタスク 1-4）
// ---------------------------------------------------------------------------

/// `d.__iter__()` はキーを挿入順に返し、`g.__iter__()` は同じジェネレータを返す（CPython の `iter(g) is g`）。
#[test]
fn test_dict_and_generator_dunder_iter() {
    let src = concat!(
        "let d: dict[str, int] = {\"b\": 1, \"a\": 2}\n",
        "mut it = d.__iter__()\n",
        "let k1 = it.next()\n",
        "let k2 = it.next()\n",
        "gen nums() -> int:\n",
        "    yield 1\n",
        "    yield 2\n",
        "mut g = nums()\n",
        "mut same = g.__iter__()\n",
        "let first = same.next()\n",
        "let second = g.next()\n",
    );
    assert_str(run_get(src, "k1"), "b");
    assert_str(run_get(src, "k2"), "a");
    assert!(matches!(run_get(src, "first"), Value::Int(1)));
    // 写しではないので、`same` で 1 つ進めると `g` の次は 2。
    assert!(matches!(run_get(src, "second"), Value::Int(2)));
}
