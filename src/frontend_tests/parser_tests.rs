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
        assert!(matches!(&stmts[0], Stmt::Let(name, _, Expr::Int(10), _) if name == "x"));
    }

    /// mut_decl のテスト。
    #[test]
    fn test_mut_decl() {
        let stmts = parse("mut y = 3.14");
        assert!(matches!(&stmts[0], Stmt::Mut(name, _, Expr::Float(_), _) if name == "y"));
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
        assert!(matches!(&stmts[0], Stmt::If { branches, else_body: None, span: _ } if branches.len() == 1));
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
            span: _,
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
            matches!(&stmts[1], Stmt::Let(name, Some(ty), Expr::Int(5), _) if name == "x" && ty == "int"),
            "got: {:?}",
            stmts[1]
        );
    }

    /// alias 名が式位置で右辺の式に展開される。
    #[test]
    fn test_alias_expands_in_expr_position() {
        let stmts = parse("alias handle: int\nlet y = handle\n");
        assert!(
            matches!(&stmts[1], Stmt::Let(name, None, Expr::Ident { name: id, .. }, _) if name == "y" && id == "int"),
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
            Stmt::AttrAssign { target, value, span: _ } => {
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
            Stmt::Let(name, _, Expr::Call { func, .. }, _) if name == "b" => {
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
            matches!(&stmts[1], Stmt::Let(name, None, Expr::Ident { name: id, .. }, _) if name == "y" && id == "k"),
            "got: {:?}",
            stmts[1]
        );
    }
    // -----------------------------------------------------------------------
    // 内包表記 — Arrow ネイティブ構文と Python 変換が**同じ AST**になること
    // -----------------------------------------------------------------------

    /// 式文 1 本を取り出すヘルパー。
    fn sole_expr(stmts: &[Stmt]) -> &Expr {
        match &stmts[0] {
            Stmt::Expr(e) => e,
            other => panic!("expected an expression statement, got: {other:?}"),
        }
    }

    /// 内包表記が `for` 式 + `loop_yield` に脱糖されていることを検証する。
    ///
    /// 期待する形（`[v * 2 for v in xs if v > 1]`）:
    /// `ForExpr { targets: ["v"], return_type: Some("list[Any]"),
    ///            body: [If { branches: [(cond, [LoopYield(elt)])], else_body: None }] }`
    fn assert_desugared_comprehension(e: &Expr) {
        let Expr::ForExpr {
            targets,
            body,
            return_type,
            ..
        } = e
        else {
            panic!("expected Expr::ForExpr, got: {e:?}");
        };
        assert_eq!(targets, &["v".to_string()]);
        // ⚠ 要素型は付けない。`->list[T]` にすると loop_yield の実行時型検査が走ってしまう。
        assert_eq!(return_type.as_deref(), Some("list[Any]"));
        assert_eq!(body.len(), 1, "body: {body:?}");
        let Stmt::If {
            branches,
            else_body,
            span: _,
        } = &body[0]
        else {
            panic!("expected the filter to become Stmt::If, got: {:?}", body[0]);
        };
        assert!(else_body.is_none());
        assert_eq!(branches.len(), 1);
        assert!(
            matches!(branches[0].1.as_slice(), [Stmt::LoopYield(_, _)]),
            "expected the element to become Stmt::LoopYield, got: {:?}",
            branches[0].1
        );
    }

    /// Arrow のネイティブ内包表記が `for` 式 + `loop_yield` に脱糖される。
    #[test]
    fn test_list_comprehension_desugars_to_for_expr() {
        let stmts = parse("[v * 2 for v in xs if v > 1]\n");
        assert_desugared_comprehension(sole_expr(&stmts));
    }

    /// ★ Python から変換した内包表記が**ネイティブ構文と同じ AST**になる。
    ///
    /// 両者は `ast::build_list_comprehension` という**同じ脱糖関数**を通す。
    /// ここが崩れると「Arrow で書いた内包表記と Python 由来の内包表記で挙動が違う」
    /// という気付きにくい形になるので、構造を固定しておく。
    #[test]
    fn test_python_list_comprehension_matches_native_ast() {
        let native = parse("[v * 2 for v in xs if v > 1]\n");
        assert_desugared_comprehension(sole_expr(&native));

        let converted = crate::python_converter::convert_python_source(
            "[v * 2 for v in xs if v > 1]\n",
            "<test>",
        )
        .expect("python conversion failed");
        assert_desugared_comprehension(sole_expr(&converted));
    }

    /// 多重 `for` は「先頭だけが `for` 式、2 つ目以降は本体の `Stmt::For`」になる。
    #[test]
    fn test_nested_comprehension_shape() {
        let stmts = parse("[a * b for a in xs for b in ys]\n");
        let Expr::ForExpr { targets, body, .. } = sole_expr(&stmts) else {
            panic!("expected Expr::ForExpr");
        };
        assert_eq!(targets, &["a".to_string()]);
        assert!(
            matches!(body.as_slice(), [Stmt::For { targets, body: inner, .. }]
                if targets == &["b".to_string()]
                    && matches!(inner.as_slice(), [Stmt::LoopYield(_, _)])),
            "got: {body:?}"
        );
    }

    /// 辞書内包は明示エラー（Arrow にペアのリストから dict を作る手段が無い）。
    #[test]
    fn test_dict_comprehension_is_rejected() {
        let err = parse_fails("{k: v for k in xs}\n");
        assert!(err.contains("dict comprehension"), "got: {err}");
    }

    // -----------------------------------------------------------------------
    // Python の `...`（Ellipsis）— 文位置と値位置で扱いが違う
    // -----------------------------------------------------------------------

    /// 文としての `...`（スタブ本体）は `pass` に読み替えられる。
    #[test]
    fn test_python_statement_ellipsis_becomes_pass() {
        let stmts =
            crate::python_converter::convert_python_source("def f():\n    ...\n", "<test>")
                .expect("python conversion failed");
        let Stmt::FnDef { body, .. } = &stmts[0] else {
            panic!("expected Stmt::FnDef, got: {:?}", stmts[0]);
        };
        assert!(
            matches!(body.as_slice(), [Stmt::Pass]),
            "expected the body to be a single Stmt::Pass, got: {body:?}"
        );
    }

    /// 値としての `...` は `None` のまま（Arrow に `Ellipsis` 値が無いため。承認済みの仕様）。
    ///
    /// ⚠ 文位置だけを `pass` にしたので、値位置が巻き添えで変わっていないことを固定する。
    #[test]
    fn test_python_value_ellipsis_stays_none() {
        let stmts =
            crate::python_converter::convert_python_source("def f():\n    x = ...\n    return x\n", "<test>")
                .expect("python conversion failed");
        let Stmt::FnDef { body, .. } = &stmts[0] else {
            panic!("expected Stmt::FnDef");
        };
        // #2 の巻き上げにより body は [Mut("x", None, None), Assign{..}, Return]。
        assert!(
            body.iter().any(|s| matches!(s, Stmt::Assign { value: Expr::None, .. })),
            "expected an assignment of Expr::None, got: {body:?}"
        );
        assert!(
            !body.iter().any(|s| matches!(s, Stmt::Pass)),
            "value-position `...` must not become `pass`, got: {body:?}"
        );
    }

// ---------------------------------------------------------------------------
// 同梱 `.pyi` スタブ（BYTECODE_VM_PLAN #19 / 群6 S4）
// ---------------------------------------------------------------------------

/// 同梱スタブは**すべて `convert_python_source` をエラー無しで通る**こと。
///
/// ⚠⚠ `load_py_type_body` は変換エラーを握り潰して行ベース抽出へ落ちる。
/// つまりスタブに未対応構文を書いても**黙って精度が落ちるだけ**で気付けない。
/// ⇒ スタブを足したり書き換えたりしたときに、ここで落ちるようにしておく。
#[test]
fn bundled_stubs_convert_without_error() {
    for name in crate::py_stubs::builtin_stub_names() {
        let src = crate::py_stubs::builtin_stub(&[name.to_string()])
            .unwrap_or_else(|| panic!("同梱スタブ '{name}' が引けない"));
        let converted = crate::python_converter::convert_python_source(src, name);
        assert!(
            converted.is_ok(),
            "同梱スタブ '{name}' が変換できない: {:?}",
            converted.err()
        );
    }
}

/// 同梱スタブの関数が**戻り値型つきで**取り込まれること。
///
/// ⚠ 変換が通っても `-> float` が落ちていれば型予測は効かない（`Any` になる）。
/// 「変換できた」だけでは足りないので、戻り値型まで見る。
#[test]
fn bundled_stubs_carry_return_types() {
    use crate::ast::Stmt;
    let src = crate::py_stubs::builtin_stub(&["math".to_string()]).expect("math スタブ");
    let body = crate::python_converter::convert_python_source(src, "math").expect("変換できる");
    let sqrt = body
        .iter()
        .find_map(|s| match s {
            Stmt::FnDef { name, return_type, .. } if name == "sqrt" => Some(return_type.clone()),
            _ => None,
        })
        .expect("math スタブに sqrt がある");
    assert_eq!(sqrt.as_deref(), Some("float"), "sqrt の戻り値型が落ちている");
}

// ── メタ関数（コンパイル時展開）の構文 ─────────────────────────────────
// 設計は implementation_logs/comptime_metafn_design.md §1。タスク 1-1。

/// `exprconst fn` が純粋メタ関数として、`exprconst !fn` が配置メタ関数として解析されること。
#[test]
fn metafn_def_distinguishes_pure_and_placing() {
    use crate::ast::Stmt;
    let stmts = parse("exprconst fn pure() -> Code:\n    pass\nexprconst !fn place() -> None:\n    pass\n");
    let flags: Vec<(String, bool)> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::MetaFnDef { name, is_placing, .. } => Some((name.clone(), *is_placing)),
            _ => None,
        })
        .collect();
    assert_eq!(flags, vec![("pure".to_string(), false), ("place".to_string(), true)]);
}

