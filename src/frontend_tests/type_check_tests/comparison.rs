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


    // --- 演算子オーバーロードの右辺（フェーズ10 10-13）---
    // ⚠ 以前は左辺がクラスだと素通しで、`Money(100) + 5` が `__add__` の中の `other.cents` で
    //   実行時の AttributeError になっていた。

    const MONEY: &str = concat!(
        "class Money:\n",
        "    mut cents: int\n",
        "    fn __add__(self, let other: Money) -> Money:\n",
        "        return Money(self.cents + other.cents)\n",
        "    fn __mul__(self, let k: int) -> Money:\n",
        "        return Money(self.cents * k)\n",
        "    fn __mul__(self, let k: float) -> Money:\n",
        "        return Money(int(float(self.cents) * k))\n",
        "    fn __lt__(self, let other: Money) -> bool:\n",
        "        return self.cents < other.cents\n",
    );

    fn binop_errors(src: &str) -> usize {
        check(src)
            .iter()
            .filter(|e| matches!(&e.kind, TypeErrorKind::IncompatibleBinOp { .. }))
            .count()
    }

    /// 右辺が `__add__` の仮引数に合わない（10-13 の再現）。
    #[test]
    fn dunder_operand_mismatch_err() {
        assert_eq!(binop_errors(&format!("{MONEY}let m = Money(100) + 5\n")), 1);
    }

    /// 順序比較の `__lt__` も同じ。
    #[test]
    fn dunder_cmp_operand_mismatch_err() {
        assert_eq!(binop_errors(&format!("{MONEY}let b = Money(1) < 3\n")), 1);
    }

    /// 多重定義はどれか 1 つが受ければ通す。受けるものが無ければ誤り。
    #[test]
    fn dunder_overloads() {
        assert!(ok(&format!("{MONEY}let a = Money(1) * 2\nlet b = Money(1) * 0.5\n")));
        assert_eq!(binop_errors(&format!("{MONEY}let c = Money(1) * Money(2)\n")), 1);
    }

    /// 合う右辺・複合代入は通る。
    #[test]
    fn dunder_operand_ok() {
        assert!(ok(&format!(
            "{MONEY}let a = Money(1) + Money(2)\nlet b = Money(1) < Money(2)\nmut t = Money(0)\nt += Money(3)\n"
        )));
    }

    /// 複合代入の右辺も検査する。
    #[test]
    fn dunder_compound_assign_mismatch_err() {
        assert_eq!(binop_errors(&format!("{MONEY}mut t = Money(0)\nt += 3\n")), 1);
    }

    // --- list 同士・tuple 同士の大小比較（python_builtins_plan.md のタスク 1-3）---

    /// 要素が比べられる list / tuple は比べられる。`[]` はどの list とも比べられる。
    #[test]
    fn seq_ordering_ok() {
        assert!(ok("let a: list[int] = [1, 2]\nlet b: list[int] = [1, 3]\nlet c = a < b\n"));
        assert!(ok("let a: tuple[int, str] = (1, \"a\")\nlet b: tuple[int, str] = (1, \"b\")\nlet c = a <= b\n"));
        assert!(ok("let a: tuple[int, int] = (1, 2)\nlet b: tuple[int, int, int] = (1, 2, 0)\nlet c = a < b\n"));
        assert!(ok("let b: list[int] = [1]\nlet c = [] < b\n"));
        // 要素の `Union`（`list[Union[int, float]]`）は構成型ごとに見る。
        assert!(ok("let c = [1, 2.5] > [1, 2]\n"));
    }

    /// 要素が比べられない組・list と tuple は誤り（CPython も実行時の `TypeError`）。
    #[test]
    fn seq_ordering_err() {
        assert!(err("let a: list[int] = [1]\nlet b: list[str] = [\"a\"]\nlet c = a < b\n"));
        assert!(err("let a: tuple[int, str] = (1, \"a\")\nlet b: tuple[int, int] = (1, 2)\nlet c = a < b\n"));
        assert!(err("let a: list[int] = [1]\nlet b: tuple[int] = (1,)\nlet c = a < b\n"));
    }
