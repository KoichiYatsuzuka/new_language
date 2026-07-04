// stmts/definitions.rs — try / enum / new_type 定義の解析。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::{Span, Token},
};
#[allow(unused_imports)]
use super::*;

impl Parser {
    /// `try / except / finally` 文をパースして `Stmt::Try` を返す。
    ///
    /// `except` 節は0個以上、`finally` 節は任意の1つ。
    /// ただし両方とも省略するとパースエラーになる。
    ///
    /// `except 型名 [as 変数名]:` の形式で例外の型と補足変数名を指定できる。
    /// 型名を省略した `except:` は全ての例外を捕捉する。
    ///
    /// # 戻り値
    /// `Stmt::Try { body, handlers, finally_body }`
    ///
    /// # エラー
    /// `except` も `finally` も存在しない場合、またはブロックのパースに失敗した場合
    pub(crate) fn parse_try_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `try` を消費
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;

        let mut handlers: Vec<ExceptHandler> = Vec::new();
        let mut finally_body: Option<Vec<Stmt>> = None;

        // Parse zero or more `except` clauses
        while *self.current() == Token::Except {
            self.advance(); // consume `except`
            // Determine exception type and optional name binding
            let (exc_type, name) = if matches!(self.current(), Token::Colon) {
                // bare `except:`
                (None, None)
            } else {
                let type_name = self.expect_ident()?;
                let alias = if *self.current() == Token::As {
                    self.advance();
                    Some(self.expect_ident()?)
                } else {
                    None
                };
                (Some(type_name), alias)
            };
            self.eat(&Token::Colon)?;
            let handler_body = self.parse_block()?;
            handlers.push(ExceptHandler {
                exc_type,
                name,
                body: handler_body,
            });
        }

        // Optional `finally` clause
        if *self.current() == Token::Finally {
            self.advance();
            self.eat(&Token::Colon)?;
            finally_body = Some(self.parse_block()?);
        }

        if handlers.is_empty() && finally_body.is_none() {
            return Err(
                "try statement requires at least one `except` or `finally` clause".to_string(),
            );
        }

        Ok(Stmt::Try {
            body,
            handlers,
            finally_body,
        })
    }

    /// `enum 名前: バリアント [= 値] ...` 定義をパースして `Stmt::EnumDef` を返す。
    ///
    /// 各バリアントは `name [= expr]` の形式。値を省略すると前のバリアントの値 + 1 から自動採番される。
    ///
    /// # 戻り値
    /// `Stmt::EnumDef { name, variants }`
    ///
    /// # エラー
    /// 識別子のパースに失敗した場合、またはインデントブロックが欠如している場合
    pub(crate) fn parse_enum_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `enum` を消費
        let name = self.expect_ident()?;
        self.eat(&Token::Colon)?;
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
        let mut variants = Vec::new();
        loop {
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.current(), Token::Dedent | Token::Eof) {
                break;
            }
            let variant_name = self.expect_ident()?;
            let value = if matches!(self.current(), Token::Eq) {
                self.advance(); // `=` を消費
                Some(self.parse_expr()?)
            } else {
                None
            };
            if matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            variants.push((variant_name, value));
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        Ok(Stmt::EnumDef { name, variants })
    }

    /// `new_type 名前: 元の型` 定義をパースして `Stmt::NewTypeDef` を返す。
    ///
    /// パース後に名前を `known_new_types` に登録し、
    /// 以降の同名への代入をパースエラーとして検出できるようにする。
    ///
    /// # 戻り値
    /// `Stmt::NewTypeDef { name, original }`
    ///
    /// # エラー
    /// 識別子または型名のパースに失敗した場合
    pub(crate) fn parse_new_type_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `new_type` を消費
        let name = self.expect_ident()?;
        self.eat(&Token::Colon)?;
        let original = self.parse_type_expr()?;
        // 名前を登録して再代入を禁止する
        self.known_new_types.insert(name.clone());
        Ok(Stmt::NewTypeDef { name, original })
    }
}
