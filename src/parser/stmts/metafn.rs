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
// ⇒ 字句だけして行に切り、`CodeLine { tokens, indent }` として持つ。
// パースは `quote` の 1 回だけ（Phase 2 の展開器が行う）。

use {
    crate::ast::{CodeLine, Param, Stmt},
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
                "ParseError: `exprconst` must be followed by `fn` or `!fn`, got `{}`",
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
                "ParseError: metafunction `{name}` cannot have template parameters \
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
        let body = self.parse_block()?;

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
            return Err("ParseError: `quote` requires a `Code` value to place".to_string());
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
                "ParseError: `code:` must be followed by a newline, got `{}`",
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
        let mut current: Vec<crate::token::Spanned> = Vec::new();

        loop {
            match self.current() {
                Token::Eof => {
                    return Err("ParseError: unterminated `code:` block".to_string());
                }
                Token::Code => {
                    return Err(
                        "ParseError: `code:` blocks cannot be nested (the inner block would \
                         be emitted as plain text, not evaluated)"
                            .to_string(),
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
                        lines.push(CodeLine { tokens: std::mem::take(&mut current), indent });
                    }
                    self.advance();
                }
                _ => {
                    current.push(self.spanned_at_pos());
                    self.advance();
                }
            }
        }
        if !current.is_empty() {
            lines.push(CodeLine { tokens: current, indent });
        }
        Ok(lines)
    }
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
