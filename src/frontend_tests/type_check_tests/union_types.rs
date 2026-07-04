// type_check_tests/union_types.rs — Union/Option・Any・new_type・trait の型検査テスト。

use super::*;

    // --- Union / Option type ---

    /// union_param_accepts_member_types_ok のテスト。
    #[test]
    fn union_param_accepts_member_types_ok() {
        assert!(ok(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "f(1)\n",
            "f(\"hi\")\n",
        )));
    }

    /// union_param_rejects_non_member_err のテスト。
    #[test]
    fn union_param_rejects_non_member_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "f(True)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// union_param_accepts_same_union_ok のテスト。
    #[test]
    fn union_param_accepts_same_union_ok() {
        assert!(ok(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "fn g(x: Union[int, str]) -> None:\n    f(x)\n",
        )));
    }

    /// union_value_binary_op_err のテスト。
    #[test]
    fn union_value_binary_op_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x + 1\n",
        ));
        assert!(errors.iter().any(
            |error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { op, .. } if op == "+")
        ));
    }

    /// union_value_comparison_err のテスト。
    #[test]
    fn union_value_comparison_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x < 10\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    /// union_value_to_typed_param_err のテスト。
    #[test]
    fn union_value_to_typed_param_err() {
        let errors = check(concat!(
            "fn needs_int(n: int) -> None:\n    pass\n",
            "fn caller(x: Union[int, str]) -> None:\n    needs_int(x)\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// union_value_attr_access_err のテスト。
    #[test]
    fn union_value_attr_access_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x.upper\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    /// union_display_format のテスト。
    #[test]
    fn union_display_format() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x + 1\n",
        ));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("Union[int, str]"));
        assert!(msg.contains("downcast"));
    }

    /// option_param_accepts_inner_type_and_none_ok のテスト。
    #[test]
    fn option_param_accepts_inner_type_and_none_ok() {
        assert!(ok(concat!(
            "fn f(x: Option[int]) -> None:\n    pass\n",
            "f(1)\n",
            "f(None)\n",
        )));
    }

    /// option_param_rejects_wrong_type_err のテスト。
    #[test]
    fn option_param_rejects_wrong_type_err() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n    pass\n",
            "f(\"oops\")\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// option_value_binary_op_err のテスト。
    #[test]
    fn option_value_binary_op_err() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n",
            "    let y = x + 1\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    /// option_display_shows_option_format のテスト。
    #[test]
    fn option_display_shows_option_format() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n",
            "    let y = x + 1\n",
        ));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("Option[int]"));
    }

    /// nested_union_option_in_union_ok のテスト。
    #[test]
    fn nested_union_option_in_union_ok() {
        assert!(ok(concat!(
            "fn f(x: Union[Option[int], str]) -> None:\n    pass\n",
            "fn g(a: Option[int], b: str) -> None:\n",
            "    f(a)\n",
            "    f(b)\n",
        )));
    }

    // --- Any type ---

    /// any_param_accepts_all_arg_types_ok のテスト。
    #[test]
    fn any_param_accepts_all_arg_types_ok() {
        assert!(ok(concat!(
            "fn wrap(x: Any) -> None:\n",
            "    pass\n",
            "wrap(1)\n",
            "wrap(\"hello\")\n",
            "wrap(True)\n",
        )));
    }

    /// any_typed_var_binary_op_err のテスト。
    #[test]
    fn any_typed_var_binary_op_err() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = x + 1\n",));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "+")));
    }

    /// any_typed_var_comparison_err のテスト。
    #[test]
    fn any_typed_var_comparison_err() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = x < 10\n",));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "<")));
    }

    /// any_typed_var_eq_err のテスト。
    #[test]
    fn any_typed_var_eq_err() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = x == 1\n",));
        assert!(errors.iter().any(
            |error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "==")
        ));
    }

    /// any_typed_var_logical_op_err のテスト。
    #[test]
    fn any_typed_var_logical_op_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x and True\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { .. })));
    }

    /// any_typed_var_unary_neg_err のテスト。
    #[test]
    fn any_typed_var_unary_neg_err() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = -x\n",));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "-")));
    }

    /// any_typed_var_attr_access_err のテスト。
    #[test]
    fn any_typed_var_attr_access_err() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = x.value\n",));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "attribute access")));
    }

    /// passing_any_to_typed_param_err のテスト。
    #[test]
    fn passing_any_to_typed_param_err() {
        let errors = check(concat!(
            "fn needs_int(n: int) -> None:\n",
            "    pass\n",
            "fn caller(x: Any) -> None:\n",
            "    needs_int(x)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind,
            TypeErrorKind::CallArgTypeMismatch { expected, got, .. }
            if *expected == InferredType::Int && *got == InferredType::Any
        )));
    }

    /// any_to_any_param_ok のテスト。
    #[test]
    fn any_to_any_param_ok() {
        assert!(ok(concat!(
            "fn accept_any(x: Any) -> None:\n",
            "    pass\n",
            "fn forward(x: Any) -> None:\n",
            "    accept_any(x)\n",
        )));
    }

    /// operation_on_any_display のテスト。
    #[test]
    fn operation_on_any_display() {
        let errors = check(concat!("fn f(x: Any) -> None:\n", "    let y = x + 1\n",));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("Any"));
        assert!(msg.contains("downcast"));
    }

    // --- new_type ---

    /// new_type_class_copy_no_type_errors のテスト。
    #[test]
    fn new_type_class_copy_no_type_errors() {
        assert!(ok(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "new_type FooAlias: Foo\n",
            "let a = Foo(1)\n",
            "let b = FooAlias(2)\n",
        )));
    }

    /// new_type_constructor_returns_named_instance のテスト。
    #[test]
    fn new_type_constructor_returns_named_instance() {
        let errors = check(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "new_type FooAlias: Foo\n",
            "let a = Foo(1)\n",
            "let b = FooAlias(2)\n",
        ));
        assert!(errors.is_empty());
    }

    /// self_type_same_class_ok のテスト。
    #[test]
    fn self_type_same_class_ok() {
        assert!(ok(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "    fn bar(self, other: Self) -> None:\n",
            "        pass\n",
            "let a = Foo(1)\n",
            "let b = Foo(2)\n",
            "a.bar(b)\n",
        )));
    }

    /// self_type_mismatch_new_type_err のテスト。
    #[test]
    fn self_type_mismatch_new_type_err() {
        let errors = check(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "    fn bar(self, other: Self) -> None:\n",
            "        pass\n",
            "new_type FooAlias: Foo\n",
            "let a = Foo(1)\n",
            "let b = FooAlias(2)\n",
            "a.bar(b)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind,
            TypeErrorKind::SelfTypeMismatch {
                expected_class,
                got_class,
                ..
            } if expected_class == "Foo" && got_class == "FooAlias"
        )));
    }

    /// self_type_mismatch_reverse_err のテスト。
    #[test]
    fn self_type_mismatch_reverse_err() {
        let errors = check(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "    fn bar(self, other: Self) -> None:\n",
            "        pass\n",
            "new_type FooAlias: Foo\n",
            "let a = Foo(1)\n",
            "let b = FooAlias(2)\n",
            "b.bar(a)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind,
            TypeErrorKind::SelfTypeMismatch {
                expected_class,
                got_class,
                ..
            } if expected_class == "FooAlias" && got_class == "Foo"
        )));
    }

    /// self_type_mismatch_display のテスト。
    #[test]
    fn self_type_mismatch_display() {
        let errors = check(concat!(
            "class Foo:\n",
            "    mut value: int\n",
            "    fn bar(self, other: Self) -> None:\n",
            "        pass\n",
            "new_type FooAlias: Foo\n",
            "let a = Foo(1)\n",
            "let b = FooAlias(2)\n",
            "a.bar(b)\n",
        ));
        let msg = errors
            .iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::SelfTypeMismatch { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("bar"));
        assert!(msg.contains("Foo"));
        assert!(msg.contains("FooAlias"));
    }

    // --- trait ---

    /// trait_with_virtual_method_no_type_errors のテスト。
    #[test]
    fn trait_with_virtual_method_no_type_errors() {
        assert!(ok(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
        )));
    }

    /// trait_with_non_virtual_method_no_type_errors のテスト。
    #[test]
    fn trait_with_non_virtual_method_no_type_errors() {
        assert!(ok(concat!(
            "trait Logger:\n",
            "    fn log(self, msg: str) -> None:\n",
            "        pass\n",
        )));
    }

    /// trait_with_fields_no_type_errors のテスト。
    #[test]
    fn trait_with_fields_no_type_errors() {
        assert!(ok(concat!(
            "trait HasValue:\n",
            "    mut value: int\n",
            "    const MAX: int = 100\n",
        )));
    }

    /// trait_class_inheriting_no_type_errors のテスト。
    #[test]
    fn trait_class_inheriting_no_type_errors() {
        assert!(ok(concat!(
            "trait Shape:\n",
            "    fn area(self) -> float:\n",
            "        ...\n",
            "class Square(Shape):\n",
            "    mut side: float\n",
            "    fn area(self) -> float:\n",
            "        pass\n",
        )));
    }

    /// trait_class_call_type_mismatch_detected のテスト。
    #[test]
    fn trait_class_call_type_mismatch_detected() {
        let errors = check(concat!(
            "trait T:\n",
            "    fn f(self) -> None:\n",
            "        ...\n",
            "class C(T):\n",
            "    mut x: int\n",
            "    fn f(self) -> None:\n",
            "        pass\n",
            "fn use_x(v: int) -> None:\n",
            "    pass\n",
            "use_x(\"wrong\")\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    /// raise_builtin_error_instance_ok のテスト。
    #[test]
    fn raise_builtin_error_instance_ok() {
        assert!(ok("raise ValueError(\"bad\")\n"));
    }

    /// raise_user_error_instance_ok のテスト。
    #[test]
    fn raise_user_error_instance_ok() {
        assert!(ok(concat!(
            "class MyError(Error):\n",
            "    pass\n",
            "raise MyError(\"bad\")\n",
        )));
    }

    /// raise_non_error_type_detected のテスト。
    #[test]
    fn raise_non_error_type_detected() {
        let errors = check("raise \"bad\"\n");
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidRaiseType { .. })));
    }

