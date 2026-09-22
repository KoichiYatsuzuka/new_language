// stmts/metafn.rs — メタ関数（コンパイル時展開）の構文解析。
//
// 対象は次の 3 つ。設計は implementation_plans/comptime_metafn_design.md §1。
//
//   exprconst fn  名前(仮引数...) -> Code:   … 純粋メタ関数（`return` で `Code` を返す）
//   exprconst !fn 名前(仮引数...) -> None:   … 配置メタ関数（`quote` で配置する）
//   code:                                     … `Code` 値を作るブロック
//   quote <Code 型の式>                       … `Code` を AST にして呼び出し位置へ配置
//
// ⚠⚠ **`code:` の中身はパースしない。** `code: if cond:` のように**本体の無い `if`** を
// 書けることが要件（未完の断片を組み立てて最後に一度だけパースする）。
// ⇒ 字句だけして行に切り、`CodeLine { pieces, indent }` として持つ。
// パースは `quote` の 1 回だけ（Phase 2 の展開器が行う）。
//
// ⚠ **例外が `<! ... !>`**（タスク 1-2 / 設計書 §1.4）。中身は地の文ではなく
// **展開時に評価される普通の式**なので、ここだけはその場でパースする。
// 素通しにすると展開器が組み立て後に `<!` を探すことになるが、`!>` は文字列
// リテラルの中にも書けるので、**境界を正しく決められるのはパーサだけ**。

use {
    crate::ast::{CodeLine, CodePiece, Param, Stmt},
    crate::parser::Parser,
    crate::token::Token,
};

impl Parser {
    /// `exprconst fn` / `exprconst !fn` をパースして [`Stmt::MetaFnDef`] を返す。
    ///
    /// ⚠ `!fn` の `!` は [`Token::Bang`]。字句側で `!=` / `!>` を先に判定しているので、
    /// ここへ来る `Bang` は単独の `!` だけ（設計書 §1.9）。
    pub(crate) fn parse_meta_fn_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `exprconst` を消費

        // `!fn`（配置メタ関数）か `fn`（純粋メタ関数）か。
        let is_placing = if *self.current() == Token::Bang {
            self.advance(); // `!` を消費
            true
        } else {
            false
        };
        if *self.current() != Token::Fn {
            return Err(format!(
                "`exprconst` must be followed by `fn` or `!fn`, got `{}`",
                self.current()
            ));
        }
        self.advance(); // `fn` を消費

        let name = self.expect_ident()?;
        // ⚠ `note_def` は `expect_ident` の直後に呼ぶ（位置は `prev_pos()` 由来なので、
        //   型注釈を先に読むとシンボルが別のトークンを指す）。
        let decl_h = self.note_def(&name, crate::parser::editor_hooks::EditorKind::Function);

        // ⚠ メタ関数にテンプレート型パラメータは持たせない。展開はテンプレート実体化より
        //   前に走る（設計書 §1.7）ので、型変数を置く意味が無い。
        if *self.current() == Token::LBracket {
            return Err(format!(
                "metafunction `{name}` cannot have template parameters \
                 (expansion runs before template instantiation)"
            ));
        }

        let params = self.parse_param_list()?;
        let return_type = if *self.current() == Token::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.note_type_ann(decl_h, return_type.as_deref());
        self.note_signature(decl_h, &name);

        self.eat(&Token::Colon)?;
        // ⚠ 本体の中では `^`（メタ情報演算子）を書ける（設計書 §1.5）。
        self.metafn_depth += 1;
        let body = self.parse_block();
        self.metafn_depth -= 1;
        let body = body?;