/// ⚠ `code:` の中身は**パースされない**。本体の無い `if` のような未完の断片が書けることが要件。
/// 行とインデント段数だけが取れていればよい。
#[test]
fn code_block_keeps_unparsable_fragment_as_lines() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("exprconst fn f() -> Code:\n    let a = code:\n        if x == None:\n            pass\n    return a\n");
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::CodeBlock(lines), _) = &body[0] else { panic!("CodeBlock を期待") };
    assert_eq!(lines.len(), 2, "`if` 行と `pass` 行の 2 行");
    assert_eq!(lines[0].indent, 0, "断片先頭からの相対インデント");
    assert_eq!(lines[1].indent, 1, "`pass` は 1 段深い");
}

/// 空の `code:`（直後にインデントが来ない）は**空の Code** になる。設計書 §1.2 の B-2。
#[test]
fn empty_code_block_is_allowed() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("exprconst fn f() -> Code:\n    let e = code:\n    return e\n");
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::CodeBlock(lines), _) = &body[0] else { panic!("CodeBlock を期待") };
    assert!(lines.is_empty(), "空ブロックは 0 行");
}

/// ⚠ `code:` の入れ子は禁止。素通しすると「中身は評価されない」規則と相まって、
/// 内側がただのテキストとして埋まる（書いた人の意図と無関係な結果になる）。
#[test]
fn nested_code_block_is_rejected() {
    let err = parse_fails("exprconst fn f() -> Code:\n    let a = code:\n        code:\n            pass\n    return a\n");
    assert!(err.contains("cannot be nested"), "実際のエラー: {err}");
}

