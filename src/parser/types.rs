// types.rs — type annotation, template parameter, and function parameter parsing.

use super::Parser;
use crate::ast::{Param, TemplateParam};
use crate::token::Token;

impl Parser {
    /// 呼び出しサイトや継承サイトの具体型引数列 `[Type1, Type2, ...]` をパースする。
    ///
    /// 現在のトークンが `[` でない場合は空のベクタを返す（型引数なし）。
    ///
    /// # 戻り値
    /// 型名の文字列リスト（型引数がない場合は空 Vec）
    ///
    /// # エラー
    /// 型名のパースに失敗した場合、または `]` がない場合
    pub(super) fn parse_type_args(&mut self) -> Result<Vec<String>, String> {
        if *self.current() != Token::LBracket {
            return Ok(vec![]);
        }
        self.advance();
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
    pub(super) fn parse_template_params(&mut self) -> Result<Vec<TemplateParam>, String> {
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
    /// `self` と可変長（`variadic`）パラメータは検査から除外する。
    pub(super) fn validate_param_defaults(params: &[Param]) -> Result<(), String> {
        let mut seen_default = false;
        for p in params {
            if p.name == "self" || p.variadic {
                continue;
            }
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
    /// 構文: `[mut|let] 識別子 [: 型] [= デフォルト式]`
    ///       `[mut|let] ... : 型`  （可変長パラメータ）
    ///
    /// # 戻り値
    /// `Param { name, mutable, type_ann, default, variadic }`
    /// - `mutable`: `mut` キーワードが先行している場合は `true`
    /// - `type_ann`: `: 型` がある場合は `Some(型名)`
    /// - `default`: `= 式` がある場合は `Some(式)`
    /// - `variadic`: `...` を名前として使用した場合は `true`
    ///
    /// # エラー
    /// 識別子または型名のパースに失敗した場合
    pub(super) fn parse_param(&mut self) -> Result<Param, String> {
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

        // 可変長パラメータ: `let ...` または `mut ...`
        let (name, variadic) = if *self.current() == Token::Ellipsis {
            self.advance(); // consume `...`
            ("...".to_string(), true)
        } else {
            (self.expect_ident()?, false)
        };

        // 型アノテーション `: 型` があればパース（可変長は必須）
        let type_ann = if *self.current() == Token::Colon {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            if variadic {
                return Err(
                    "ParseError: variadic parameter `...` requires a type annotation (e.g. `let ...: int`)".to_string()
                );
            }
            None
        };

        // デフォルト値 `= 式` があればパース（可変長は不可）
        let default = if *self.current() == Token::Eq {
            if variadic {
                return Err(
                    "ParseError: variadic parameter `...` cannot have a default value".to_string()
                );
            }
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Param {
            name,
            mutable,
            type_ann,
            default,
            variadic,
        })
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
    pub(super) fn parse_type_expr(&mut self) -> Result<String, String> {
        match self.current().clone() {
            Token::Union => {
                self.advance();
                if *self.current() != Token::LBracket {
                    return Err(
                        "Union requires type arguments: Union[Type1, Type2, ...]".to_string()
                    );
                }
                self.advance(); // consume '['
                let mut args = Vec::new();
                while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                    args.push(self.parse_type_expr()?);
                    if *self.current() == Token::Comma {
                        self.advance();
                    }
                }
                self.eat(&Token::RBracket)?;
                if args.len() < 2 {
                    return Err(format!(
                        "Union requires at least 2 type arguments, got {}",
                        args.len()
                    ));
                }
                return Ok(format!("Union[{}]", args.join(",")));
            }
            Token::Intersection => {
                self.advance();
                if *self.current() != Token::LBracket {
                    return Err(
                        "Intersection requires type arguments: Intersection[Type1, Type2, ...]"
                            .to_string(),
                    );
                }
                self.advance(); // consume '['
                let mut args = Vec::new();
                while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                    args.push(self.parse_type_expr()?);
                    if *self.current() == Token::Comma {
                        self.advance();
                    }
                }
                self.eat(&Token::RBracket)?;
                if args.len() < 2 {
                    return Err(format!(
                        "Intersection requires at least 2 type arguments, got {}",
                        args.len()
                    ));
                }
                return Ok(format!("Intersection[{}]", args.join(",")));
            }
            Token::Option => {
                self.advance();
                if *self.current() != Token::LBracket {
                    return Err("Option requires a type argument: Option[Type]".to_string());
                }
                self.advance(); // consume '['
                let inner = self.parse_type_expr()?;
                if *self.current() == Token::Comma {
                    self.advance();
                }
                self.eat(&Token::RBracket)?;
                return Ok(format!("Option[{inner}]"));
            }
            _ => {}
        }

        let base = match self.current().clone() {
            Token::Ident(name) => {
                self.advance();
                name
            }
            Token::None => {
                self.advance();
                "None".to_string()
            }
            Token::Undefined => {
                self.advance();
                "Undefined".to_string()
            }
            Token::Any => {
                self.advance();
                "Any".to_string()
            }
            Token::SelfType => {
                if self.class_or_trait_depth == 0 {
                    return Err(
                        "ParseError: 'Self' can only be used inside class or trait definitions"
                            .to_string(),
                    );
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
            if *self.current() == Token::Comma {
                self.advance();
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("type[{inner}]"));
        }
        // tuple[T1, T2, ...] — preserve element types for the type checker.
        if base == "tuple" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let mut args = Vec::new();
            while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                args.push(self.parse_type_expr()?);
                if *self.current() == Token::Comma {
                    self.advance();
                }
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
            if *self.current() == Token::Comma {
                self.advance();
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("list[{elem}]"));
        }
        // fixed_list[T] — preserve element type
        if base == "fixed_list" && *self.current() == Token::LBracket {
            self.advance();
            let elem = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("fixed_list[{elem}]"));
        }
        // list_like[T] — preserve element type
        if base == "list_like" && *self.current() == Token::LBracket {
            self.advance();
            let elem = self.parse_type_expr()?;
            if *self.current() == Token::Comma { self.advance(); }
            self.eat(&Token::RBracket)?;
            return Ok(format!("list_like[{elem}]"));
        }
        // set[T] — preserve element type
        if base == "set" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let elem = self.parse_type_expr()?;
            if *self.current() == Token::Comma {
                self.advance();
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("set[{elem}]"));
        }
        // dict[K, V] — preserve key and value types
        if base == "dict" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let key = self.parse_type_expr()?;
            if *self.current() == Token::Comma {
                self.advance();
            }
            let val = self.parse_type_expr()?;
            if *self.current() == Token::Comma {
                self.advance();
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("dict[{key},{val}]"));
        }
        // Result[T, E] — preserve Ok type and Err type
        if base == "Result" && *self.current() == Token::LBracket {
            self.advance(); // consume '['
            let ok_ty = self.parse_type_expr()?;
            if *self.current() == Token::Comma {
                self.advance();
            }
            let err_ty = self.parse_type_expr()?;
            if *self.current() == Token::Comma {
                self.advance();
            }
            self.eat(&Token::RBracket)?;
            return Ok(format!("Result[{ok_ty}, {err_ty}]"));
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
                        Token::Mut => {
                            self.advance();
                            true
                        }
                        Token::Let => {
                            self.advance();
                            false
                        }
                        _ => false,
                    };
                    let ty = self.parse_type_expr()?;
                    let prefix = if mutable { "mut" } else { "let" };
                    params.push(format!("{prefix} param{auto_idx}:{ty}"));
                    auto_idx += 1;
                    if *self.current() == Token::Comma {
                        self.advance();
                    }
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
                        Token::Mut => {
                            self.advance();
                            true
                        }
                        Token::Let => {
                            self.advance();
                            false
                        }
                        _ => false,
                    };
                    let name = self.expect_ident()?;
                    self.eat(&Token::Colon)?;
                    let ty = self.parse_type_expr()?;
                    let prefix = if mutable { "mut" } else { "let" };
                    params.push(format!("{prefix} {name}:{ty}"));
                    if *self.current() == Token::Comma {
                        self.advance();
                    }
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
    pub(super) fn is_template_instantiation(&self) -> bool {
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
    pub(super) fn expect_ident(&mut self) -> Result<String, String> {
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
    pub(super) fn expect_attr_name(&mut self) -> Result<String, String> {
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
    pub(super) fn expect_guard_type_name(&mut self) -> Result<String, String> {
        match self.current().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(name)
            }
            Token::None => {
                self.advance();
                Ok("None".to_string())
            }
            Token::Undefined => {
                self.advance();
                Ok("Undefined".to_string())
            }
            tok => Err(format!("expected type name after `is`, got `{tok}`")),
        }
    }
}
