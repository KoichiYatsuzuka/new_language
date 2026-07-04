// type_check_tests/comparison.rs — 順序比較演算子の型検査テスト。

use super::*;

    // --- Ordering comparison ---

    /// int_int_lt_ok のテスト。
    #[test]
    fn int_int_lt_ok() {
        assert!(ok("1 < 2"));
    }

    /// float_float_lt_ok のテスト。
    #[test]
    fn float_float_lt_ok() {
        assert!(ok("1.0 < 2.0"));
    }

    /// int_float_lt_ok のテスト。
    #[test]
    fn int_float_lt_ok() {
        assert!(ok("1 < 2.0"));
    }

    /// str_str_lt_ok のテスト。
    #[test]
    fn str_str_lt_ok() {
        assert!(ok(r#""a" < "b""#));
    }

    /// str_int_lt_err のテスト。
    #[test]
    fn str_int_lt_err() {
        assert!(err(r#""hello" < 42"#));
    }

    /// int_str_gt_err のテスト。
    #[test]
    fn int_str_gt_err() {
        assert!(err(r#"42 > "hello""#));
    }

    /// bool_int_lt_err のテスト。
    #[test]
    fn bool_int_lt_err() {
        assert!(err("True < 1"));
    }

    /// str_float_le_err のテスト。
    #[test]
    fn str_float_le_err() {
        assert!(err(r#""x" <= 1.5"#));
    }

    /// eq_different_types_ok のテスト。
    #[test]
    fn eq_different_types_ok() {
        assert!(ok(r#"1 == "hello""#));
    }

    /// neq_different_types_ok のテスト。
    #[test]
    fn neq_different_types_ok() {
        assert!(ok(r#"True != "x""#));
    }

    /// unknown_param_comparison_ok のテスト。
    #[test]
    fn unknown_param_comparison_ok() {
        let errors = check("fn f(x):\n    x < 1\n");
        assert!(!errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::IncompatibleComparison { .. })));
        let errors = check("fn f(x):\n    x < \"hello\"\n");
        assert!(!errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::IncompatibleComparison { .. })));
    }

    /// int_str_lt_is_error のテスト。
    #[test]
    fn int_str_lt_is_error() {
        assert!(err("mut x = 1\nx < \"hello\""));
    }

    /// collects_multiple_errors のテスト。
    #[test]
    fn collects_multiple_errors() {
        let errors = check("let a = 1\na = 2\nlet b = 1\nb = 3\n");
        assert_eq!(errors.len(), 2);
    }

    /// error_display_assign のテスト。
    #[test]
    fn error_display_assign() {
        let errors = check("let x = 1\nx = 2");
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("immutable"));
        assert!(errors[0].to_string().contains("x"));
    }

    /// error_display_comparison のテスト。
    #[test]
    fn error_display_comparison() {
        let errors = check(r#""a" < 1"#);
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("str"));
        assert!(errors[0].to_string().contains("int"));
    }

