// exprs.rs — expression parsing (precedence chain, literals, subscript, f-strings).

use super::Parser;
use crate::ast::{BinOp, CallArg, Expr, UnaryOp};
use crate::token::{FStrPart, Span, Token};
use crate::lexer;

impl Parser {
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
            left = Expr::BinOp {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
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
            left = Expr::BinOp {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
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
            return Ok(Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
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
            return Ok(Expr::IsType {
                expr: Box::new(left),
                negated: false,
                type_name,
                span,
            });
        }
        if *self.current() == Token::IsNot {
            self.advance();
            let type_name = self.expect_guard_type_name()?;
            return Ok(Expr::IsType {
                expr: Box::new(left),
                negated: true,
                type_name,
                span,
            });
        }
        if *self.current() == Token::MustBe {
            self.advance();
            let guard_type = self.parse_mustbe_type()?;
            return Ok(Expr::MustBe {
                expr: Box::new(left),
                guard_type,
                span,
            });
        }
        if *self.current() == Token::In {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp {
                op: BinOp::In,
                left: Box::new(left),
                right: Box::new(right),
                span,
            });
        }
        if *self.current() == Token::NotIn {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp {
                op: BinOp::NotIn,
                left: Box::new(left),
                right: Box::new(right),
                span,
            });
        }
        let op = match self.current() {
            Token::EqEq => Some(BinOp::Eq),
            Token::EqEqEq => Some(BinOp::RefEq),
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
            return Ok(Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            });
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
            left = Expr::BinOp {
                op: BinOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
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
            left = Expr::BinOp {
                op: BinOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
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
            left = Expr::BinOp {
                op: BinOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
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
                left = Expr::BinOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                };
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
                left = Expr::BinOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                };
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
                left = Expr::BinOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                };
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
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(self.parse_unary()?),
                })
            }
            // ビット NOT（`~`）
            Token::Tilde => {
                self.advance();
                Ok(Expr::UnaryOp {
                    op: UnaryOp::BitNot,
                    operand: Box::new(self.parse_unary()?),
                })
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
            Ok(Expr::BinOp {
                op: BinOp::Pow,
                left: Box::new(base),
                right: Box::new(exp),
                span,
            })
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
                        // 可変長引数: `... = A, B, C` — 最後の引数としてのみ使用可能
                        if *self.current() == Token::Ellipsis && *self.peek1() == Token::Eq {
                            self.advance(); // consume `...`
                            self.advance(); // consume `=`
                            let mut variadic_exprs = Vec::new();
                            while *self.current() != Token::RParen && *self.current() != Token::Eof {
                                variadic_exprs.push(self.parse_expr()?);
                                if *self.current() == Token::Comma {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            if variadic_exprs.is_empty() {
                                return Err("ParseError: variadic argument `... = ...` requires at least one expression".to_string());
                            }
                            args.push(CallArg::Variadic(variadic_exprs));
                            break; // variadic は最後の引数
                        }
                        // キーワード引数の判定: `Ident =`（`==` ではない）
                        let arg = if let Token::Ident(name) = self.current().clone() {
                            if *self.peek1() == Token::Eq {
                                let name = name.clone();
                                self.advance(); // Ident を消費
                                self.advance(); // `=` を消費
                                CallArg::Keyword {
                                    name,
                                    value: self.parse_expr()?,
                                }
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
                    expr = Expr::Call {
                        func: Box::new(expr),
                        args,
                        span: call_span,
                        cache: Default::default(),
                    };
                }
                Token::Dot => {
                    let dot_span = self.current_span();
                    self.advance(); // `.` を消費
                    let attr = self.expect_attr_name()?;
                    expr = Expr::Attr {
                        object: Box::new(expr),
                        attr,
                        span: dot_span,
                        cache: Default::default(),
                    };
                }
                Token::ColonColon => {
                    // `obj::TraitName.attr` 形式のトレイトアクセス
                    self.advance(); // `::` を消費
                    let trait_name = self.expect_ident()?;
                    self.eat(&Token::Dot)?;
                    let attr = self.expect_attr_name()?;
                    expr = Expr::TraitAccess {
                        object: Box::new(expr),
                        trait_name,
                        attr,
                    };
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
                    expr = Expr::Cast {
                        object: Box::new(expr),
                        type_name,
                        span,
                    };
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
                if *self.current() == Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            self.eat(&Token::RBracket)?;
            Ok(Expr::TemplateInstantiate {
                base: Box::new(expr),
                type_args,
            })
        } else {
            self.advance(); // `[` を消費

            // スライス構文の検出: `[:]`, `[:end]`, `[begin:]`, `[begin:end]`, `[::step]` 等
            // 注意: `::` は lexer が Token::ColonColon として 1 トークンにまとめる。
            let index = if *self.current() == Token::ColonColon {
                // `[::step]` または `[::]`
                self.advance(); // `::` を消費
                let step = self.parse_slice_part()?;
                Expr::Slice {
                    begin: None,
                    end: None,
                    step,
                }
            } else if *self.current() == Token::Colon {
                // `[:...]` — begin なし
                self.advance(); // `:` を消費
                let end = self.parse_slice_part()?;
                let step = self.parse_slice_step()?;
                Expr::Slice {
                    begin: None,
                    end,
                    step,
                }
            } else {
                // 先に式を1つ読む。その後 `:` / `::` が来ればスライス、なければ通常添字。
                let first = self.parse_expr()?;
                if *self.current() == Token::ColonColon {
                    // `[begin::step]` または `[begin::]`
                    self.advance(); // `::` を消費
                    let step = self.parse_slice_part()?;
                    Expr::Slice {
                        begin: Some(Box::new(first)),
                        end: None,
                        step,
                    }
                } else if *self.current() == Token::Colon {
                    self.advance(); // `:` を消費
                    let end = self.parse_slice_part()?;
                    let step = self.parse_slice_step()?;
                    Expr::Slice {
                        begin: Some(Box::new(first)),
                        end,
                        step,
                    }
                } else {
                    first // 通常添字（スライスなし）
                }
            };

            self.eat(&Token::RBracket)?;
            Ok(Expr::Subscript {
                object: Box::new(expr),
                index: Box::new(index),
            })
        }
    }

    /// スライスの begin/end/step 部分（省略可能な式）をパースする。
    /// `]`, `:`, `::` または EOF が来た場合は `None` を返す。
    fn parse_slice_part(&mut self) -> Result<Option<Box<Expr>>, String> {
        if matches!(
            *self.current(),
            Token::RBracket | Token::Colon | Token::ColonColon | Token::Eof
        ) {
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
                        cache: Default::default(),
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
    /// - `{...}` → `parse_dict_or_set_literal()`
    ///
    /// # エラー
    /// 未対応トークンが現れた場合、または `Self` をクラス外で使用した場合
    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.current().clone() {
            Token::Int(n) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            Token::Float(f) => {
                self.advance();
                Ok(Expr::Float(f))
            }
            Token::ImaginaryFloat(f) => {
                self.advance();
                Ok(Expr::ImaginaryLit(f))
            }
            Token::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Token::FStr(parts) => {
                self.advance();
                Ok(self.desugar_fstring(parts)?)
            }
            Token::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Token::None => {
                self.advance();
                Ok(Expr::None)
            }
            Token::Undefined => {
                self.advance();
                Ok(Expr::Undefined)
            }
            Token::Any => {
                self.advance();
                Ok(Expr::Ident("Any".to_string()))
            }
            Token::Union => {
                self.advance();
                Ok(Expr::Ident("Union".to_string()))
            }
            Token::Option => {
                self.advance();
                Ok(Expr::Ident("Option".to_string()))
            }
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
            Token::Ident(name) if name == "local" => {
                self.advance();
                if *self.current() == Token::ColonColon {
                    self.advance(); // consume ::
                    let var_name = self.expect_ident()?;
                    Ok(Expr::LocalVar(var_name))
                } else {
                    Err("ParseError: 'local' is reserved for the local namespace (use local::args)".to_string())
                }
            }
            Token::Ident(name) => {
                self.advance();
                // 別名（alias）は式位置では保存済みの右辺 AST に置換する（純粋な構文置換）。
                // 後続の postfix（`(...)` / `[...]` / `.attr`）は展開後の式に適用される。
                match self.aliases.get(&name).map(|e| (*e.expr).clone()) {
                    Some(expanded) => Ok(expanded),
                    None => Ok(Expr::Ident(name)),
                }
            }
            Token::SelfType => {
                if self.class_or_trait_depth == 0 {
                    return Err(
                        "ParseError: 'Self' can only be used inside class or trait definitions"
                            .to_string(),
                    );
                }
                self.advance();
                Ok(Expr::Ident("Self".to_string()))
            }
            Token::LParen => self.parse_paren_expr(),
            Token::LBracket => self.parse_list_literal(),
            Token::LBrace => self.parse_dict_or_set_literal(),
            // `if cond [->Type]: body [elif/else]` — if 式
            Token::If => {
                self.advance();
                let (branches, else_body, return_type) = self.parse_if_components()?;
                Ok(Expr::IfExpr {
                    branches,
                    else_body,
                    return_type,
                })
            }
            // `for target in iter [->Type]: body` — for 式
            Token::For => {
                self.advance();
                let target = self.expect_ident()?;
                self.eat(&Token::In)?;
                let iter = self.parse_expr()?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::ForExpr {
                    target,
                    iter: Box::new(iter),
                    body: self.parse_block()?,
                    return_type,
                })
            }
            // `while cond [->Type]: body` — while 式
            Token::While => {
                self.advance();
                let cond = self.parse_expr()?;
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::WhileExpr {
                    cond: Box::new(cond),
                    body: self.parse_block()?,
                    return_type,
                })
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
                Ok(Expr::MatchExpr {
                    subject: Box::new(subject),
                    arms,
                    return_type,
                })
            }
            // `block [->Type]: body` — ブロック式。block_return/block_yield で値を返す。
            Token::Block => {
                self.advance(); // `block` を消費
                let return_type = self.parse_opt_return_type()?;
                self.eat(&Token::Colon)?;
                Ok(Expr::Block {
                    stmts: self.parse_block()?,
                    return_type,
                })
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
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
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
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(Expr::List(items))
    }

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
                if *self.current() == Token::RBrace {
                    break;
                }
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
                if *self.current() == Token::RBrace {
                    break;
                }
                items.push(self.parse_expr()?);
            }
            self.eat(&Token::RBrace)?;
            Ok(Expr::Set(items))
        }
    }
}
