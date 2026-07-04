// type_check_tests/decorators_generics.rs — デコレータとコレクションジェネリクスの型検査テスト。

use super::*;

    // --- Decorator static type checking ---

    /// decorator_fn_correct_signature_ok のテスト。
    #[test]
    fn decorator_fn_correct_signature_ok() {
        assert!(ok(concat!(
            "fn log(let f: function) -> function:\n",
            "    return f\n",
            "@log\n",
            "fn greet(let name: str) -> str:\n",
            "    return name\n",
        )));
    }

    /// decorator_fn_wrong_param_type_err のテスト。
    #[test]
    fn decorator_fn_wrong_param_type_err() {
        let errors = check(concat!(
            "fn bad_dec(let x: int) -> function:\n",
            "    return x\n",
            "@bad_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    /// decorator_fn_wrong_return_type_err のテスト。
    #[test]
    fn decorator_fn_wrong_return_type_err() {
        let errors = check(concat!(
            "fn bad_dec(let f: function) -> int:\n",
            "    return 0\n",
            "@bad_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    /// decorator_fn_no_params_err のテスト。
    #[test]
    fn decorator_fn_no_params_err() {
        let errors = check(concat!(
            "fn no_param_dec() -> function:\n",
            "    pass\n",
            "@no_param_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    /// decorator_class_correct_signature_ok のテスト。
    #[test]
    fn decorator_class_correct_signature_ok() {
        assert!(ok(concat!(
            "class Singleton:\n",
            "    fn __init__(self, let cls: type) -> None:\n",
            "        pass\n",
            "    fn __call__(self) -> type:\n",
            "        pass\n",
            "@Singleton\n",
            "class MyClass:\n",
            "    pass\n",
        )));
    }

    /// decorator_class_init_wrong_param_err のテスト。
    #[test]
    fn decorator_class_init_wrong_param_err() {
        let errors = check(concat!(
            "class BadDec:\n",
            "    fn __init__(self, let x: int) -> None:\n",
            "        pass\n",
            "    fn __call__(self) -> type:\n",
            "        pass\n",
            "@BadDec\n",
            "class MyClass:\n",
            "    pass\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    /// decorator_class_call_wrong_return_err のテスト。
    #[test]
    fn decorator_class_call_wrong_return_err() {
        let errors = check(concat!(
            "class BadDec:\n",
            "    fn __init__(self, let cls: type) -> None:\n",
            "        pass\n",
            "    fn __call__(self) -> int:\n",
            "        pass\n",
            "@BadDec\n",
            "class MyClass:\n",
            "    pass\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    /// decorator_fn_on_fn_class_decorator_ok のテスト。
    #[test]
    fn decorator_fn_on_fn_class_decorator_ok() {
        assert!(ok(concat!(
            "class Retry:\n",
            "    fn __init__(self, let f: function) -> None:\n",
            "        pass\n",
            "    fn __call__(self) -> function:\n",
            "        pass\n",
            "@Retry\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        )));
    }

    /// decorator_stacked_both_valid_ok のテスト。
    #[test]
    fn decorator_stacked_both_valid_ok() {
        assert!(ok(concat!(
            "fn log(let f: function) -> function:\n",
            "    return f\n",
            "fn retry(let f: function) -> function:\n",
            "    return f\n",
            "@log\n",
            "@retry\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        )));
    }

    /// decorator_stacked_second_wrong_err のテスト。
    #[test]
    fn decorator_stacked_second_wrong_err() {
        let errors = check(concat!(
            "fn good(let f: function) -> function:\n",
            "    return f\n",
            "fn bad(let x: int) -> function:\n",
            "    return x\n",
            "@good\n",
            "@bad\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors
            .iter()
            .any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    // --- Collection generics type checking ---

    /// list_of_int_matches_list_of_int_ok のテスト。
    #[test]
    fn list_of_int_matches_list_of_int_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs: list[int] = [1, 2, 3]\n",
            "f(xs)\n",
        )));
    }

    /// list_of_str_to_list_of_int_err のテスト。
    #[test]
    fn list_of_str_to_list_of_int_err() {
        assert!(err(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs: list[str] = [\"a\", \"b\"]\n",
            "f(xs)\n",
        )));
    }

    /// list_literal_inferred_as_list_of_int_ok のテスト。
    #[test]
    fn list_literal_inferred_as_list_of_int_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "f([1, 2, 3])\n",
        )));
    }

    /// list_literal_wrong_elem_type_err のテスト。
    #[test]
    fn list_literal_wrong_elem_type_err() {
        assert!(err(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "f([\"a\", \"b\"])\n",
        )));
    }

    /// untyped_list_matches_list_of_any_ok のテスト。
    #[test]
    fn untyped_list_matches_list_of_any_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs = []\n",
            "f(xs)\n",
        )));
    }

    /// set_of_int_to_set_of_str_err のテスト。
    #[test]
    fn set_of_int_to_set_of_str_err() {
        assert!(err(concat!(
            "fn f(s: set[str]) -> int:\n",
            "    return 0\n",
            "let xs: set[int] = {1, 2}\n",
            "f(xs)\n",
        )));
    }

    /// dict_of_str_int_ok のテスト。
    #[test]
    fn dict_of_str_int_ok() {
        assert!(ok(concat!(
            "fn f(d: dict[str,int]) -> int:\n",
            "    return 0\n",
            "let d: dict[str,int] = {\"a\": 1}\n",
            "f(d)\n",
        )));
    }

    /// dict_key_type_mismatch_err のテスト。
    #[test]
    fn dict_key_type_mismatch_err() {
        assert!(err(concat!(
            "fn f(d: dict[str,int]) -> int:\n",
            "    return 0\n",
            "let d: dict[int,int] = {1: 2}\n",
            "f(d)\n",
        )));
    }