/// `exprconst` の後は `fn` か `!fn` だけ。
#[test]
fn exprconst_requires_fn() {
    let err = parse_fails("exprconst class C:\n    let v: int\n");
    assert!(err.contains("must be followed by `fn` or `!fn`"), "実際のエラー: {err}");
}

/// ⚠ メタ関数にテンプレート型パラメータは持たせない。展開はテンプレート実体化より前に走る。
#[test]
fn metafn_rejects_template_params() {
    let err = parse_fails("exprconst fn f[T]() -> Code:\n    pass\n");
    assert!(err.contains("cannot have template parameters"), "実際のエラー: {err}");
}

/// `quote` は配置する `Code` を必ず伴う。
#[test]
fn quote_requires_a_value() {
    let err = parse_fails("exprconst !fn f() -> None:\n    quote\n");
    assert!(err.contains("requires a `Code` value"), "実際のエラー: {err}");
}

/// ⚠⚠ `quote` は**メタ関数の外では書けない**（タスク 1-6）。`Op::Return` に落ちるので、
/// 素通しすると囲む関数を**黙って抜ける**（最上位ならプログラムが exit 0 で終わる。実測）。
#[test]
fn quote_outside_a_metafunction_is_rejected() {
    for src in [
        "quote code:\n    pass\n",
        "fn g() -> None:\n    quote code:\n        pass\n    print(1)\n",
    ] {
        let err = parse_fails(src);
        assert!(
            err.contains("`quote` can only be written inside a metafunction"),
            "実際のエラー: {err}"
        );
    }
}

/// ⚠⚠ `code:` も**メタ関数の外では書けない**（タスク 1-6）。式文として書いた `code:` は
/// 型検査の束縛点（1-5）を通らないので、**中身が一度も走らずに黙って完走した**（実測）。
#[test]
fn code_block_outside_a_metafunction_is_rejected() {
    for src in ["code:\n    print(1)\n", "fn g() -> None:\n    code:\n        print(1)\n"] {
        let err = parse_fails(src);
        assert!(
            err.contains("`code:` blocks can only be written inside a metafunction"),
            "実際のエラー: {err}"
        );
    }
}

/// ⚠ メタ関数の中の**入れ子関数**では書ける（装飾子ファクトリが返すクロージャ・4-8）。
#[test]
fn quote_and_code_are_accepted_in_a_closure_inside_a_metafunction() {
    let stmts = parse(concat!(
        "exprconst fn factory():\n",
        "    fn deco(m) -> None:\n",
        "        quote code:\n",
        "            pass\n",
        "    return deco\n",
    ));
    assert_eq!(stmts.len(), 1);
}

/// `<! 式 !>` は `code:` の中で**唯一パースされる**場所（設計書 §1.4 / タスク 1-2）。
/// 地の文はトークンのまま、スプライスだけが式として取れていること。
#[test]
fn splice_inside_code_block_is_parsed_as_an_expression() {
    use crate::ast::{CodePiece, Expr, Stmt};
    let stmts = parse(
        "exprconst fn f(n) -> Code:\n    let a = code:\n        let <! n !> = 1\n    return a\n",
    );
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::CodeBlock(lines), _) = &body[0] else { panic!("CodeBlock を期待") };
    assert_eq!(lines.len(), 1, "1 行");
    let splices: Vec<&Expr> = lines[0]
        .pieces
        .iter()
        .filter_map(|p| match p {
            CodePiece::Splice(e) => Some(e),
            CodePiece::Token(_) => None,
        })
        .collect();
    assert_eq!(splices.len(), 1, "スプライスは 1 つ");
    let Expr::Ident { name, .. } = splices[0] else { panic!("Ident を期待") };
    assert_eq!(name, "n");
    // 地の文（`let` / `=` / `1`）はトークンのまま残る。
    let tokens = lines[0].pieces.iter().filter(|p| matches!(p, CodePiece::Token(_))).count();
    assert_eq!(tokens, 3, "`let` `=` `1` の 3 トークン");
}

