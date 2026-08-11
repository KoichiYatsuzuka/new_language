// frontend_tests/parser_tests.rs — 構文解析器(Parser)の単体テスト。

    use crate::ast::{BinOp, CallArg, Expr, FieldKind, Stmt, UnaryOp};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// テスト用ヘルパー: ソース文字列を解析して AST を返す。
    fn parse(source: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(source, "").tokenize();
        Parser::new(tokens, None)
            .parse_program()
            .expect("parse error")
    }

    /// テスト用ヘルパー: パースエラーが発生することを確認してエラーメッセージを返す。
    fn parse_fails(source: &str) -> String {
        let tokens = Lexer::new(source, "").tokenize();
        Parser::new(tokens, None)
            .parse_program()
            .expect_err("expected parse error")
    }

    /// literal_expr のテスト。
    #[test]
    fn test_literal_expr() {
        let stmts = parse("42");
        assert!(matches!(stmts[0], Stmt::Expr(Expr::Int(42))));
    }

    /// freeze_stmt のテスト。
    #[test]
    fn test_freeze_stmt() {
        let stmts = parse("mut x = 5\nfreeze x\n");
        assert!(matches!(&stmts[0], Stmt::Mut(name, ..) if name == "x")); // ..: ignores type_ann field
        assert!(matches!(&stmts[1], Stmt::Freeze(name, ..) if name == "x"));
    }

    /// freeze_requires_ident のテスト。
    #[test]
    fn test_freeze_requires_ident() {
        let tokens = crate::lexer::Lexer::new("freeze 42\n", "").tokenize();
        let err = Parser::new(tokens, None)
            .parse_program()
            .expect_err("expected parse error");
        assert!(err.contains("expected identifier"), "got: {err}");
    }

    /// let_decl のテスト。
    #[test]
    fn test_let_decl() {
        let stmts = parse("let x = 10");
        assert!(matches!(&stmts[0], Stmt::Let(name, _, Expr::Int(10)) if name == "x"));
    }

    /// mut_decl のテスト。
    #[test]
    fn test_mut_decl() {
        let stmts = parse("mut y = 3.14");
        assert!(matches!(&stmts[0], Stmt::Mut(name, _, Expr::Float(_)) if name == "y"));
    }

    /// assign のテスト。
    #[test]
    fn test_assign() {
        let stmts = parse("mut x = 0\nx = 5");
        assert!(matches!(&stmts[1], Stmt::Assign { name, value: Expr::Int(5), .. } if name == "x"));
    }

    /// compound_assign のテスト。
    #[test]
    fn test_compound_assign() {
        let stmts = parse("mut x = 0\nx += 1");
        assert!(matches!(
            &stmts[1],
            Stmt::CompoundAssign { name, op: BinOp::Add, value: Expr::Int(1), .. } if name == "x"
        ));
    }

    /// binop_precedence のテスト。
    #[test]
    fn test_binop_precedence() {
        let stmts = parse("2 + 3 * 4");
        if let Stmt::Expr(Expr::BinOp {
            op: BinOp::Add,
            right,
            ..
        }) = &stmts[0]
        {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Mul, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    /// call_expr のテスト。
    #[test]
    fn test_call_expr() {
        let stmts = parse(r#"print("hello")"#);
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Call { .. })));
    }

    /// unary_neg のテスト。
    #[test]
    fn test_unary_neg() {
        let stmts = parse("-5");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::UnaryOp {
                op: UnaryOp::Neg,
                ..
            })
        ));
    }

    /// power_right_assoc のテスト。
    #[test]
    fn test_power_right_assoc() {
        let stmts = parse("2 ** 3 ** 2");
        if let Stmt::Expr(Expr::BinOp {
            op: BinOp::Pow,
            right,
            ..
        }) = &stmts[0]
        {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Pow, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    /// if_stmt のテスト。
    #[test]
    fn test_if_stmt() {
        let stmts = parse("if True:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::If { branches, else_body: None } if branches.len() == 1));
    }

    /// if_else_stmt のテスト。
    #[test]
    fn test_if_else_stmt() {
        let stmts = parse("if True:\n    pass\nelse:\n    pass\n");
        assert!(matches!(
            &stmts[0],
            Stmt::If {
                else_body: Some(_),
                ..
            }
        ));
    }

    /// if_elif_else_stmt のテスト。
    #[test]
    fn test_if_elif_else_stmt() {
        let stmts = parse("if True:\n    pass\nelif False:\n    pass\nelse:\n    pass\n");
        if let Stmt::If {
            branches,
            else_body,
        } = &stmts[0]
        {
            assert_eq!(branches.len(), 2);
            assert!(else_body.is_some());
        } else {
            panic!("expected If");
        }
    }

    /// while_stmt のテスト。
    #[test]
    fn test_while_stmt() {
        let stmts = parse("while True:\n    break\n");
        assert!(matches!(&stmts[0], Stmt::While { .. }));
    }

    /// for_stmt のテスト。
    #[test]
    fn test_for_stmt() {
        let stmts = parse("for i in [1, 2, 3]:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::For { targets, .. } if targets == &["i"]));
    }

    /// block_stmt のテスト。
    #[test]
    fn test_block_stmt() {
        let stmts = parse("block:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::Block(_)));
    }

    /// list_literal のテスト。
    #[test]
    fn test_list_literal() {
        let stmts = parse("[1, 2, 3]");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::List(_))));
    }

    // --- fn ---

    /// fn_def のテスト。
    #[test]
    fn test_fn_def() {
        let stmts = parse("fn add(a, b):\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "add"));
    }

    /// fn_no_params のテスト。
    #[test]
    fn test_fn_no_params() {
        let stmts = parse("fn hello():\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert!(params.is_empty());
        } else {
            panic!("expected FnDef");
        }
    }

    /// fn_mut_param のテスト。
    #[test]
    fn test_fn_mut_param() {
        let stmts = parse("fn modify(mut x):\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert!(params[0].mutable);
            assert_eq!(params[0].name, "x");
        } else {
            panic!("expected FnDef");
        }
    }

    /// fn_type_annotations のテスト。
    #[test]
    fn test_fn_type_annotations() {
        let stmts = parse("fn add(a: int, b: int) -> int:\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "a");
            assert_eq!(params[1].name, "b");
        } else {
            panic!("expected FnDef");
        }
    }

    /// fn_generic_type_annotation のテスト。
    #[test]
    fn test_fn_generic_type_annotation() {
        let stmts = parse("fn first(items: list[int]) -> int:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "first"));
    }

    /// fn_with_body のテスト。
    #[test]
    fn test_fn_with_body() {
        let stmts = parse("fn abs(x):\n    if x < 0:\n        return -x\n    return x\n");
        if let Stmt::FnDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 2);
        } else {
            panic!("expected FnDef");
        }
    }

    // --- class ---

    /// class_empty のテスト。
    #[test]
    fn test_class_empty() {
        let stmts = parse("class Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::ClassDef { name, bases, .. }
            if name == "Foo" && bases.is_empty()));
    }

    /// class_with_non_trait_base_errors のテスト。
    #[test]
    fn test_class_with_non_trait_base_errors() {
        let err = parse_fails("class Bar(Foo):\n    pass\n");
        assert!(err.contains("cannot inherit from `Foo`"), "got: {err}");
    }

    /// class_multiple_non_trait_bases_errors のテスト。
    #[test]
    fn test_class_multiple_non_trait_bases_errors() {
        let err = parse_fails("class C(A, B):\n    pass\n");
        assert!(err.contains("cannot inherit from"), "got: {err}");
    }

    /// protected_in_class_is_parse_err のテスト。
    #[test]
    fn test_protected_in_class_is_parse_err() {
        let err = parse_fails("class MyClass:\n    protected:\n    mut z: int\n");
        assert!(err.contains("ParseError"), "got: {err}");
        assert!(err.contains("protected"), "got: {err}");
    }

    /// class_with_method のテスト。
    #[test]
    fn test_class_with_method() {
        let stmts = parse("class Foo:\n    fn greet(self):\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, .. } if name == "greet"));
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_multiple_methods のテスト。
    #[test]
    fn test_class_multiple_methods() {
        let src = "class Counter:\n    fn inc(mut self):\n        pass\n    fn dec(mut self):\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 2);
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_method_with_params のテスト。
    #[test]
    fn test_class_method_with_params() {
        let src = "class Adder:\n    fn add(self, a: int, b: int) -> int:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            if let Stmt::FnDef { params, .. } = &body[0] {
                assert_eq!(params.len(), 3); // self, a, b
            } else {
                panic!("expected FnDef");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_with_field_and_method のテスト。
    #[test]
    fn test_class_with_field_and_method() {
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n    fn move(mut self, dx: int, dy: int) -> None:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 3);
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_field_parsed_as_field_stmt のテスト。
    #[test]
    fn test_class_field_parsed_as_field_stmt() {
        let src = "class Foo:\n    mut x: int = 0\n    let y: str = \"\"\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(
                matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "x" && type_ann == "int")
            );
            assert!(
                matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "y" && type_ann == "str")
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_generated のテスト。
    #[test]
    fn test_class_auto_init_generated() {
        let src = "class Point:\n    mut x: int\n    mut y: int\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(
                init.is_some(),
                "auto __init__ should be present for required fields"
            );
            if let Some(Stmt::FnDef {
                params,
                return_type,
                ..
            }) = init
            {
                assert_eq!(params.len(), 3); // self + x + y
                assert_eq!(params[0].name, "self");
                assert_eq!(params[1].name, "x");
                assert_eq!(params[2].name, "y");
                assert_eq!(params[1].type_ann.as_deref(), Some("int"));
                assert_eq!(return_type.as_deref(), Some("None"));
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_not_generated_all_fields_have_defaults のテスト。
    #[test]
    fn test_class_auto_init_not_generated_all_fields_have_defaults() {
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(
                init.is_none(),
                "no auto __init__ when all fields have defaults"
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_generated_with_list_field のテスト。
    #[test]
    fn test_class_auto_init_generated_with_list_field() {
        let src = "class Foo:\n    mut items: list[int]\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(
                init.is_some(),
                "auto __init__ should be present for required fields"
            );
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params[1].type_ann.as_deref(), Some("list[int]"));
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_override_exact_match のテスト。
    #[test]
    fn test_class_auto_init_override_exact_match() {
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body
                .iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(
                inits.len(),
                1,
                "exact-match explicit __init__ overrides auto-init"
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_overload_different_sig のテスト。
    #[test]
    fn test_class_auto_init_overload_different_sig() {
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int, y: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body
                .iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(
                inits.len(),
                2,
                "different-sig explicit __init__ + auto-init both present"
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_auto_init_not_generated_without_required_fields のテスト。
    #[test]
    fn test_class_auto_init_not_generated_without_required_fields() {
        let src = "class Foo:\n    fn greet(self) -> str:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(
                init.is_none(),
                "no auto __init__ when there are no required fields"
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// class_field_requires_type_annotation のテスト。
    #[test]
    fn test_class_field_requires_type_annotation() {
        let result = std::panic::catch_unwind(|| parse("class Foo:\n    mut x = 0\n"));
        assert!(
            result.is_err(),
            "missing type annotation should cause a parse error"
        );
    }

    /// nested_if のテスト。
    #[test]
    fn test_nested_if() {
        let src = "if True:\n    if False:\n        pass\n    pass\n";
        let stmts = parse(src);
        if let Stmt::If { branches, .. } = &stmts[0] {
            assert_eq!(branches[0].1.len(), 2);
        } else {
            panic!("expected If");
        }
    }

    // --- keyword arguments ---

    /// call_positional_args のテスト。
    #[test]
    fn test_call_positional_args() {
        let stmts = parse("f(1, 2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Positional(_)));
            assert!(matches!(&args[1], CallArg::Positional(_)));
        } else {
            panic!("expected Call");
        }
    }

    /// call_keyword_arg のテスト。
    #[test]
    fn test_call_keyword_arg() {
        let stmts = parse("f(x=1, y=2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Keyword { name, .. } if name == "x"));
            assert!(matches!(&args[1], CallArg::Keyword { name, .. } if name == "y"));
        } else {
            panic!("expected Call");
        }
    }

    /// call_mixed_args のテスト。
    #[test]
    fn test_call_mixed_args() {
        let stmts = parse("f(1, y=2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Positional(_)));
            assert!(matches!(&args[1], CallArg::Keyword { name, .. } if name == "y"));
        } else {
            panic!("expected Call");
        }
    }

    // --- trait ---

    /// trait_empty のテスト。
    #[test]
    fn test_trait_empty() {
        let stmts = parse("trait Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::TraitDef { name, .. } if name == "Foo"));
    }

    /// trait_with_fields のテスト。
    #[test]
    fn test_trait_with_fields() {
        let stmts = parse("trait HasName:\n    mut name: str\n    let id: int\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(
                matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "name" && type_ann == "str")
            );
            assert!(
                matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "id" && type_ann == "int")
            );
        } else {
            panic!("expected TraitDef");
        }
    }

    /// trait_virtual_method_is_abstract のテスト。
    #[test]
    fn test_trait_virtual_method_is_abstract() {
        let stmts = parse("trait Animal:\n    fn speak(self) -> str:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(
                matches!(&body[0], Stmt::FnDef { name, is_abstract: true, .. } if name == "speak"),
                "method with `...` body should have is_abstract: true"
            );
        } else {
            panic!("expected TraitDef");
        }
    }

    /// trait_non_virtual_method_is_not_virtual のテスト。
    #[test]
    fn test_trait_non_virtual_method_is_not_virtual() {
        let stmts = parse("trait Logger:\n    fn log(self, msg: str) -> None:\n        pass\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(
                matches!(&body[0], Stmt::FnDef { name, is_abstract: false, .. } if name == "log"),
                "method with real body should have is_abstract: false"
            );
        } else {
            panic!("expected TraitDef");
        }
    }

    /// trait_virtual_body_is_empty のテスト。
    #[test]
    fn test_trait_virtual_body_is_empty() {
        let stmts = parse("trait T:\n    fn f(self) -> int:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            if let Stmt::FnDef {
                body: fn_body,
                is_abstract,
                ..
            } = &body[0]
            {
                assert!(*is_abstract);
                assert!(fn_body.is_empty(), "virtual method body should be empty");
            } else {
                panic!("expected FnDef");
            }
        } else {
            panic!("expected TraitDef");
        }
    }

    /// trait_cannot_inherit のテスト。
    #[test]
    fn test_trait_cannot_inherit() {
        let result = std::panic::catch_unwind(|| parse("trait Foo(Bar):\n    pass\n"));
        assert!(
            result.is_err(),
            "trait with base class should cause a parse error"
        );
    }

    /// class_inherits_trait_ok のテスト。
    #[test]
    fn test_class_inherits_trait_ok() {
        let stmts = parse(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
            "class Dog(Animal):\n",
            "    fn speak(self) -> str:\n",
            "        pass\n",
        ));
        assert_eq!(stmts.len(), 2);
        assert!(matches!(&stmts[0], Stmt::TraitDef { name, .. } if name == "Animal"));
        assert!(matches!(&stmts[1], Stmt::ClassDef { name, .. } if name == "Dog"));
    }

    /// class_missing_virtual_override_error のテスト。
    #[test]
    fn test_class_missing_virtual_override_error() {
        let result = std::panic::catch_unwind(|| {
            parse(concat!(
                "trait Animal:\n",
                "    fn speak(self) -> str:\n",
                "        ...\n",
                "class Cat(Animal):\n",
                "    pass\n",
            ))
        });
        assert!(
            result.is_err(),
            "missing virtual method override should cause a parse error"
        );
    }

    /// class_inherits_trait_combined_init_generated のテスト。
    #[test]
    fn test_class_inherits_trait_combined_init_generated() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "combined __init__ should be generated");
            if let Some(Stmt::FnDef {
                params,
                return_type,
                ..
            }) = init
            {
                assert_eq!(params.len(), 3);
                assert_eq!(params[0].name, "self");
                assert_eq!(params[1].name, "x");
                assert_eq!(params[2].name, "y");
                assert_eq!(params[1].type_ann.as_deref(), Some("int"));
                assert_eq!(params[2].type_ann.as_deref(), Some("int"));
                assert_eq!(return_type.as_deref(), Some("None"));
            }
        } else {
            panic!("expected ClassDef at stmts[1]");
        }
    }

    /// class_inherits_trait_combined_init_body_uses_trait_access のテスト。
    #[test]
    fn test_class_inherits_trait_combined_init_body_uses_trait_access() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            if let Some(Stmt::FnDef {
                body: init_body, ..
            }) = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
            {
                assert!(
                    matches!(&init_body[0],
                        Stmt::AttrAssign { target: Expr::TraitAccess { trait_name, attr, .. }, .. }
                        if trait_name == "HasX" && attr == "x"
                    ),
                    "trait field assignment should use TraitAccess"
                );
                assert!(
                    matches!(&init_body[1],
                        Stmt::AttrAssign { target: Expr::Attr { attr, .. }, .. }
                        if attr == "y"
                    ),
                    "class field assignment should use Attr"
                );
            } else {
                panic!("__init__ not found");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    /// trait_access_expr_parsed のテスト。
    #[test]
    fn test_trait_access_expr_parsed() {
        let stmts = parse("self::MyTrait.field\n");
        if let Stmt::Expr(Expr::TraitAccess {
            trait_name, attr, ..
        }) = &stmts[0]
        {
            assert_eq!(trait_name, "MyTrait");
            assert_eq!(attr, "field");
        } else {
            panic!("expected Stmt::Expr(Expr::TraitAccess)");
        }
    }

    /// fn_is_not_virtual_by_default のテスト。
    #[test]
    fn test_fn_is_not_virtual_by_default() {
        let stmts = parse("fn hello() -> None:\n    pass\n");
        assert!(matches!(
            &stmts[0],
            Stmt::FnDef {
                is_abstract: false,
                ..
            }
        ));
    }

    /// class_method_is_not_virtual のテスト。
    #[test]
    fn test_class_method_is_not_virtual() {
        let stmts = parse("class Foo:\n    fn greet(self) -> str:\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(
                matches!(&body[0], Stmt::FnDef { name, is_abstract: false, .. } if name == "greet")
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// trait_combined_init_override_by_exact_match のテスト。
    #[test]
    fn test_trait_combined_init_override_by_exact_match() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Foo(HasX):\n",
            "    mut y: int\n",
            "    fn __init__(mut self, x: int, y: int) -> None:\n",
            "        pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let inits: Vec<_> = body
                .iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(
                inits.len(),
                1,
                "exact-match explicit __init__ should override auto-init"
            );
        } else {
            panic!("expected ClassDef");
        }
    }

    /// trait_with_multiple_virtual_methods_all_must_be_overridden のテスト。
    #[test]
    fn test_trait_with_multiple_virtual_methods_all_must_be_overridden() {
        let result = std::panic::catch_unwind(|| {
            parse(concat!(
                "trait Ops:\n",
                "    fn add(self, x: int) -> int:\n",
                "        ...\n",
                "    fn sub(self, x: int) -> int:\n",
                "        ...\n",
                "class MyOps(Ops):\n",
                "    fn add(self, x: int) -> int:\n",
                "        pass\n",
            ))
        });
        assert!(
            result.is_err(),
            "not overriding all virtual methods should be a parse error"
        );
    }

    /// trait_class_only_trait_required_fields_no_class_fields のテスト。
    #[test]
    fn test_trait_class_only_trait_required_fields_no_class_fields() {
        let stmts = parse(concat!(
            "trait Named:\n",
            "    mut name: str\n",
            "class Widget(Named):\n",
            "    pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body
                .iter()
                .find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some());
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params.len(), 2); // self + name
                assert_eq!(params[1].name, "name");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    // ---- alias (compile-time AST substitution) ----

    /// alias 名が型注釈位置で右辺の型に展開される。
    #[test]
    fn test_alias_expands_in_type_position() {
        let stmts = parse("alias handle: int\nlet x: handle = 5\n");
        // alias 定義は Pass に消去され、let の型注釈は "int" に展開される。
        assert!(matches!(&stmts[0], Stmt::Pass));
        assert!(
            matches!(&stmts[1], Stmt::Let(name, Some(ty), Expr::Int(5)) if name == "x" && ty == "int"),
            "got: {:?}",
            stmts[1]
        );
    }

    /// alias 名が式位置で右辺の式に展開される。
    #[test]
    fn test_alias_expands_in_expr_position() {
        let stmts = parse("alias handle: int\nlet y = handle\n");
        assert!(
            matches!(&stmts[1], Stmt::Let(name, None, Expr::Ident { name: id, .. }) if name == "y" && id == "int"),
            "got: {:?}",
            stmts[1]
        );
    }

    /// lvalue への alias は代入対象として透過的に展開される（AttrAssign へのルーティング）。
    #[test]
    fn test_alias_lvalue_transparent_assignment() {
        let stmts = parse(concat!(
            "mut d: dict[str, int] = {\"k\": 1}\n",
            "alias item: d[\"k\"]\n",
            "item = 5\n",
        ));
        // `item = 5` は `d["k"] = 5`（Subscript を target とする AttrAssign）になる。
        match &stmts[2] {
            Stmt::AttrAssign { target, value } => {
                assert!(matches!(target, Expr::Subscript { .. }), "target: {:?}", target);
                assert!(matches!(value, Expr::Int(5)));
            }
            other => panic!("expected AttrAssign, got {:?}", other),
        }
    }

    /// 既知テンプレートの `Base[Arg]` alias はテンプレート具体化として解釈される。
    #[test]
    fn test_alias_template_instantiation() {
        let stmts = parse(concat!(
            "class Box[T]:\n",
            "    mut item: T\n",
            "alias IntBox: Box[int]\n",
            "let b = IntBox(1)\n",
        ));
        // stmts: [0] ClassDef, [1] Pass(alias), [2] Let("b", ...)
        // `IntBox(1)` → `Box[int](1)`（func が TemplateInstantiate の Call）。
        match &stmts[2] {
            Stmt::Let(name, _, Expr::Call { func, .. }) if name == "b" => {
                assert!(
                    matches!(func.as_ref(), Expr::TemplateInstantiate { .. }),
                    "func: {:?}",
                    func
                );
            }
            other => panic!("expected Let with Call, got {:?}", other),
        }
    }

    /// block 式（値専用）の alias を型注釈に使うとパースエラー。
    #[test]
    fn test_alias_block_expr_not_usable_as_type() {
        let err = parse_fails(concat!(
            "alias f: block->function:\n",
            "    block_return 1\n",
            "let x: f = 1\n",
        ));
        assert!(err.contains("cannot be used as a type"), "got: {err}");
    }

    /// 同一スコープでの alias 再定義はパースエラー。
    #[test]
    fn test_alias_redefinition_is_error() {
        let err = parse_fails("alias a: int\nalias a: str\n");
        assert!(err.contains("already defined"), "got: {err}");
    }

    /// alias はブロックスコープ: 宣言したブロックを抜けると不可視になる。
    #[test]
    fn test_alias_is_block_scoped() {
        let stmts = parse(concat!(
            "fn f() -> int:\n",
            "    alias k: 1\n",
            "    return k\n",
            "let y = k\n",
        ));
        // stmts: [0] FnDef, [1] Let("y", ...) — 関数外の `k` は alias 展開されず素の識別子。
        assert!(
            matches!(&stmts[1], Stmt::Let(name, None, Expr::Ident { name: id, .. }) if name == "y" && id == "k"),
            "got: {:?}",
            stmts[1]
        );
    }
