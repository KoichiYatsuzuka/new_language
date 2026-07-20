// stmts/core.rs — 文パースの中核: parse_program / parse_block / parse_tuple_unpack / parse_stmt。

use {
    crate::parser::Parser,
    crate::ast::{Stmt, TupleTarget},
    crate::token::Token,
};

impl Parser {
    /// プログラム全体をパースして文のリストを返す。
    ///
    /// EOF に達するまで `parse_stmt()` を繰り返し呼び出す。
    ///
    /// # 戻り値
    /// パースに成功した場合は `Vec<Stmt>`、失敗した場合はエラー文字列
    ///
    /// # エラー
    /// いずれかの文のパースに失敗した場合にエラーを返す。
    pub fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        // 先頭の空白行やインデントをスキップ
        self.skip_newlines();
        while *self.current() != Token::Eof {
            stmts.push(self.parse_stmt()?);
            // 文と文の間の空白行をスキップ
            self.skip_newlines();
        }
        Ok(stmts)
    }

    /// インデントブロック（`NEWLINE INDENT stmt* DEDENT`）をパースして文のリストを返す。
    ///
    /// # 戻り値
    /// ブロック内の文のリスト
    ///
    /// # エラー
    /// `NEWLINE` または `INDENT` が欠如している場合、もしくは内部の文のパースに失敗した場合
    pub(crate) fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
        // ブロックはエイリアスのスコープ境界。開始時にスナップショットを取り、
        // 終了時に復元することで、ブロック内で宣言した alias をブロック外に漏らさない。
        let saved_aliases = self.aliases.clone();
        let mut stmts = Vec::new();
        loop {
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.current(), Token::Dedent | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        self.aliases = saved_aliases;
        Ok(stmts)
    }

    /// `let x, mut y, _ = expr` 形式のタプルアンパック宣言をパースする。
    /// `first` は既にパースされた先頭ターゲット。現在位置は最初のカンマを指している。
    pub(crate) fn parse_tuple_unpack(&mut self, first: TupleTarget) -> Result<Stmt, String> {
        let span = self.current_span();
        let mut targets = vec![first];
        while *self.current() == Token::Comma {
            self.advance();
            match self.current().clone() {
                Token::Let => {
                    self.advance();
                    targets.push(TupleTarget::Let(self.expect_ident()?));
                }
                Token::Mut => {
                    self.advance();
                    targets.push(TupleTarget::Mut(self.expect_ident()?));
                }
                Token::Ident(n) if n == "_" => {
                    self.advance();
                    targets.push(TupleTarget::Wildcard);
                    break; // _ absorbs the rest — nothing can follow
                }
                Token::Ident(n) => {
                    let name = n.clone();
                    self.advance();
                    targets.push(TupleTarget::Bare(name));
                }
                other => {
                    return Err(format!(
                        "expected `let`, `mut`, or `_` in tuple unpack, got `{other}`"
                    ))
                }
            }
        }
        self.eat(&Token::Eq)?;
        Ok(Stmt::LetTuple {
            targets,
            value: self.parse_expr()?,
            span,
        })
    }

    /// 1文をパースして `Stmt` を返す。
    ///
    /// 先頭トークンの種別によって適切なサブパーサにディスパッチする。
    ///
    /// # 戻り値
    /// パースに成功した場合は `Stmt`、失敗した場合はエラー文字列
    ///
    /// # エラー
    /// 未対応のトークンが先頭に現れた場合、またはサブパーサがエラーを返した場合
    pub(crate) fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.current().clone() {
            // `let 変数名 [: 型] = 式` — イミュータブル変数宣言
            // `let x, mut y, _ = expr` — タプルアンパック宣言
            // `let dbg::name = expr` — デバッガ REPL 内一時変数宣言
            Token::Let => {
                self.advance();
                let name = self.expect_ident()?;
                // `let dbg::varname = expr` — debug namespace declaration
                if name == "dbg" && *self.current() == Token::ColonColon {
                    self.advance(); // consume ::
                    let dbg_name = self.expect_ident()?;
                    self.eat(&Token::Eq)?;
                    return Ok(Stmt::DebugLet(dbg_name, self.parse_expr()?));
                }
                if *self.current() == Token::Comma {
                    return self.parse_tuple_unpack(TupleTarget::Let(name));
                }
                // 型アノテーションを保存する（Protocol 型検査で使用）
                let type_ann = if *self.current() == Token::Colon {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                self.eat(&Token::Eq)?;
                Ok(Stmt::Let(name, type_ann, self.parse_expr()?))
            }
            // `const 変数名 [: 型] = 式` — 定数宣言
            Token::Const => {
                self.advance();
                let name = self.expect_ident()?;
                let type_ann = if *self.current() == Token::Colon {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                self.eat(&Token::Eq)?;
                Ok(Stmt::Const(name, type_ann, self.parse_expr()?))
            }
            // `mut 変数名 [: 型] = 式` — ミュータブル変数宣言
            // `mut x, let y, _ = expr` — タプルアンパック宣言
            Token::Mut => {
                self.advance();
                let name = self.expect_ident()?;
                if *self.current() == Token::Comma {
                    return self.parse_tuple_unpack(TupleTarget::Mut(name));
                }
                let type_ann = if *self.current() == Token::Colon {
                    self.advance();
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };
                self.eat(&Token::Eq)?;
                Ok(Stmt::Mut(name, type_ann, self.parse_expr()?))
            }
            // `static mut 変数名 [: 型] = 式` — 静的可変変数宣言（全呼び出しでセル共有）
            Token::Static => {
                let span = self.current_span();
                self.advance();
                self.eat(&Token::Mut)?;
                let name = self.expect_ident()?;
                if *self.current() == Token::Colon {
                    self.advance();
                    self.parse_type_expr()?;
                }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Static(name, self.parse_expr()?, span))
            }
            // `freeze 変数名` — 変数をイミュータブルに凍結
            Token::Freeze => {
                let span = self.current_span();
                self.advance();
                let name = self.expect_ident()?;
                Ok(Stmt::Freeze(name, span))
            }
            // ジャンプ文・単純文
            Token::Pass => {
                self.advance();
                Ok(Stmt::Pass)
            }
            Token::Break => {
                self.advance();
                Ok(Stmt::Break)
            }
            Token::Continue => {
                self.advance();
                Ok(Stmt::Continue)
            }
            // `return [式]` — 値なし return と値あり return を区別
            Token::Return => {
                self.advance();
                if matches!(
                    self.current(),
                    Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent
                ) {
                    // 値なし return
                    Ok(Stmt::Return(None))
                } else {
                    // 値あり return
                    Ok(Stmt::Return(Some(self.parse_expr()?)))
                }
            }
            // `block_return 式` / `loop_yield 式` — block/loop スコープからの脱出
            Token::BlockReturn => {
                let span = self.current_span();
                self.advance();
                Ok(Stmt::BlockReturn(self.parse_expr()?, span))
            }
            Token::LoopYield => {
                self.advance();
                Ok(Stmt::LoopYield(self.parse_expr()?))
            }
            // `break_point` — デバッガ REPL を起動して実行を一時停止する
            Token::BreakPoint => {
                let span = self.current_span();
                self.advance();
                Ok(Stmt::BreakPoint { span })
            }
            // 制御構文
            Token::If => self.parse_if_stmt(),
            Token::Match => self.parse_match_stmt(),
            Token::While => {
                self.advance();
                let cond = self.parse_expr()?;
                let _ = self.parse_opt_return_type()?; // stmt level: parse and discard
                self.eat(&Token::Colon)?;
                Ok(Stmt::While {
                    cond,
                    body: self.parse_block()?,
                })
            }
            Token::For => {
                self.advance();
                let first = self.expect_ident()?;
                let mut targets = vec![first];
                while *self.current() == Token::Comma {
                    self.advance();
                    targets.push(self.expect_ident()?);
                }
                self.eat(&Token::In)?;
                let iter = self.parse_expr()?;
                let _ = self.parse_opt_return_type()?; // stmt level: parse and discard
                self.eat(&Token::Colon)?;
                Ok(Stmt::For {
                    targets,
                    iter,
                    body: self.parse_block()?,
                })
            }
            // `block [->Type]:` — 値を返せるスコープブロック
            Token::Block => {
                self.advance();
                let _ = self.parse_opt_return_type()?; // stmt level: parse and discard
                self.eat(&Token::Colon)?;
                Ok(Stmt::Block(self.parse_block()?))
            }
            // `yield 式` — ジェネレータ関数内で値を yield する
            Token::Yield => {
                self.advance();
                Ok(Stmt::Yield(self.parse_expr()?))
            }
            Token::Try => self.parse_try_stmt(),
            // `raise [式]` — 例外を送出する
            Token::Raise => {
                let span = self.current_span();
                self.advance();
                // 送出する例外式がなければ None（再 raise 相当）
                let exc = if matches!(
                    self.current(),
                    Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent
                ) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Raise { exc, span })
            }
            Token::At => {
                let decorators = self.parse_decorators()?;
                match self.current().clone() {
                    Token::Fn => self.parse_fn_def_decorated(decorators),
                    Token::Class => self.parse_class_def_decorated(decorators),
                    tok => Err(format!(
                        "ParseError: '@' decorator must be followed by 'fn' or 'class', got `{tok}`"
                    )),
                }
            }
            Token::Fn => self.parse_fn_def(),
            Token::Gen => self.parse_gen_def(),
            Token::Class => self.parse_class_def(),
            Token::Enum => self.parse_enum_def(),
            Token::Trait => self.parse_trait_def(),
            Token::Protocol => self.parse_protocol_def(),
            Token::NewType => self.parse_new_type_def(),
            Token::Alias => self.parse_alias_def(),
            Token::Import => self.parse_import_stmt(),
            Token::From => self.parse_from_import_stmt(),
            Token::Ident(_) => self.parse_ident_stmt(),
            _ => {
                let span = self.current_span();
                let expr = self.parse_expr()?;
                if let Some(event_stmt) = self.try_parse_event_stmt(expr.clone(), span)? {
                    Ok(event_stmt)
                } else {
                    Ok(Stmt::Expr(expr))
                }
            }
        }
    }

}