/// ⚠ スプライスの中は**式**なので、呼び出しや演算も書ける。
/// ここが式として取れていないと、展開器が評価すべきものを見失う。
#[test]
fn splice_accepts_a_full_expression() {
    use crate::ast::{CodePiece, Expr, Stmt};
    let stmts = parse(
        "exprconst fn f(n) -> Code:\n    let a = code:\n        <! prefix(n) + \"_x\" !>()\n    return a\n",
    );
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::CodeBlock(lines), _) = &body[0] else { panic!("CodeBlock を期待") };
    let CodePiece::Splice(e) = &lines[0].pieces[0] else { panic!("先頭はスプライス") };
    assert!(matches!(e, Expr::BinOp { .. }), "二項演算として取れていること");
}

/// ⚠ 空のスプライスは書き損じである方が多いので弾く。
#[test]
fn empty_splice_is_rejected() {
    let err =
        parse_fails("exprconst fn f() -> Code:\n    let a = code:\n        <!!>\n    return a\n");
    assert!(err.contains("must contain an expression"), "実際のエラー: {err}");
}

/// ⚠ 閉じ忘れを「地の文」として飲み込まない。
#[test]
fn unclosed_splice_is_rejected() {
    let err = parse_fails(
        "exprconst fn f(n) -> Code:\n    let a = code:\n        let <! n = 1\n    return a\n",
    );
    assert!(err.contains("expected `!>` to close the splice"), "実際のエラー: {err}");
}

/// ⚠ 対応する `<!` の無い `!>` も弾く。素通しすると地の文に紛れて消える。
#[test]
fn unmatched_splice_close_is_rejected() {
    let err = parse_fails(
        "exprconst fn f() -> Code:\n    let a = code:\n        let x = 1 !>\n    return a\n",
    );
    assert!(err.contains("no matching `<!`"), "実際のエラー: {err}");
}

/// `<! !>` を書けるのは `code:` の中だけ（設計書 §1.4）。
/// ⚠ `unexpected token` だけだと「どこでなら書けるのか」が分からないので名指しする。
#[test]
fn splice_outside_a_code_block_is_rejected() {
    let err = parse_fails("let x = <! 1 !>\n");
    assert!(err.contains("can only appear inside a `code:` block"), "実際のエラー: {err}");
}

/// ⚠⚠ `^` は `.` より**強く**束縛する（設計書 §1.5 / 参考B）。
/// `^T.fields` は `(^T).fields`（メタ情報への射影）であって `^(T.fields)` ではない。
/// ここが逆だと参考B の射影一覧がすべて別の意味になる。
#[test]
fn meta_info_binds_tighter_than_attribute_access() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("exprconst fn f(t) -> Code:\n    let a = ^t.fields\n    return code:\n");
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::Attr { object, attr, .. }, _) = &body[0] else {
        panic!("最外は Attr（射影）を期待")
    };
    assert_eq!(attr, "fields");
    assert!(matches!(**object, Expr::MetaInfo(_)), "その内側が `^t`");
}

/// `^Box[int].fields` — テンプレート実体化は**対象側**に含める。
/// ⚠ `[...]` を後置チェーンに任せると `(^Box)[int]` になり、別物になる。
#[test]
fn meta_info_takes_template_instantiation_as_its_target() {
    use crate::ast::{Expr, Stmt};
    let stmts =
        parse("exprconst fn f() -> Code:\n    let a = ^Box[int].fields\n    return code:\n");
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::Attr { object, .. }, _) = &body[0] else { panic!("Attr を期待") };
    let Expr::MetaInfo(target) = &**object else { panic!("`^` が外側にあること") };
    assert!(
        matches!(**target, Expr::TemplateInstantiate { .. } | Expr::Subscript { .. }),
        "対象に `Box[int]` まで含まれること"
    );
}

/// ⚠ 既存の二項 `^`（ビット XOR）を奪っていないこと。前置と中置は位置で区別される。
#[test]
fn caret_is_still_binary_xor_in_infix_position() {
    use crate::ast::{BinOp, Expr, Stmt};
    let stmts = parse("let a = 6 ^ 3\n");
    let Stmt::Let(_, _, Expr::BinOp { op, .. }, _) = &stmts[0] else { panic!("BinOp を期待") };
    assert_eq!(*op, BinOp::BitXor);
}

/// `^` を書けるのは**メタ関数の中か、呼び出しの実引数の中**だけ（設計書 §1.5）。
#[test]
fn meta_info_is_allowed_in_a_call_argument() {
    use crate::ast::{CallArg, Expr, Stmt};
    let stmts = parse("let y = g(^x)\n");
    let Stmt::Let(_, _, Expr::Call { args, .. }, _) = &stmts[0] else { panic!("Call を期待") };
    let CallArg::Positional(arg) = &args[0] else { panic!("位置引数を期待") };
    assert!(matches!(arg, Expr::MetaInfo(_)));
}

/// ⚠ 普通のコードで `^` を書いたら弾く。書ける場所を名指しする。
#[test]
fn meta_info_outside_a_metafn_or_call_argument_is_rejected() {
    let err = parse_fails("let x = 1\nlet y = ^x\n");
    assert!(err.contains("can only be written inside a metafunction"), "実際のエラー: {err}");
}

