// classes.rs — class and trait definition parsing for the tl parser.

use super::Parser;
use crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt};
use crate::token::{Span, Token};
use std::collections::HashMap;

impl Parser {
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
    pub(super) fn parse_trait_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // `trait` を消費
        let name = self.expect_ident()?;
        // テンプレートパラメータをパース（`[T: Trait, ...]` 形式）
        let template_params = self.parse_template_params()?;
        // トレイトは継承不可
        if *self.current() == Token::LParen {
            return Err(format!(
                "StaticTypeError: trait `{name}` cannot inherit from another type"
            ));
        }
        self.eat(&Token::Colon)?;
        // Self 型が有効なスコープに入る
        self.class_or_trait_depth += 1;
        let body = self.parse_class_body(true)?;
        self.class_or_trait_depth -= 1;

        // 非仮想・仮想メソッドどちらも型アノテーションを必須とする
        for stmt in &body {
            if let Stmt::FnDef {
                name: mname,
                params,
                return_type,
                is_abstract,
                ..
            } = stmt
            {
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
        let fields: Vec<(String, FieldKind, String, bool)> = body
            .iter()
            .filter_map(|s| {
                if let Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    default,
                    ..
                } = s
                {
                    Some((
                        fname.clone(),
                        kind.clone(),
                        type_ann.clone(),
                        default.is_some(),
                    ))
                } else {
                    None
                }
            })
            .collect();
        // 仮想メソッド（is_abstract: true）の名前リストを収集
        let virtual_methods: Vec<String> = body
            .iter()
            .filter_map(|s| {
                if let Stmt::FnDef {
                    name: mname,
                    is_abstract: true,
                    ..
                } = s
                {
                    Some(mname.clone())
                } else {
                    None
                }
            })
            .collect();
        // 後続のクラス定義が参照できるよう known_traits に登録
        self.known_traits.insert(
            name.clone(),
            (template_params.clone(), fields, virtual_methods),
        );

        Ok(Stmt::TraitDef {
            name,
            template_params,
            body,
        })
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
    pub(super) fn parse_class_def(&mut self) -> Result<Stmt, String> {
        self.parse_class_def_decorated(vec![])
    }

    /// デコレータ付きクラス定義をパースして `Stmt::ClassDef` を返す。
    /// `parse_class_def` から呼ばれるほか、デコレータ構文のパス（`@decorator class ...`）でも使われる。
    pub(super) fn parse_class_def_decorated(
        &mut self,
        decorators: Vec<Expr>,
    ) -> Result<Stmt, String> {
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
        let mut body = self.parse_class_body(false)?;
        self.class_or_trait_depth -= 1;

        // 仮想メソッドのオーバーライド検証とトレイト必須フィールドの収集
        let trait_required =
            self.collect_trait_fields_and_check_virtuals(&name, &bases_with_args, &body)?;

        // クラス自身のデフォルトなし mut/let フィールドを収集
        let class_required: Vec<(String, String)> = body
            .iter()
            .filter_map(|s| {
                if let Stmt::Field {
                    name: fname,
                    kind: FieldKind::Mut | FieldKind::Let,
                    type_ann,
                    default: None,
                    ..
                } = s
                {
                    Some((fname.clone(), type_ann.clone()))
                } else {
                    None
                }
            })
            .collect();

        // 必要に応じて __init__ を自動生成して body に追加
        self.generate_auto_init_if_needed(&trait_required, &class_required, &mut body);

        Ok(Stmt::ClassDef {
            name,
            template_params,
            bases,
            body,
            decorators,
        })
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
                let type_map: HashMap<String, String> = trait_tparams
                    .iter()
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
                        let resolved = type_map
                            .get(ftype)
                            .cloned()
                            .unwrap_or_else(|| ftype.clone());
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
        let all_required: Vec<(String, String)> = trait_required
            .iter()
            .map(|(_, fname, ftype)| (fname.clone(), ftype.clone()))
            .chain(class_required.iter().cloned())
            .collect();

        // 完全一致する既存 __init__ がある場合は auto-init を生成しない（override）
        let has_exact_match = body.iter().any(|s| {
            if let Stmt::FnDef {
                name: n, params, ..
            } = s
            {
                n == "__init__" && Self::init_sig_matches(&all_required, params)
            } else {
                false
            }
        });

        if has_exact_match {
            return;
        }

        // `mut self` に続いてトレイトフィールド、クラスフィールドの順でパラメータを構築
        let mut params = vec![Param {
            name: "self".to_string(),
            mutable: true,
            type_ann: None,
            default: None,
        }];
        for (_, fname, ftype) in trait_required {
            params.push(Param {
                name: fname.clone(),
                mutable: false,
                type_ann: Some(ftype.clone()),
                default: None,
            });
        }
        for (fname, ftype) in class_required {
            params.push(Param {
                name: fname.clone(),
                mutable: false,
                type_ann: Some(ftype.clone()),
                default: None,
            });
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
    fn parse_class_body(&mut self, is_trait: bool) -> Result<Vec<Stmt>, String> {
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
                    if *self.current() == Token::Protected && !is_trait {
                        return Err(
                            "ParseError: `protected:` sections are not allowed in class definitions; \
                             declare protected fields in a trait instead"
                                .to_string(),
                        );
                    }
                    current_access = match self.current() {
                        Token::Public => Accessibility::Public,
                        Token::Private => Accessibility::Private,
                        _ => Accessibility::Protected,
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
                    Token::Mut => FieldKind::Mut,
                    Token::Let => FieldKind::Let,
                    _ => FieldKind::Const,
                };
                // エラーメッセージ用にキーワード文字列を保持
                let keyword = match &kind {
                    FieldKind::Mut => "mut",
                    FieldKind::Let => "let",
                    FieldKind::Const => "const",
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
                Ok(Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    default,
                    access: Accessibility::Public,
                })
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
                        Ok(Stmt::Field {
                            name: fname,
                            kind: FieldKind::StaticMut,
                            type_ann,
                            default,
                            access: Accessibility::Public,
                        })
                    }
                    tok => Err(format!(
                        "expected `fn` or `mut` after `static` in class body, got `{tok}`"
                    )),
                }
            }
            Token::ClassMethod => {
                self.advance(); // consume `class_method`
                if *self.current() != Token::Fn {
                    return Err(format!(
                        "expected `fn` after `class_method`, got `{}`",
                        self.current()
                    ));
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
            && non_self
                .iter()
                .zip(required_fields.iter())
                .all(|(p, (_, ftype))| p.type_ann.as_deref() == Some(ftype.as_str()))
    }
}

