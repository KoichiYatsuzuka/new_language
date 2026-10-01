// type_check_tests/calls.rs — 関数呼び出し引数・型注釈欠落・キーワード引数・オーバーロードの型検査テスト。

use super::*;

    // --- Function call argument checking ---

    /// call_correct_types_ok のテスト。
    #[test]
    fn call_correct_types_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2)\n"));
    }

    /// call_arg_type_mismatch_err のテスト。
    #[test]
    fn call_arg_type_mismatch_err() {
        assert!(err(
            "fn add(a: int, b: int) -> int:\n    pass\nadd(1, \"hello\")\n"
        ));
    }

    /// call_arg_count_too_few_err のテスト。
    #[test]
    fn call_arg_count_too_few_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1)\n"));
    }

    /// call_arg_count_too_many_err のテスト。
    #[test]
    fn call_arg_count_too_many_err() {
        assert!(err(
            "fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2, 3)\n"
        ));
    }

    /// call_no_annotation_no_type_mismatch のテスト。
    #[test]
    fn call_no_annotation_no_type_mismatch() {
        let errors = check("fn f(x, y):\n    pass\nf(1, \"hello\")\n");
        assert!(!errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// call_unknown_arg_skipped_ok のテスト。
    #[test]
    fn call_unknown_arg_skipped_ok() {
        assert!(ok(
            "fn add(a: int, b: int) -> int:\n    pass\nmut x = 1\nadd(x, x)\n"
        ));
    }

    /// call_forward_definition_checked のテスト。
    #[test]
    fn call_forward_definition_checked() {
        assert!(err(
            "add(1, \"oops\")\nfn add(a: int, b: int) -> int:\n    pass\n"
        ));
    }

    /// call_return_type_inferred のテスト。
    #[test]
    fn call_return_type_inferred() {
        assert!(ok(
            "fn get_int() -> int:\n    pass\nlet v = get_int()\nv < 10\n"
        ));
    }

    /// error_display_call_count のテスト。
    #[test]
    fn error_display_call_count() {
        let errors = check("fn f(a: int, b: int) -> None:\n    pass\nf(1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("2"));
        assert!(msg.contains("1"));
    }

    /// error_display_call_type のテスト。
    #[test]
    fn error_display_call_type() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(\"hello\")\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("int"));
        assert!(msg.contains("str"));
    }

    // --- Missing type annotation ---

    /// fn_fully_annotated_ok のテスト。
    #[test]
    fn fn_fully_annotated_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\n"));
    }

    /// fn_missing_param_ann_err のテスト。
    #[test]
    fn fn_missing_param_ann_err() {
        assert!(err("fn f(x) -> int:\n    pass\n"));
    }

    /// fn_missing_return_ann_err のテスト。
    #[test]
    fn fn_missing_return_ann_err() {
        assert!(err("fn f(x: int):\n    pass\n"));
    }

    /// fn_missing_both_ann_err のテスト。
    #[test]
    fn fn_missing_both_ann_err() {
        let errors = check("fn f(x):\n    pass\n");
        assert_eq!(errors.len(), 2);
    }

    /// fn_multiple_missing_params_err のテスト。
    #[test]
    fn fn_multiple_missing_params_err() {
        let errors = check("fn f(a, b, c) -> int:\n    pass\n");
        assert_eq!(errors.len(), 3);
    }

    /// fn_no_params_missing_return_err のテスト。
    #[test]
    fn fn_no_params_missing_return_err() {
        assert!(err("fn greet():\n    pass\n"));
    }

    /// error_display_missing_param_ann のテスト。
    #[test]
    fn error_display_missing_param_ann() {
        let errors = check("fn f(x) -> int:\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("x"));
        assert!(msg.contains("f"));
    }

    /// error_display_missing_return_ann のテスト。
    #[test]
    fn error_display_missing_return_ann() {
        let errors = check("fn f(x: int):\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
    }

    // --- Keyword arguments ---

    /// kwarg_correct_ok のテスト。
    #[test]
    fn kwarg_correct_ok() {
        assert!(ok(
            "fn f(a: int, b: str) -> None:\n    pass\nf(a=1, b=\"hi\")\n"
        ));
    }

    /// kwarg_reversed_order_ok のテスト。
    #[test]
    fn kwarg_reversed_order_ok() {
        assert!(ok(
            "fn f(a: int, b: str) -> None:\n    pass\nf(b=\"hi\", a=1)\n"
        ));
    }

    /// kwarg_unknown_name_err のテスト。
    #[test]
    fn kwarg_unknown_name_err() {
        assert!(err(
            "fn f(a: int, b: int) -> None:\n    pass\nf(a=1, z=2)\n"
        ));
    }

    /// kwarg_type_mismatch_err のテスト。
    #[test]
    fn kwarg_type_mismatch_err() {
        assert!(err("fn f(a: int) -> None:\n    pass\nf(a=\"hello\")\n"));
    }

    /// kwarg_mixed_positional_keyword_ok のテスト。
    #[test]
    fn kwarg_mixed_positional_keyword_ok() {
        assert!(ok(
            "fn f(a: int, b: str) -> None:\n    pass\nf(1, b=\"hi\")\n"
        ));
    }

    /// error_display_unknown_kwarg のテスト。
    #[test]
    fn error_display_unknown_kwarg() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(z=1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("z"));
    }

    // --- Overloading ---

    /// overload_by_count_ok のテスト。
    #[test]
    fn overload_by_count_ok() {
        assert!(ok(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1)\n",
            "f(1, 2)\n",
        )));
    }

    /// overload_by_type_ok のテスト。
    #[test]
    fn overload_by_type_ok() {
        assert!(ok(concat!(
            "fn show(x: int) -> None:\n    pass\n",
            "fn show(x: str) -> None:\n    pass\n",
            "show(1)\n",
            "show(\"hi\")\n",
        )));
    }

    /// overload_wrong_count_err のテスト。
    #[test]
    fn overload_wrong_count_err() {
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind,
            TypeErrorKind::NoMatchingOverload { got: 3, .. }
        )));
    }

    /// overload_single_def_count_err_uses_count_mismatch のテスト。
    #[test]
    fn overload_single_def_count_err_uses_count_mismatch() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(1, 2)\n");
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgCountMismatch { .. })));
    }

    /// overload_multiple_count_match_skips_type_check のテスト。
    #[test]
    fn overload_multiple_count_match_skips_type_check() {
        let errors = check(concat!(
            "fn f(x: int) -> None:\n    pass\n",
            "fn f(x: str) -> None:\n    pass\n",
            "f(True)\n",
        ));
        assert!(!errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// overload_display_no_matching のテスト。
    #[test]
    fn overload_display_no_matching() {
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::NoMatchingOverload { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains('3'));
    }


    // --- 組み込み関数の戻り値の型（`src/built_in_stab/builtins.ars`）---
    //
    // 以前は `range` / `len` / `enumerate` / `zip` 以外の組み込みの結果がすべて `Unresolved`
    // （＝何でも通る）で、`let n: int = open(..)` すら静的に通っていた。

    /// 宣言ファイルが解析できて、実行時の組み込み関数が並んでいる（壊れると全部の型が消える）。
    #[test]
    fn builtins_ars_parses() {
        let names = crate::type_check::builtins::declared_names();
        for n in ["print", "open", "close", "repr", "getenv", "id", "range", "len"] {
            assert!(names.iter().any(|d| d == n), "builtins.ars に '{n}' が無い: {names:?}");
        }
    }

    /// `open` の結果は `FileObject`。`FileObject` は注釈・型の判定に書ける。
    #[test]
    fn open_returns_file_object() {
        assert!(ok(concat!(
            "let f: FileObject = open(\"a.txt\", FileOpenMode.read)\n",
            "if f is FileObject:\n    close(f)\n",
        )));
        let errors = check("let n: int = open(\"a.txt\", FileOpenMode.read)\n");
        assert!(
            errors.iter().any(|e| e.to_string().contains("FileObject")),
            "{errors:?}"
        );
    }

    /// 宣言に戻り値の型がある組み込みは、その型で検査される。
    #[test]
    fn builtin_return_types_are_checked() {
        assert!(err("let r: int = repr(1)\n"));
        assert!(err("let s: int = getenv(\"HOME\")\n"));
        assert!(err("let x: int = print(\"a\")\n"));
        assert!(ok("let r: str = repr(1)\nlet s: str = getenv(\"HOME\", \"none\")\n"));
        // `id` は `pointer`（実行時の `new_type pointer: uint` 相当のクラス）。
        assert!(ok("let p: pointer = id(1)\nif p is pointer:\n    print(p)\n"));
    }

    /// 宣言に戻り値の型が無い組み込み（`next` / `parse_ar`）は従来どおり型を付けない。
    /// `Any` にすると結果を使う式がすべて静的エラーになる（`builtins.ars` 冒頭の規則）。
    #[test]
    fn builtin_without_declared_return_type_stays_unknown() {
        assert!(ok(concat!(
            "gen g() -> int:\n    yield 1\n",
            "let it = g()\n",
            "let n: int = next(it) + 1\n",
        )));
    }

    /// 利用者が同じ名前を宣言したら、そちらが勝つ（組み込みの宣言は使わない）。
    #[test]
    fn user_function_shadows_builtin_declaration() {
        assert!(ok("fn repr(let v: int) -> int:\n    return v\nlet r: int = repr(1)\n"));
    }

    /// 型の変換（名前が型の名前）は宣言からは型を付けない（`int` などは「型の値を呼ぶ」規則）。
    #[test]
    fn conversions_are_not_typed_from_the_declarations() {
        assert!(crate::type_check::builtins::is_conversion("int"));
        assert!(crate::type_check::builtins::is_conversion("list"));
        assert!(!crate::type_check::builtins::is_conversion("open"));
        assert!(!crate::type_check::builtins::is_conversion("path"));
        assert!(err("let s: str = int(\"3\")\n"));
    }

    // --- 組み込みの型（`builtins.ars` の `enum` / `class`）---
    //
    // 以前は `FileOpenMode` などの組み込みの列挙と `FileObject` のメソッドに定義が無く、
    // `FileOpenMode.bogus` も `f.write(1)` も `f.nope()` も静的に通っていた。

    /// 組み込みの列挙は型の名前として書け、要素・`.value` が検査される。
    #[test]
    fn builtin_enums_are_typed() {
        assert!(ok(concat!(
            "let m: FileOpenMode = FileOpenMode.read\n",
            "let v: int = m.value\n",
            "let e: Encoding = Encoding.UTF_8\n",
        )));
        assert!(err("print(FileOpenMode.bogus)\n"));
        assert!(err("let s: str = StartPoint.top\n"));
        assert!(err("let m: FileOpenMode = Encoding.UTF_8\n"));
    }

    /// `FileObject` のメソッドは存在・引数の個数・引数の型・戻り値の型が検査される。
    /// 読みの結果（テキストなら `str`・バイトなら `list[int]`）は実行時にしか決まらないので型を付けない。
    #[test]
    fn file_object_methods_are_checked() {
        let open = "let f = open(\"a.txt\", FileOpenMode.rewrite)\n";
        assert!(ok(&format!(
            "{open}let s: str = f.read()\nlet t = f.read_line(backward = True)\nf.write(\"x\")\nf.write_line([1, 2])\n"
        )));
        assert!(err(&format!("{open}f.nope()\n")));
        assert!(err(&format!("{open}f.write(1)\n")));
        assert!(err(&format!("{open}f.read(True, False)\n")));
        assert!(err(&format!("{open}let n: int = f.write(\"a\")\n")));
    }

    /// 組み込みの `class` は Arrow のクラスではない（VM が `Value::Instance` と見なしてはいけない）。
    #[test]
    fn builtin_classes_are_not_arrow_classes() {
        let tokens = Lexer::new("let f = open(\"a.txt\", FileOpenMode.read)\n", "").tokenize();
        let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
        let (_, _, annotations) = TypeChecker::check_program(&stmts);
        assert!(!annotations.is_arrow_class("FileObject"));
    }

    // --- 組み込み関数の引数（`builtins.ars` の仮引数）---
    //
    // 以前は `range` / `len` 以外の組み込みの引数を検査していなかった
    // （`open("a.txt", 1)` / `close("x")` / `getenv(1)` が実行時まで通っていた）。

    /// 個数・キーワード引数の名前・型を宣言で検査する。
    #[test]
    fn builtin_args_are_checked_from_declarations() {
        assert!(ok(concat!(
            "let f = open(\"a.txt\", FileOpenMode.read, StartPoint.top, encoding = Encoding.UTF_8)\n",
            "let g = open(path(\"b.txt\"), open_mode = FileOpenMode.rewrite)\n",
            "close(f)\nclose(g)\n",
            "let e: str = getenv(\"HOME\", \"none\")\n",
            "let r: str = repr([1])\n",
        )));
        assert!(err("let f = open(\"a.txt\", 1)\n"));
        assert!(err("let f = open(\"a.txt\")\n"));
        assert!(err("let f = open(\"a.txt\", FileOpenMode.read, mode = 1)\n"));
        assert!(err("close(\"x\")\n"));
        assert!(err("let e = getenv(1)\n"));
        assert!(err("let r = repr(1, 2)\n"));
        assert!(err("for i, x in enumerate([1], start = \"a\"):\n    print(i)\n"));
    }

    /// 個数自由の位置引数（`let ...: T`）は何個でも通し、キーワード引数・`... =` を弾く。
    #[test]
    fn variadic_builtins_take_positional_args_only() {
        assert!(ok("print()\nprint(1, \"a\", [2])\nfor t in zip([1], [\"a\"], [True]):\n    print(t)\n"));
        assert!(err("print(1, sep = \" \")\n"));
        assert!(err("print(... = 1, 2)\n"));
    }

    /// `let ...: T` を持つ組み込みは固定の仮引数を前に持たない（`check_builtin_call` が前提にしている）。
    #[test]
    fn variadic_builtins_have_no_fixed_params() {
        for name in crate::type_check::builtins::declared_names() {
            let Some(decl) = crate::type_check::builtins::call_decl(&name) else { continue };
            if decl.variadic.is_some() {
                assert!(decl.params.is_empty(), "'{name}' は `let ...: T` と固定の仮引数を両方持っている");
            }
        }
    }

    /// 同じ名前の利用者の関数があれば、そちらのシグネチャで検査する（組み込みの宣言は使わない）。
    #[test]
    fn user_function_shadows_builtin_args() {
        assert!(ok("fn close(let s: str) -> None:\n    print(s)\nclose(\"x\")\n"));
    }