/// ⚠ `^^` は D10 で廃止。素通しすると `^(^x)` として通ってしまう。
#[test]
fn double_caret_is_rejected() {
    let err = parse_fails("exprconst fn f(x) -> Code:\n    let a = ^^x\n    return code:\n");
    assert!(err.contains("`^^` was removed"), "実際のエラー: {err}");
}

/// `!装飾子` は対象の宣言を**包む**（設計書 §1.6 / タスク 1-4）。
/// ⚠ `@` の `decorators` 欄とは別物。あちらは実行時クロージャ装飾子で据え置き。
#[test]
fn meta_decorator_wraps_its_target() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("!deco\nclass C:\n    mut x: int\n");
    let Stmt::MetaDecorated { decorators, target, .. } = &stmts[0] else {
        panic!("MetaDecorated を期待")
    };
    assert_eq!(decorators.len(), 1);
    assert!(matches!(&decorators[0], Expr::Ident { name, .. } if name == "deco"));
    assert!(matches!(**target, Stmt::ClassDef { .. }));
}

/// 対象は**変数束縛**でもよい（§1.6）。`@` は `fn` / `class` しか取らない。
#[test]
fn meta_decorator_accepts_a_variable_binding() {
    use crate::ast::Stmt;
    let stmts = parse("!deco\nlet v = 1\n");
    let Stmt::MetaDecorated { target, .. } = &stmts[0] else { panic!("MetaDecorated を期待") };
    assert!(matches!(**target, Stmt::Let(..)));
}

/// 対象は**クラスフィールド**でもよい（§1.6）。
#[test]
fn meta_decorator_accepts_a_class_field() {
    use crate::ast::Stmt;
    let stmts = parse("class C:\n    !deco\n    mut x: int\n");
    let Stmt::ClassDef { body, .. } = &stmts[0] else { panic!("ClassDef を期待") };
    let Stmt::MetaDecorated { target, .. } = &body[0] else { panic!("MetaDecorated を期待") };
    assert!(matches!(**target, Stmt::Field { .. }));
}

/// 引数付きの装飾子（`!deco(1, "x")`）も書ける。呼び出し式としてパースされる。
#[test]
fn meta_decorator_can_take_arguments() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("!deco(1, \"x\")\nfn g() -> int:\n    return 1\n");
    let Stmt::MetaDecorated { decorators, .. } = &stmts[0] else { panic!("MetaDecorated を期待") };
    assert!(matches!(&decorators[0], Expr::Call { .. }));
}

/// 複数の装飾子は 1 つの `MetaDecorated` にまとまる（上から順）。
#[test]
fn stacked_meta_decorators_collect_into_one_wrapper() {
    use crate::ast::{Expr, Stmt};
    let stmts = parse("!a\n!b\nfn g() -> int:\n    return 1\n");
    let Stmt::MetaDecorated { decorators, target, .. } = &stmts[0] else {
        panic!("MetaDecorated を期待")
    };
    let names: Vec<&str> = decorators
        .iter()
        .map(|d| match d {
            Expr::Ident { name, .. } => name.as_str(),
            _ => panic!("Ident を期待"),
        })
        .collect();
    assert_eq!(names, vec!["a", "b"], "書いた順に並ぶ");
    assert!(matches!(**target, Stmt::FnDef { .. }));
}

/// ⚠ 対象にできない文を黙って包まない。包むと展開器が扱えない形が AST に入る。
#[test]
fn meta_decorator_on_a_non_declaration_is_rejected() {
    let err = parse_fails("!deco\nprint(1)\n");
    assert!(
        err.contains("can only be applied to a class, function, field or variable binding"),
        "実際のエラー: {err}"
    );
}

/// ⚠ `private:` 節の中の装飾子付きフィールドにもアクセス指定が効くこと。
/// 包みを剥がさずに access を入れると、装飾したフィールドだけ public のまま残る。
#[test]
fn access_section_reaches_through_a_meta_decorator() {
    use crate::ast::{Accessibility, Stmt};
    let stmts = parse("class C:\n    private:\n    !deco\n    mut secret: int\n");
    let Stmt::ClassDef { body, .. } = &stmts[0] else { panic!("ClassDef を期待") };
    let Stmt::MetaDecorated { target, .. } = &body[0] else { panic!("MetaDecorated を期待") };
    let Stmt::Field { access, .. } = &**target else { panic!("Field を期待") };
    assert_eq!(*access, Accessibility::Private);
}

/// ⚠⚠ `quote` は関数を抜ける（設計書 §1.3）。同じブロックのそれ以降は**決して走らない**。
/// 黙って捨てると「書いたのに動かない」——メタ関数で一番避けたい失敗形になる（タスク 3-5）。
#[test]
fn a_statement_after_quote_is_rejected() {
    let err = parse_fails("exprconst !fn f() -> None:\n    quote code:\n        pass\n    print(1)\n");
    assert!(err.contains("never runs"), "実際のエラー: {err}");
}

