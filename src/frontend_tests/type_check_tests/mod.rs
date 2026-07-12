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


mod variables;
mod access;
mod bridge_mutability;
mod comparison;
mod calls;
mod union_types;
mod guards_fntype;
mod decorators_generics;
