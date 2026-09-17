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

    /// 異型の `==` は**静的エラー**（決定 D-15・タスク 7.6）。
    ///
    /// ⚠⚠ **このテストは以前 `eq_different_types_ok` という名前で `ok(..)` を主張していた。**
    /// タスク 4.4 の時点では「異型の等値比較は `False` を返す仕様」として厳密化を撤回して
    /// いたが、7.6 で実測し直したところ、その仕様を主張していたのは**例題 1 ファイル**だけで、
    /// 移行は全比較地点 237 のうち 10 箇所（4.2%）だった。⇒ 決定を覆した。
    ///
    /// ⚠ **実行時の意味論は変えていない。** 型が決まらない経路から到達すれば従来どおり
    /// `False` を返す（`examples/basics/equality_numeric_promotion.ar`）。
    #[test]
    fn eq_different_types_err() {
        assert!(err(r#"1 == "hello""#));
    }

    /// 異型の `!=` も同じ（`bool` は数値の昇格ラティスの対象外）。
    #[test]
    fn neq_different_types_err() {
        assert!(err(r#"True != "x""#));
    }

    /// 数値族（`int` / `float` / `complex`）は相互に比較できる。
    ///
    /// ⚠ 実行時が `uint → int → float` の昇格ラティスで比べるので、ここを厳密にすると
    /// `if n == 0`（`n: float`）のような自然な式が落ちる。
    #[test]
    fn eq_numeric_family_ok() {
        assert!(ok("1 == 1.0"));
    }

    /// `None` かどうかの判定は **`is None`**（`== None` ではない）。
    ///
    /// ⚠⚠ **`Option[T] == None` はタスク 7.6 より前から静的エラー**だった。
    /// `check_binop` の冒頭が「`Union` を被演算子にする二項演算」を一律
    /// `OperationOnUnion` で弾いているため（`==` も対象）。
    /// ⇒ Arrow に `== None` という作法はもともと無い。
    ///
    /// 利用者の決定「`Option` 以外の `None` を弾く」の効果は
    /// **「`int == None` のような非 Option との比較を弾く」**ことで、
    /// `Option` 側の書き方は変わらない。
    #[test]
    fn option_is_none_ok() {
        assert!(ok("let x: Option[int] = None\nif x is None:\n    pass\n"));
    }

    /// 非 `Option` 型と `None` の比較は静的エラー（利用者の決定・タスク 7.6）。
    #[test]
    fn eq_none_on_non_option_err() {
        assert!(err("let x: int = 1\nlet b = x == None\n"));
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