/// ⚠ **`if` の枝の中の `quote` の後に、その `if` より後ろの文が続くのは正常**。
/// 枝を通らなければ走る。⇒ 見るのは「同じブロックの後ろ」だけ。
#[test]
fn a_quote_inside_an_if_does_not_make_the_rest_unreachable() {
    use crate::ast::Stmt;
    let stmts = parse(
        "exprconst !fn f(n: int) -> None:\n    if n > 0:\n        quote code:\n            pass\n    quote code:\n",
    );
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    assert_eq!(body.len(), 2, "`if` と後続の `quote` の 2 文");
}

/// 入れ子のブロックの中でも同じ規則が効くこと。
#[test]
fn a_statement_after_quote_inside_a_nested_block_is_rejected() {
    let err = parse_fails(
        "exprconst !fn f(n: int) -> None:\n    if n > 0:\n        quote code:\n            pass\n        print(1)\n    quote code:\n",
    );
    assert!(err.contains("never runs"), "実際のエラー: {err}");
}

/// `x is Stack[int]` は型式として読む（フェーズ10 10-10。以前は `[` で ParseError）。
#[test]
fn is_guard_accepts_template_instance() {
    let stmts = parse("let b = s is Stack[int]\nlet c = p is not Pair[str, int]\n");
    match &stmts[0] {
        Stmt::Let(_, _, Expr::IsType { type_name, negated: false, .. }, _) => assert_eq!(type_name, "Stack[int]"),
        other => panic!("expected IsType, got {other:?}"),
    }
    match &stmts[1] {
        Stmt::Let(_, _, Expr::IsType { type_name, negated: true, .. }, _) => assert_eq!(type_name, "Pair[str,int]"),
        other => panic!("expected IsType, got {other:?}"),
    }
}

/// `match` の `is` 腕も同じ（`is Stack[int]:`）。
#[test]
fn match_is_arm_accepts_template_instance() {
    let stmts = parse("match s:\n    is Stack[int]:\n        print(1)\n    is Box:\n        print(2)\n");
    let Stmt::Match { arms, .. } = &stmts[0] else { panic!("expected match") };
    assert!(matches!(&arms[0].pattern, crate::ast::MatchPattern::IsType(t) if t == "Stack[int]"));
    assert!(matches!(&arms[1].pattern, crate::ast::MatchPattern::IsType(t) if t == "Box"));
}

/// このファイルのテンプレートの名前に付いた `[..]` は、呼び出しでなくても具体化（フェーズ10 10-9）。
/// テンプレートでない名前の添字（`xs[0].n`）は従来どおり添字。
#[test]
fn template_name_bracket_is_instantiation_even_without_call() {
    let stmts = parse(concat!(
        "class Stack[T]:\n",
        "    mut items: list[T]\n",
        "let s = Stack[int].empty()\n",
        "let make = Stack[str]\n",
        "let xs = [1]\n",
        "let n = xs[0].real\n",
    ));
    let Stmt::Let(_, _, Expr::Call { func, .. }, _) = &stmts[1] else { panic!("expected call: {:?}", stmts[1]) };
    let Expr::Attr { object, .. } = func.as_ref() else { panic!("expected attr") };
    assert!(matches!(object.as_ref(), Expr::TemplateInstantiate { type_args, .. } if type_args == &["int".to_string()]));
    assert!(matches!(&stmts[2], Stmt::Let(_, _, Expr::TemplateInstantiate { .. }, _)));
    let Stmt::Let(_, _, Expr::Attr { object, .. }, _) = &stmts[4] else { panic!("expected attr") };
    assert!(matches!(object.as_ref(), Expr::Subscript { .. }));
}

/// 構文の誤りの位置（フェーズ10 10-17）。CLI が文面に ` at file:line:col` を添える。
#[test]
fn parse_error_location_points_at_the_failing_token() {
    let tokens = Lexer::new("let a = 1\nlet x = 1 +\n", "m.ar").tokenize();
    let mut p = Parser::new(tokens, None);
    let err = p.parse_program().expect_err("parse error");
    assert_eq!(p.error_location(&err), " at m.ar:2:12", "{err}");
    // すでに位置で終わっている文面（import 先のファイルの誤り）には足さない。
    assert_eq!(p.error_location("unexpected token at sub.ar:5:3"), "");
    assert!(Parser::ends_with_location("x at C:/a b/c.ar:10:2"));
    assert!(!Parser::ends_with_location("expected `]`, got `EOF`"));
}

/// 型注釈にモジュールの型（`t.Tag` / `a.b.Tag`・`t.Box[int]`）を書ける（フェーズ10 10-8。以前は `.` で ParseError）。
#[test]
fn dotted_type_names_in_annotations() {
    let stmts = parse("fn f(let x: t.Tag, let y: a.b.Box[int]) -> list[t.Tag]:\n    return [x]\nlet ok = v is t.Tag\n");
    let Stmt::FnDef { params, return_type, .. } = &stmts[0] else { panic!("expected fn") };
    assert_eq!(params[0].type_ann.as_deref(), Some("t.Tag"));
    assert_eq!(params[1].type_ann.as_deref(), Some("a.b.Box[int]"));
    assert_eq!(return_type.as_deref(), Some("list[t.Tag]"));
    assert!(matches!(&stmts[1], Stmt::Let(_, _, Expr::IsType { type_name, .. }, _) if type_name == "t.Tag"));
}

