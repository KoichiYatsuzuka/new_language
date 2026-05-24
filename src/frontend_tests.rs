// frontend_tests.rs — 字句解析器・構文解析器・静的型検査器の単体テスト

// ============================================================================
// 字句解析器テスト (Lexer)
// ============================================================================
mod lexer_tests {
    use crate::lexer::Lexer;
    use crate::token::Token;

    /// テスト用ヘルパー: ソース文字列を字句解析してトークン種別のみを返す。
    fn lex(source: &str) -> Vec<Token> {
        Lexer::new(source, "").tokenize().into_iter().map(|spanned| spanned.token).collect()
    }

    #[test]
    fn test_variable_keywords() {
        assert_eq!(lex("let const mut freeze"), vec![
            Token::Let, Token::Const, Token::Mut, Token::Freeze, Token::Eof,
        ]);
    }

    #[test]
    fn test_value_literals() {
        assert_eq!(lex("True False None"), vec![
            Token::True, Token::False, Token::None, Token::Eof,
        ]);
    }

    #[test]
    fn test_two_word_keywords() {
        assert_eq!(lex("not in"), vec![Token::NotIn, Token::Eof]);
        assert_eq!(lex("is not"), vec![Token::IsNot, Token::Eof]);
        assert_eq!(lex("yield from"), vec![Token::YieldFrom, Token::Eof]);
    }

    #[test]
    fn test_not_followed_by_other_word() {
        let tokens = lex("not insert");
        assert_eq!(tokens[0], Token::Not);
        assert_eq!(tokens[1], Token::Ident("insert".to_string()));
    }

