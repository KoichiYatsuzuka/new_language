// stmts/control_flow.rs — 制御構造の解析: 戻り型注釈 / if / match。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::{Span, Token},
};
#[allow(unused_imports)]
use super::*;

impl Parser {
    /// `->Type` アノテーションを省略可能な形でパースして返す。
    ///
    /// 現在トークンが `->` の場合のみ型アノテーション文字列を返し、それ以外は `None`。
    pub(crate) fn parse_opt_return_type(&mut self) -> Result<Option<String>, String> {
        if *self.current() == Token::Arrow {
            self.advance();
            Ok(Some(self.parse_type_expr()?))
        } else {
            Ok(None)
        }
    }

    /// `if` キーワードを消費した後の節をパースする共有ヘルパー。
    ///
    /// `(branches, else_body, return_type)` を返す。
    /// `return_type` は最初の `if` 直後の `->Type` アノテーション。`elif`/`else` にはない。
    pub(crate) fn parse_if_components(
        &mut self,
    ) -> Result<(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>, Option<String>), String> {
        let cond = self.parse_expr()?;
        let return_type = self.parse_opt_return_type()?;
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;
        let mut branches = vec![(cond, body)];
        let mut else_body = None;
        loop {
            match self.current().clone() {
                Token::Elif => {
                    self.advance();
                    let c = self.parse_expr()?;
                    let _ = self.parse_opt_return_type()?; // elif の ->Type は無視
                    self.eat(&Token::Colon)?;
                    branches.push((c, self.parse_block()?));
                }
                Token::Else => {
                    self.advance();
                    self.eat(&Token::Colon)?;
                    else_body = Some(self.parse_block()?);
                    break;
                }
                _ => break,
            }
        }
        Ok((branches, else_body, return_type))
    }

    /// `if / elif / else` 文をパースして `Stmt::If` を返す。
    ///
    /// `if` トークンはすでに現在位置にあることを前提とする。
    /// `elif` 節は複数連続してよく、`else` 節は最後に1つだけ現れる。
    /// `->Type` アノテーションがあってもパースのみ行い、文レベルでは無視する。
    ///
    /// # 戻り値
    /// `Stmt::If { branches, else_body }` — branches は `(条件式, 本体)` のリスト
    ///
    /// # エラー
    /// 条件式または本体のパースに失敗した場合
    pub(crate) fn parse_if_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `if` を消費
        let (branches, else_body, _return_type) = self.parse_if_components()?;
        Ok(Stmt::If {
            branches,
            else_body,
        })
    }

    /// match アームリストをパースする共有ヘルパー（`match subject:` の後、INDENT済みの状態で呼ぶ）。
    ///
    /// `case` アームと `is` アームを混在させるとパースエラー。
    /// `case _:` はワイルドカードアームとして解釈される。
    ///
    /// # エラー
    /// - `case` と `is` のアームが混在する場合
    /// - 予期しないトークンがアームの先頭に現れた場合
    pub(crate) fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, String> {
        let mut arms: Vec<MatchArm> = Vec::new();
        let mut is_case_kind: Option<bool> = None;
        loop {
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.current(), Token::Dedent | Token::Eof) {
                break;
            }
            match self.current().clone() {
                Token::Case => {
                    if is_case_kind == Some(false) {
                        return Err("match statement cannot mix 'case' and 'is' arms".to_string());
                    }
                    is_case_kind = Some(true);
                    self.advance();
                    let pattern_expr = self.parse_expr()?;
                    self.eat(&Token::Colon)?;
                    arms.push(MatchArm {
                        pattern: MatchPattern::Case(pattern_expr),
                        body: self.parse_block()?,
                    });
                }
                Token::Is => {
                    if is_case_kind == Some(true) {
                        return Err("match statement cannot mix 'case' and 'is' arms".to_string());
                    }
                    is_case_kind = Some(false);
                    self.advance();
                    let type_name = self.expect_ident()?;
                    self.eat(&Token::Colon)?;
                    arms.push(MatchArm {
                        pattern: MatchPattern::IsType(type_name),
                        body: self.parse_block()?,
                    });
                }
                tok => {
                    return Err(format!(
                        "expected 'case' or 'is' in match body, got `{tok}`"
                    ))
                }
            }
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        Ok(arms)
    }

    pub(crate) fn parse_match_stmt(&mut self) -> Result<Stmt, String> {
        let span = self.current_span();
        self.advance(); // `match` を消費
        let subject = self.parse_expr()?;
        let _ = self.parse_opt_return_type()?; // ->Type at stmt level: parse and discard
        self.eat(&Token::Colon)?;
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
        let arms = self.parse_match_arms()?;
        Ok(Stmt::Match {
            subject,
            arms,
            span,
        })
    }

}
