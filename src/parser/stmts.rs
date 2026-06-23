// stmts.rs — statement parsing for the tl parser.

use super::Parser;
use crate::ast::{
    Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget,
};
use crate::token::{Span, Token};

/// 複合代入演算子トークンを対応する二項演算子（`BinOp`）に変換する。
///
/// `+=`, `-=`, `*=` などのトークンを対応する `BinOp` にマッピングする。
/// 複合代入演算子でないトークンの場合は `None` を返す。
fn token_to_compound_op(token: &Token) -> Option<BinOp> {
    match token {
        Token::PlusEq => Some(BinOp::Add),
        Token::MinusEq => Some(BinOp::Sub),
        Token::StarEq => Some(BinOp::Mul),
        Token::SlashEq => Some(BinOp::Div),
        Token::SlashSlashEq => Some(BinOp::FloorDiv),
        Token::PercentEq => Some(BinOp::Mod),
        Token::StarStarEq => Some(BinOp::Pow),
        Token::AmpEq => Some(BinOp::BitAnd),
        Token::PipeEq => Some(BinOp::BitOr),
        Token::CaretEq => Some(BinOp::BitXor),
        Token::LtLtEq => Some(BinOp::LShift),
        Token::GtGtEq => Some(BinOp::RShift),
        _ => None,
    }
}

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
    pub(super) fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
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
        Ok(stmts)
    }

    /// `let x, mut y, _ = expr` 形式のタプルアンパック宣言をパースする。
    /// `first` は既にパースされた先頭ターゲット。現在位置は最初のカンマを指している。
    fn parse_tuple_unpack(&mut self, first: TupleTarget) -> Result<Stmt, String> {
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
    fn parse_stmt(&mut self) -> Result<Stmt, String> {
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
                // 型アノテーションがあれば読み飛ばす（静的型検査で使用）
                if *self.current() == Token::Colon {
                    self.advance();
                    self.parse_type_expr()?;
                }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Let(name, self.parse_expr()?))
            }
            // `const 変数名 [: 型] = 式` — 定数宣言
            Token::Const => {
                self.advance();
                let name = self.expect_ident()?;
                // 型アノテーションがあれば読み飛ばす
                if *self.current() == Token::Colon {
                    self.advance();
                    self.parse_type_expr()?;
                }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Const(name, self.parse_expr()?))
            }
            // `mut 変数名 [: 型] = 式` — ミュータブル変数宣言
            // `mut x, let y, _ = expr` — タプルアンパック宣言
            Token::Mut => {
                self.advance();
                let name = self.expect_ident()?;
                if *self.current() == Token::Comma {
                    return self.parse_tuple_unpack(TupleTarget::Mut(name));
                }
                // 型アノテーションがあれば読み飛ばす
                if *self.current() == Token::Colon {
                    self.advance();
                    self.parse_type_expr()?;
                }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Mut(name, self.parse_expr()?))
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
            Token::NewType => self.parse_new_type_def(),
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

    /// `->Type` アノテーションを省略可能な形でパースして返す。
    ///
    /// 現在トークンが `->` の場合のみ型アノテーション文字列を返し、それ以外は `None`。
    pub(super) fn parse_opt_return_type(&mut self) -> Result<Option<String>, String> {
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
    pub(super) fn parse_if_components(
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
    fn parse_if_stmt(&mut self) -> Result<Stmt, String> {
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
    pub(super) fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, String> {
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

    fn parse_match_stmt(&mut self) -> Result<Stmt, String> {
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
    fn parse_ident_stmt(&mut self) -> Result<Stmt, String> {
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
    pub(super) fn try_parse_event_stmt(
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
    fn parse_compound(&mut self, op: BinOp) -> Result<Stmt, String> {
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
        })
    }

    /// `@decorator` 構文のリストをパースして式のリストを返す。
    ///
    /// 現在のトークンが `@` の間、以下を繰り返す:
    /// 1. `@` を消費する
    /// 2. デコレータ式をパース（識別子・属性アクセス・関数呼び出し可）
    /// 3. 末尾の改行を消費する
    ///
    /// 戻り値: `Vec<Expr>` — 上から順に並んだデコレータ式のリスト
    fn parse_decorators(&mut self) -> Result<Vec<Expr>, String> {
        let mut decorators = Vec::new();
        while *self.current() == Token::At {
            self.advance(); // `@` を消費
            let expr = self.parse_expr()?;
            decorators.push(expr);
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
        }
        Ok(decorators)
    }

    /// `fn` 関数定義をパースして `Stmt::FnDef` を返す。
    ///
    /// デコレータなし・静的でない通常の関数定義に使用する委譲ヘルパー。
    pub(super) fn parse_fn_def(&mut self) -> Result<Stmt, String> {
        self.parse_fn_def_with_flags(vec![], false, false)
    }

    /// デコレータ付き `fn` 関数定義をパースして `Stmt::FnDef` を返す。
    ///
    /// `@decorator` の後に `fn` が続く場合に呼ばれる。
    pub(super) fn parse_fn_def_decorated(
        &mut self,
        decorators: Vec<Expr>,
    ) -> Result<Stmt, String> {
        self.parse_fn_def_with_flags(decorators, false, false)
    }

    /// `fn` 関数定義の共通パース処理。デコレータ・`static`・`class_method` フラグを受け取る。
    ///
    /// テンプレートパラメータ・引数リスト・戻り値型・本体ブロックを順番にパースする。
    /// 本体が `...`（省略記号のみ）の場合は抽象メソッド（`is_abstract: true`）として扱う。
    ///
    /// # 引数
    /// - `decorators`: 事前にパース済みのデコレータ式リスト
    /// - `is_static`: `static fn` かどうか
    /// - `is_class_method`: `class_method fn` かどうか
    ///
    /// # 戻り値
    /// `Stmt::FnDef { name, template_params, params, return_type, body, is_abstract, ... }`
    ///
    /// # エラー
    /// 識別子・括弧・本体ブロックのパースに失敗した場合
    pub(super) fn parse_fn_def_with_flags(
        &mut self,
        decorators: Vec<Expr>,
        is_static: bool,
        is_class_method: bool,
    ) -> Result<Stmt, String> {
        self.advance(); // `fn` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータ `[T: Trait, ...]` をパース（なければ空 Vec）
        let template_params = self.parse_template_params()?;
        self.eat(&Token::LParen)?;
        let mut params: Vec<Param> = Vec::new();
        // 引数リストをカンマ区切りでパース
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            params.push(self.parse_param()?);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RParen)?;
        // 可変長パラメータは1つのみ・末尾にのみ配置可能
        let variadic_count = params.iter().filter(|p| p.variadic).count();
        if variadic_count > 1 {
            return Err(format!(
                "ParseError: function `{name}` has more than one variadic parameter `...`"
            ));
        }
        if variadic_count == 1 && !params.last().map(|p| p.variadic).unwrap_or(false) {
            return Err(format!(
                "ParseError: function `{name}` variadic parameter `...` must be the last parameter"
            ));
        }
        // デフォルト値なしのパラメータがデフォルト値ありのパラメータの後に来ていないか検証
        Self::validate_param_defaults(&params)?;
        // `-> 戻り値型` があればパース
        let return_type = if *self.current() == Token::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.eat(&Token::Colon)?;
        // 本体が `...` のみなら抽象メソッド（NEWLINE INDENT ELLIPSIS [NEWLINE] DEDENT）
        let (body, is_abstract) = if self.is_abstract_body() {
            self.advance(); // Newline
            self.advance(); // Indent
            self.advance(); // Ellipsis
            // 末尾の改行をスキップ
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if *self.current() == Token::Dedent {
                self.advance();
            }
            (vec![], true)
        } else {
            // 通常のブロックをパース
            (self.parse_block()?, false)
        };
        Ok(Stmt::FnDef {
            name,
            template_params,
            params,
            return_type,
            body,
            is_abstract,
            is_static,
            is_class_method,
            decorators,
            access: Accessibility::Public,
        })
    }

    /// `gen` ジェネレータ関数定義をパースして `Stmt::GenDef` を返す。
    ///
    /// ジェネレータ関数の制約:
    /// - `self` 以外のパラメータに `mut` は使用不可
    /// - 本体に `return` 文を含めてはならない
    ///
    /// # 戻り値
    /// `Stmt::GenDef { name, template_params, params, yield_type, body }`
    ///
    /// # エラー
    /// `self` 以外の `mut` パラメータが存在する場合、
    /// または本体に `return` 文が含まれる場合にエラーを返す。
    pub(super) fn parse_gen_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `gen` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータをパース（なければ空 Vec）
        let template_params = self.parse_template_params()?;
        self.eat(&Token::LParen)?;
        let mut params = Vec::new();
        // 引数リストをパース。self 以外の mut パラメータはエラー
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            let param = self.parse_param()?;
            if param.mutable && param.name != "self" {
                return Err(format!(
                    "ParseError: generator function `{name}`: parameter `{}` cannot be `mut`; \
                     generator parameters must be `let` or `const`",
                    param.name
                ));
            }
            params.push(param);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RParen)?;
        // 可変長パラメータは1つのみ・末尾にのみ配置可能
        let variadic_count_gen = params.iter().filter(|p| p.variadic).count();
        if variadic_count_gen > 1 {
            return Err(format!(
                "ParseError: generator `{name}` has more than one variadic parameter `...`"
            ));
        }
        if variadic_count_gen == 1 && !params.last().map(|p| p.variadic).unwrap_or(false) {
            return Err(format!(
                "ParseError: generator `{name}` variadic parameter `...` must be the last parameter"
            ));
        }
        Self::validate_param_defaults(&params)?;
        // `-> yield型` があればパース
        let yield_type = if *self.current() == Token::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;
        // ジェネレータ本体に return 文が含まれていないか検証
        if Self::body_has_return(&body) {
            return Err(format!(
                "ParseError: generator function `{name}` must not contain a `return` statement"
            ));
        }
        Ok(Stmt::GenDef {
            name,
            template_params,
            params,
            yield_type,
            body,
            access: Accessibility::Public,
        })
    }

    /// 文のリストに `return` 文が含まれるかを再帰的に検査する。
    ///
    /// ネストされた `fn`/`gen` 定義の内部には降りない（それらは独自の return スコープを持つ）。
    ///
    /// # 引数
    /// - `stmts`: 検査対象の文のスライス
    ///
    /// # 戻り値
    /// `return` 文が見つかれば `true`、見つからなければ `false`
    pub(super) fn body_has_return(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Return(_) => return true,
                Stmt::If {
                    branches,
                    else_body,
                } => {
                    if branches.iter().any(|(_, b)| Self::body_has_return(b)) {
                        return true;
                    }
                    if else_body.as_deref().map_or(false, Self::body_has_return) {
                        return true;
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    if Self::body_has_return(body) {
                        return true;
                    }
                }
                // Do not descend into nested fn/gen — they have their own return scope.
                _ => {}
            }
        }
        false
    }

    /// 現在位置から `NEWLINE INDENT ELLIPSIS` の並びが続くかを確認する。
    ///
    /// この並びはトレイトの仮想メソッド（抽象メソッド）の本体 `...` を示す。
    ///
    /// # 戻り値
    /// `NEWLINE INDENT ELLIPSIS` の並びが続く場合は `true`
    pub(super) fn is_abstract_body(&self) -> bool {
        let t0 = self
            .tokens
            .get(self.pos)
            .map(|s| &s.token)
            .unwrap_or(&Token::Eof);
        let t1 = self
            .tokens
            .get(self.pos + 1)
            .map(|s| &s.token)
            .unwrap_or(&Token::Eof);
        let t2 = self
            .tokens
            .get(self.pos + 2)
            .map(|s| &s.token)
            .unwrap_or(&Token::Eof);
        matches!(t0, Token::Newline)
            && matches!(t1, Token::Indent)
            && matches!(t2, Token::Ellipsis)
    }

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
    fn parse_try_stmt(&mut self) -> Result<Stmt, String> {
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
    fn parse_enum_def(&mut self) -> Result<Stmt, String> {
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
    fn parse_new_type_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `new_type` を消費
        let name = self.expect_ident()?;
        self.eat(&Token::Colon)?;
        let original = self.parse_type_expr()?;
        // 名前を登録して再代入を禁止する
        self.known_new_types.insert(name.clone());
        Ok(Stmt::NewTypeDef { name, original })
    }
}

