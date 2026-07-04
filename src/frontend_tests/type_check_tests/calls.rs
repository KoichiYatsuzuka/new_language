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

