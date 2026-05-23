use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{Accessibility, BinOp, CallArg, ExceptHandler, Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt, TemplateParam, TupleTarget, UnaryOp};
use crate::token::{FStrPart, Span, Spanned, Token};
use crate::python_converter;
use crate::lexer;

/// 複合代入演算子トークンを対応する二項演算子（`BinOp`）に変換する。
///
/// # 引数
/// - `token`: 変換対象のトークン参照
///
/// # 戻り値
/// 複合代入演算子に対応する `BinOp` を `Some` で返す。
/// 複合代入演算子でないトークンの場合は `None` を返す。
fn token_to_compound_op(token: &Token) -> Option<BinOp> {
    match token {
        Token::PlusEq      => Some(BinOp::Add),
        Token::MinusEq     => Some(BinOp::Sub),
        Token::StarEq      => Some(BinOp::Mul),
        Token::SlashEq     => Some(BinOp::Div),
        Token::SlashSlashEq => Some(BinOp::FloorDiv),
        Token::PercentEq   => Some(BinOp::Mod),
        Token::StarStarEq  => Some(BinOp::Pow),
        Token::AmpEq       => Some(BinOp::BitAnd),
        Token::PipeEq      => Some(BinOp::BitOr),
        Token::CaretEq     => Some(BinOp::BitXor),
        Token::LtLtEq      => Some(BinOp::LShift),
        Token::GtGtEq      => Some(BinOp::RShift),
        _                  => None,
    }
}

pub struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
    /// trait name → (template_params, fields: [(name, kind, type_ann, has_default)], virtual_methods: [name])
    known_traits: HashMap<String, (Vec<TemplateParam>, Vec<(String, FieldKind, String, bool)>, Vec<String>)>,
    /// Incremented when entering a class/trait body; `Self` is only valid when this is > 0.
    class_or_trait_depth: usize,
    /// Names declared with `new_type` — any reassignment to these is a parse error.
    known_new_types: HashSet<String>,
    /// 現在パース中のファイルのディレクトリ（import の第一検索先）。
    source_dir: PathBuf,
    /// メインエントリーファイルのディレクトリ（import のフォールバック検索先）。
    /// サブパーサにも変更せず引き継がれる。
    root_dir: PathBuf,
    /// モジュールキャッシュ: (lang, 解決済みパス) → 変換済み tl AST。
    /// パース時に同じモジュールを複数回読み込まないために使用する。
    module_cache: HashMap<(String, PathBuf), Vec<Stmt>>,
    /// 循環 import 検出用: 現在読み込み中のモジュールパスのセット。
    loading: HashSet<PathBuf>,
}

impl Parser {
    /// パーサを初期化する。
    ///
    /// 組み込みの `Error` トレイトを `known_traits` に事前登録し、
    /// ユーザー定義クラスが `Error` を継承できるようにする。
    ///
    /// # 引数
    /// - `tokens`: レキサが生成したトークン列（`Spanned` の `Vec`）
    ///
    /// # 戻り値
    /// 初期化済みの `Parser` インスタンス
    /// パーサを初期化する。`source_dir` には .tl ファイルのディレクトリを渡す。
    pub fn new(tokens: Vec<Spanned>, source_dir: Option<PathBuf>) -> Self {
        // 組み込み `Error` トレイトを事前登録する。
        // フィールド: message（let・必須）、code_context/file（mut・デフォルトあり）、line/col（mut・デフォルトあり）
        let mut known_traits: HashMap<String, (Vec<TemplateParam>, Vec<(String, FieldKind, String, bool)>, Vec<String>)> = HashMap::new();
        known_traits.insert("Error".to_string(), (
            vec![],
            vec![
                ("message".to_string(),      FieldKind::Let, "str".to_string(), false),
                ("code_context".to_string(), FieldKind::Mut, "str".to_string(), true),
                ("file".to_string(),         FieldKind::Mut, "str".to_string(), true),
                ("line".to_string(),         FieldKind::Mut, "int".to_string(), true),
                ("col".to_string(),          FieldKind::Mut, "int".to_string(), true),
            ],
            vec![],
        ));
        let resolved = source_dir.unwrap_or_else(|| PathBuf::from("."));
        Self {
            tokens,
            pos: 0,
            known_traits,
            class_or_trait_depth: 0,
            known_new_types: HashSet::new(),
            source_dir: resolved.clone(),
            root_dir: resolved,
            module_cache: HashMap::new(),
            loading: HashSet::new(),
        }
    }

    /// 現在位置のトークンへの参照を返す。
    /// トークン列を超えた場合は `Token::Eof` を返す。
    fn current(&self) -> &Token {
        self.tokens.get(self.pos).map(|s| &s.token).unwrap_or(&Token::Eof)
    }

    /// 現在位置の1つ先（先読み1トークン）への参照を返す。
    /// トークン列を超えた場合は `Token::Eof` を返す。
    fn peek1(&self) -> &Token {
        self.tokens.get(self.pos + 1).map(|s| &s.token).unwrap_or(&Token::Eof)
    }

    /// 現在位置のトークンの `Span`（ファイル名・行・列）を返す。
    /// トークン列を超えた場合は `Span::unknown()` を返す。
    fn current_span(&self) -> Span {
        self.tokens.get(self.pos).map(|s| s.span.clone()).unwrap_or_else(Span::unknown)
    }

