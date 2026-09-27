// type_check_tests/guards_fntype.rs — 型ガード(is/is not)・関数型・type[T] の型検査テスト。

use super::*;

    // --- Type guard (is / is not) ---

    /// type_guard_is_narrows_in_if_body_ok のテスト。
    #[test]
    fn type_guard_is_narrows_in_if_body_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let y = x + 1\n",
        )));
    }

    /// type_guard_is_union_without_narrowing_err のテスト。
    #[test]
    fn type_guard_is_union_without_narrowing_err() {
        let errors = check(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "let y = x + 1\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    /// type_guard_is_not_narrows_in_if_body_ok のテスト。
    #[test]
    fn type_guard_is_not_narrows_in_if_body_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is not None:\n    let y = x + 1\n",
        )));
    }

    /// type_guard_is_not_on_non_union_err のテスト。
    #[test]
    fn type_guard_is_not_on_non_union_err() {
        let errors = check(concat!("let x = 5\n", "if x is not str:\n    pass\n",));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::IsNotOnNonUnion { .. })));
    }

    /// type_guard_elif_narrows_ok のテスト。
    #[test]
    fn type_guard_elif_narrows_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let a = x + 1\n",
            "elif x is None:\n    pass\n",
        )));
    }

    /// type_guard_elif_chain_ok のテスト。
    #[test]
    fn type_guard_elif_chain_ok() {
        assert!(ok(concat!(
            "fn f() -> Union[int, str]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let a = x + 1\n",
            "elif x is str:\n    let b = x\n",
        )));
    }

    /// type_guard_elif_is_not_on_non_union_err のテスト。
    #[test]
    fn type_guard_elif_is_not_on_non_union_err() {
        let errors = check(concat!(
            "let x = 5\n",
            "if x == 1:\n    pass\n",
            "elif x is not str:\n    pass\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::IsNotOnNonUnion { .. })));
    }

    // --- function type ---

    /// function_type_bare_param_ok のテスト。
    #[test]
    fn function_type_bare_param_ok() {
        assert!(ok(concat!(
            "fn caller(let f: function) -> None:\n",
            "    pass\n",
        )));
    }

    /// function_type_positional_params_ok のテスト。
    #[test]
    fn function_type_positional_params_ok() {
        assert!(ok(concat!(
            "fn make() -> function[let int]->int:\n",
            "    fn inner(let x: int) -> int:\n",
            "        return x\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(1)\n",
        )));
    }

    /// function_type_return_type_inferred_ok のテスト。
    #[test]
    fn function_type_return_type_inferred_ok() {
        assert!(ok(concat!(
            "fn make() -> function[let int]->int:\n",
            "    fn inner(let x: int) -> int:\n",
            "        return x\n",
            "    return inner\n",
            "let f = make()\n",
            "let r: int = f(1)\n",
        )));
    }

    /// function_type_wrong_arg_type_err のテスト。
    #[test]
    fn function_type_wrong_arg_type_err() {
        let errors = check(concat!(
            "fn make() -> function[let int]->int:\n",
            "    fn inner(let x: int) -> int:\n",
            "        return x\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(\"hello\")\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// function_type_wrong_arg_count_err のテスト。
    #[test]
    fn function_type_wrong_arg_count_err() {
        let errors = check(concat!(
            "fn make() -> function[let int]->int:\n",
            "    fn inner(let x: int) -> int:\n",
            "        return x\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(1, 2)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgCountMismatch { .. })));
    }

    /// function_type_named_param_keyword_ok のテスト。
    #[test]
    fn function_type_named_param_keyword_ok() {
        assert!(ok(concat!(
            "fn make() -> function{let value:int}->int:\n",
            "    fn inner(let value: int) -> int:\n",
            "        return value\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(value = 1)\n",
        )));
    }

    /// function_type_named_param_unknown_keyword_err のテスト。
    #[test]
    fn function_type_named_param_unknown_keyword_err() {
        let errors = check(concat!(
            "fn make() -> function{let value:int}->int:\n",
            "    fn inner(let value: int) -> int:\n",
            "        return value\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(param = 1)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::UnknownKeywordArg { .. })));
    }

    /// function_type_mut_param_with_immutable_arg_err のテスト。
    #[test]
    fn function_type_mut_param_with_immutable_arg_err() {
        let errors = check(concat!(
            "fn make() -> function{mut value:int}->int:\n",
            "    fn inner(mut value: int) -> int:\n",
            "        return value\n",
            "    return inner\n",
            "let f = make()\n",
            "let z = 5\n",
            "let r = f(value = z)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind,
            TypeErrorKind::CallMutParamWithImmutableArg { .. }
        )));
    }

    /// function_type_mut_param_with_mutable_arg_ok のテスト。
    #[test]
    fn function_type_mut_param_with_mutable_arg_ok() {
        assert!(ok(concat!(
            "fn make() -> function{mut value:int}->int:\n",
            "    fn inner(mut value: int) -> int:\n",
            "        return value\n",
            "    return inner\n",
            "let f = make()\n",
            "mut x = 5\n",
            "let r = f(value = x)\n",
        )));
    }

    /// function_type_chained_call_ok のテスト。
    #[test]
    fn function_type_chained_call_ok() {
        assert!(ok(concat!(
            "fn make() -> function[let int]->int:\n",
            "    fn inner(let x: int) -> int:\n",
            "        return x\n",
            "    return inner\n",
            "mut result = make()(3)\n",
        )));
    }

    /// function_type_zero_params_ok のテスト。
    #[test]
    fn function_type_zero_params_ok() {
        assert!(ok(concat!(
            "fn make() -> function[]->int:\n",
            "    fn inner() -> int:\n",
            "        return 42\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f()\n",
        )));
    }

    /// function_type_zero_params_wrong_count_err のテスト。
    #[test]
    fn function_type_zero_params_wrong_count_err() {
        let errors = check(concat!(
            "fn make() -> function[]->int:\n",
            "    fn inner() -> int:\n",
            "        return 42\n",
            "    return inner\n",
            "let f = make()\n",
            "let r = f(1)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgCountMismatch { .. })));
    }

    // --- type[T] ---

    /// type_val_of_exact_match_ok のテスト。
    #[test]
    fn type_val_of_exact_match_ok() {
        assert!(ok(concat!(
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(int)\n",
        )));
    }

    /// type_val_of_wrong_primitive_err のテスト。
    #[test]
    fn type_val_of_wrong_primitive_err() {
        let errors = check(concat!(
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(float)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// type_val_of_new_type_upcast_ok のテスト。
    #[test]
    fn type_val_of_new_type_upcast_ok() {
        assert!(ok(concat!(
            "new_type Index: int\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(Index)\n",
        )));
    }

    /// type_val_of_new_type_chain_ok のテスト。
    #[test]
    fn type_val_of_new_type_chain_ok() {
        assert!(ok(concat!(
            "new_type A: int\n",
            "new_type B: A\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(B)\n",
        )));
    }

    /// type_val_of_new_type_wrong_origin_err のテスト。
    #[test]
    fn type_val_of_new_type_wrong_origin_err() {
        let errors = check(concat!(
            "new_type Name: str\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(Name)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// type_val_of_trait_upcast_ok のテスト。
    #[test]
    fn type_val_of_trait_upcast_ok() {
        assert!(ok(concat!(
            "trait MyTrait:\n    pass\n",
            "class MyClass(MyTrait):\n    pass\n",
            "fn f(let x: type[MyTrait]) -> None:\n    pass\n",
            "f(MyClass)\n",
        )));
    }

    /// type_val_of_trait_wrong_class_err のテスト。
    #[test]
    fn type_val_of_trait_wrong_class_err() {
        let errors = check(concat!(
            "trait MyTrait:\n    pass\n",
            "class Other:\n    pass\n",
            "fn f(let x: type[MyTrait]) -> None:\n    pass\n",
            "f(Other)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// type_val_bare_accepts_any_type_value_ok のテスト。
    #[test]
    fn type_val_bare_accepts_any_type_value_ok() {
        assert!(ok(concat!(
            "fn f(let x: type) -> None:\n    pass\n",
            "f(int)\n",
            "f(str)\n",
        )));
    }

    /// type_val_of_display のテスト。
    #[test]
    fn type_val_of_display() {
        let type_value = InferredType::TypeValOf(Box::new(InferredType::Int));
        assert_eq!(type_value.to_string(), "type[int]");
    }


    // --- `is` の型名の妥当性（フェーズ10 10-10）---

    const STACKS: &str = concat!(
        "class Stack[T]:\n",
        "    mut items: list[T]\n",
        "    fn size(self) -> int:\n",
        "        return len(self.items)\n",
        "let a = Stack[int]([1])\n",
        "let b = Stack[str]([\"s\"])\n",
    );

    /// `is Stack[int]` で絞り込んだ先でメンバーが引ける。
    #[test]
    fn is_template_instance_narrows_ok() {
        let errs = check_expanded(&format!(
            "{STACKS}fn f(let x: Union[Stack[int], Stack[str]]) -> int:\n    if x is Stack[int]:\n        return x.size()\n    return 0\nprint(f(a))\n"
        ));
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// `if x is NoSuch:` も存在しない型として弾く（以前は `match` の腕でしか見ていなかった）。
    #[test]
    fn is_unknown_type_in_if_err() {
        let errs = check("let x: int = 1\nif x is NoSuch:\n    print(1)\n");
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UnknownGuardType { .. })), "{errs:?}");
    }

    /// 式の `x is Nope` も同じ。
    #[test]
    fn is_unknown_type_in_expr_err() {
        let errs = check("let x: int = 1\nlet b = x is Nope\n");
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UnknownGuardType { .. })), "{errs:?}");
    }

    /// 要素型つきの組み込みの容器は判定できない（`is list` と書く）。
    #[test]
    fn is_container_with_element_type_err() {
        let errs = check("let x: Union[list[str], int] = 1\nlet b = x is list[int]\n");
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::GuardTypeNotTestable { outer: Some(o), .. } if o == "list")),
            "{errs:?}"
        );
    }

    /// テンプレートでない名前に `[..]` が付いていたら未知の型。
    #[test]
    fn is_non_template_with_args_err() {
        let errs = check("let x: int = 1\nlet b = x is str[int]\n");
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UnknownGuardType { .. })), "{errs:?}");
    }

    /// `[` の無い綴りは従来どおり正当（`list` / `function` / `generator` / `None`）。
    #[test]
    fn is_bare_builtin_names_ok() {
        assert!(ok(concat!(
            "fn f(let x: Any) -> None:\n",
            "    let a = x is list\n",
            "    let b = x is function\n",
            "    let c = x is generator\n",
            "    let d = x is None\n",
            "    print(a, b, c, d)\n",
        )));
    }
