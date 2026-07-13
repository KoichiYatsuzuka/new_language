// stmts/definitions.rs — try / enum / new_type 定義の解析。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::{Span, Spanned, Token},
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

    /// `alias 名前: 右辺` 定義をパースする。右辺は任意の式・型・テンプレート適用・
    /// `block` 式などを取れる。別名は純粋なコンパイル時 AST 置換として振る舞い、
    /// 参照箇所ごとに右辺がそのまま差し込まれる（`new_type` の準サブタイプと異なり完全等価）。
    ///
    /// 右辺を1回だけ式としてパースして AST を得ると同時に、その生トークン列を保存する。
    /// 式位置では保存した AST を、型注釈位置では保存トークンを型として再パースして展開する
    /// （`parse_primary` / `parse_type_expr` を参照）。
    ///
    /// 別名定義自体は AST 上に残さず `Stmt::Pass`（no-op）を返す。展開はすべてパース時に完了する。
    ///
    /// # 戻り値
    /// `Stmt::Pass`（別名は `self.aliases` に登録され、以降の参照で展開される）
    ///
    /// # エラー
    /// 同名の別名が既に可視スコープに存在する場合、または右辺のパースに失敗した場合
    ///
    /// # 使用例
    /// ```text
    /// alias object_handle: int                 # 型としても式（コンストラクタ）としても使える
    /// alias item: data_dict["often_used_key"]  # lvalue 透過: item = 5 → data_dict[...] = 5
    /// ```
    pub(crate) fn parse_alias_def(&mut self) -> Result<Stmt, String> {
        let def_span = self.current_span();
        self.advance(); // `alias` を消費
        let name = self.expect_ident()?;
        // 同一可視スコープでの再定義・シャドウは禁止（予期しない置換を避ける）。
        if self.aliases.contains_key(&name) {
            return Err(format!("alias `{name}` is already defined in this scope"));
        }
        self.eat(&Token::Colon)?;
        // 右辺の生トークン範囲を記録しつつ、式として1回パースする。
        // `block` 式など複数行に及ぶ右辺も parse_expr が正しく1つの式として消費する。
        let start = self.pos;
        let expr = self.parse_alias_rhs()?;
        let end = self.pos;
        // 型位置での再パース用に生トークンを複製し、末尾に Eof 番兵を付す。
        let mut tokens: Vec<Spanned> = self.tokens[start..end].to_vec();
        tokens.push(Spanned { token: Token::Eof, span: def_span });
        self.aliases.insert(
            name,
            crate::parser::AliasEntry {
                expr: std::rc::Rc::new(expr),
                tokens: std::rc::Rc::new(tokens),
            },
        );
        Ok(Stmt::Pass)
    }

    /// alias の右辺式をパースする。
    ///
    /// 既知テンプレート名に続く `[...]`（例: `Box[MyInt]`）はテンプレート具体化
    /// （`Expr::TemplateInstantiate`）として解釈する。これにより後続の使用箇所で
    /// `AliasName(args)` が `Box[MyInt](args)` として正しくコンストラクタ呼び出しになる
    /// （標準の subscript 判定は末尾の `(` に依存するため、単独の `Box[MyInt]` は
    /// そのままでは subscript と解釈されてしまう）。
    /// それ以外は通常の式としてパースする（添字・block 式・任意の式を許容）。
    fn parse_alias_rhs(&mut self) -> Result<Expr, String> {
        if matches!(self.current(), Token::Ident(n) if self.known_templates.contains(n))
            && *self.peek1() == Token::LBracket
        {
            let base = self.expect_ident()?;
            let type_args = self.parse_type_args()?;
            return Ok(Expr::TemplateInstantiate {
                base: Box::new(Expr::Ident(base)),
                type_args,
            });
        }
        self.parse_expr()
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