        Ok(Stmt::MetaFnDef { name, params, return_type, body, is_placing })
    }

    /// `quote <式>` をパースして [`Stmt::Quote`] を返す。
    ///
    /// ⚠ 後ろに来るのは **`Code` 型の式**。型の検査は 1-5 で入れる。
    pub(crate) fn parse_quote(&mut self) -> Result<Stmt, String> {
        self.advance(); // `quote` を消費
        if matches!(
            self.current(),
            Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent
        ) {
            return Err("`quote` requires a `Code` value to place".to_string());
        }
        Ok(Stmt::Quote(self.parse_expr()?))
    }

    /// `code:` ブロックをパースして行のリストを返す。
    ///
    /// ⚠⚠ **中身をパースしない**のがこの関数の要点。`Indent` / `Dedent` / `Newline` で
    /// 行とインデント段数だけを取り出し、それ以外のトークンはそのまま行に積む。
    ///
    /// ⚠ **空ブロックを許す**（設計書 §1.2）。`code:` の直後にインデントが来なければ
    /// 「空の `Code`」になる。`parse_block` は `Indent` を必須にしているので流用できない。
    ///
    /// ⚠ **入れ子は禁止**（設計書 §1.2）。中に `code` が現れたらエラーにする。
    /// 素通しすると「中身は評価されない」規則と相まって、書いた人の意図と無関係に
    /// ただのテキストとして埋まる。
    pub(crate) fn parse_code_block(&mut self) -> Result<Vec<CodeLine>, String> {
        self.advance(); // `code` を消費
        self.eat(&Token::Colon)?;

        // 改行が無い（`code:` の後に同じ行で何か書いてある）のは受け付けない。
        if *self.current() != Token::Newline {
            return Err(format!(
                "`code:` must be followed by a newline, got `{}`",
                self.current()
            ));
        }
        self.advance(); // `Newline` を消費
        // 空行は読み飛ばす。ここで `Indent` が来なければ**空の Code**。
        while *self.current() == Token::Newline {
            self.advance();
        }
        if *self.current() != Token::Indent {
            return Ok(Vec::new());
        }
        self.advance(); // 最初の `Indent` を消費

        let mut lines: Vec<CodeLine> = Vec::new();
        let mut indent: i32 = 0;
        let mut current: Vec<CodePiece> = Vec::new();

        loop {
            match self.current() {
                Token::Eof => {
                    return Err("unterminated `code:` block".to_string());
                }
                Token::Code => {
                    return Err(
                        "`code:` blocks cannot be nested (the inner block would \
                         be emitted as plain text, not evaluated)"
                            .to_string(),
                    );
                }
                // `<! 式 !>` — `code:` の中で唯一「中身をパースする」場所（設計書 §1.4）。
                Token::SpliceOpen => current.push(CodePiece::Splice(self.parse_splice()?)),
                // 対応する `<!` の無い `!>`。素通しすると地の文に紛れて消えるので弾く。
                Token::SpliceClose => {
                    return Err(
                        "`!>` has no matching `<!` in this `code:` block".to_string(),
                    );
                }
                Token::Indent => {
                    indent += 1;
                    self.advance();
                }
                Token::Dedent => {
                    if indent == 0 {
                        // 最初の `Indent` に対応する `Dedent` ＝ ブロックの終わり。
                        self.advance();
                        break;
                    }
                    indent -= 1;
                    self.advance();
                }
                Token::Newline | Token::Semicolon => {
                    if !current.is_empty() {
                        lines.push(CodeLine { pieces: std::mem::take(&mut current), indent });
                    }
                    self.advance();
                }
                _ => {
                    current.push(CodePiece::Token(self.spanned_at_pos()));
                    self.advance();
                }
            }
        }
        if !current.is_empty() {
            lines.push(CodeLine { pieces: current, indent });
        }
        Ok(lines)
    }

    /// `<! 式 !>` の中身をパースする（設計書 §1.4）。`<!` の位置で呼ぶ。
    ///
    /// ⚠ **空のスプライス `<!!>` は弾く。** 「何も差し込まない」を書きたいなら空文字列を
    /// 渡せばよく、空の `<! !>` は書き損じである方が圧倒的に多い。
    ///
    /// ⚠ 中身は**1 行に収まる値だけ**（`Code` は入れられない・§1.4）。
    /// その検査は型検査（1-5）の仕事で、ここでは構文だけを見る。
    fn parse_splice(&mut self) -> Result<crate::ast::Expr, String> {
        self.advance(); // `<!` を消費
        if *self.current() == Token::SpliceClose {
            return Err(
                "`<! !>` must contain an expression \
                 (it is spliced into the surrounding `code:` line)"
                    .to_string(),
            );
        }
        let expr = self.parse_expr()?;
        if *self.current() != Token::SpliceClose {
            return Err(format!(
                "expected `!>` to close the splice, got `{}`",
                self.current()
            ));
        }
        self.advance(); // `!>` を消費
        Ok(expr)
    }
}

impl Parser {
    /// `!装飾子名` の並び + 装飾される宣言をパースして [`Stmt::MetaDecorated`] を返す。
    /// `!` の位置で呼ぶ（設計書 §1.6 / タスク 1-4）。
    ///
    /// `in_class` が真ならクラス本体の中（対象はフィールド・メソッド）。
    ///
    /// ⚠ `@` の [`Parser::parse_decorators`] と**別実装にしてある**。`@` は実行時クロージャ
    /// 装飾子として据え置きと決まっており（§1.6）、受け付ける対象も `fn` / `class` だけで違う。
    ///
    /// ⚠ **対象の種類をここで検査する。** 通さないと `!deco import x` のような書き方が
    /// 黙って `MetaDecorated(Import)` になり、展開器が扱えない形が AST に入る。
    pub(crate) fn parse_meta_decorated(&mut self, in_class: bool) -> Result<Stmt, String> {
        let mut decorators = Vec::new();
        while *self.current() == Token::Bang {
            self.advance(); // `!` を消費
            decorators.push(self.parse_expr()?);
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
        }

        let target = if in_class { self.parse_class_stmt()? } else { self.parse_stmt()? };
        if !is_decoratable(&target) {
            return Err(format!(
                "`!` decorators can only be applied to a class, function, field or \
                 variable binding, got `{}`",
                crate::interpreter::tw_stats::stmt_kind_of(&target)
            ));
        }
        Ok(Stmt::MetaDecorated { decorators, target: Box::new(target) })
    }
}

/// 装飾子を付けられる宣言かどうか（設計書 §1.6）。
///
/// ⚠ **網羅 match にしない**（`Stmt` は 40 種類以上あり、ほとんどが対象外）。
/// 代わりに**受け付けるものを列挙**する。増やすときはここ 1 箇所。
fn is_decoratable(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::ClassDef { .. }
            | Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::Field { .. }
            | Stmt::Let(..)
            | Stmt::Mut(..)
            | Stmt::Const(..)
            | Stmt::Static { .. }
            // 入れ子の装飾子（`!a` の対象がさらに装飾された宣言）も通す。
            | Stmt::MetaDecorated { .. }
    )
}

/// 仮引数リストのパース補助。`parse_fn_def_with_flags` と同じ形（`parse_param` のループ）。
impl Parser {
    fn parse_param_list(&mut self) -> Result<Vec<Param>, String> {
        self.eat(&Token::LParen)?;
        let mut params: Vec<Param> = Vec::new();
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            params.push(self.parse_param()?);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RParen)?;
        Ok(params)
    }
}