/// 型の定義と `import` はモジュールの最上位だけ（フェーズ10 10-16。以前は実行時の内部エラー `VmForceError`）。
#[test]
fn definitions_and_imports_only_at_module_top_level() {
    for src in [
        "fn f() -> None:\n    class C:\n        let v: int\n",
        "fn f() -> None:\n    trait T:\n        fn m(self) -> int:\n            ...\n",
        "fn f() -> None:\n    new_type Id: int\n",
        "if True:\n    class C:\n        let v: int\n",
        "fn f() -> None:\n    @deco\n    class C:\n        let v: int\n",
    ] {
        let err = parse_fails(src);
        assert!(err.contains("must be defined at the top level of a module"), "{src:?} → {err}");
    }
    for src in ["fn f() -> None:\n    import m\n", "while True:\n    from m import x\n"] {
        let err = parse_fails(src);
        assert!(err.contains("`import` must be at the top level of a module"), "{src:?} → {err}");
    }
    // 最上位は従来どおり。
    parse("class C:\n    let v: int\nfn f() -> int:\n    return 1\n");
}

/// Python の関数の中の `class` は変換する（デコレータ・クラスのファクトリ）。`import` は関数の中でも
/// モジュール直下のブロックの中でも認めず、モジュール直下のブロックの中の `class` も認めない（10-16）。
#[test]
fn python_nested_class_and_import_rules() {
    let conv = |src: &str| crate::python_converter::convert_python_source(src, "<test>");
    let ok = conv("def make(step):\n    if step:\n        class A:\n            pass\n        return A\n    class B:\n        pass\n    return B\n")
        .expect("class inside a function must convert");
    let Stmt::FnDef { body, .. } = &ok[0] else { panic!("expected fn: {ok:?}") };
    assert!(body.iter().any(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "B")), "{body:?}");

    let err = conv("def f():\n    import math\n    return 1\n").expect_err("import inside a function");
    assert!(err.contains("an `import` inside a function is not supported"), "{err}");
    let err = conv("try:\n    import math\nexcept ImportError:\n    pass\n").expect_err("import inside a block");
    assert!(err.contains("an `import` inside a block"), "{err}");
    let err = conv("if True:\n    class C:\n        pass\n").expect_err("class inside a module-level block");
    assert!(err.contains("class 'C' is defined inside a block"), "{err}");
    // 関数の中のブロックの中の `class` は認める（上の `make`）。メソッドの中の import は認めない。
    let err = conv("class K:\n    def m(self):\n        from os import path\n        return 1\n").expect_err("import in a method");
    assert!(err.contains("inside a function"), "{err}");
}

/// `except` の型にモジュールを通した名前を書ける（`except m.Err:`・python_builtins_plan.md のタスク 4-3）。
/// Arrow の構文も Python の変換も同じ綴り（`"a.b.Err"`）にする。式（`except f().Err:`）は変換の誤り。
#[test]
fn except_accepts_a_dotted_name() {
    let handler_types = |stmts: &[Stmt]| -> Vec<Option<String>> {
        let Some(Stmt::Try { handlers, .. }) = stmts.first() else { panic!("expected try: {stmts:?}") };
        handlers.iter().map(|h| h.exc_type.clone()).collect()
    };
    let stmts = parse("try:\n    pass\nexcept m.Err as e:\n    pass\nexcept a.b.C:\n    pass\n");
    assert_eq!(handler_types(&stmts), vec![Some("m.Err".to_string()), Some("a.b.C".to_string())]);

    let conv = |src: &str| crate::python_converter::convert_python_source(src, "<test>");
    let stmts = conv("try:\n    pass\nexcept requests.exceptions.HTTPError as e:\n    pass\n").expect("dotted except");
    assert_eq!(handler_types(&stmts), vec![Some("requests.exceptions.HTTPError".to_string())]);
    let err = conv("try:\n    pass\nexcept f().Err:\n    pass\n").expect_err("computed except type");
    assert!(err.contains("computed types are not"), "{err}");
}

/// Python の `except (A, B) as e:` は型ごとに同じ本体の節を並べる（python_builtins_plan.md のタスク 4-4）。
/// 組の入れ子は CPython 3.12 も実行時の `TypeError` なので変換の誤り。
#[test]
fn python_except_tuple_becomes_one_handler_per_type() {
    let conv = |src: &str| crate::python_converter::convert_python_source(src, "<test>");
    let stmts = conv("try:\n    pass\nexcept (ValueError, m.Err) as e:\n    x = 1\n").expect("except tuple");
    // ⚠ 変換器は `x` の宣言（`mut x`）を `try` の前へ出すので、`try` を探す。
    let Some(Stmt::Try { handlers, .. }) = stmts.iter().find(|s| matches!(s, Stmt::Try { .. })) else {
        panic!("expected try: {stmts:?}")
    };
    let types: Vec<Option<&str>> = handlers.iter().map(|h| h.exc_type.as_deref()).collect();
    assert_eq!(types, vec![Some("ValueError"), Some("m.Err")]);
    assert!(handlers.iter().all(|h| h.name.as_deref() == Some("e") && h.body.len() == 1), "{handlers:?}");
    let err = conv("try:\n    pass\nexcept (A, (B, C)):\n    pass\n").expect_err("nested tuple");
    assert!(err.contains("a tuple of them"), "{err}");
}

