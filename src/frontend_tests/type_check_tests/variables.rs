// type_check_tests/variables.rs — 変数の再宣言・不変代入・不変フィールド代入の静的型検査テスト。

use super::*;

    // --- Variable redeclaration ---

    /// let_redeclaration_same_scope のテスト。
    #[test]
    fn let_redeclaration_same_scope() {
        assert!(err("let a = 5\nlet a = 6\n"));
    }

    /// mut_redeclaration_same_scope のテスト。
    #[test]
    fn mut_redeclaration_same_scope() {
        assert!(err("mut a = 5\nmut a = 6\n"));
    }

    /// let_then_mut_redeclaration のテスト。
    #[test]
    fn let_then_mut_redeclaration() {
        assert!(err("let a = 5\nmut a = 6\n"));
    }

    /// const_redeclaration_same_scope のテスト。
    #[test]
    fn const_redeclaration_same_scope() {
        assert!(err("const A = 5\nconst A = 6\n"));
    }

    /// redeclaration_in_inner_scope のテスト（外側スコープの変数と同名）。
    #[test]
    fn redeclaration_in_inner_scope() {
        assert!(err("let x = 1\nif True:\n    let x = 2\n"));
    }

    /// redeclaration_in_function_body のテスト（外側の let と同名）。
    #[test]
    fn redeclaration_in_function_body() {
        assert!(err("let x = 1\nfn f() -> None:\n    let x = 2\n"));
    }

    /// redeclaration_tuple_target のテスト。
    #[test]
    fn redeclaration_tuple_target() {
        assert!(err("let a = 1\nlet a, let b = (2, 3)\n"));
    }

    /// underscore_redeclaration_allowed のテスト（_ は再宣言を許可）。
    #[test]
    fn underscore_redeclaration_allowed() {
        assert!(ok("let _ = 1\nlet _ = 2\n"));
    }

    /// redeclaration_error_mentions_name のテスト（エラーメッセージに変数名が含まれる）。
    #[test]
    fn redeclaration_error_mentions_name() {
        let errors = check("let foo = 1\nlet foo = 2\n");
        assert!(!errors.is_empty());
        let msg = errors[0].to_string();
        assert!(msg.contains("foo"), "error should mention variable name, got: {msg}");
        assert!(msg.contains("already declared"), "error should say 'already declared', got: {msg}");
    }

    // --- Immutable assignment ---

    /// let_immutable_assign のテスト。
    #[test]
    fn let_immutable_assign() {
        assert!(err("let x = 1\nx = 2"));
    }

    /// const_immutable_assign のテスト。
    #[test]
    fn const_immutable_assign() {
        assert!(err("const X = 1\nX = 2"));
    }

    /// mut_assign_ok のテスト。
    #[test]
    fn mut_assign_ok() {
        assert!(ok("mut x = 1\nx = 2"));
    }

    /// let_compound_assign_immutable のテスト。
    #[test]
    fn let_compound_assign_immutable() {
        assert!(err("let x = 1\nx += 1"));
    }

    /// mut_compound_assign_ok のテスト。
    #[test]
    fn mut_compound_assign_ok() {
        assert!(ok("mut x = 1\nx += 1"));
    }

    /// immutable_assign_inside_if のテスト。
    #[test]
    fn immutable_assign_inside_if() {
        assert!(err("let x = 1\nif True:\n    x = 2\n"));
    }

    /// mut_assign_inside_if_ok のテスト。
    #[test]
    fn mut_assign_inside_if_ok() {
        assert!(ok("mut x = 1\nif True:\n    x = 2\n"));
    }

    // --- Immutable field assignment ---

    /// let_field_assign_outside_class_err のテスト。
    #[test]
    fn let_field_assign_outside_class_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "let t = Token(\"ident\")\n",
            "t.kind = \"op\"\n",
        )));
    }

    /// let_field_assign_in_other_method_err のテスト。
    #[test]
    fn let_field_assign_in_other_method_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn reset(mut self) -> None:\n",
            "        self.kind = \"op\"\n",
        )));
    }

    /// let_field_assign_in_init_ok のテスト。
    #[test]
    fn let_field_assign_in_init_ok() {
        assert!(ok(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn __init__(mut self, k: str) -> None:\n",
            "        self.kind = k\n",
        )));
    }

    /// `mut` 束縛のフィールドへの代入は通る。
    #[test]
    fn mut_binding_field_assign_ok() {
        assert!(ok(concat!(
            "class Counter:\n",
            "    mut count: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.count = 0\n",
            "mut c = Counter()\n",
            "c.count = 5\n",
        )));
    }

    /// `let` 束縛のフィールドへの代入は**静的エラー**（タスク 7.2・検体 `M5`）。
    ///
    /// ⚠⚠ **このテストは以前 `mut_field_assign_ok` という名前で `ok(..)` を主張していた。**
    /// フィールドが `mut count: int` なら通る、という読みだったが、**実行時は同じコードを
    /// 拒否する**（`TypeError: cannot assign to immutable field 'count'`）。
    /// ⇒ テストが静的検査の穴を仕様として固定していた形。実行時に合わせて `err` に直した。
    ///
    /// 見るべきは「**束縛**が `let` か」で、「フィールドが `let` 宣言か」は別の検査
    /// （`check_immutable_field_assign`）。
    #[test]
    fn let_binding_field_assign_err() {
        assert!(err(concat!(
            "class Counter:\n",
            "    mut count: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.count = 0\n",
            "let c = Counter()\n",
            "c.count = 5\n",
        )));
    }

    /// let_field_compound_assign_outside_err のテスト。
    #[test]
    fn let_field_compound_assign_outside_err() {
        assert!(err(concat!(
            "class Node:\n",
            "    let value: int\n",
            "let n = Node(1)\n",
            "n.value += 1\n",
        )));
    }

