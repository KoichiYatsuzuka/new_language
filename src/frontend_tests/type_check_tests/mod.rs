// type_check_tests/mod.rs — 静的型検査器の単体テストの束ね。
// 共通ヘルパー(check/ok/err)を定義し、機能別サブモジュールを宣言する。

    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::type_check::{InferredType, StaticTypeError, TypeChecker, TypeErrorKind};

    /// ソースコードを字句解析・構文解析・型検査して、検出されたエラーの一覧を返すヘルパー。
    fn check(source: &str) -> Vec<StaticTypeError> {
        let tokens = Lexer::new(source, "").tokenize();
        let stmts = Parser::new(tokens, None)
            .parse_program()
            .expect("parse error");
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

    /// **展開してから**型検査する（テンプレートの具体化を使うテスト用・本番と同じ順序）。
    /// ⚠ `check` は展開しないので、`Stack[int]` の具体クラスが無い（型検査の `debug_assert` が鳴る）。
    fn check_expanded(source: &str) -> Vec<StaticTypeError> {
        let tokens = Lexer::new(source, "").tokenize();
        let mut parser = Parser::new(tokens, None);
        let stmts = parser.parse_program().expect("parse error");
        let stmts = crate::meta_expand::expand_program(stmts, parser.node_counter(), parser.known_traits())
            .expect("expand error");
        TypeChecker::check(&stmts)
    }


mod variables;
mod access;
mod annotations;
mod bridge_mutability;
mod comparison;
mod calls;
mod union_types;
mod guards_fntype;
mod decorators_generics;
mod generators;