    /// 現在位置を1つ進める。トークン列の末尾では何もしない。
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    /// 現在のトークンが `expected` と一致すれば消費して `Ok(())` を返す。
    ///
    /// # エラー
    /// 一致しない場合は「expected `X`, got `Y`」形式のエラー文字列を返す。
    fn eat(&mut self, expected: &Token) -> Result<(), String> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected `{}`, got `{}`", expected, self.current()))
        }
    }

    /// 改行・インデント・デデント・セミコロンをまとめてスキップする。
    /// ブロック境界をまたぐ前後のクリーンアップに使用する。
    fn skip_newlines(&mut self) {
        while matches!(
            self.current(),
            Token::Newline | Token::Indent | Token::Dedent | Token::Semicolon
        ) {
            self.advance();
        }
    }

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
    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
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
            self.advance(); // skip ','
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
                other => return Err(format!("expected `let`, `mut`, or `_` in tuple unpack, got `{other}`")),
            }
        }
        self.eat(&Token::Eq)?;
        Ok(Stmt::LetTuple { targets, value: self.parse_expr()?, span })
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
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Let(name, self.parse_expr()?))
            }
            // `const 変数名 [: 型] = 式` — 定数宣言
            Token::Const => {
                self.advance();
                let name = self.expect_ident()?;
                // 型アノテーションがあれば読み飛ばす
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
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
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Mut(name, self.parse_expr()?))
            }
            // `static mut 変数名 [: 型] = 式` — 静的可変変数宣言（全呼び出しでセル共有）
            Token::Static => {
                let span = self.current_span();
                self.advance();
                self.eat(&Token::Mut)?;
                let name = self.expect_ident()?;
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
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
            Token::Pass     => { self.advance(); Ok(Stmt::Pass) }
            Token::Break    => { self.advance(); Ok(Stmt::Break) }
            Token::Continue => { self.advance(); Ok(Stmt::Continue) }
            // `return [式]` — 値なし return と値あり return を区別
            Token::Return => {
                self.advance();
                if matches!(self.current(), Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent) {
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
            Token::LoopYield   => { self.advance(); Ok(Stmt::LoopYield(self.parse_expr()?)) }
            // `break_point` — デバッガ REPL を起動して実行を一時停止する
            Token::BreakPoint => {
                let span = self.current_span();
                self.advance();
                Ok(Stmt::BreakPoint { span })
            }
            // 制御構文
            Token::If    => self.parse_if_stmt(),
            Token::Match => self.parse_match_stmt(),
            Token::While => {
                self.advance();
                let cond = self.parse_expr()?;
                let _ = self.parse_opt_return_type()?; // stmt level: parse and discard
                self.eat(&Token::Colon)?;
                Ok(Stmt::While { cond, body: self.parse_block()? })
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
                Ok(Stmt::For { targets, iter, body: self.parse_block()? })
            }
            // `block [->Type]:` — 値を返せるスコープブロック
            Token::Block => {
                self.advance();
                let _ = self.parse_opt_return_type()?; // stmt level: parse and discard
                self.eat(&Token::Colon)?;
                Ok(Stmt::Block(self.parse_block()?))
            }
            // `yield 式` — ジェネレータ関数内で値を yield する
            Token::Yield => { self.advance(); Ok(Stmt::Yield(self.parse_expr()?)) }
            Token::Try   => self.parse_try_stmt(),
            // `raise [式]` — 例外を送出する
            Token::Raise => {
                let span = self.current_span();
                self.advance();
                // 送出する例外式がなければ None（再 raise 相当）
                let exc = if matches!(self.current(), Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                Ok(Stmt::Raise { exc, span })
            }
            Token::At => {
                let decorators = self.parse_decorators()?;
                match self.current().clone() {
                    Token::Fn    => self.parse_fn_def_decorated(decorators),
                    Token::Class => self.parse_class_def_decorated(decorators),
                    tok => Err(format!(
                        "ParseError: '@' decorator must be followed by 'fn' or 'class', got `{tok}`"
                    )),
                }
            }
            Token::Fn      => self.parse_fn_def(),
            Token::Gen     => self.parse_gen_def(),
            Token::Class   => self.parse_class_def(),
            Token::Enum    => self.parse_enum_def(),
            Token::Trait   => self.parse_trait_def(),
            Token::NewType => self.parse_new_type_def(),
            Token::Import  => self.parse_import_stmt(),
            Token::From    => self.parse_from_import_stmt(),
            Token::Ident(_) => self.parse_ident_stmt(),
            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    /// `->Type` アノテーションを省略可能な形でパースして返す。
    ///
    /// 現在トークンが `->` の場合のみ型アノテーション文字列を返し、それ以外は `None`。
    fn parse_opt_return_type(&mut self) -> Result<Option<String>, String> {
        if *self.current() == Token::Arrow {
            self.advance(); // `->` を消費
            Ok(Some(self.parse_type_expr()?))
        } else {
            Ok(None)
        }
    }

    /// `if` キーワードを消費した後の節をパースする共有ヘルパー。
    ///
    /// `(branches, else_body, return_type)` を返す。
    /// `return_type` は最初の `if` 直後の `->Type` アノテーション。`elif`/`else` にはない。
    fn parse_if_components(&mut self) -> Result<(Vec<(Expr, Vec<Stmt>)>, Option<Vec<Stmt>>, Option<String>), String> {
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
        Ok(Stmt::If { branches, else_body })
    }

    /// `match expr:` 文をパースして `Stmt::Match` を返す。
    ///
    /// 構文:
    /// ```text
    /// match expr:
    ///     case pattern:
    ///         body
    ///     is TypeName:
    ///         body
    /// ```
    ///
    /// `case` アームと `is` アームを混在させるとパースエラー。
    /// `case _:` はワイルドカードアームとして解釈される。
    ///
    /// # エラー
    /// - `case` と `is` のアームが混在する場合
    /// - 予期しないトークンがアームの先頭に現れた場合
    /// match アームリストをパースする共有ヘルパー（`match subject:` の後、INDENT済みの状態で呼ぶ）。
    fn parse_match_arms(&mut self) -> Result<Vec<MatchArm>, String> {
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
                    arms.push(MatchArm { pattern: MatchPattern::Case(pattern_expr), body: self.parse_block()? });
                }
                Token::Is => {
                    if is_case_kind == Some(true) {
                        return Err("match statement cannot mix 'case' and 'is' arms".to_string());
                    }
                    is_case_kind = Some(false);
                    self.advance();
                    let type_name = self.expect_ident()?;
                    self.eat(&Token::Colon)?;
                    arms.push(MatchArm { pattern: MatchPattern::IsType(type_name), body: self.parse_block()? });
                }
                tok => return Err(format!("expected 'case' or 'is' in match body, got `{tok}`")),
            }
        }
        if *self.current() == Token::Dedent { self.advance(); }
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
        Ok(Stmt::Match { subject, arms, span })
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
                return Ok(Stmt::AsyncAssign { target, return_type, stmts });
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
                Ok(Stmt::Assign { name, value: self.parse_expr()?, span })
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
                        Ok(Stmt::AttrAssign { target: expr, value: self.parse_expr()? })
                    } else if let Some(op) = token_to_compound_op(&cur) {
                        // `expr += 値` など — 属性複合代入
                        self.advance();
                        Ok(Stmt::AttrCompoundAssign { target: expr, op, value: self.parse_expr()? })
                    } else {
                        // 代入でなければ式文として返す
                        Ok(Stmt::Expr(expr))
                    }
                }
            }
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
        Ok(Stmt::CompoundAssign { name, op, value: self.parse_expr()?, span })
    }

    /// `fn` 関数定義をパースして `Stmt::FnDef` を返す。
    ///
    /// テンプレートパラメータ・引数リスト・戻り値型・本体ブロックを順番にパースする。
    /// 本体が `...`（省略記号のみ）の場合は抽象メソッド（`is_abstract: true`）として扱う。
    ///
    /// # 戻り値
    /// `Stmt::FnDef { name, template_params, params, return_type, body, is_abstract }`
    ///
    /// # エラー
    /// 識別子・括弧・本体ブロックのパースに失敗した場合
    /// `@decorator` 構文のリストをパースして式のリストを返す。
    ///
    /// 現在のトークンが `@` の間、以下を繰り返す:
    /// 1. `@` を消費する
    /// 2. デコレータ式をパース（識別子・属性アクセス・関数呼び出し可）
    /// 3. 末尾の改行を消費する
    ///
    /// 戻り値: `Vec<Expr>` — 上から順に並んだデコレータ式のリスト
    fn parse_decorators(&mut self) -> Result<Vec<crate::ast::Expr>, String> {
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

    fn parse_fn_def(&mut self) -> Result<Stmt, String> {
        self.parse_fn_def_with_flags(vec![], false, false)
    }

    fn parse_fn_def_decorated(&mut self, decorators: Vec<crate::ast::Expr>) -> Result<Stmt, String> {
        self.parse_fn_def_with_flags(decorators, false, false)
    }

    fn parse_fn_def_with_flags(&mut self, decorators: Vec<crate::ast::Expr>, is_static: bool, is_class_method: bool) -> Result<Stmt, String> {
        self.advance(); // `fn` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータ `[T: Trait, ...]` をパース（なければ空 Vec）
        let template_params = self.parse_template_params()?;
        self.eat(&Token::LParen)?;
        let mut params = Vec::new();
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
        Ok(Stmt::FnDef { name, template_params, params, return_type, body, is_abstract, is_static, is_class_method, decorators, access: Accessibility::Public })
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
    fn parse_gen_def(&mut self) -> Result<Stmt, String> {
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
        Ok(Stmt::GenDef { name, template_params, params, yield_type, body, access: Accessibility::Public })
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
    fn body_has_return(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Return(_) => return true,
                Stmt::If { branches, else_body } => {
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
    fn is_abstract_body(&self) -> bool {
        let t0 = self.tokens.get(self.pos).map(|s| &s.token).unwrap_or(&Token::Eof);
        let t1 = self.tokens.get(self.pos + 1).map(|s| &s.token).unwrap_or(&Token::Eof);
        let t2 = self.tokens.get(self.pos + 2).map(|s| &s.token).unwrap_or(&Token::Eof);
        matches!(t0, Token::Newline) && matches!(t1, Token::Indent) && matches!(t2, Token::Ellipsis)
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
            handlers.push(ExceptHandler { exc_type, name, body: handler_body });
        }

        // Optional `finally` clause
        if *self.current() == Token::Finally {
            self.advance();
            self.eat(&Token::Colon)?;
            finally_body = Some(self.parse_block()?);
        }

        if handlers.is_empty() && finally_body.is_none() {
            return Err("try statement requires at least one `except` or `finally` clause".to_string());
        }

        Ok(Stmt::Try { body, handlers, finally_body })
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

    // -----------------------------------------------------------------------
    // import 解析
    // -----------------------------------------------------------------------

    /// `import[lang] module.sub as alias` をパースして `Stmt::Import` を返す。
    ///
    /// - `import[py] math as m`
    /// - `import[py] os.path as p`
    fn parse_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `import` を消費

        // `[lang]` を読む。省略時は "tl-auto" (auto-select: prefer .tlc over .tl)
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "tl-auto".to_string()
        };

        // cpp-dll / cpp-lib: `import[cpp-dll] "path.dll" with "path.h" as alias`
        if lang == "cpp-dll" || lang == "cpp-lib" {
            return self.parse_cpp_import(lang);
        }

        // モジュールパス (`a.b.c`)
        let module = self.parse_module_path()?;

        // `as alias` (省略可)
        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        // モジュールの tl AST を取得（キャッシュ込み）
        let body = self.load_module(&lang, &module)?;

        Ok(Stmt::Import { lang, module, with_file: None, alias, body })
    }

    /// `import[cpp-dll] "lib.dll" with "lib.h" as alias` をパースする。
    /// `import[cpp-lib] "lib.lib" with "lib.h" as alias` も同様。
    fn parse_cpp_import(&mut self, lang: String) -> Result<Stmt, String> {
        // ファイルパス (文字列リテラル)
        let file_path = match self.current().clone() {
            Token::Str(s) => { self.advance(); s }
            other => return Err(format!(
                "import[{lang}]: expected string literal for library path, got `{other}`"
            )),
        };

        // `with "header.h"` (省略可)
        let with_file = if *self.current() == Token::With {
            self.advance();
            match self.current().clone() {
                Token::Str(s) => { self.advance(); Some(s) }
                other => return Err(format!(
                    "import[{lang}]: expected string literal after `with`, got `{other}`"
                )),
            }
        } else {
            None
        };

        // `as alias` — cpp imports はエイリアスを強く推奨するが省略可
        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        // ファイルパスをモジュール識別子として使う (キャッシュキー兼デフォルト名に利用)
        let module = vec![file_path];
        Ok(Stmt::Import { lang, module, with_file, alias, body: vec![] })
    }

    /// `from module import[lang] Name1, Name2 as N2` をパースして `Stmt::FromImport` を返す。
    fn parse_from_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `from` を消費

        // モジュールパス
        let module = self.parse_module_path()?;

        // `import[lang]` または `import`（省略時は "tl-auto"）
        self.eat(&Token::Import)?;
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "tl-auto".to_string()
        };

        // 名前リスト: `Name1, Name2 as N2, ...`
        let mut names: Vec<(String, Option<String>)> = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let alias = if *self.current() == Token::As {
                self.advance();
                Some(self.expect_ident()?)
            } else {
                None
            };
            names.push((name, alias));
            if *self.current() == Token::Comma {
                self.advance();
                // 行末に来たら終了
                if matches!(self.current(), Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent) {
                    break;
                }
            } else {
                break;
            }
        }

        // モジュールの tl AST を取得
        let body = self.load_module(&lang, &module)?;

        Ok(Stmt::FromImport { lang, module, with_file: None, names, body })
    }

    /// `[lang]` トークン列をパースして言語識別子文字列を返す。
    fn parse_lang_bracket(&mut self) -> Result<String, String> {
        self.eat(&Token::LBracket)?;
        let mut lang = match self.current().clone() {
            Token::Ident(s) => { self.advance(); s }
            other => return Err(format!("expected language identifier, got `{other}`")),
        };
        // ハイフン区切りの識別子を許容（例: `py-int`）
        while *self.current() == Token::Minus {
            self.advance();
            match self.current().clone() {
                Token::Ident(s) => { self.advance(); lang = format!("{lang}-{s}"); }
                other => return Err(format!("expected identifier after '-' in lang tag, got `{other}`")),
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(lang)
    }

    /// ドット区切りのモジュールパスをパースして `Vec<String>` を返す。
    /// 例: `os.path` → `["os", "path"]`
    fn parse_module_path(&mut self) -> Result<Vec<String>, String> {
        let mut segments = vec![self.expect_ident()?];
        while *self.current() == Token::Dot {
            self.advance();
            segments.push(self.expect_ident()?);
        }
        Ok(segments)
    }

    /// モジュールを検索・読み込み・変換して tl AST を返す。キャッシュを使用する。
    fn load_module(&mut self, lang: &str, module: &[String]) -> Result<Vec<Stmt>, String> {
        match lang {
            // default (no bracket): prefer .tlc, fall back to .tl
            "tl-auto" => self.load_tl_module(module),
            // import[tl]: force .tl source, skip .tlc
            "tl"      => self.load_tl_source_module(module),
            // import[tlc]: force .tlc, error if not found
            "tlc"     => self.load_tlc_module(module),
            "py"      => self.load_python_module(module),
            // py-int: .pyi を優先し、なければ .py にフォールバック
            // body は型検査専用（実行時は PyO3 経由）
            "py-int"  => self.load_python_interface_module(module),
            other => Err(format!("unknown import language '{other}'")),
        }
    }

    /// `.tl` / `.tlc` モジュールをロードして AST を返す。
    ///
    /// 各検索ディレクトリ (`source_dir` → `root_dir`) に対して以下の優先順で試す:
    /// 1. `module.tlc`         — コンパイル済みモジュール（埋め込みソース付きバイナリ）
    /// 2. `module.tl`          — ソースファイルモジュール
    /// 3. `module/__init__.tl` — パッケージモジュール
    fn load_tl_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel  = module_base.with_extension("tlc");
        let file_rel = module_base.with_extension("tl");
        let init_rel = module_base.join("__init__.tl");

        // 検索ディレクトリリスト（source_dir と root_dir が同じなら重複させない）
        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        // (パス, コンパイル済みか) の候補リスト — .tlc が .tl より先になる
        let candidates: Vec<(PathBuf, bool)> = search_dirs.iter()
            .flat_map(|dir| [
                (dir.join(&tlc_rel),  true),
                (dir.join(&file_rel), false),
                (dir.join(&init_rel), false),
            ])
            .collect();

        let (abs_path, is_compiled) = candidates.iter()
            .find(|(p, _)| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates.iter()
                    .map(|(p, _)| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("cannot find module '{}' (looked at {})", module.join("."), paths)
            })?;

        let cache_key = ("tl-auto".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        // ソースを取得: .tlc はバイナリから埋め込みソースを抽出、.tl は直読み
        let (source, filename) = if is_compiled {
            let (mod_name, src) = crate::partial_compiler::load_tlc(&abs_path)
                .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
            let label = format!("<compiled:{mod_name}>");
            (src, label)
        } else {
            let src = std::fs::read_to_string(&abs_path)
                .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
            (src, abs_path.to_string_lossy().into_owned())
        };

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path.parent().map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut sub = Parser::new(tokens, Some(module_dir));
        // 親のキャッシュ・循環検出セット・ルートディレクトリを引き継ぐ
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;

        // 子パーサが生成したキャッシュエントリを親にマージする
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[tl]`: `.tl` ソースのみをロードする。`.tlc` があっても無視する。
    fn load_tl_source_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let file_rel = module_base.with_extension("tl");
        let init_rel = module_base.join("__init__.tl");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        let candidates: Vec<PathBuf> = search_dirs.iter()
            .flat_map(|dir| [dir.join(&file_rel), dir.join(&init_rel)])
            .collect();

        let abs_path = candidates.iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates.iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find source module '{}' (looked at {})",
                    module.join("."), paths
                )
            })?;

        let cache_key = ("tl".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!("circular import detected: '{}'", abs_path.display()));
        }

        let source = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
        let filename = abs_path.to_string_lossy().into_owned();

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path.parent().map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[tlc]`: `.tlc` コンパイル済みモジュールのみをロードする。`.tl` があっても無視する。
    fn load_tlc_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("tlc");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        let candidates: Vec<PathBuf> = search_dirs.iter()
            .map(|dir| dir.join(&tlc_rel))
            .collect();

        let abs_path = candidates.iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates.iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find compiled module '{}' (looked at {}; compile with: cargo run --release -- --compile <source.tl>)",
                    module.join("."), paths
                )
            })?;

        let cache_key = ("tlc".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!("circular import detected: '{}'", abs_path.display()));
        }

        let (mod_name, source) = crate::partial_compiler::load_tlc(&abs_path)
            .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
        let filename = format!("<compiled:{mod_name}>");

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path.parent().map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// Python モジュールを検索・変換する（キャッシュ込み）。
    fn load_python_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        // .py ファイルパスを解決する
        let rel_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("py");
        let abs_path = self.source_dir.join(&rel_path);
        let cache_key = ("py".to_string(), abs_path.clone());

        // キャッシュに存在する場合はそのまま返す
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // 循環 import 検出
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        // ファイルが存在しない場合はエラー
        let source = std::fs::read_to_string(&abs_path).map_err(|_| {
            format!(
                "cannot find module '{}' (looked at '{}')",
                module.join("."),
                abs_path.display()
            )
        })?;

        // 読み込み中マーカーをセット
        self.loading.insert(abs_path.clone());

        // Python → tl AST 変換
        let filename = abs_path.to_string_lossy().to_string();
        let body = python_converter::convert_python_source(&source, &filename)?;

        // 読み込み中マーカーを解除してキャッシュに登録
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[py-int]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// body は型検査専用（実行時は PyO3 経由で別ロジックが動く）。
    fn load_python_interface_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let pyi_rel = module_base.with_extension("pyi");
        let py_rel  = module_base.with_extension("py");

        // .pyi → .py の順に search_dirs で探す
        let search_dirs = self.python_search_dirs();
        for dir in &search_dirs {
            let pyi_abs = dir.join(&pyi_rel);
            if pyi_abs.exists() {
                return self.load_pyi_file(module, &pyi_abs);
            }
        }
        for dir in &search_dirs {
            let py_abs = dir.join(&py_rel);
            if py_abs.exists() {
                return self.load_pyi_file(module, &py_abs);
            }
        }

        // 見つからなければ空の body を返す（型検査スキップ、実行時は PyO3 が担当）
        Ok(vec![])
    }

    /// .pyi または .py ファイルをパースして型検査用 body を返す。
    /// 変換エラーが発生したステートメントは無視する（ベストエフォート）。
    fn load_pyi_file(&mut self, module: &[String], abs_path: &PathBuf) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py-int".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        let source = std::fs::read_to_string(abs_path).map_err(|_| {
            format!("cannot read interface file for module '{}'", module.join("."))
        })?;
        self.loading.insert(abs_path.clone());
        let filename = abs_path.to_string_lossy().to_string();
        // 変換エラーは無視して空 body を返す（.pyi は実行不要）
        let body = python_converter::convert_python_source(&source, &filename).unwrap_or_default();
        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// Python モジュールの検索ディレクトリリストを返す。
    /// source_dir を先頭に、PYTHONPATH 環境変数、Python site-packages を続ける。
    fn python_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.source_dir.clone()];
        if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
            for p in std::env::split_paths(&pythonpath) {
                dirs.push(p);
            }
        }
        // Python インタープリタの sys.prefix から site-packages を推測
        if let Ok(prefix) = std::env::var("PYTHONHOME") {
            dirs.push(PathBuf::from(&prefix).join("Lib").join("site-packages"));
        }
        dirs
    }

    /// `trait` 定義をパースして `Stmt::TraitDef` を返す。
    ///
    /// パース完了後に trait 名・テンプレートパラメータ・フィールド情報・
    /// 仮想メソッド名を `known_traits` に登録する。
    /// これにより、クラス定義時に継承チェックと auto-init 生成が行える。
    ///
    /// トレイトは他のトレイトを継承できない（`trait Foo(Bar):` はエラー）。
    /// 全てのメソッド（仮想・非仮想を問わず）に型アノテーションが必要。
    ///
    /// # 戻り値
    /// `Stmt::TraitDef { name, template_params, body }`
    ///
    /// # エラー
    /// トレイトが基底型を持つ場合、メソッドの型アノテーションが欠如している場合、
    /// または本体のパースに失敗した場合
    fn parse_trait_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `trait` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータをパース（`[T: Trait, ...]` 形式）
        let template_params = self.parse_template_params()?;
        // トレイトは継承不可
        if *self.current() == Token::LParen {
            return Err(format!("StaticTypeError: trait `{name}` cannot inherit from another type"));
        }
        self.eat(&Token::Colon)?;
        // Self 型が有効なスコープに入る
        self.class_or_trait_depth += 1;
        let body = self.parse_class_body()?;
        self.class_or_trait_depth -= 1;

        // 非仮想・仮想メソッドどちらも型アノテーションを必須とする
        for stmt in &body {
            if let Stmt::FnDef { name: mname, params, return_type, is_abstract, .. } = stmt {
                if !is_abstract {
                    // 非仮想メソッドの戻り値型チェック
                    if return_type.is_none() {
                        return Err(format!(
                            "StaticTypeError: trait method `{mname}` is missing a return type annotation"
                        ));
                    }
                    // 非仮想メソッドのパラメータ型チェック（self は除外）
                    for p in params {
                        if p.name != "self" && p.type_ann.is_none() {
                            return Err(format!(
                                "StaticTypeError: parameter `{}` of trait method `{mname}` is missing a type annotation",
                                p.name
                            ));
                        }
                    }
                } else {
                    // 仮想メソッド（`...` 本体）も型アノテーション必須
                    if return_type.is_none() {
                        return Err(format!(
                            "StaticTypeError: virtual method `{mname}` is missing a return type annotation"
                        ));
                    }
                    for p in params {
                        if p.name != "self" && p.type_ann.is_none() {
                            return Err(format!(
                                "StaticTypeError: parameter `{}` of virtual method `{mname}` is missing a type annotation",
                                p.name
                            ));
                        }
                    }
                }
            }
        }

        // フィールド情報（名前・種別・型・デフォルト有無）を収集
        let fields: Vec<(String, FieldKind, String, bool)> = body.iter()
            .filter_map(|s| {
                if let Stmt::Field { name: fname, kind, type_ann, default, .. } = s {
                    Some((fname.clone(), kind.clone(), type_ann.clone(), default.is_some()))
                } else {
                    None
                }
            })
            .collect();
        // 仮想メソッド（is_abstract: true）の名前リストを収集
        let virtual_methods: Vec<String> = body.iter()
            .filter_map(|s| {
                if let Stmt::FnDef { name: mname, is_abstract: true, .. } = s {
                    Some(mname.clone())
                } else {
                    None
                }
            })
            .collect();
        // 後続のクラス定義が参照できるよう known_traits に登録
        self.known_traits.insert(name.clone(), (template_params.clone(), fields, virtual_methods));

        Ok(Stmt::TraitDef { name, template_params, body })
    }

    /// `class` 定義をパースして `Stmt::ClassDef` を返す。
    ///
    /// テンプレートパラメータ・基底トレイトリスト・クラス本体を順番にパースする。
    /// 基底クラスには他のクラスを指定できず、`known_traits` に登録済みのトレイトのみ許可される。
    ///
    /// パース後:
    /// 1. `collect_trait_fields_and_check_virtuals()` で仮想メソッドのオーバーライドを検証し、
    ///    トレイトの必須フィールドを収集する
    /// 2. `generate_auto_init_if_needed()` で必要に応じて `__init__` を自動生成する
    ///
    /// # 戻り値
    /// `Stmt::ClassDef { name, template_params, bases, body }`
    ///
    /// # エラー
    /// 非トレイト型を基底に指定した場合、仮想メソッドを未オーバーライドの場合、
    /// または本体のパースに失敗した場合
    fn parse_class_def(&mut self) -> Result<Stmt, String> {
        self.parse_class_def_decorated(vec![])
    }

    fn parse_class_def_decorated(&mut self, decorators: Vec<crate::ast::Expr>) -> Result<Stmt, String> {
        self.advance(); // `class` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータをパース（`[T: Trait, ...]` 形式）
        let template_params = self.parse_template_params()?;
        // 基底トレイトリストとそれぞれの型引数を収集する
        // bases_with_args: (トレイト名, 具体型引数リスト)
        let mut bases_with_args: Vec<(String, Vec<String>)> = Vec::new();
        let mut bases = Vec::new();
        if *self.current() == Token::LParen {
            self.advance();
            while *self.current() != Token::RParen && *self.current() != Token::Eof {
                let base_name = self.expect_ident()?;
                // テンプレートトレイトを具体化する型引数（例: `Container[int]`）
                let type_args = self.parse_type_args()?;
                bases_with_args.push((base_name.clone(), type_args));
                bases.push(base_name);
                if *self.current() == Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            self.eat(&Token::RParen)?;
        }
        // 基底にトレイト以外（クラス等）を指定するとエラー
        for base in &bases {
            if !self.known_traits.contains_key(base.as_str()) {
                return Err(format!(
                    "ParseError: class `{name}` cannot inherit from `{base}` (only traits are allowed as bases)"
                ));
            }
        }
        self.eat(&Token::Colon)?;
        // Self 型が有効なスコープに入る
        self.class_or_trait_depth += 1;
        let mut body = self.parse_class_body()?;
        self.class_or_trait_depth -= 1;

        // 仮想メソッドのオーバーライド検証とトレイト必須フィールドの収集
        let trait_required =
            self.collect_trait_fields_and_check_virtuals(&name, &bases_with_args, &body)?;

        // クラス自身のデフォルトなし mut/let フィールドを収集
        let class_required: Vec<(String, String)> = body.iter()
            .filter_map(|s| {
                if let Stmt::Field { name: fname, kind: FieldKind::Mut | FieldKind::Let, type_ann, default: None, .. } = s {
                    Some((fname.clone(), type_ann.clone()))
                } else {
                    None
                }
            })
            .collect();

        // 必要に応じて __init__ を自動生成して body に追加
        self.generate_auto_init_if_needed(&trait_required, &class_required, &mut body);

        Ok(Stmt::ClassDef { name, template_params, bases, body, decorators })
    }

    /// 継承トレイトの仮想メソッドオーバーライドを検証し、必須フィールドを収集する。
    ///
    /// 各基底トレイトについて:
    /// 1. 仮想メソッドがクラス本体でオーバーライドされているか確認する
    /// 2. デフォルト値なしのフィールドを `(トレイト名, フィールド名, 型名)` として収集する
    ///
    /// テンプレートトレイトの型変数は `concrete_args` で置換する。
    ///
    /// # 引数
    /// - `class_name`: 検証対象のクラス名（エラーメッセージ用）
    /// - `bases_with_args`: 基底トレイトと具体型引数のリスト
    /// - `body`: クラス本体の文リスト
    ///
    /// # 戻り値
    /// `(トレイト名, フィールド名, 解決済み型名)` のタプルリスト
    ///
    /// # エラー
    /// 仮想メソッドがオーバーライドされていない場合
    fn collect_trait_fields_and_check_virtuals(
        &self,
        class_name: &str,
        bases_with_args: &[(String, Vec<String>)],
        body: &[Stmt],
    ) -> Result<Vec<(String, String, String)>, String> {
        let mut trait_required = Vec::new();
        for (base, concrete_args) in bases_with_args {
            if let Some((trait_tparams, trait_fields, virtual_methods)) =
                self.known_traits.get(base).cloned()
            {
                // テンプレートパラメータ → 具体型 の変換マップを構築
                let type_map: HashMap<String, String> = trait_tparams.iter()
                    .zip(concrete_args.iter())
                    .map(|(tp, arg)| (tp.name.clone(), arg.clone()))
                    .collect();
                // 各仮想メソッドがクラス本体でオーバーライドされているか確認
                for virt in &virtual_methods {
                    let overridden = body.iter().any(|s| {
                        matches!(s, Stmt::FnDef { name: n, is_abstract: false, .. } if n == virt)
                    });
                    if !overridden {
                        return Err(format!(
                            "StaticTypeError: class `{class_name}` must override virtual method \
                             `{virt}` from trait `{base}`"
                        ));
                    }
                }
                // デフォルト値なしのフィールドを必須フィールドとして収集
                // 型変数が含まれる場合は具体型に解決する
                for (fname, _kind, ftype, has_default) in &trait_fields {
                    if !has_default {
                        let resolved = type_map.get(ftype).cloned().unwrap_or_else(|| ftype.clone());
                        trait_required.push((base.clone(), fname.clone(), resolved));
                    }
                }
            }
        }
        Ok(trait_required)
    }

    /// 必須フィールドが存在し、かつ完全一致する `__init__` が未定義の場合に
    /// デフォルトコンストラクタを自動生成して `body` に追加する。
    ///
    /// 生成される `__init__` のシグネチャ:
    /// `fn __init__(mut self, trait_field1: T1, ..., class_field1: T2, ...) -> None:`
    ///
    /// 既存の `__init__` の引数型・個数が自動生成と完全一致する場合は生成しない（override）。
    /// 異なる場合は既存定義と共存する（overload）。
    ///
    /// # 引数
    /// - `trait_required`: トレイトから継承した必須フィールド（トレイト名, フィールド名, 型名）
    /// - `class_required`: クラス自身の必須フィールド（フィールド名, 型名）
    /// - `body`: クラス本体の文リスト。生成した `__init__` を末尾に追加する
    fn generate_auto_init_if_needed(
        &self,
        trait_required: &[(String, String, String)],
        class_required: &[(String, String)],
        body: &mut Vec<Stmt>,
    ) {
        // 必須フィールドが1つもなければ auto-init は不要
        if trait_required.is_empty() && class_required.is_empty() {
            return;
        }

        // トレイトとクラス双方の必須フィールドを統合したリストを作成
        let all_required: Vec<(String, String)> = trait_required.iter()
            .map(|(_, fname, ftype)| (fname.clone(), ftype.clone()))
            .chain(class_required.iter().cloned())
            .collect();

        // 完全一致する既存 __init__ がある場合は auto-init を生成しない（override）
        let has_exact_match = body.iter().any(|s| {
            if let Stmt::FnDef { name: n, params, .. } = s {
                n == "__init__" && Self::init_sig_matches(&all_required, params)
            } else {
                false
            }
        });

        if has_exact_match {
            return;
        }

        // `mut self` に続いてトレイトフィールド、クラスフィールドの順でパラメータを構築
        let mut params = vec![Param { name: "self".to_string(), mutable: true, type_ann: None, default: None }];
        for (_, fname, ftype) in trait_required {
            params.push(Param { name: fname.clone(), mutable: false, type_ann: Some(ftype.clone()), default: None });
        }
        for (fname, ftype) in class_required {
            params.push(Param { name: fname.clone(), mutable: false, type_ann: Some(ftype.clone()), default: None });
        }

        // __init__ 本体を構築する
        // トレイトフィールドは TraitAccess、クラスフィールドは Attr で代入する
        let mut init_body: Vec<Stmt> = Vec::new();
        for (tname, fname, _) in trait_required {
            // `self::TraitName.field = field` の形式で代入
            init_body.push(Stmt::AttrAssign {
                target: Expr::TraitAccess {
                    object: Box::new(Expr::Ident("self".to_string())),
                    trait_name: tname.clone(),
                    attr: fname.clone(),
                },
                value: Expr::Ident(fname.clone()),
            });
        }
        for (fname, _) in class_required {
            // `self.field = field` の形式で代入
            init_body.push(Stmt::AttrAssign {
                target: Expr::Attr {
                    object: Box::new(Expr::Ident("self".to_string())),
                    attr: fname.clone(),
                    span: Span::unknown(),
                },
                value: Expr::Ident(fname.clone()),
            });
        }

        // 生成した __init__ を body の末尾に追加
        body.push(Stmt::FnDef {
            name: "__init__".to_string(),
            template_params: vec![],
            params,
            return_type: Some("None".to_string()),
            body: init_body,
            is_abstract: false,
            is_static: false,
            is_class_method: false,
            decorators: vec![],
            access: Accessibility::Public,
        });
    }

    /// クラス定義のインデントブロックをパースして文のリストを返す。
    ///
    /// `parse_block()` と同様に `NEWLINE INDENT stmt* DEDENT` を処理するが、
    /// 内部では `parse_class_stmt()` を呼ぶ点が異なる。
    /// フィールド宣言には型アノテーションが必須。
    ///
    /// # 戻り値
    /// クラス本体の文リスト
    ///
    /// # エラー
    /// `NEWLINE` / `INDENT` が欠如している場合、または `parse_class_stmt()` がエラーを返した場合
    fn parse_class_body(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
        let mut stmts = Vec::new();
        let mut current_access = Accessibility::Public;
        loop {
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.current(), Token::Dedent | Token::Eof) {
                break;
            }
            // accessibility section header: `public:` / `private:` / `protected:`
            match self.current().clone() {
                Token::Public | Token::Private | Token::Protected => {
                    current_access = match self.current() {
                        Token::Public    => Accessibility::Public,
                        Token::Private   => Accessibility::Private,
                        _                => Accessibility::Protected,
                    };
                    self.advance(); // consume public/private/protected
                    self.eat(&Token::Colon)?;
                    while matches!(self.current(), Token::Newline | Token::Semicolon) {
                        self.advance();
                    }
                    continue;
                }
                _ => {}
            }
            let mut stmt = self.parse_class_stmt()?;
            // apply current accessibility to fields and method definitions
            match &mut stmt {
                Stmt::Field { access, .. } => *access = current_access.clone(),
                Stmt::FnDef { access, .. } => *access = current_access.clone(),
                Stmt::GenDef { access, .. } => *access = current_access.clone(),
                _ => {}
            }
            stmts.push(stmt);
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        Ok(stmts)
    }

    /// クラス本体内の1文をパースする。
    ///
    /// 受け入れる文の種類:
    /// - `mut`/`let`/`const` + 識別子 + `: 型` + `[= デフォルト値]` → フィールド宣言
    /// - `fn` → メソッド定義
    /// - `gen` → ジェネレータメソッド定義
    /// - `pass` → 空文
    ///
    /// フィールド宣言では型アノテーション（`: 型`）が必須。
    /// `const` フィールドはデフォルト値が必須。
    ///
    /// # 戻り値
    /// `Stmt::Field` / `Stmt::FnDef` / `Stmt::GenDef` / `Stmt::Pass`
    ///
    /// # エラー
    /// 型アノテーション欠如・`const` のデフォルト値欠如・未対応トークン
    fn parse_class_stmt(&mut self) -> Result<Stmt, String> {
        match self.current().clone() {
            // フィールド宣言: mut/let/const キーワードで始まる
            Token::Mut | Token::Let | Token::Const => {
                let kind = match self.current() {
                    Token::Mut   => FieldKind::Mut,
                    Token::Let   => FieldKind::Let,
                    _            => FieldKind::Const,
                };
                // エラーメッセージ用にキーワード文字列を保持
                let keyword = match &kind {
                    FieldKind::Mut       => "mut",
                    FieldKind::Let       => "let",
                    FieldKind::Const     => "const",
                    FieldKind::StaticMut => "static mut",
                };
                self.advance();
                let fname = self.expect_ident()?;
                // 型アノテーションは必須
                if *self.current() != Token::Colon {
                    return Err(format!(
                        "class field `{fname}` must have a type annotation (e.g., `{keyword} {fname}: int = 0`)"
                    ));
                }
                self.advance(); // `:` を消費
                let type_ann = self.parse_type_expr()?;
                // `= デフォルト値` がある場合はパース
                let default = if *self.current() == Token::Eq {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                // const フィールドはデフォルト値が必須
                if kind == FieldKind::Const && default.is_none() {
                    return Err(format!(
                        "class variable `{fname}` declared with `const` must have an initial value (e.g., `const {fname}: int = 0`)"
                    ));
                }
                Ok(Stmt::Field { name: fname, kind, type_ann, default, access: Accessibility::Public })
            }
            Token::Fn => self.parse_fn_def(),
            Token::Gen => self.parse_gen_def(),
            Token::Static => {
                self.advance(); // consume `static`
                match self.current().clone() {
                    Token::Fn => self.parse_fn_def_with_flags(vec![], true, false),
                    Token::Mut => {
                        self.advance(); // consume `mut`
                        let fname = self.expect_ident()?;
                        if *self.current() != Token::Colon {
                            return Err(format!(
                                "class static field `{fname}` must have a type annotation (e.g., `static mut {fname}: int = 0`)"
                            ));
                        }
                        self.advance(); // consume `:`
                        let type_ann = self.parse_type_expr()?;
                        let default = if *self.current() == Token::Eq {
                            self.advance();
                            Some(self.parse_expr()?)
                        } else {
                            None
                        };
                        Ok(Stmt::Field { name: fname, kind: FieldKind::StaticMut, type_ann, default, access: Accessibility::Public })
                    }
                    tok => Err(format!("expected `fn` or `mut` after `static` in class body, got `{tok}`")),
                }
            }
            Token::ClassMethod => {
                self.advance(); // consume `class_method`
                if *self.current() != Token::Fn {
                    return Err(format!("expected `fn` after `class_method`, got `{}`", self.current()));
                }
                self.parse_fn_def_with_flags(vec![], false, true)
            }
            Token::Pass => {
                self.advance();
                Ok(Stmt::Pass)
            }
            tok => Err(format!("unexpected statement in class body: `{tok}`")),
        }
    }

    /// 既存の `__init__` のシグネチャが自動生成のシグネチャと完全一致するかを判定する。
    ///
    /// `self` を除く引数の個数と各引数の型アノテーションを比較する。
    /// 完全一致する場合は auto-init の生成をスキップする（override）。
    ///
    /// # 引数
    /// - `required_fields`: 必須フィールドの `(フィールド名, 型名)` リスト
    /// - `params`: 既存 `__init__` のパラメータリスト
    ///
    /// # 戻り値
    /// 完全一致する場合は `true`
    fn init_sig_matches(required_fields: &[(String, String)], params: &[Param]) -> bool {
        let non_self: Vec<_> = params.iter().filter(|p| p.name != "self").collect();
        non_self.len() == required_fields.len()
            && non_self.iter().zip(required_fields.iter()).all(|(p, (_, ftype))| {
                p.type_ann.as_deref() == Some(ftype.as_str())
            })
    }

    /// 呼び出しサイトや継承サイトの具体型引数列 `[Type1, Type2, ...]` をパースする。
    ///
    /// 現在のトークンが `[` でない場合は空のベクタを返す（型引数なし）。
    ///
    /// # 戻り値
    /// 型名の文字列リスト（型引数がない場合は空 Vec）
    ///
    /// # エラー
    /// 型名のパースに失敗した場合、または `]` がない場合
    fn parse_type_args(&mut self) -> Result<Vec<String>, String> {
        if *self.current() != Token::LBracket {
            return Ok(vec![]);
        }
        self.advance(); // consume `[`
        let mut args = Vec::new();
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            args.push(self.parse_type_expr()?);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(args)
    }

    /// テンプレートパラメータ列 `[T1: Trait1 and Trait2, T2: Trait3]` をパースする。
    ///
    /// 現在のトークンが `[` でない場合は空のベクタを返す（テンプレートなし）。
    /// 各パラメータは `名前: トレイト1 [and トレイト2 ...]` の形式。
    ///
    /// # 戻り値
    /// `TemplateParam` のリスト（テンプレートパラメータがない場合は空 Vec）
    ///
    /// # エラー
    /// 識別子やトレイト名のパースに失敗した場合、または `]` がない場合
    fn parse_template_params(&mut self) -> Result<Vec<TemplateParam>, String> {
        if *self.current() != Token::LBracket {
            return Ok(vec![]);
        }
        self.advance(); // consume `[`
        let mut params = Vec::new();
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            // 型名として完全な型式（例: `int`, `dict[str, int]`, `T`）を受け付ける。
            // `__cast__[int]` のように制約なしの具体型名を使える。
            let name = self.parse_type_expr()?;
            // `: constraint` は省略可能。省略した場合は制約なし。
            let constraints = if *self.current() == Token::Colon {
                self.advance(); // consume `:`
                let mut cs = vec![self.expect_ident()?];
                // `and` で複数のトレイト制約を結合: `T: TraitA and TraitB`
                while *self.current() == Token::And {
                    self.advance();
                    cs.push(self.expect_ident()?);
                }
                cs
            } else {
                vec![]
            };
            params.push(TemplateParam { name, constraints });
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(params)
    }

    /// デフォルト値のないパラメータがデフォルト値ありのパラメータの後に来ていないか検証する。
    ///
    /// `self` パラメータはデフォルト値を持たないが先頭に位置するため検査から除外する。
    fn validate_param_defaults(params: &[Param]) -> Result<(), String> {
        let mut seen_default = false;
        for p in params {
            if p.name == "self" { continue; }
            if p.default.is_some() {
                seen_default = true;
            } else if seen_default {
                return Err(format!(
                    "ParseError: non-default parameter '{}' follows a parameter with a default value",
                    p.name
                ));
            }
        }
        Ok(())
    }

    /// 関数パラメータを1つパースして `Param` を返す。
    ///
    /// 構文: `[mut] 識別子 [: 型] [= デフォルト式]`
    ///
    /// # 戻り値
    /// `Param { name, mutable, type_ann, default }`
    /// - `mutable`: `mut` キーワードが先行している場合は `true`
    /// - `type_ann`: `: 型` がある場合は `Some(型名)`、ない場合は `None`
    /// - `default`: `= 式` がある場合は `Some(式)`、ない場合は `None`
    ///
    /// # エラー
    /// 識別子または型名のパースに失敗した場合
    fn parse_param(&mut self) -> Result<Param, String> {
        // `mut` / `let` qualifier — mut means mutable, let (or absent) means immutable
        let mutable = if *self.current() == Token::Mut {
            self.advance();
            true
        } else {
            if *self.current() == Token::Let {
                self.advance(); // consume optional `let`, treated as immutable
            }
            false
        };
        let name = self.expect_ident()?;
        // 型アノテーション `: 型` があればパース
        let type_ann = if *self.current() == Token::Colon {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        // デフォルト値 `= 式` があればパース
        let default = if *self.current() == Token::Eq {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param { name, mutable, type_ann, default })
    }

    /// 型アノテーション文字列をパースして基底型名を返す。
    ///
    /// ジェネリック引数（`list[int]`・`dict[str, int]` 等）はスキップし基底型名のみ返す。
    /// ただし `tuple[T1, T2, ...]` は要素型を保持した形式（`"tuple[int,str]"` など）で返す。
    /// `Union[T1, T2]` / `Option[T]` も完全な文字列表現で返す。
    ///
    /// 受け入れる型名:
    /// - 識別子（`int`, `str`, `MyClass` 等）
    /// - `None`, `Any`（キーワードトークン）
    /// - `Self`（クラス・トレイト内のみ有効）
    /// - `Union[...]`, `Option[...]`（複合型）
    ///
    /// # 戻り値
    /// 型名の文字列
    ///
    /// # エラー
    /// `Self` をクラス・トレイト外で使用した場合、
    /// 型名として認識できないトークンが現れた場合、
    /// または `Union` に2つ未満の型引数を指定した場合
    fn parse_type_expr(&mut self) -> Result<String, String> {
        match self.current().clone() {
            Token::Union => {
                self.advance();
                if *self.current() != Token::LBracket {
                    return Err("Union requires type arguments: Union[Type1, Type2, ...]".to_string());
                }
                self.advance(); // consume '['
                let mut args = Vec::new();
                while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                    args.push(self.parse_type_expr()?);
                    if *self.current() == Token::Comma { self.advance(); }
                }
                self.eat(&Token::RBracket)?;
                if args.len() < 2 {
                    return Err(format!("Union requires at least 2 type arguments, got {}", args.len()));
                }
                return Ok(format!("Union[{}]", args.join(",")));
            }
            Token::Option => {
                self.advance();
                if *self.current() != Token::LBracket {
                    return Err("Option requires a type argument: Option[Type]".to_string());
                }
                self.advance(); // consume '['
                let inner = self.parse_type_expr()?;
                if *self.current() == Token::Comma { self.advance(); }
                self.eat(&Token::RBracket)?;
                return Ok(format!("Option[{inner}]"));
            }
            _ => {}
        }

        let base = match self.current().clone() {
            Token::Ident(name) => { self.advance(); name }
            Token::None => { self.advance(); "None".to_string() }
            Token::Any => { self.advance(); "Any".to_string() }
            Token::SelfType => {
                if self.class_or_trait_depth == 0 {
                    return Err("ParseError: 'Self' can only be used inside class or trait definitions".to_string());
                }
                self.advance();
                "Self".to_string()
            }
            tok => return Err(format!("expected type name, got `{tok}`")),
        };
        // type[T] — preserve inner type for TypeValOf checking in the type checker.
        if base == "type" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let inner = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("type[{inner}]"));
        }
        // tuple[T1, T2, ...] — preserve element types for the type checker.
        if base == "tuple" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let mut args = Vec::new();
            while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                args.push(self.parse_type_expr()?);
                if *self.current() == Token::Comma { self.advance(); }
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("tuple[{}]", args.join(",")));
        }
        // function type — function, function[params]->ret, function{params}->ret
        if base == "function" {
            return self.parse_function_type_ann();
        }
        // list[T] — preserve element type
        if base == "list" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let elem = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("list[{elem}]"));
        }
        // set[T] — preserve element type
        if base == "set" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let elem = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("set[{elem}]"));
        }
        // dict[K, V] — preserve key and value types
        if base == "dict" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let key = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            let val = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("dict[{key},{val}]"));
        }
        // Skip optional generic parameters for all other types (custom classes, etc.)
        if *self.current() == Token::LBracket {
            self.advance();
            let mut depth = 1usize;
            while depth > 0 && *self.current() != Token::Eof {
                if *self.current() == Token::LBracket {
                    depth += 1;
                } else if *self.current() == Token::RBracket {
                    depth -= 1;
                }
                self.advance();
            }
        }
        Ok(base)
    }

    /// `function` キーワードを消費済みの状態で呼び出し、関数型アノテーション文字列を返す。
    ///
    /// 構文:
    /// - `function`                        — 任意の関数型
    /// - `function[let T, mut T2, ...]`    — 位置引数型付き（引数名は自動生成）
    /// - `function{let name: T, ...}`      — 名前付き引数型付き
    /// - `function[...]->RetType`          — 戻り値型付き
    fn parse_function_type_ann(&mut self) -> Result<String, String> {
        let params_str = match self.current() {
            Token::LBracket => {
                // 位置引数: function[let int, mut str, ...]
                self.advance(); // consume '['
                let mut params = Vec::new();
                let mut auto_idx = 1usize;
                while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                    let mutable = match self.current() {
                        Token::Mut => { self.advance(); true }
                        Token::Let => { self.advance(); false }
                        _ => false,
                    };
                    let ty = self.parse_type_expr()?;
                    let prefix = if mutable { "mut" } else { "let" };
                    params.push(format!("{prefix} param{auto_idx}:{ty}"));
                    auto_idx += 1;
                    if *self.current() == Token::Comma { self.advance(); }
                }
                self.eat(&Token::RBracket)?;
                Some(format!("[{}]", params.join(",")))
            }
            Token::LBrace => {
                // 名前付き引数: function{let name: type, mut name2: type2, ...}
                self.advance(); // consume '{'
                let mut params = Vec::new();
                while *self.current() != Token::RBrace && *self.current() != Token::Eof {
                    let mutable = match self.current() {
                        Token::Mut => { self.advance(); true }
                        Token::Let => { self.advance(); false }
                        _ => false,
                    };
                    let name = self.expect_ident()?;
                    self.eat(&Token::Colon)?;
                    let ty = self.parse_type_expr()?;
                    let prefix = if mutable { "mut" } else { "let" };
                    params.push(format!("{prefix} {name}:{ty}"));
                    if *self.current() == Token::Comma { self.advance(); }
                }
                self.eat(&Token::RBrace)?;
                Some(format!("{{{}}}", params.join(",")))
            }
            _ => None,
        };

        let ret_str = if *self.current() == Token::Arrow {
            self.advance(); // consume '->'
            let ret = self.parse_type_expr()?;
            format!("->{ret}")
        } else {
            String::new()
        };

        match params_str {
            Some(p) => Ok(format!("function{p}{ret_str}")),
            None => {
                if ret_str.is_empty() {
                    Ok("function".to_string())
                } else {
                    Ok(format!("function{ret_str}"))
                }
            }
        }
    }

    /// 現在位置の `[...]` がテンプレート呼び出しか subscript かを先読みで判定する。
    ///
    /// `]` の直後に `(` が続く場合はテンプレート呼び出し（`f[T](args)` 形式）、
    /// そうでない場合は subscript（`obj[index]` 形式）とみなす。
    ///
    /// ネストした `[]` を正しく追跡するため深さカウンタを使用する。
    ///
    /// # 戻り値
    /// テンプレート呼び出しと判定した場合は `true`、subscript の場合は `false`
    fn is_template_instantiation(&self) -> bool {
        let mut i = self.pos + 1; // skip the opening `[`
        let mut depth = 1usize;
        while i < self.tokens.len() {
            match &self.tokens[i].token {
                Token::LBracket => depth += 1,
                Token::RBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.tokens.get(i + 1).map(|s| &s.token),
                            Some(Token::LParen)
                        );
                    }
                }
                Token::Eof => break,
                _ => {}
            }
            i += 1;
        }
        false
    }

    /// 現在のトークンが識別子（`Token::Ident`）であれば消費して名前を返す。
    ///
    /// # 戻り値
    /// 識別子の名前文字列
    ///
    /// # エラー
    /// 現在のトークンが識別子でない場合
    fn expect_ident(&mut self) -> Result<String, String> {
        if let Token::Ident(name) = self.current().clone() {
            self.advance();
            Ok(name)
        } else {
            Err(format!("expected identifier, got `{}`", self.current()))
        }
    }

    /// `.attr` コンテキストで属性名を取得する。
    ///
    /// 通常の識別子に加えて、メソッド名として使用されるキーワード（`match`, `format`,
    /// `split` など）も受け付ける。これにより `obj.match(...)` のような呼び出しを許可する。
    fn expect_attr_name(&mut self) -> Result<String, String> {
        let name = match self.current().clone() {
            Token::Ident(s) => s,
            // Allow any keyword that can also be used as a method name
            tok => match tok.keyword_str() {
                Some(s) => s.to_string(),
                None => return Err(format!("expected attribute name, got `{}`", self.current())),
            },
        };
        self.advance();
        Ok(name)
    }

    /// 型ガード式（`is` / `is not`）の右辺に書ける型名をパースして文字列で返す。
    ///
    /// 通常の識別子に加えて `None` キーワードも型名として受け付ける。
    fn expect_guard_type_name(&mut self) -> Result<String, String> {
        match self.current().clone() {
            Token::Ident(name) => { self.advance(); Ok(name) }
            Token::None => { self.advance(); Ok("None".to_string()) }
            tok => Err(format!("expected type name after `is`, got `{tok}`")),
        }
    }

    // --- 式のパース（優先順位昇順）---

    /// 式をパースする（演算子優先順位の最低レベル）。
    ///
    /// 内部的には `parse_or()` に委譲し、`or` 演算子から始まる優先順位の連鎖を経て
    /// 最終的に `parse_primary()` まで到達する。
    ///
    /// # 戻り値
    /// パースした式（`Expr`）
    ///
    /// # エラー
    /// 式のパースに失敗した場合
    pub fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    /// `or` 二項演算子（短絡評価）を左結合でパースする。
    ///
    /// 優先順位: `or` < `and` の関係。
    /// `parse_and()` を下位に委譲する。
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while *self.current() == Token::Or {
            let span = self.current_span();
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    /// `and` 二項演算子（短絡評価）を左結合でパースする。
    ///
    /// 優先順位: `and` < `not`。
    /// `parse_not()` を下位に委譲する。
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while *self.current() == Token::And {
            let span = self.current_span();
            self.advance();
            let right = self.parse_not()?;
            left = Expr::BinOp { op: BinOp::And, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    /// `not` 単項演算子（論理否定）を右結合でパースする。
    ///
    /// `not not x` のように連鎖する場合も再帰で処理する。
    /// `not` でない場合は `parse_comparison()` に委譲する。
    fn parse_not(&mut self) -> Result<Expr, String> {
        if *self.current() == Token::Not {
            self.advance();
            // 再帰で右結合を実現
            let operand = self.parse_not()?;
            return Ok(Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(operand) });
        }
        self.parse_comparison()
    }

    /// 比較演算子（`==`, `!=`, `<`, `>`, `<=`, `>=`）をパースする。
    ///
    /// 連鎖比較（`a < b < c` 等）は非対応。1つの比較式のみを生成する。
    /// 比較演算子がない場合は `parse_bitor()` に委譲する。
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let left = self.parse_bitor()?;
        let span = self.current_span();
        // `is` / `is not` 型ガード: 右辺は型名（識別子または None キーワード）のみ受け付ける
        if *self.current() == Token::Is {
            self.advance();
            let type_name = self.expect_guard_type_name()?;
            return Ok(Expr::IsType { expr: Box::new(left), negated: false, type_name, span });
        }
        if *self.current() == Token::IsNot {
            self.advance();
            let type_name = self.expect_guard_type_name()?;
            return Ok(Expr::IsType { expr: Box::new(left), negated: true, type_name, span });
        }
        if *self.current() == Token::In {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp { op: BinOp::In, left: Box::new(left), right: Box::new(right), span });
        }
        if *self.current() == Token::NotIn {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp { op: BinOp::NotIn, left: Box::new(left), right: Box::new(right), span });
        }
        let op = match self.current() {
            Token::EqEq => Some(BinOp::Eq),
            Token::NotEq => Some(BinOp::NotEq),
            Token::Lt => Some(BinOp::Lt),
            Token::Gt => Some(BinOp::Gt),
            Token::LtEq => Some(BinOp::LtEq),
            Token::GtEq => Some(BinOp::GtEq),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span });
        }
        Ok(left)
    }

    /// ビット OR 演算子（`|`）を左結合でパースする。
    ///
    /// 優先順位: `|` < `^` < `&`。
    /// `parse_bitxor()` を下位に委譲する。
    fn parse_bitor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitxor()?;
        while *self.current() == Token::Pipe {
            let span = self.current_span();
            self.advance();
            let right = self.parse_bitxor()?;
            left = Expr::BinOp { op: BinOp::BitOr, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    /// ビット XOR 演算子（`^`）を左結合でパースする。
    ///
    /// `parse_bitand()` を下位に委譲する。
    fn parse_bitxor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitand()?;
        while *self.current() == Token::Caret {
            let span = self.current_span();
            self.advance();
            let right = self.parse_bitand()?;
            left = Expr::BinOp { op: BinOp::BitXor, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    /// ビット AND 演算子（`&`）を左結合でパースする。
    ///
    /// `parse_shift()` を下位に委譲する。
    fn parse_bitand(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_shift()?;
        while *self.current() == Token::Amp {
            let span = self.current_span();
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::BinOp { op: BinOp::BitAnd, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    /// シフト演算子（`<<`, `>>`）を左結合でパースする。
    ///
    /// `parse_additive()` を下位に委譲する。
    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            let span = self.current_span();
            let op = match self.current() {
                Token::LtLt => Some(BinOp::LShift),
                Token::GtGt => Some(BinOp::RShift),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_additive()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// 加算・減算演算子（`+`, `-`）を左結合でパースする。
    ///
    /// 文字列の `+` 連結もここで処理される。
    /// `parse_multiplicative()` を下位に委譲する。
    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let span = self.current_span();
            let op = match self.current() {
                Token::Plus => Some(BinOp::Add),
                Token::Minus => Some(BinOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_multiplicative()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// 乗算・除算・剰余演算子（`*`, `/`, `//`, `%`）を左結合でパースする。
    ///
    /// `parse_unary()` を下位に委譲する。
    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let span = self.current_span();
            let op = match self.current() {
                Token::Star => Some(BinOp::Mul),
                Token::Slash => Some(BinOp::Div),
                Token::SlashSlash => Some(BinOp::FloorDiv),
                Token::Percent => Some(BinOp::Mod),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_unary()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
            } else {
                break;
            }
        }
        Ok(left)
    }

    /// 単項演算子（`-`, `~`, 単項 `+`）をパースする。
    ///
    /// 再帰構造により `--x` のような連鎖も正しく処理する。
    /// 単項演算子でない場合は `parse_power()` に委譲する。
    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.current() {
            // 単項マイナス（算術否定）
            Token::Minus => {
                self.advance();
                Ok(Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(self.parse_unary()?) })
            }
            // ビット NOT（`~`）
            Token::Tilde => {
                self.advance();
                Ok(Expr::UnaryOp { op: UnaryOp::BitNot, operand: Box::new(self.parse_unary()?) })
            }
            // 単項プラス（`+x` は `x` と同義）
            Token::Plus => {
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    /// 冪乗演算子（`**`）を右結合でパースする。
    ///
    /// 右結合のため右辺は `parse_unary()` で再帰する（`parse_power()` でなく）。
    /// `**` がない場合は `parse_call()` の結果をそのまま返す。
    fn parse_power(&mut self) -> Result<Expr, String> {
        let base = self.parse_call()?;
        if *self.current() == Token::StarStar {
            let span = self.current_span();
            self.advance();
            // 右結合: 右辺は unary レベルから再帰する
            let exp = self.parse_unary()?;
            Ok(Expr::BinOp { op: BinOp::Pow, left: Box::new(base), right: Box::new(exp), span })
        } else {
            Ok(base)
        }
    }

    /// 後置演算子（関数呼び出し・属性アクセス・subscript・テンプレート呼び出し）をパースする。
    ///
    /// `parse_primary()` で基本式をパースした後、後続するトークンに応じてループする:
    /// - `(` → 関数呼び出し（位置引数・キーワード引数）
    /// - `.` → 属性アクセス（`Expr::Attr`）
    /// - `::` → トレイトアクセス（`Expr::TraitAccess`）
    /// - `[` → `parse_bracket_suffix()` に委譲（template or subscript）
    fn parse_call(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.current() {
                Token::LParen => {
                    let call_span = self.current_span();
                    self.advance(); // `(` を消費
                    let mut args = Vec::new();
                    // 引数リストをパース。キーワード引数（`name=expr`）と位置引数を区別する
                    while *self.current() != Token::RParen && *self.current() != Token::Eof {
                        // キーワード引数の判定: `Ident =`（`==` ではない）
                        let arg = if let Token::Ident(name) = self.current().clone() {
                            if *self.peek1() == Token::Eq {
                                let name = name.clone();
                                self.advance(); // Ident を消費
                                self.advance(); // `=` を消費
                                CallArg::Keyword { name, value: self.parse_expr()? }
                            } else {
                                // `==` 等の場合は位置引数として処理
                                CallArg::Positional(self.parse_expr()?)
                            }
                        } else {
                            CallArg::Positional(self.parse_expr()?)
                        };
                        args.push(arg);
                        if *self.current() == Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.eat(&Token::RParen)?;
                    expr = Expr::Call { func: Box::new(expr), args, span: call_span };
                }
                Token::Dot => {
                    let dot_span = self.current_span();
                    self.advance(); // `.` を消費
                    let attr = self.expect_attr_name()?;
                    expr = Expr::Attr { object: Box::new(expr), attr, span: dot_span };
                }
                Token::ColonColon => {
                    // `obj::TraitName.attr` 形式のトレイトアクセス
                    self.advance(); // `::` を消費
                    let trait_name = self.expect_ident()?;
                    self.eat(&Token::Dot)?;
                    let attr = self.expect_attr_name()?;
                    expr = Expr::TraitAccess { object: Box::new(expr), trait_name, attr };
                }
                Token::LBracket => {
                    // `[` の後がテンプレート呼び出しか subscript かを判定して処理
                    expr = self.parse_bracket_suffix(expr)?;
                }
                Token::FatArrow => {
                    // `expr => Type` — キャスト式。`=>` を消費して型名をパースする。
                    let span = self.current_span();
                    self.advance(); // consume `=>`
                    let type_name = self.parse_type_expr()?;
                    expr = Expr::Cast { object: Box::new(expr), type_name, span };
                    // ループを継続して `.method()` などをチェーン可能にする
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    /// `expr[...]` を `Expr::TemplateInstantiate` または `Expr::Subscript` としてパースする。
    ///
    /// `is_template_instantiation()` の先読み結果に基づいて分岐する:
    /// - `]` の直後に `(` がある → テンプレート呼び出し（`f[T](args)` 形式）
    /// - それ以外 → サブスクリプト（`obj[index]` 形式）
    ///
    /// # 引数
    /// - `expr`: `[...]` の左側にある式
    ///
    /// # 戻り値
    /// `Expr::TemplateInstantiate` または `Expr::Subscript`
    ///
    /// # エラー
    /// 型引数・インデックス式・`]` のパースに失敗した場合
    fn parse_bracket_suffix(&mut self, expr: Expr) -> Result<Expr, String> {
        if self.is_template_instantiation() {
            // テンプレート呼び出し: 型引数リストをパース
            self.advance(); // `[` を消費
            let mut type_args = Vec::new();
            while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                type_args.push(self.parse_type_expr()?);
                if *self.current() == Token::Comma { self.advance(); } else { break; }
            }
            self.eat(&Token::RBracket)?;
            Ok(Expr::TemplateInstantiate { base: Box::new(expr), type_args })
        } else {
            self.advance(); // `[` を消費

            // スライス構文の検出: `[:]`, `[:end]`, `[begin:]`, `[begin:end]`, `[::step]` 等
            // 注意: `::` は lexer が Token::ColonColon として 1 トークンにまとめる。
            let index = if *self.current() == Token::ColonColon {
                // `[::step]` または `[::]`
                self.advance(); // `::` を消費
                let step = self.parse_slice_part()?;
                Expr::Slice { begin: None, end: None, step }
            } else if *self.current() == Token::Colon {
                // `[:...]` — begin なし
                self.advance(); // `:` を消費
                let end = self.parse_slice_part()?;
                let step = self.parse_slice_step()?;
                Expr::Slice { begin: None, end, step }
            } else {
                // 先に式を1つ読む。その後 `:` / `::` が来ればスライス、なければ通常添字。
                let first = self.parse_expr()?;
                if *self.current() == Token::ColonColon {
                    // `[begin::step]` または `[begin::]`
                    self.advance(); // `::` を消費
                    let step = self.parse_slice_part()?;
                    Expr::Slice { begin: Some(Box::new(first)), end: None, step }
                } else if *self.current() == Token::Colon {
                    self.advance(); // `:` を消費
                    let end = self.parse_slice_part()?;
                    let step = self.parse_slice_step()?;
                    Expr::Slice { begin: Some(Box::new(first)), end, step }
                } else {
                    first // 通常添字（スライスなし）
                }
            };

            self.eat(&Token::RBracket)?;
            Ok(Expr::Subscript { object: Box::new(expr), index: Box::new(index) })
        }
    }

    /// スライスの begin/end/step 部分（省略可能な式）をパースする。
    /// `]`, `:`, `::` または EOF が来た場合は `None` を返す。
    fn parse_slice_part(&mut self) -> Result<Option<Box<Expr>>, String> {
        if matches!(*self.current(), Token::RBracket | Token::Colon | Token::ColonColon | Token::Eof) {
            Ok(None)
        } else {
            Ok(Some(Box::new(self.parse_expr()?)))
        }
    }

    /// end の後の step 部分をパースする（`:step` または `::step` に対応）。
    fn parse_slice_step(&mut self) -> Result<Option<Box<Expr>>, String> {
        if *self.current() == Token::Colon {
            self.advance(); // `:` を消費
            self.parse_slice_part()
        } else if *self.current() == Token::ColonColon {
            // `end::step` — end の直後に `::` (ありえないが念のため対応)
            self.advance();
            self.parse_slice_part()
        } else {
            Ok(None)
        }
    }

    /// f-string パーツ列を文字列連結式（`BinOp::Add`）にデシュガーする。
    ///
    /// `f"Hello {name}!"` → `"Hello " + str(name) + "!"`
    fn desugar_fstring(&mut self, parts: Vec<FStrPart>) -> Result<Expr, String> {
        if parts.is_empty() {
            return Ok(Expr::Str(String::new()));
        }
        let span = Span::unknown();
        let mut exprs: Vec<Expr> = Vec::new();
        for part in parts {
            match part {
                FStrPart::Lit(s) => exprs.push(Expr::Str(s)),
                FStrPart::Expr(src) => {
                    // Re-lex and re-parse the expression source
                    let tokens = lexer::Lexer::new(&src, "<fstring>").tokenize();
                    let mut sub_parser = Parser::new(tokens, None);
                    let expr = sub_parser.parse_expr()?;
                    // Wrap in str() call to convert to string
                    exprs.push(Expr::Call {
                        func: Box::new(Expr::Ident("str".to_string())),
                        args: vec![crate::ast::CallArg::Positional(expr)],
                        span: span.clone(),
                    });
                }
            }
        }
        // Fold into left-associative BinOp::Add chain
        let mut iter = exprs.into_iter();
        let first = iter.next().unwrap();
        let result = iter.fold(first, |acc, e| Expr::BinOp {
            op: BinOp::Add,
            left: Box::new(acc),
            right: Box::new(e),
            span: span.clone(),
        });
        Ok(result)
    }

    /// 基本式（リテラル・識別子・括弧式・リスト・辞書）をパースする。
    ///
    /// 先頭トークンに応じて以下を生成する:
    /// - 整数・浮動小数点数・文字列・真偽値リテラル
    /// - `None`, `Any`, `Union`, `Option` 識別子
    /// - 一般識別子（`Expr::Ident`）
    /// - `Self`（クラス・トレイト内のみ有効）
    /// - `(...)` → `parse_paren_expr()`（グループ式またはタプル）
    /// - `[...]` → `parse_list_literal()`
    /// - `{...}` → `parse_dict_literal()`
    ///
    /// # エラー
    /// 未対応トークンが現れた場合、または `Self` をクラス外で使用した場合
    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.current().clone() {
            Token::Int(n)   => { self.advance(); Ok(Expr::Int(n)) }
            Token::Float(f) => { self.advance(); Ok(Expr::Float(f)) }
            Token::Str(s)   => { self.advance(); Ok(Expr::Str(s)) }
            Token::FStr(parts) => {
                self.advance();
                Ok(self.desugar_fstring(parts)?)
            }
            Token::True     => { self.advance(); Ok(Expr::Bool(true)) }
            Token::False    => { self.advance(); Ok(Expr::Bool(false)) }
            Token::None     => { self.advance(); Ok(Expr::None) }
            Token::Any      => { self.advance(); Ok(Expr::Ident("Any".to_string())) }
            Token::Union    => { self.advance(); Ok(Expr::Ident("Union".to_string())) }
            Token::Option   => { self.advance(); Ok(Expr::Ident("Option".to_string())) }
            Token::Ident(name) if name == "dbg" => {
                self.advance();
                if *self.current() == Token::ColonColon {
                    self.advance(); // consume ::
                    let dbg_name = self.expect_ident()?;
                    Ok(Expr::DebugVar(dbg_name))
                } else {
                    Err("ParseError: 'dbg' is a reserved name for the debugger namespace (use dbg::varname)".to_string())
                }
            }
            Token::Ident(name) => { self.advance(); Ok(Expr::Ident(name)) }
            Token::SelfType => {
                if self.class_or_trait_depth == 0 {
                    return Err("ParseError: 'Self' can only be used inside class or trait definitions".to_string());
                }
                self.advance();
                Ok(Expr::Ident("Self".to_string()))
            }
            Token::LParen   => self.parse_paren_expr(),
            Token::LBracket => self.parse_list_literal(),
            Token::LBrace   => self.parse_dict_or_set_literal(),
            // `if cond [->Type]: body [elif/else]` — if 式
            Token::If => {
                self.advance();
                let (branches, else_body, return_type) = self.parse_if_components()?;
                Ok(Expr::IfExpr { branches, else_body, return_type })
            }
            // `for target in iter [->Type]: body` — for 式
            Token::For => {
                self.advance();
                let target = self.expect_ident()?;
                self.eat(&Token::In)?;
                let iter = self.parse_expr()?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::ForExpr { target, iter: Box::new(iter), body: self.parse_block()?, return_type })
            }
            // `while cond [->Type]: body` — while 式
            Token::While => {
                self.advance();
                let cond = self.parse_expr()?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::WhileExpr { cond: Box::new(cond), body: self.parse_block()?, return_type })
            }
            // `match subject [->Type]: arms` — match 式
            Token::Match => {
                self.advance();
                let subject = self.parse_expr()?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                self.eat(&Token::Newline)?;
                self.eat(&Token::Indent)?;
                let arms = self.parse_match_arms()?;
                Ok(Expr::MatchExpr { subject: Box::new(subject), arms, return_type })
            }
            // `block [->Type]: body` — ブロック式。block_return/block_yield で値を返す。
            Token::Block => {
                self.advance(); // `block` を消費
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::Block { stmts: self.parse_block()?, return_type })
            }
            tok => Err(format!("unexpected token: `{tok}`")),
        }
    }

    /// `(...)` をパースし、グループ式またはタプルを返す。
    ///
    /// 判定ルール:
    /// - `()` → 空タプル（`Expr::Tuple([])`）
    /// - `(expr)` → グループ式（タプルではない。`expr` をそのまま返す）
    /// - `(expr,)` → 単要素タプル（末尾カンマ必須）
    /// - `(expr, expr, ...)` → 多要素タプル
    ///
    /// 括弧内では改行がスキップされるため、多行タプルリテラルも有効。
    ///
    /// # 戻り値
    /// グループ式の場合はその式、タプルの場合は `Expr::Tuple(items)`
    ///
    /// # エラー
    /// 内部式または `)` のパースに失敗した場合
    fn parse_paren_expr(&mut self) -> Result<Expr, String> {
        self.advance(); // `(` を消費
        // 空タプル `()` を処理
        if *self.current() == Token::RParen {
            self.advance();
            return Ok(Expr::Tuple(vec![]));
        }
        let first = self.parse_expr()?;
        // `)` が続けばグループ式（タプルではない）
        if *self.current() == Token::RParen {
            self.advance();
            return Ok(first);
        }
        // `,` が続けば単要素タプルまたは多要素タプル
        self.eat(&Token::Comma)?;
        let mut items = vec![first];
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            items.push(self.parse_expr()?);
            // 末尾カンマがあればスキップ
            if *self.current() == Token::Comma { self.advance(); } else { break; }
        }
        self.eat(&Token::RParen)?;
        Ok(Expr::Tuple(items))
    }

    /// `[expr, ...]` リストリテラルをパースして `Expr::List` を返す。
    ///
    /// 空リスト `[]` も有効。
    ///
    /// # エラー
    /// 内部式または `]` のパースに失敗した場合
    fn parse_list_literal(&mut self) -> Result<Expr, String> {
        self.advance(); // consume `[`
        let mut items = Vec::new();
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            items.push(self.parse_expr()?);
            if *self.current() == Token::Comma { self.advance(); } else { break; }
        }
        self.eat(&Token::RBracket)?;
        Ok(Expr::List(items))
    }

    /// `{key: value, ...}` 辞書リテラルをパースして `Expr::Dict` を返す。
    /// `{...}` をパースして辞書またはセットリテラルを返す。
    ///
    /// - `{}` → 空辞書 `Expr::Dict([])`
    /// - `{key: val, ...}` → 辞書 `Expr::Dict([...])`
    /// - `{val, val, ...}` → セット `Expr::Set([...])`
    ///
    /// 最初の式の後に `:` が続けば辞書、`,` または `}` が続けばセットとして扱う。
    /// 括弧内では改行がスキップされるため、多行リテラルも有効。
    ///
    /// # エラー
    /// 式または `}` のパースに失敗した場合
    fn parse_dict_or_set_literal(&mut self) -> Result<Expr, String> {
        self.advance(); // consume `{`
        // Empty braces → empty dict
        if *self.current() == Token::RBrace {
            self.advance();
            return Ok(Expr::Dict(vec![]));
        }
        // Parse first expression to determine dict vs set
        let first = self.parse_expr()?;
        if *self.current() == Token::Colon {
            // Dict path
            self.advance(); // consume `:`
            let val = self.parse_expr()?;
            let mut pairs = vec![(first, val)];
            while *self.current() == Token::Comma {
                self.advance();
                if *self.current() == Token::RBrace { break; }
                let key = self.parse_expr()?;
                self.eat(&Token::Colon)?;
                let val = self.parse_expr()?;
                pairs.push((key, val));
            }
            self.eat(&Token::RBrace)?;
            Ok(Expr::Dict(pairs))
        } else {
            // Set path
            let mut items = vec![first];
            while *self.current() == Token::Comma {
                self.advance();
                if *self.current() == Token::RBrace { break; }
                items.push(self.parse_expr()?);
            }
            self.eat(&Token::RBrace)?;
            Ok(Expr::Set(items))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(src, "").tokenize();
        Parser::new(tokens, None).parse_program().expect("parse error")
    }

    fn parse_fails(src: &str) -> String {
        let tokens = Lexer::new(src, "").tokenize();
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
        // type annotations on params and return type are parsed without error
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
        // Class-to-class inheritance is not allowed; only traits may be listed as bases.
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
        // Fields WITH defaults don't produce an auto-init; no auto-init here.
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n    fn move(mut self, dx: int, dy: int) -> None:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            // 2 fields + 1 method (no auto-init: both fields have defaults)
            assert_eq!(body.len(), 3);
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_field_parsed_as_field_stmt() {
        // Field declarations produce Stmt::Field nodes with type annotation.
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
        // Auto __init__ is generated for mut/let fields WITHOUT a default value.
        let src = "class Point:\n    mut x: int\n    mut y: int\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
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
        // No auto-init when all fields have default values.
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when all fields have defaults");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_generated_with_list_field() {
        // Fields without defaults always trigger auto-init regardless of type.
        let src = "class Foo:\n    mut items: list[int]\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
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
        // Explicit __init__ with same types/count suppresses auto-init (override).
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ overrides auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_overload_different_sig() {
        // Explicit __init__ with different count coexists as overload.
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int, y: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 2, "different-sig explicit __init__ + auto-init both present");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_not_generated_without_required_fields() {
        // No auto __init__ when class has no fields.
        let src = "class Foo:\n    fn greet(self) -> str:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when there are no required fields");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_field_requires_type_annotation() {
        // Field declarations without `: Type` must produce a parse error.
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
        // TraitDef then ClassDef — combined __init__ takes trait fields first, class fields second.
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "combined __init__ should be generated");
            if let Some(Stmt::FnDef { params, return_type, .. }) = init {
                // self + x (from trait HasX) + y (from class Point)
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
                body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
            {
                // First assignment must be TraitAccess (trait field x)
                assert!(matches!(&init_body[0],
                    Stmt::AttrAssign { target: Expr::TraitAccess { trait_name, attr, .. }, .. }
                    if trait_name == "HasX" && attr == "x"
                ), "trait field assignment should use TraitAccess");
                // Second assignment must be regular Attr (class field y)
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
        // self::MyTrait.field — 括弧なし形式のみ有効
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
        // Explicit __init__ with same sig as combined auto-init suppresses generation.
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
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ should override auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_with_multiple_virtual_methods_all_must_be_overridden() {
        // All virtual methods must be overridden.
        let result = std::panic::catch_unwind(|| parse(concat!(
            "trait Ops:\n",
            "    fn add(self, x: int) -> int:\n",
            "        ...\n",
            "    fn sub(self, x: int) -> int:\n",
            "        ...\n",
            "class MyOps(Ops):\n",
            "    fn add(self, x: int) -> int:\n",    // overrides add
            "        pass\n",
            // sub NOT overridden → error
        )));
        assert!(result.is_err(), "not overriding all virtual methods should be a parse error");
    }

    #[test]
    fn test_trait_class_only_trait_required_fields_no_class_fields() {
        // Class has no own required fields; combined init takes only trait fields.
        let stmts = parse(concat!(
            "trait Named:\n",
            "    mut name: str\n",
            "class Widget(Named):\n",
            "    pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
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
