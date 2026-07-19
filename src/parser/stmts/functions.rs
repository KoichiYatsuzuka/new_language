// stmts/functions.rs — 関数/ジェネレータ定義の解析: デコレータ / fn / gen / 抽象ボディ・return 検査。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::{Span, Token},
};
#[allow(unused_imports)]
use super::*;

impl Parser {
    /// `@decorator` 構文のリストをパースして式のリストを返す。
    ///
    /// 現在のトークンが `@` の間、以下を繰り返す:
    /// 1. `@` を消費する
    /// 2. デコレータ式をパース（識別子・属性アクセス・関数呼び出し可）
    /// 3. 末尾の改行を消費する
    ///
    /// 戻り値: `Vec<Expr>` — 上から順に並んだデコレータ式のリスト
    pub(crate) fn parse_decorators(&mut self) -> Result<Vec<Expr>, String> {
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
    pub(crate) fn parse_fn_def(&mut self) -> Result<Stmt, String> {
        self.parse_fn_def_with_flags(vec![], false, false)
    }

    /// デコレータ付き `fn` 関数定義をパースして `Stmt::FnDef` を返す。
    ///
    /// `@decorator` の後に `fn` が続く場合に呼ばれる。
    pub(crate) fn parse_fn_def_decorated(
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
    pub(crate) fn parse_fn_def_with_flags(
        &mut self,
        decorators: Vec<Expr>,
        is_static: bool,
        is_class_method: bool,
    ) -> Result<Stmt, String> {
        self.advance(); // `fn` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータ `[T: Trait, ...]` をパース（なければ空 Vec）
        let template_params = self.parse_template_params()?;
        // テンプレート関数は alias RHS の `base[Args]` をテンプレート具体化として解釈するため記録する。
        if !template_params.is_empty() {
            self.known_templates.insert(name.clone());
        }
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
    pub(crate) fn parse_gen_def(&mut self) -> Result<Stmt, String> {
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
    pub(crate) fn body_has_return(stmts: &[Stmt]) -> bool {
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
                    if else_body.as_deref().is_some_and(Self::body_has_return) {
                        return true;
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body)
                    if Self::body_has_return(body) => {
                        return true;
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
    pub(crate) fn is_abstract_body(&self) -> bool {
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

}
