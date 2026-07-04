// stmts/assignment.rs — 識別子始まりの文・代入・イベント文・複合代入の解析。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::{Span, Token},
};
#[allow(unused_imports)]
use super::*;

impl Parser {
    /// 識別子で始まる文をパースする。
    ///
    /// 先読み1トークンによって以下の4パターンに分岐する:
    /// - `name =`  → 変数への代入（`Stmt::Assign`）
    /// - `name +=` 等 → 変数への複合代入（`Stmt::CompoundAssign`）
    /// - `expr =`（属性アクセス等） → 属性代入（`Stmt::AttrAssign`）
    /// - `expr +=` 等（属性アクセス等） → 属性複合代入（`Stmt::AttrCompoundAssign`）
    /// - それ以外 → 式文（`Stmt::Expr`）
    ///
    /// # エラー
    /// `new_type` で宣言された名前への代入・複合代入はパースエラーとなる。
    /// 式または右辺のパースに失敗した場合もエラーを返す。
    pub(crate) fn parse_ident_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek1().clone() {
            // `target <- async [->Type]: body` — 非同期タスクを AsyncManager に追加する
            Token::LeftArrow => {
                let target = self.expect_ident()?;
                self.advance(); // `<-` を消費
                self.eat(&Token::Async)?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                let stmts = self.parse_block()?;
                return Ok(Stmt::AsyncAssign {
                    target,
                    return_type,
                    stmts,
                });
            }
            // 次のトークンが `=` なら変数への単純代入
            Token::Eq => {
                let span = self.current_span();
                let name = self.expect_ident()?;
                // new_type 名への再代入はパースエラー
                if self.known_new_types.contains(&name) {
                    return Err(format!(
                        "ParseError: cannot reassign new_type `{name}` — new_type bindings are const"
                    ));
                }
                self.advance(); // `=` を消費
                Ok(Stmt::Assign {
                    name,
                    value: self.parse_expr()?,
                    span,
                    slot: Default::default(),
                })
            }
            tok => {
                if let Some(op) = token_to_compound_op(&tok) {
                    // 次のトークンが複合代入演算子なら変数への複合代入
                    self.parse_compound(op)
                } else {
                    // 式文・属性代入・属性複合代入の判定
                    // まず式全体をパースしてから後続トークンで分岐する
                    let expr = self.parse_expr()?;
                    let cur = self.current().clone();
                    if cur == Token::Eq {
                        // `expr = 値` — 属性代入（self.x = v など）
                        self.advance();
                        Ok(Stmt::AttrAssign {
                            target: expr,
                            value: self.parse_expr()?,
                        })
                    } else if let Some(op) = token_to_compound_op(&cur) {
                        // `expr += 値` など — 属性複合代入
                        self.advance();
                        Ok(Stmt::AttrCompoundAssign {
                            target: expr,
                            op,
                            value: self.parse_expr()?,
                        })
                    } else if matches!(
                        self.current(),
                        Token::On | Token::Once | Token::Off
                    ) {
                        // イベントハンドラ文（on/once/off）
                        self.try_parse_event_stmt(expr, Span::unknown())?
                            .ok_or_else(|| "internal: expected event stmt".to_string())
                    } else {
                        // 代入でなければ式文として返す
                        Ok(Stmt::Expr(expr))
                    }
                }
            }
        }
    }

    /// 式 `lhs` の後に `on`/`once`/`off` キーワードが続く場合、イベントハンドラ文を返す。
    /// 続かない場合は `None` を返す（呼び出し元が通常の式文として処理する）。
    ///
    /// 構文:
    /// - `source on [async] handler`   → `Stmt::EventSubscribe { is_once: false, is_async: ? }`
    /// - `source once [async] handler` → `Stmt::EventSubscribe { is_once: true,  is_async: ? }`
    /// - `source off handler`          → `Stmt::EventUnsubscribe`
    pub(crate) fn try_parse_event_stmt(
        &mut self,
        source: Expr,
        span: Span,
    ) -> Result<Option<Stmt>, String> {
        match self.current().clone() {
            Token::On | Token::Once => {
                let is_once = *self.current() == Token::Once;
                self.advance(); // consume `on` or `once`
                let is_async = if *self.current() == Token::Async {
                    self.advance(); // consume `async`
                    true
                } else {
                    false
                };
                let handler = self.parse_expr()?;
                Ok(Some(Stmt::EventSubscribe {
                    source,
                    handler,
                    is_once,
                    is_async,
                    span,
                }))
            }
            Token::Off => {
                self.advance(); // consume `off`
                let handler = self.parse_expr()?;
                Ok(Some(Stmt::EventUnsubscribe {
                    source,
                    handler,
                    span,
                }))
            }
            _ => Ok(None),
        }
    }

    /// 変数への複合代入文（`x += expr` 等）をパースして `Stmt::CompoundAssign` を返す。
    ///
    /// # 引数
    /// - `op`: 複合代入に対応する二項演算子（`token_to_compound_op` で変換済み）
    ///
    /// # 戻り値
    /// `Stmt::CompoundAssign { name, op, value, span }`
    ///
    /// # エラー
    /// `new_type` で宣言された名前への複合代入はパースエラー。
    /// 右辺の式のパースに失敗した場合もエラーを返す。
    pub(crate) fn parse_compound(&mut self, op: BinOp) -> Result<Stmt, String> {
        let span = self.current_span(); // 変数識別子の位置情報
        let name = self.expect_ident()?;
        // new_type 名への再代入はパースエラー
        if self.known_new_types.contains(&name) {
            return Err(format!(
                "ParseError: cannot reassign new_type `{name}` — new_type bindings are const"
            ));
        }
        self.advance(); // 複合代入演算子トークンを消費
        Ok(Stmt::CompoundAssign {
            name,
            op,
            value: self.parse_expr()?,
            span,
            slot: Default::default(),
        })
    }

}