/// Arrow に無い組み込みを参照するモジュールは変換時の誤り（python_builtins_plan.md のタスク 1-1）。
/// 呼ばれない関数の中でも誤りにし、名前ごとに最初の行を並べる。束縛した名前・デコレータ・クラスの基底・
/// `super().m()` の `super`・Arrow にある組み込みは通す。
#[test]
fn python_unsupported_builtins_are_rejected() {
    let conv = |src: &str| crate::python_converter::convert_python_source(src, "m.py");
    let err = conv("def f(x):\n    return format(x)\n\ndef g():\n    exit(1)\n    return vars()\n")
        .expect_err("unsupported builtins");
    assert!(err.contains("'format' (line 2), 'exit' (line 5), 'vars' (line 6)"), "{err}");
    // 束縛した名前（def / 代入 / 仮引数 / import の別名）は組み込みを隠す。
    conv("def format(x):\n    return x\n\ndef g():\n    return format(1)\n").expect("def shadows");
    conv("vars = 3\n\ndef g(input):\n    return vars + input\n").expect("assignment and parameter shadow");
    conv("from os import path as exit\n\ndef g():\n    return exit\n").expect("import alias shadows");
    // 変換器が形で受ける位置。
    let ok = "class A(object):\n    @staticmethod\n    def make():\n        return 1\n\nclass B(A):\n    def m(self):\n        return super().make()\n";
    conv(ok).expect("decorator / base / super()");
    // Arrow にある組み込み（足した組み込みは自動で外れる）。
    conv("def g(xs):\n    return sorted(map(abs, xs)), isinstance(xs, list), hex(3)\n").expect("supported builtins");
    // 値として使う `super` は誤り。
    let err = conv("def g():\n    return super\n").expect_err("bare super value");
    assert!(err.contains("'super' (line 2)"), "{err}");
}

/// 文が**文の位置**（先頭のトークン）を持つ（フェーズ10 10-17）。以前は `let` / `return` / `if` などが
/// 位置を持たず、型検査の誤りが `<unknown>`・デバッガが直前の行を出し続けていた。
#[test]
fn statements_carry_their_position() {
    let stmts = parse(concat!(
        "let a = 1\n",
        "mut b = a\n",
        "if b:\n",
        "    b = 2\n",
        "while false:\n",
        "    pass\n",
        "for i in [1]:\n",
        "    pass\n",
        "try:\n",
        "    pass\n",
        "except Exception as e:\n",
        "    pass\n",
        "fn f(mut o: Box) -> int:\n",
        "    o.v = 3\n",
        "    return o.v\n",
    ));
    let lines: Vec<Option<(usize, usize)>> =
        stmts.iter().map(|s| s.position().map(|p| (p.line, p.col))).collect();
    // let / mut / if / while / for / try / fn（fn は元のソースの範囲の先頭）
    assert_eq!(
        lines,
        vec![Some((1, 1)), Some((2, 1)), Some((3, 1)), Some((5, 1)), Some((7, 1)), Some((9, 1)), Some((13, 1))]
    );
    let Stmt::FnDef { body, .. } = &stmts[6] else { panic!("expected fn") };
    // 属性への代入・return は字下げの位置（列 5）。
    assert_eq!(body[0].position().map(|p| (p.line, p.col)), Some((14, 5)));
    assert_eq!(body[1].position().map(|p| (p.line, p.col)), Some((15, 5)));
    // 位置を持たない文（`pass`）は `None`。
    let Stmt::While { body, .. } = &stmts[3] else { panic!("expected while") };
    assert!(body[0].position().is_none());
}

/// Python のクラス本体の中のクラスは、外側のクラスより前へ `Outer.Inner` の名前で持ち上がり、外側のクラスには
/// クラス変数 `Inner` が付く（フェーズ10 10-18。以前は黙って捨てていた）。
#[test]
fn python_nested_class_is_hoisted_with_its_qualname() {
    let stmts = crate::python_converter::convert_python_source(
        "class Outer:\n    class Inner:\n        pass\n    class Sub(Inner):\n        pass\n",
        "<test>",
    )
    .expect("python conversion failed");
    let names: Vec<&str> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::ClassDef { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["Outer.Inner", "Outer.Sub", "Outer"]);
    // 同じ本体で先に定義した入れ子のクラスを基底にすると、持ち上げた名前で引く。
    let Stmt::ClassDef { bases, .. } = &stmts[1] else { panic!("expected class") };
    assert_eq!(bases, &vec!["Outer.Inner".to_string()]);
    let Stmt::ClassDef { body, .. } = &stmts[2] else { panic!("expected class") };
    assert!(
        body.iter().any(|s| matches!(s, Stmt::Field { name, type_ann, .. }
            if name == "Inner" && type_ann == "type[Outer.Inner]")),
        "{body:?}"
    );
    // 変換しない文は黙って捨てずに誤りにする。docstring と `...` は読み飛ばす。
    let err = crate::python_converter::convert_python_source(
        "class K:\n    \"\"\"doc\"\"\"\n    for i in range(2):\n        pass\n",
        "<test>",
    )
    .expect_err("a `for` in a class body");
    assert!(err.contains("a `for` statement in the body of class 'K' is not supported"), "{err}");
}