// ── メタプログラミングの型（設計書 §1.2 / §1.5・タスク 1-5）────────────────

/// ⚠⚠ `Code` は**メタ関数の外へ持ち出せない**（設計書 §1.2）。
/// 展開時にしか存在しない値なので、通常の変数に束縛できると
/// 「実行時に評価できない値」が型検査をすり抜けて実行まで届く。
///
/// ⚠ `code:` そのものはメタ関数の外では**パースで**弾かれる（タスク 1-6）。ここで見るのは
/// 残る経路 —— **純粋メタ関数の返り値**（`Code`）を持ち出す形。展開しないエディタでは
/// この検査が唯一の網になる。
fn escapes(source: &str) -> bool {
    check(source)
        .iter()
        .any(|e| matches!(e.kind, TypeErrorKind::CodeEscapesMetafunction { .. }))
}

#[test]
fn code_value_cannot_be_bound_outside_a_metafunction() {
    assert!(escapes(concat!(
        "exprconst fn frag() -> Code:\n",
        "    let a = code:\n",
        "        print(1)\n",
        "    return a\n",
        "let f = frag()\n",
    )));
}

/// ⚠ 普通の関数の中でも同じ。`Code` を扱えるのはメタ関数の中だけ。
#[test]
fn code_value_cannot_escape_through_a_plain_function() {
    assert!(escapes(concat!(
        "exprconst fn frag() -> Code:\n",
        "    let a = code:\n",
        "        pass\n",
        "    return a\n",
        "fn g() -> None:\n",
        "    let b = frag()\n",
    )));
}

/// メタ情報型は**注釈として書ける**（設計書 §1.5）。
/// ⚠ Arrow は関数の仮引数の注釈が必須なので、これが無いとメタ関数そのものが書けない。
#[test]
fn meta_type_names_are_accepted_as_annotations() {
    use crate::type_check::InferredType;
    for name in ["Code", "meta_instance", "meta_function", "meta_class", "meta_member"] {
        assert!(
            InferredType::from_ann(name).is_some(),
            "`{name}` を型注釈として解釈できること"
        );
    }
}

