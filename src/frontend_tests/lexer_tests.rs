// frontend_tests/lexer_tests.rs — 字句解析器(Lexer)の単体テスト。

    use crate::lexer::Lexer;
    use crate::token::Token;

    /// テスト用ヘルパー: ソース文字列を字句解析してトークン種別のみを返す。
    fn lex(source: &str) -> Vec<Token> {
        Lexer::new(source, "")
            .tokenize()
            .into_iter()
            .map(|spanned| spanned.token)
            .collect()
    }

    /// variable_keywords のテスト。
    #[test]
    fn test_variable_keywords() {
        assert_eq!(
            lex("let const mut freeze"),
            vec![
                Token::Let,
                Token::Const,
                Token::Mut,
                Token::Freeze,
                Token::Eof,
            ]
        );
    }

    /// value_literals のテスト。
    #[test]
    fn test_value_literals() {
        assert_eq!(
            lex("True False None"),
            vec![Token::True, Token::False, Token::None, Token::Eof,]
        );
    }

    /// two_word_keywords のテスト。
    #[test]
    fn test_two_word_keywords() {
        assert_eq!(lex("not in"), vec![Token::NotIn, Token::Eof]);
        assert_eq!(lex("is not"), vec![Token::IsNot, Token::Eof]);
        assert_eq!(lex("yield from"), vec![Token::YieldFrom, Token::Eof]);
        assert_eq!(lex("mustbe"), vec![Token::MustBe, Token::Eof]);
    }

    /// not_followed_by_other_word のテスト。
    #[test]
    fn test_not_followed_by_other_word() {
        let tokens = lex("not insert");
        assert_eq!(tokens[0], Token::Not);
        assert_eq!(tokens[1], Token::Ident("insert".to_string()));
    }

    /// arithmetic_operators のテスト。
    #[test]
    fn test_arithmetic_operators() {
        assert_eq!(
            lex("+ - * / // % ** @"),
            vec![
                Token::Plus,
                Token::Minus,
                Token::Star,
                Token::Slash,
                Token::SlashSlash,
                Token::Percent,
                Token::StarStar,
                Token::At,
                Token::Eof,
            ]
        );
    }

    /// compound_assignment のテスト。
    #[test]
    fn test_compound_assignment() {
        assert_eq!(
            lex("+= -= *= /= //= %= **= @="),
            vec![
                Token::PlusEq,
                Token::MinusEq,
                Token::StarEq,
                Token::SlashEq,
                Token::SlashSlashEq,
                Token::PercentEq,
                Token::StarStarEq,
                Token::AtEq,
                Token::Eof,
            ]
        );
    }

    /// comparison_operators のテスト。
    #[test]
    fn test_comparison_operators() {
        assert_eq!(
            lex("== != < > <= >="),
            vec![
                Token::EqEq,
                Token::NotEq,
                Token::Lt,
                Token::Gt,
                Token::LtEq,
                Token::GtEq,
                Token::Eof,
            ]
        );
    }

    /// bitwise_operators のテスト。
    #[test]
    fn test_bitwise_operators() {
        assert_eq!(
            lex("& | ^ ~ << >>"),
            vec![
                Token::Amp,
                Token::Pipe,
                Token::Caret,
                Token::Tilde,
                Token::LtLt,
                Token::GtGt,
                Token::Eof,
            ]
        );
    }

    /// shift_assign のテスト。
    #[test]
    fn test_shift_assign() {
        assert_eq!(
            lex("<<= >>="),
            vec![Token::LtLtEq, Token::GtGtEq, Token::Eof]
        );
    }

    /// integer_literals のテスト。
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

    /// float_literals のテスト。
    #[test]
    fn test_float_literals() {
        let tokens = lex("3.14 1.0e10 2.5E-3");
        assert_eq!(tokens[0], Token::Float(3.14));
        assert_eq!(tokens[1], Token::Float(1.0e10));
        assert_eq!(tokens[2], Token::Float(2.5e-3));
    }

    /// string_literals のテスト。
    #[test]
    fn test_string_literals() {
        let tokens = lex(r#""hello" 'world'"#);
        assert_eq!(tokens[0], Token::Str("hello".to_string()));
        assert_eq!(tokens[1], Token::Str("world".to_string()));
    }

    /// string_escape のテスト。
    #[test]
    fn test_string_escape() {
        let tokens = lex(r#""\n\t\\""#);
        assert_eq!(tokens[0], Token::Str("\n\t\\".to_string()));
    }

    /// triple_quoted_string のテスト。
    #[test]
    fn test_triple_quoted_string() {
        let tokens = lex(r#""""hello world""""#);
        assert_eq!(tokens[0], Token::Str("hello world".to_string()));
    }

    /// indentation のテスト。
    #[test]
    fn test_indentation() {
        let src = "if True:\n    pass\n";
        let tokens = lex(src);
        assert!(tokens.contains(&Token::If));
        assert!(tokens.contains(&Token::Indent));
        assert!(tokens.contains(&Token::Pass));
        assert!(tokens.contains(&Token::Dedent));
    }

    /// nested_indentation のテスト。
    #[test]
    fn test_nested_indentation() {
        let src = "if True:\n    if False:\n        pass\nx\n";
        let tokens = lex(src);
        let dedent_count = tokens.iter().filter(|tok| **tok == Token::Dedent).count();
        assert_eq!(dedent_count, 2);
    }

    /// blank_lines_skipped のテスト。
    #[test]
    fn test_blank_lines_skipped() {
        let src = "x\n\ny\n";
        let tokens = lex(src);
        assert_eq!(tokens[0], Token::Ident("x".to_string()));
        assert_eq!(tokens[1], Token::Newline);
        assert_eq!(tokens[2], Token::Ident("y".to_string()));
    }

    /// comment_skipped のテスト。
    #[test]
    fn test_comment_skipped() {
        let src = "x # comment\ny\n";
        let tokens = lex(src);
        assert_eq!(tokens[0], Token::Ident("x".to_string()));
        assert_eq!(tokens[1], Token::Newline);
        assert_eq!(tokens[2], Token::Ident("y".to_string()));
    }

    /// newline_inside_brackets_ignored のテスト。
    #[test]
    fn test_newline_inside_brackets_ignored() {
        let src = "(\n    1,\n    2\n)\n";
        let tokens = lex(src);
        assert!(!tokens.contains(&Token::Indent));
        assert!(!tokens.contains(&Token::Dedent));
        let newline_count = tokens.iter().filter(|tok| **tok == Token::Newline).count();
        assert_eq!(newline_count, 1);
    }

    /// arrow_ellipsis_walrus のテスト。
    #[test]
    fn test_arrow_ellipsis_walrus() {
        assert_eq!(
            lex("-> ... :="),
            vec![Token::Arrow, Token::Ellipsis, Token::ColonEq, Token::Eof,]
        );
    }

    /// delimiters のテスト。
    #[test]
    fn test_delimiters() {
        assert_eq!(
            lex("()[]{}"),
            vec![
                Token::LParen,
                Token::RParen,
                Token::LBracket,
                Token::RBracket,
                Token::LBrace,
                Token::RBrace,
                Token::Eof,
            ]
        );
    }

    /// span_line_col のテスト。
    #[test]
    fn test_span_line_col() {
        let src = "let x = 1\nmut y = 2\n";
        let spanned = Lexer::new(src, "test.ar").tokenize();
        // `let` は行1・列1
        assert_eq!(spanned[0].span.line, 1);
        assert_eq!(spanned[0].span.col, 1);
        // `mut` は行2・列1
        let mut_tok = spanned
            .iter()
            .find(|spanned_tok| spanned_tok.token == Token::Mut)
            .unwrap();
        assert_eq!(mut_tok.span.line, 2);
        assert_eq!(mut_tok.span.col, 1);
    }

    /// span_filename のテスト。
    #[test]
    fn test_span_filename() {
        let spanned = Lexer::new("x\n", "foo.ar").tokenize();
        assert_eq!(&*spanned[0].span.file, "foo.ar");
    }

    // --- trait / :: ---

    /// trait_keyword のテスト。
    #[test]
    fn test_trait_keyword() {
        assert_eq!(lex("trait"), vec![Token::Trait, Token::Eof]);
    }

    /// colon_colon_token のテスト。
    #[test]
    fn test_colon_colon_token() {
        assert_eq!(lex("::"), vec![Token::ColonColon, Token::Eof]);
    }

    /// colon_vs_colon_colon_vs_colon_eq のテスト。
    #[test]
    fn test_colon_vs_colon_colon_vs_colon_eq() {
        assert_eq!(
            lex(": :: :="),
            vec![Token::Colon, Token::ColonColon, Token::ColonEq, Token::Eof,]
        );
    }

    /// trait_access_syntax_tokens のテスト。
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