    #[test]
    fn test_arithmetic_operators() {
        assert_eq!(lex("+ - * / // % ** @"), vec![
            Token::Plus, Token::Minus, Token::Star, Token::Slash,
            Token::SlashSlash, Token::Percent, Token::StarStar, Token::At,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_compound_assignment() {
        assert_eq!(lex("+= -= *= /= //= %= **= @="), vec![
            Token::PlusEq, Token::MinusEq, Token::StarEq, Token::SlashEq,
            Token::SlashSlashEq, Token::PercentEq, Token::StarStarEq, Token::AtEq,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_comparison_operators() {
        assert_eq!(lex("== != < > <= >="), vec![
            Token::EqEq, Token::NotEq, Token::Lt, Token::Gt,
            Token::LtEq, Token::GtEq,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_bitwise_operators() {
        assert_eq!(lex("& | ^ ~ << >>"), vec![
            Token::Amp, Token::Pipe, Token::Caret, Token::Tilde,
            Token::LtLt, Token::GtGt,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_shift_assign() {
        assert_eq!(lex("<<= >>="), vec![Token::LtLtEq, Token::GtGtEq, Token::Eof]);
    }

    #[test]
    fn test_integer_literals() {
        let tokens = lex("42 0 1_000_000 0xFF 0o17 0b1010");
        assert_eq!(tokens[0], Token::Int(42));
        assert_eq!(tokens[1], Token::Int(0));
        assert_eq!(tokens[2], Token::Int(1_000_000));
        assert_eq!(tokens[3], Token::Int(255));
        assert_eq!(tokens[4], Token::Int(15));
        assert_eq!(tokens[5], Token::Int(10));
    }

    #[test]
    fn test_float_literals() {
        let tokens = lex("3.14 1.0e10 2.5E-3");
        assert_eq!(tokens[0], Token::Float(3.14));
        assert_eq!(tokens[1], Token::Float(1.0e10));
        assert_eq!(tokens[2], Token::Float(2.5e-3));
    }

    #[test]
    fn test_string_literals() {
        let tokens = lex(r#""hello" 'world'"#);
        assert_eq!(tokens[0], Token::Str("hello".to_string()));
        assert_eq!(tokens[1], Token::Str("world".to_string()));
    }

    #[test]
    fn test_string_escape() {
        let tokens = lex(r#""\n\t\\""#);
        assert_eq!(tokens[0], Token::Str("\n\t\\".to_string()));
    }

    #[test]
    fn test_triple_quoted_string() {
        let tokens = lex(r#""""hello world""""#);
        assert_eq!(tokens[0], Token::Str("hello world".to_string()));
    }

    #[test]
    fn test_indentation() {
        let src = "if True:\n    pass\n";
        let tokens = lex(src);
        assert!(tokens.contains(&Token::If));
        assert!(tokens.contains(&Token::Indent));
        assert!(tokens.contains(&Token::Pass));
        assert!(tokens.contains(&Token::Dedent));
    }

    #[test]
    fn test_nested_indentation() {
        let src = "if True:\n    if False:\n        pass\nx\n";
        let tokens = lex(src);
        let dedent_count = tokens.iter().filter(|tok| **tok == Token::Dedent).count();
        assert_eq!(dedent_count, 2);
    }

    #[test]
    fn test_blank_lines_skipped() {
        let src = "x\n\ny\n";
        let tokens = lex(src);
        assert_eq!(tokens[0], Token::Ident("x".to_string()));
        assert_eq!(tokens[1], Token::Newline);
        assert_eq!(tokens[2], Token::Ident("y".to_string()));
    }

    #[test]
    fn test_comment_skipped() {
        let src = "x # comment\ny\n";
        let tokens = lex(src);
        assert_eq!(tokens[0], Token::Ident("x".to_string()));
        assert_eq!(tokens[1], Token::Newline);
        assert_eq!(tokens[2], Token::Ident("y".to_string()));
    }

    #[test]
    fn test_newline_inside_brackets_ignored() {
        let src = "(\n    1,\n    2\n)\n";
        let tokens = lex(src);
        assert!(!tokens.contains(&Token::Indent));
        assert!(!tokens.contains(&Token::Dedent));
        let newline_count = tokens.iter().filter(|tok| **tok == Token::Newline).count();
        assert_eq!(newline_count, 1);
    }

    #[test]
    fn test_arrow_ellipsis_walrus() {
        assert_eq!(lex("-> ... :="), vec![
            Token::Arrow, Token::Ellipsis, Token::ColonEq, Token::Eof,
        ]);
    }

    #[test]
    fn test_delimiters() {
        assert_eq!(lex("()[]{}"), vec![
            Token::LParen, Token::RParen,
            Token::LBracket, Token::RBracket,
            Token::LBrace, Token::RBrace,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_span_line_col() {
        let src = "let x = 1\nmut y = 2\n";
        let spanned = Lexer::new(src, "test.tl").tokenize();
        // `let` は行1・列1
        assert_eq!(spanned[0].span.line, 1);
        assert_eq!(spanned[0].span.col, 1);
        // `mut` は行2・列1
        let mut_tok = spanned.iter().find(|spanned_tok| spanned_tok.token == Token::Mut).unwrap();
        assert_eq!(mut_tok.span.line, 2);
        assert_eq!(mut_tok.span.col, 1);
    }

    #[test]
    fn test_span_filename() {
        let spanned = Lexer::new("x\n", "foo.tl").tokenize();
        assert_eq!(&*spanned[0].span.file, "foo.tl");
    }

    // --- trait / :: ---

    #[test]
    fn test_trait_keyword() {
        assert_eq!(lex("trait"), vec![Token::Trait, Token::Eof]);
    }

    #[test]
    fn test_colon_colon_token() {
        assert_eq!(lex("::"), vec![Token::ColonColon, Token::Eof]);
    }

    #[test]
    fn test_colon_vs_colon_colon_vs_colon_eq() {
        assert_eq!(lex(": :: :="), vec![
            Token::Colon, Token::ColonColon, Token::ColonEq, Token::Eof,
        ]);
    }

    #[test]
    fn test_trait_access_syntax_tokens() {
        // self::MyTrait.field — 括弧なし形式
        let tokens = lex("self::MyTrait.field");
        assert_eq!(tokens[0], Token::Ident("self".to_string()));
        assert_eq!(tokens[1], Token::ColonColon);
        assert_eq!(tokens[2], Token::Ident("MyTrait".to_string()));
        assert_eq!(tokens[3], Token::Dot);
        assert_eq!(tokens[4], Token::Ident("field".to_string()));
    }
}

// ============================================================================
// 構文解析器テスト (Parser)
// ============================================================================
mod parser_tests {
    use crate::ast::{BinOp, CallArg, Expr, FieldKind, Stmt, UnaryOp};
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    /// テスト用ヘルパー: ソース文字列を解析して AST を返す。
    fn parse(source: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(source, "").tokenize();
        Parser::new(tokens, None).parse_program().expect("parse error")
    }

    /// テスト用ヘルパー: パースエラーが発生することを確認してエラーメッセージを返す。
    fn parse_fails(source: &str) -> String {
        let tokens = Lexer::new(source, "").tokenize();
        Parser::new(tokens, None).parse_program().expect_err("expected parse error")
    }

    #[test]
    fn test_literal_expr() {
        let stmts = parse("42");
        assert!(matches!(stmts[0], Stmt::Expr(Expr::Int(42))));
    }

    #[test]
    fn test_freeze_stmt() {
        let stmts = parse("mut x = 5\nfreeze x\n");
        assert!(matches!(&stmts[0], Stmt::Mut(name, ..) if name == "x"));
        assert!(matches!(&stmts[1], Stmt::Freeze(name, ..) if name == "x"));
    }

    #[test]
    fn test_freeze_requires_ident() {
        let tokens = crate::lexer::Lexer::new("freeze 42\n", "").tokenize();
        let err = Parser::new(tokens, None).parse_program().expect_err("expected parse error");
        assert!(err.contains("expected identifier"), "got: {err}");
    }

    #[test]
    fn test_let_decl() {
        let stmts = parse("let x = 10");
        assert!(matches!(&stmts[0], Stmt::Let(name, Expr::Int(10)) if name == "x"));
    }

    #[test]
    fn test_mut_decl() {
        let stmts = parse("mut y = 3.14");
        assert!(matches!(&stmts[0], Stmt::Mut(name, Expr::Float(_)) if name == "y"));
    }

    #[test]
    fn test_assign() {
        let stmts = parse("mut x = 0\nx = 5");
        assert!(matches!(&stmts[1], Stmt::Assign { name, value: Expr::Int(5), .. } if name == "x"));
    }

    #[test]
    fn test_compound_assign() {
        let stmts = parse("mut x = 0\nx += 1");
        assert!(matches!(
            &stmts[1],
            Stmt::CompoundAssign { name, op: BinOp::Add, value: Expr::Int(1), .. } if name == "x"
        ));
    }

    #[test]
    fn test_binop_precedence() {
        let stmts = parse("2 + 3 * 4");
        if let Stmt::Expr(Expr::BinOp { op: BinOp::Add, right, .. }) = &stmts[0] {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Mul, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    #[test]
    fn test_call_expr() {
        let stmts = parse(r#"print("hello")"#);
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Call { .. })));
    }

    #[test]
    fn test_unary_neg() {
        let stmts = parse("-5");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::UnaryOp { op: UnaryOp::Neg, .. })));
    }

    #[test]
    fn test_power_right_assoc() {
        let stmts = parse("2 ** 3 ** 2");
        if let Stmt::Expr(Expr::BinOp { op: BinOp::Pow, right, .. }) = &stmts[0] {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Pow, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    #[test]
    fn test_if_stmt() {
        let stmts = parse("if True:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::If { branches, else_body: None } if branches.len() == 1));
    }

    #[test]
    fn test_if_else_stmt() {
        let stmts = parse("if True:\n    pass\nelse:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::If { else_body: Some(_), .. }));
    }

    #[test]
    fn test_if_elif_else_stmt() {
        let stmts = parse("if True:\n    pass\nelif False:\n    pass\nelse:\n    pass\n");
        if let Stmt::If { branches, else_body } = &stmts[0] {
            assert_eq!(branches.len(), 2);
            assert!(else_body.is_some());
        } else {
            panic!("expected If");
        }
    }

    #[test]
    fn test_while_stmt() {
        let stmts = parse("while True:\n    break\n");
        assert!(matches!(&stmts[0], Stmt::While { .. }));
    }

    #[test]
    fn test_for_stmt() {
        let stmts = parse("for i in [1, 2, 3]:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::For { targets, .. } if targets == &["i"]));
    }

    #[test]
    fn test_block_stmt() {
        let stmts = parse("block:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::Block(_)));
    }

    #[test]
    fn test_list_literal() {
        let stmts = parse("[1, 2, 3]");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::List(_))));
    }

    // --- fn ---

    #[test]
    fn test_fn_def() {
        let stmts = parse("fn add(a, b):\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "add"));
    }

    #[test]
    fn test_fn_no_params() {
        let stmts = parse("fn hello():\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert!(params.is_empty());
        } else {
            panic!("expected FnDef");
        }
    }

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

    #[test]
    fn test_fn_generic_type_annotation() {
        let stmts = parse("fn first(items: list[int]) -> int:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "first"));
    }

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

    #[test]
    fn test_class_empty() {
        let stmts = parse("class Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::ClassDef { name, bases, .. }
            if name == "Foo" && bases.is_empty()));
    }

    #[test]
    fn test_class_with_non_trait_base_errors() {
        let err = parse_fails("class Bar(Foo):\n    pass\n");
        assert!(err.contains("cannot inherit from `Foo`"), "got: {err}");
    }

    #[test]
    fn test_class_multiple_non_trait_bases_errors() {
        let err = parse_fails("class C(A, B):\n    pass\n");
        assert!(err.contains("cannot inherit from"), "got: {err}");
    }

    #[test]
    fn test_class_with_method() {
        let stmts = parse("class Foo:\n    fn greet(self):\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, .. } if name == "greet"));
        } else {
            panic!("expected ClassDef");
        }
    }

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

    #[test]
    fn test_class_field_parsed_as_field_stmt() {
        let src = "class Foo:\n    mut x: int = 0\n    let y: str = \"\"\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "x" && type_ann == "int"));
            assert!(matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "y" && type_ann == "str"));
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_generated() {
        let src = "class Point:\n    mut x: int\n    mut y: int\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "auto __init__ should be present for required fields");
            if let Some(Stmt::FnDef { params, return_type, .. }) = init {
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

    #[test]
    fn test_class_auto_init_not_generated_all_fields_have_defaults() {
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when all fields have defaults");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_generated_with_list_field() {
        let src = "class Foo:\n    mut items: list[int]\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "auto __init__ should be present for required fields");
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params[1].type_ann.as_deref(), Some("list[int]"));
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_override_exact_match() {
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ overrides auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_overload_different_sig() {
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int, y: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 2, "different-sig explicit __init__ + auto-init both present");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_not_generated_without_required_fields() {
        let src = "class Foo:\n    fn greet(self) -> str:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when there are no required fields");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_field_requires_type_annotation() {
        let result = std::panic::catch_unwind(|| {
            parse("class Foo:\n    mut x = 0\n")
        });
        assert!(result.is_err(), "missing type annotation should cause a parse error");
    }

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

    #[test]
    fn test_trait_empty() {
        let stmts = parse("trait Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::TraitDef { name, .. } if name == "Foo"));
    }

    #[test]
    fn test_trait_with_fields() {
        let stmts = parse("trait HasName:\n    mut name: str\n    let id: int\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "name" && type_ann == "str"));
            assert!(matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "id" && type_ann == "int"));
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_virtual_method_is_abstract() {
        let stmts = parse("trait Animal:\n    fn speak(self) -> str:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_abstract: true, .. } if name == "speak"),
                "method with `...` body should have is_abstract: true");
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_non_virtual_method_is_not_virtual() {
        let stmts = parse("trait Logger:\n    fn log(self, msg: str) -> None:\n        pass\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_abstract: false, .. } if name == "log"),
                "method with real body should have is_abstract: false");
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_virtual_body_is_empty() {
        let stmts = parse("trait T:\n    fn f(self) -> int:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            if let Stmt::FnDef { body: fn_body, is_abstract, .. } = &body[0] {
                assert!(*is_abstract);
                assert!(fn_body.is_empty(), "virtual method body should be empty");
            } else {
                panic!("expected FnDef");
            }
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_cannot_inherit() {
        let result = std::panic::catch_unwind(|| parse("trait Foo(Bar):\n    pass\n"));
        assert!(result.is_err(), "trait with base class should cause a parse error");
    }

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

    #[test]
    fn test_class_missing_virtual_override_error() {
        let result = std::panic::catch_unwind(|| parse(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
            "class Cat(Animal):\n",
            "    pass\n",
        )));
        assert!(result.is_err(), "missing virtual method override should cause a parse error");
    }

    #[test]
    fn test_class_inherits_trait_combined_init_generated() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "combined __init__ should be generated");
            if let Some(Stmt::FnDef { params, return_type, .. }) = init {
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

    #[test]
    fn test_class_inherits_trait_combined_init_body_uses_trait_access() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            if let Some(Stmt::FnDef { body: init_body, .. }) =
                body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
            {
                assert!(matches!(&init_body[0],
                    Stmt::AttrAssign { target: Expr::TraitAccess { trait_name, attr, .. }, .. }
                    if trait_name == "HasX" && attr == "x"
                ), "trait field assignment should use TraitAccess");
                assert!(matches!(&init_body[1],
                    Stmt::AttrAssign { target: Expr::Attr { attr, .. }, .. }
                    if attr == "y"
                ), "class field assignment should use Attr");
            } else {
                panic!("__init__ not found");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_access_expr_parsed() {
        let stmts = parse("self::MyTrait.field\n");
        if let Stmt::Expr(Expr::TraitAccess { trait_name, attr, .. }) = &stmts[0] {
            assert_eq!(trait_name, "MyTrait");
            assert_eq!(attr, "field");
        } else {
            panic!("expected Stmt::Expr(Expr::TraitAccess)");
        }
    }

    #[test]
    fn test_fn_is_not_virtual_by_default() {
        let stmts = parse("fn hello() -> None:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { is_abstract: false, .. }));
    }

    #[test]
    fn test_class_method_is_not_virtual() {
        let stmts = parse("class Foo:\n    fn greet(self) -> str:\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_abstract: false, .. } if name == "greet"));
        } else {
            panic!("expected ClassDef");
        }
    }

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
            let inits: Vec<_> = body.iter()
                .filter(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ should override auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_with_multiple_virtual_methods_all_must_be_overridden() {
        let result = std::panic::catch_unwind(|| parse(concat!(
            "trait Ops:\n",
            "    fn add(self, x: int) -> int:\n",
            "        ...\n",
            "    fn sub(self, x: int) -> int:\n",
            "        ...\n",
            "class MyOps(Ops):\n",
            "    fn add(self, x: int) -> int:\n",
            "        pass\n",
        )));
        assert!(result.is_err(), "not overriding all virtual methods should be a parse error");
    }

    #[test]
    fn test_trait_class_only_trait_required_fields_no_class_fields() {
        let stmts = parse(concat!(
            "trait Named:\n",
            "    mut name: str\n",
            "class Widget(Named):\n",
            "    pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|stmt| matches!(stmt, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some());
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params.len(), 2); // self + name
                assert_eq!(params[1].name, "name");
            }
        } else {
            panic!("expected ClassDef");
        }
    }
}

// ============================================================================
// 静的型検査器テスト (TypeChecker)
// ============================================================================
mod type_check_tests {
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::type_check::{InferredType, StaticTypeError, TypeChecker, TypeErrorKind};

    /// ソースコードを字句解析・構文解析・型検査して、検出されたエラーの一覧を返すヘルパー。
    fn check(source: &str) -> Vec<StaticTypeError> {
        let tokens = Lexer::new(source, "").tokenize();
        let stmts = Parser::new(tokens, None).parse_program().expect("parse error");
        TypeChecker::check(&stmts)
    }

    /// 型エラーが 0 件の場合に `true` を返すヘルパー。
    fn ok(source: &str) -> bool {
        check(source).is_empty()
    }

    /// 型エラーが 1 件以上の場合に `true` を返すヘルパー。
    fn err(source: &str) -> bool {
        !check(source).is_empty()
    }

    // --- Immutable assignment ---

    #[test]
    fn let_immutable_assign() {
        assert!(err("let x = 1\nx = 2"));
    }

    #[test]
    fn const_immutable_assign() {
        assert!(err("const X = 1\nX = 2"));
    }

    #[test]
    fn mut_assign_ok() {
        assert!(ok("mut x = 1\nx = 2"));
    }

    #[test]
    fn let_compound_assign_immutable() {
        assert!(err("let x = 1\nx += 1"));
    }

    #[test]
    fn mut_compound_assign_ok() {
        assert!(ok("mut x = 1\nx += 1"));
    }

    #[test]
    fn immutable_assign_inside_if() {
        assert!(err("let x = 1\nif True:\n    x = 2\n"));
    }

    #[test]
    fn mut_assign_inside_if_ok() {
        assert!(ok("mut x = 1\nif True:\n    x = 2\n"));
    }

    // --- Immutable field assignment ---

    #[test]
    fn let_field_assign_outside_class_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "let t = Token(\"ident\")\n",
            "t.kind = \"op\"\n",
        )));
    }

    #[test]
    fn let_field_assign_in_other_method_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn reset(mut self) -> None:\n",
            "        self.kind = \"op\"\n",
        )));
    }

    #[test]
    fn let_field_assign_in_init_ok() {
        assert!(ok(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn __init__(mut self, k: str) -> None:\n",
            "        self.kind = k\n",
        )));
    }

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

    #[test]
    fn let_field_compound_assign_outside_err() {
        assert!(err(concat!(
            "class Node:\n",
            "    let value: int\n",
            "let n = Node(1)\n",
            "n.value += 1\n",
        )));
    }

    // --- Private / protected field access ---

    #[test]
    fn private_field_read_outside_err() {
        assert!(err(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "let obj = MyClass()\n",
            "print(obj.y)\n",
        )));
    }

    #[test]
    fn private_field_read_inside_ok() {
        assert!(ok(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "    fn get_y(self) -> int:\n",
            "        return self.y\n",
        )));
    }

    #[test]
    fn private_field_write_outside_err() {
        assert!(err(concat!(
            "class MyClass:\n",
            "    private:\n",
            "    mut y: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.y = 0\n",
            "let obj = MyClass()\n",
            "obj.y = 5\n",
        )));
    }

    #[test]
    fn public_field_read_outside_ok() {
        assert!(ok(concat!(
            "class MyClass:\n",
            "    public:\n",
            "    mut x: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.x = 1\n",
            "let obj = MyClass()\n",
            "print(obj.x)\n",
        )));
    }

    #[test]
    fn protected_field_read_outside_err() {
        assert!(err(concat!(
            "class MyClass:\n",
            "    protected:\n",
            "    mut z: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.z = 0\n",
            "let obj = MyClass()\n",
            "print(obj.z)\n",
        )));
    }

    #[test]
    fn protected_field_read_same_class_ok() {
        assert!(ok(concat!(
            "class MyClass:\n",
            "    protected:\n",
            "    mut z: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.z = 0\n",
            "    fn get_z(self) -> int:\n",
            "        return self.z\n",
        )));
    }

    #[test]
    fn private_field_error_message() {
        let errors = check(concat!(
            "class A:\n",
            "    private:\n",
            "    mut secret: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.secret = 1\n",
            "let a = A()\n",
            "print(a.secret)\n",
        ));
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::PrivateAccessError { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("secret"));
        assert!(msg.contains("A"));
    }

    // --- Ordering comparison ---

    #[test]
    fn int_int_lt_ok() {
        assert!(ok("1 < 2"));
    }

    #[test]
    fn float_float_lt_ok() {
        assert!(ok("1.0 < 2.0"));
    }

    #[test]
    fn int_float_lt_ok() {
        assert!(ok("1 < 2.0"));
    }

    #[test]
    fn str_str_lt_ok() {
        assert!(ok(r#""a" < "b""#));
    }

    #[test]
    fn str_int_lt_err() {
        assert!(err(r#""hello" < 42"#));
    }

    #[test]
    fn int_str_gt_err() {
        assert!(err(r#"42 > "hello""#));
    }

    #[test]
    fn bool_int_lt_err() {
        assert!(err("True < 1"));
    }

    #[test]
    fn str_float_le_err() {
        assert!(err(r#""x" <= 1.5"#));
    }

    #[test]
    fn eq_different_types_ok() {
        assert!(ok(r#"1 == "hello""#));
    }

    #[test]
    fn neq_different_types_ok() {
        assert!(ok(r#"True != "x""#));
    }

    #[test]
    fn unknown_param_comparison_ok() {
        let errors = check("fn f(x):\n    x < 1\n");
        assert!(!errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::IncompatibleComparison { .. })));
        let errors = check("fn f(x):\n    x < \"hello\"\n");
        assert!(!errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::IncompatibleComparison { .. })));
    }

    #[test]
    fn int_str_lt_is_error() {
        assert!(err("mut x = 1\nx < \"hello\""));
    }

    #[test]
    fn collects_multiple_errors() {
        let errors = check("let a = 1\na = 2\nlet b = 1\nb = 3\n");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn error_display_assign() {
        let errors = check("let x = 1\nx = 2");
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("immutable"));
        assert!(errors[0].to_string().contains("x"));
    }

    #[test]
    fn error_display_comparison() {
        let errors = check(r#""a" < 1"#);
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("str"));
        assert!(errors[0].to_string().contains("int"));
    }

    // --- Function call argument checking ---

    #[test]
    fn call_correct_types_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2)\n"));
    }

    #[test]
    fn call_arg_type_mismatch_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1, \"hello\")\n"));
    }

    #[test]
    fn call_arg_count_too_few_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1)\n"));
    }

    #[test]
    fn call_arg_count_too_many_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2, 3)\n"));
    }

    #[test]
    fn call_no_annotation_no_type_mismatch() {
        let errors = check("fn f(x, y):\n    pass\nf(1, \"hello\")\n");
        assert!(!errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn call_unknown_arg_skipped_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\nmut x = 1\nadd(x, x)\n"));
    }

    #[test]
    fn call_forward_definition_checked() {
        assert!(err("add(1, \"oops\")\nfn add(a: int, b: int) -> int:\n    pass\n"));
    }

    #[test]
    fn call_return_type_inferred() {
        assert!(ok("fn get_int() -> int:\n    pass\nlet v = get_int()\nv < 10\n"));
    }

    #[test]
    fn error_display_call_count() {
        let errors = check("fn f(a: int, b: int) -> None:\n    pass\nf(1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("2"));
        assert!(msg.contains("1"));
    }

    #[test]
    fn error_display_call_type() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(\"hello\")\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("int"));
        assert!(msg.contains("str"));
    }

    // --- Missing type annotation ---

    #[test]
    fn fn_fully_annotated_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\n"));
    }

    #[test]
    fn fn_missing_param_ann_err() {
        assert!(err("fn f(x) -> int:\n    pass\n"));
    }

    #[test]
    fn fn_missing_return_ann_err() {
        assert!(err("fn f(x: int):\n    pass\n"));
    }

    #[test]
    fn fn_missing_both_ann_err() {
        let errors = check("fn f(x):\n    pass\n");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn fn_multiple_missing_params_err() {
        let errors = check("fn f(a, b, c) -> int:\n    pass\n");
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn fn_no_params_missing_return_err() {
        assert!(err("fn greet():\n    pass\n"));
    }

    #[test]
    fn error_display_missing_param_ann() {
        let errors = check("fn f(x) -> int:\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("x"));
        assert!(msg.contains("f"));
    }

    #[test]
    fn error_display_missing_return_ann() {
        let errors = check("fn f(x: int):\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
    }

    // --- Keyword arguments ---

    #[test]
    fn kwarg_correct_ok() {
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(a=1, b=\"hi\")\n"));
    }

    #[test]
    fn kwarg_reversed_order_ok() {
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(b=\"hi\", a=1)\n"));
    }

    #[test]
    fn kwarg_unknown_name_err() {
        assert!(err("fn f(a: int, b: int) -> None:\n    pass\nf(a=1, z=2)\n"));
    }

    #[test]
    fn kwarg_type_mismatch_err() {
        assert!(err("fn f(a: int) -> None:\n    pass\nf(a=\"hello\")\n"));
    }

    #[test]
    fn kwarg_mixed_positional_keyword_ok() {
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(1, b=\"hi\")\n"));
    }

    #[test]
    fn error_display_unknown_kwarg() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(z=1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains("z"));
    }

    // --- Overloading ---

    #[test]
    fn overload_by_count_ok() {
        assert!(ok(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1)\n",
            "f(1, 2)\n",
        )));
    }

    #[test]
    fn overload_by_type_ok() {
        assert!(ok(concat!(
            "fn show(x: int) -> None:\n    pass\n",
            "fn show(x: str) -> None:\n    pass\n",
            "show(1)\n",
            "show(\"hi\")\n",
        )));
    }

    #[test]
    fn overload_wrong_count_err() {
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        assert!(errors.iter().any(|error| matches!(
            &error.kind, TypeErrorKind::NoMatchingOverload { got: 3, .. }
        )));
    }

    #[test]
    fn overload_single_def_count_err_uses_count_mismatch() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(1, 2)\n");
        assert!(errors.iter().any(|error| matches!(
            &error.kind, TypeErrorKind::CallArgCountMismatch { .. }
        )));
    }

    #[test]
    fn overload_multiple_count_match_skips_type_check() {
        let errors = check(concat!(
            "fn f(x: int) -> None:\n    pass\n",
            "fn f(x: str) -> None:\n    pass\n",
            "f(True)\n",
        ));
        assert!(!errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn overload_display_no_matching() {
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::NoMatchingOverload { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("f"));
        assert!(msg.contains('3'));
    }

    // --- Union / Option type ---

    #[test]
    fn union_param_accepts_member_types_ok() {
        assert!(ok(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "f(1)\n",
            "f(\"hi\")\n",
        )));
    }

    #[test]
    fn union_param_rejects_non_member_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "f(True)\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn union_param_accepts_same_union_ok() {
        assert!(ok(concat!(
            "fn f(x: Union[int, str]) -> None:\n    pass\n",
            "fn g(x: Union[int, str]) -> None:\n    f(x)\n",
        )));
    }

    #[test]
    fn union_value_binary_op_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x + 1\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { op, .. } if op == "+")));
    }

    #[test]
    fn union_value_comparison_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x < 10\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    #[test]
    fn union_value_to_typed_param_err() {
        let errors = check(concat!(
            "fn needs_int(n: int) -> None:\n    pass\n",
            "fn caller(x: Union[int, str]) -> None:\n    needs_int(x)\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn union_value_attr_access_err() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x.upper\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    #[test]
    fn union_display_format() {
        let errors = check(concat!(
            "fn f(x: Union[int, str]) -> None:\n",
            "    let y = x + 1\n",
        ));
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. }))
            .unwrap().to_string();
        assert!(msg.contains("Union[int, str]"));
        assert!(msg.contains("downcast"));
    }

    #[test]
    fn option_param_accepts_inner_type_and_none_ok() {
        assert!(ok(concat!(
            "fn f(x: Option[int]) -> None:\n    pass\n",
            "f(1)\n",
            "f(None)\n",
        )));
    }

    #[test]
    fn option_param_rejects_wrong_type_err() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n    pass\n",
            "f(\"oops\")\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn option_value_binary_op_err() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n",
            "    let y = x + 1\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    #[test]
    fn option_display_shows_option_format() {
        let errors = check(concat!(
            "fn f(x: Option[int]) -> None:\n",
            "    let y = x + 1\n",
        ));
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. }))
            .unwrap().to_string();
        assert!(msg.contains("Option[int]"));
    }

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

    #[test]
    fn any_typed_var_binary_op_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x + 1\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "+")));
    }

    #[test]
    fn any_typed_var_comparison_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x < 10\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "<")));
    }

    #[test]
    fn any_typed_var_eq_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x == 1\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "==")));
    }

    #[test]
    fn any_typed_var_logical_op_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x and True\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { .. })));
    }

    #[test]
    fn any_typed_var_unary_neg_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = -x\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "-")));
    }

    #[test]
    fn any_typed_var_attr_access_err() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x.value\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { op } if op == "attribute access")));
    }

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

    #[test]
    fn any_to_any_param_ok() {
        assert!(ok(concat!(
            "fn accept_any(x: Any) -> None:\n",
            "    pass\n",
            "fn forward(x: Any) -> None:\n",
            "    accept_any(x)\n",
        )));
    }

    #[test]
    fn operation_on_any_display() {
        let errors = check(concat!(
            "fn f(x: Any) -> None:\n",
            "    let y = x + 1\n",
        ));
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::OperationOnAny { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("Any"));
        assert!(msg.contains("downcast"));
    }

    // --- new_type ---

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
        let msg = errors.iter()
            .find(|error| matches!(&error.kind, TypeErrorKind::SelfTypeMismatch { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("bar"));
        assert!(msg.contains("Foo"));
        assert!(msg.contains("FooAlias"));
    }

    // --- trait ---

    #[test]
    fn trait_with_virtual_method_no_type_errors() {
        assert!(ok(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
        )));
    }

    #[test]
    fn trait_with_non_virtual_method_no_type_errors() {
        assert!(ok(concat!(
            "trait Logger:\n",
            "    fn log(self, msg: str) -> None:\n",
            "        pass\n",
        )));
    }

    #[test]
    fn trait_with_fields_no_type_errors() {
        assert!(ok(concat!(
            "trait HasValue:\n",
            "    mut value: int\n",
            "    const MAX: int = 100\n",
        )));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    // --- Type guard (is / is not) ---

    #[test]
    fn type_guard_is_narrows_in_if_body_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let y = x + 1\n",
        )));
    }

    #[test]
    fn type_guard_is_union_without_narrowing_err() {
        let errors = check(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "let y = x + 1\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::OperationOnUnion { .. })));
    }

    #[test]
    fn type_guard_is_not_narrows_in_if_body_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is not None:\n    let y = x + 1\n",
        )));
    }

    #[test]
    fn type_guard_is_not_on_non_union_err() {
        let errors = check(concat!(
            "let x = 5\n",
            "if x is not str:\n    pass\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::IsNotOnNonUnion { .. })));
    }

    #[test]
    fn type_guard_elif_narrows_ok() {
        assert!(ok(concat!(
            "fn f() -> Option[int]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let a = x + 1\n",
            "elif x is None:\n    pass\n",
        )));
    }

    #[test]
    fn type_guard_elif_chain_ok() {
        assert!(ok(concat!(
            "fn f() -> Union[int, str]:\n    return 1\n",
            "let x = f()\n",
            "if x is int:\n    let a = x + 1\n",
            "elif x is str:\n    let b = x\n",
        )));
    }

    #[test]
    fn type_guard_elif_is_not_on_non_union_err() {
        let errors = check(concat!(
            "let x = 5\n",
            "if x == 1:\n    pass\n",
            "elif x is not str:\n    pass\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::IsNotOnNonUnion { .. })));
    }

    // --- function type ---

    #[test]
    fn function_type_bare_param_ok() {
        assert!(ok(concat!(
            "fn caller(let f: function) -> None:\n",
            "    pass\n",
        )));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgCountMismatch { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::UnknownKeywordArg { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallMutParamWithImmutableArg { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgCountMismatch { .. })));
    }

    // --- type[T] ---

    #[test]
    fn type_val_of_exact_match_ok() {
        assert!(ok(concat!(
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(int)\n",
        )));
    }

    #[test]
    fn type_val_of_wrong_primitive_err() {
        let errors = check(concat!(
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(float)\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn type_val_of_new_type_upcast_ok() {
        assert!(ok(concat!(
            "new_type Index: int\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(Index)\n",
        )));
    }

    #[test]
    fn type_val_of_new_type_chain_ok() {
        assert!(ok(concat!(
            "new_type A: int\n",
            "new_type B: A\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(B)\n",
        )));
    }

    #[test]
    fn type_val_of_new_type_wrong_origin_err() {
        let errors = check(concat!(
            "new_type Name: str\n",
            "fn f(let x: type[int]) -> None:\n    pass\n",
            "f(Name)\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn type_val_of_trait_upcast_ok() {
        assert!(ok(concat!(
            "trait MyTrait:\n    pass\n",
            "class MyClass(MyTrait):\n    pass\n",
            "fn f(let x: type[MyTrait]) -> None:\n    pass\n",
            "f(MyClass)\n",
        )));
    }

    #[test]
    fn type_val_of_trait_wrong_class_err() {
        let errors = check(concat!(
            "trait MyTrait:\n    pass\n",
            "class Other:\n    pass\n",
            "fn f(let x: type[MyTrait]) -> None:\n    pass\n",
            "f(Other)\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn type_val_bare_accepts_any_type_value_ok() {
        assert!(ok(concat!(
            "fn f(let x: type) -> None:\n    pass\n",
            "f(int)\n",
            "f(str)\n",
        )));
    }

    #[test]
    fn type_val_of_display() {
        let type_value = InferredType::TypeValOf(Box::new(InferredType::Int));
        assert_eq!(type_value.to_string(), "type[int]");
    }

    // --- Decorator static type checking ---

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

    #[test]
    fn decorator_fn_wrong_param_type_err() {
        let errors = check(concat!(
            "fn bad_dec(let x: int) -> function:\n",
            "    return x\n",
            "@bad_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    #[test]
    fn decorator_fn_wrong_return_type_err() {
        let errors = check(concat!(
            "fn bad_dec(let f: function) -> int:\n",
            "    return 0\n",
            "@bad_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    #[test]
    fn decorator_fn_no_params_err() {
        let errors = check(concat!(
            "fn no_param_dec() -> function:\n",
            "    pass\n",
            "@no_param_dec\n",
            "fn my_func(let a: int) -> int:\n",
            "    return a\n",
        ));
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

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
        assert!(errors.iter().any(|error| matches!(&error.kind, TypeErrorKind::InvalidDecorator { .. })));
    }

    // --- Collection generics type checking ---

    #[test]
    fn list_of_int_matches_list_of_int_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs: list[int] = [1, 2, 3]\n",
            "f(xs)\n",
        )));
    }

    #[test]
    fn list_of_str_to_list_of_int_err() {
        assert!(err(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs: list[str] = [\"a\", \"b\"]\n",
            "f(xs)\n",
        )));
    }

    #[test]
    fn list_literal_inferred_as_list_of_int_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "f([1, 2, 3])\n",
        )));
    }

    #[test]
    fn list_literal_wrong_elem_type_err() {
        assert!(err(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "f([\"a\", \"b\"])\n",
        )));
    }

    #[test]
    fn untyped_list_matches_list_of_any_ok() {
        assert!(ok(concat!(
            "fn f(items: list[int]) -> int:\n",
            "    return 0\n",
            "let xs = []\n",
            "f(xs)\n",
        )));
    }

    #[test]
    fn set_of_int_to_set_of_str_err() {
        assert!(err(concat!(
            "fn f(s: set[str]) -> int:\n",
            "    return 0\n",
            "let xs: set[int] = {1, 2}\n",
            "f(xs)\n",
        )));
    }

    #[test]
    fn dict_of_str_int_ok() {
        assert!(ok(concat!(
            "fn f(d: dict[str,int]) -> int:\n",
            "    return 0\n",
            "let d: dict[str,int] = {\"a\": 1}\n",
            "f(d)\n",
        )));
    }

    #[test]
    fn dict_key_type_mismatch_err() {
        assert!(err(concat!(
            "fn f(d: dict[str,int]) -> int:\n",
            "    return 0\n",
            "let d: dict[int,int] = {1: 2}\n",
            "f(d)\n",
        )));
    }
}