/// ⚠ `meta_*` は**別々の型**。フィールドとメソッドを混ぜない（設計書 §1.5）。
#[test]
fn meta_kinds_are_distinct_types() {
    use crate::type_check::InferredType;
    let member = InferredType::from_ann("meta_member").unwrap();
    let function = InferredType::from_ann("meta_function").unwrap();
    assert_ne!(member, function);
    assert_eq!(member.to_string(), "meta_member");
    assert_eq!(function.to_string(), "meta_function");
}

    // --- 変更できない受け手への `mut self` メソッド（フェーズ10 10-15）---
    // ⚠ 以前は実行時のガード任せで、`let` / `freeze` は実行時の TypeError、`let` 仮引数・
    //   `mut` でない `self` の先は写しを黙って書き換えて何も起きなかった。

    const BOX: &str = concat!(
        "class Box:\n",
        "    mut v: int\n",
        "    fn set(mut self, let x: int) -> None:\n",
        "        self.v = x\n",
        "    fn peek(self) -> int:\n",
        "        return self.v\n",
    );

    /// `freeze` した束縛へ `mut self` メソッド（10-15 の再現）。
    #[test]
    fn mut_self_method_on_frozen_err() {
        assert!(err(&format!("{BOX}mut b = Box(1)\nfreeze b\nb.set(2)\n")));
    }

    /// `let` 束縛へ `mut self` メソッド。
    #[test]
    fn mut_self_method_on_let_err() {
        assert!(err(&format!("{BOX}let b = Box(1)\nb.set(2)\n")));
    }

    /// `let` 仮引数へ `mut self` メソッド（実行時は写しを黙って書き換えていた）。
    #[test]
    fn mut_self_method_on_let_param_err() {
        assert!(err(&format!("{BOX}fn f(let p: Box) -> None:\n    p.set(3)\n")));
    }

    /// `mut` でない `self` の先のフィールドへ `mut self` メソッド。
    #[test]
    fn mut_self_method_through_immutable_self_err() {
        assert!(err(&format!(
            "{BOX}class Holder:\n    mut b: Box\n    fn poke(self) -> None:\n        self.b.set(5)\n"
        )));
    }

    /// `mut` 束縛・`mut` 仮引数・`mut self` の先は通る。`mut self` でないメソッドは `let` でも通る。
    #[test]
    fn mut_self_method_on_mutable_ok() {
        assert!(ok(&format!(concat!(
            "{}",
            "mut b = Box(1)\n",
            "b.set(2)\n",
            "fn f(mut p: Box) -> None:\n",
            "    p.set(3)\n",
            "f(b)\n",
            "let c = Box(4)\n",
            "print(c.peek())\n",
            "class Holder:\n",
            "    mut b: Box\n",
            "    fn poke(mut self) -> None:\n",
            "        self.b.set(5)\n",
        ), BOX)));
    }

    /// 一時値（根が識別子でない）は通す。
    #[test]
    fn mut_self_method_on_temporary_ok() {
        assert!(ok(&format!("{BOX}Box(1).set(2)\n")));
    }

    // --- 未定義の名前（フェーズ10 10-12）---
    // ⚠ 以前は静的に通り、実行時の NameError だった。判定は保守的（`type_check::names` の doc）。

    fn undefined_names(src: &str) -> Vec<String> {
        check(src)
            .iter()
            .filter_map(|e| match &e.kind {
                TypeErrorKind::UndefinedName { name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// どこにも無い名前（10-12 の再現）。
    #[test]
    fn undefined_name_err() {
        assert_eq!(undefined_names("print(nonexistent(2))\n"), vec!["nonexistent".to_string()]);
    }

    /// 後で宣言される最上位の名前を関数の中から読むのは通る（前方参照）。
    #[test]
    fn forward_reference_ok() {
        assert!(undefined_names("fn f() -> int:\n    return LIMIT + g()\nfn g() -> int:\n    return 1\nlet LIMIT = 3\nprint(f())\n").is_empty());
    }

    /// 組み込みの名前・`case _:`・`for` の変数・`except` の別名・クラス・列挙は通る。
    #[test]
    fn builtin_and_bound_names_ok() {
        let src = concat!(
            "class P:\n",
            "    mut x: int\n",
            "enum Color:\n",
            "    Red\n",
            "let r = Ok(1)\n",
            "let e = Err(\"bad\")\n",
            "for i in range(2):\n",
            "    print(i, len([1]), repr(i))\n",
            "try:\n",
            "    raise ValueError(\"v\")\n",
            "except ValueError as err:\n",
            "    print(err.message)\n",
            "match 1:\n",
            "    case 1:\n",
            "        print(P(1).x, Color.Red)\n",
            "    case _:\n",
            "        print(\"other\")\n",
        );
        assert!(undefined_names(src).is_empty(), "{:?}", undefined_names(src));
    }


    /// 式の側に位置が無い誤りは**文の位置**で報告する（フェーズ10 10-17。以前は `<unknown>`）。
    #[test]
    fn error_without_expression_position_uses_statement_position() {
        // 右辺が識別子（位置を持たない）の宣言。
        let errs = check("let s = 1\nlet z: str = s\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
        let at = errs[0].span.as_ref().map(|s| (s.line, s.col));
        assert_eq!(at, Some((2, 1)), "{errs:?}");
        // 式文の中の呼び出しの引数の誤り（式文は文の位置を持たないので、呼び出しの位置で代える）。
        let errs = check("fn f(let x: int) -> None:\n    pass\n\nf(\"s\")\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].span.as_ref().map(|s| s.line), Some(4), "{errs:?}");
    }

    /// `Exception` はすべての例外の基底（フェーズ10 10-19）。組み込みの例外も `Error` を実装した利用者の
    /// 例外も `Exception` の欄・引数に入る。`except Error` は `except Exception` を促す誤り。
    #[test]
    fn exception_is_the_base_of_every_exception() {
        let src = concat!(
            "class MyErr(Error):\n",
            "    fn __init__(mut self, message: str) -> None:\n",
            "        self.message = message\n",
            "fn take(let e: Exception) -> str:\n",
            "    return e.message\n",
            "let a: Exception = ValueError(\"v\")\n",
            "let b = take(MyErr(\"m\"))\n",
            "let c = take(KeyError(\"k\"))\n",
        );
        assert!(ok(src), "{:?}", check(src));
        // 例外でない値は入らない。
        assert!(err("let d: Exception = 5\n"));
        let errs = check("try:\n    pass\nexcept Error as e:\n    pass\n");
        assert!(
            errs.iter().any(|e| matches!(e.kind, TypeErrorKind::ExceptOnErrorTrait)),
            "{errs:?}"
        );
    }

    /// 組み込みの例外は CPython の階層を持つ（python_builtins_plan.md のタスク 4-1）。子は親の欄に入り、
    /// 親は子の欄に入らない。足した例外・警告のクラスも `except` に書ける。
    #[test]
    fn builtin_exceptions_follow_the_cpython_hierarchy() {
        let src = concat!(
            "let a: LookupError = KeyError(\"k\")\n",
            "let b: OSError = FileNotFoundError(\"f\")\n",
            "let c: IOError = PermissionError(\"p\")\n",
            "let d: BaseException = ValueError(\"v\")\n",
            "let w: Warning = FutureWarning(\"w\")\n",
            "try:\n    pass\nexcept UserWarning as e:\n    pass\n",
        );
        assert!(ok(src), "{:?}", check(src));
        assert!(err("let e: KeyError = LookupError(\"x\")\n"));
        assert!(err("let e: Exception = GeneratorExit(\"g\")\n"));
    }

    /// 組み込みの例外は `args`（組）を持つ（タスク 4-2）。`Error` を実装しただけの利用者の例外は持たない。
    #[test]
    fn builtin_exceptions_have_args() {
        assert!(ok("let e = ValueError(\"a\", 1)\nlet n = len(e.args)\n"));
        assert!(err("let e = ValueError(\"a\")\nlet n: int = e.args\n"));
        let src = concat!(
            "class MyErr(Error):\n",
            "    fn __init__(mut self, message: str) -> None:\n",
            "        self.message = message\n",
            "let m = MyErr(\"m\")\n",
            "let x = m.args\n",
        );
        assert!(err(src), "{:?}", check(src));
    }
