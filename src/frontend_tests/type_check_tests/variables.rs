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

    /// mut_field_assign_ok のテスト。
    #[test]
    fn mut_field_assign_ok() {
        assert!(ok(concat!(
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

