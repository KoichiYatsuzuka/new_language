// imports/cpp.rs — cpp-dll / cpp-lib import の解析: ヘッダ解決と型スタブ(Stmt::FnDef)生成。

#[allow(unused_imports)]
use {
    crate::parser::Parser,
    crate::ast::{Accessibility, FieldKind, Param, Stmt},
    crate::token::Token, crate::lexer, crate::python_converter,
    std::path::PathBuf,
};
#[allow(unused_imports)]
use super::*;

impl Parser {
    /// `import[cpp-lib] Dir.Name as alias` をパースする。
    /// `import[cpp-dll] Dir.Name as alias` も同様。
    ///
    /// ドット区切り識別子をヘッダファイルパスに解決する:
    ///   `DxLib.DxLib` → `{source_dir}/DxLib/DxLib.h`
    ///   最後のコンポーネントに `.h` 拡張子が付く。
    ///
    /// ヘッダが存在する場合は静的型情報として Stmt::FnDef スタブを body に積む。
    pub(crate) fn parse_cpp_import(&mut self, lang: String) -> Result<Stmt, String> {
        // ヘッダパス: IDENT ('.' IDENT)*
        let first = match self.current().clone() {
            Token::Ident(s) => {
                self.advance();
                s
            }
            other => {
                return Err(format!(
                    "import[{lang}]: expected dotted identifier for header path, got `{other}`"
                ))
            }
        };
        let mut parts = vec![first];
        while *self.current() == Token::Dot {
            self.advance();
            parts.push(self.expect_ident()?);
        }

        // ドット区切りパーツを source_dir 基準のヘッダパスに解決する
        // 例: DxLib.DxLib → {source_dir}/DxLib/DxLib.h
        let mut resolved = self.source_dir.clone();
        let n = parts.len();
        for (i, part) in parts.iter().enumerate() {
            if i == n - 1 {
                resolved.push(format!("{part}.h"));
            } else {
                resolved.push(part.as_str());
            }
        }
        let file_path = resolved.to_string_lossy().into_owned();

        // `as alias` — 省略可
        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        // ヘッダファイルを読み込んで静的型スタブを生成する。
        // ファイルが存在しない場合は空の body になる（実行時に解決される）。
        // 非 UTF-8 バイト（例: DxLib.h の Shift-JIS コメント）を含むヘッダでも
        // スタブを生成できるよう、`read_to_string`（厳格 UTF-8）ではなく
        // `read` + `from_utf8_lossy` を使う（実行時の `load_cpp_module` と一致）。
        // これがないと型チェッカーが空の body を受け取り、`T*` + let の静的検査
        // （P5）が働かない。
        let body = std::fs::read(&resolved)
            .ok()
            .map(|raw| String::from_utf8_lossy(&raw).into_owned())
            .map(|content| {
                let cfg = crate::interpreter::cpp_bridge::load_cpp_config(
                    resolved.parent().unwrap_or(std::path::Path::new(".")),
                );
                let typedefs = crate::interpreter::cpp_bridge::load_system_typedefs(
                    &cfg.system_headers,
                    &cfg.precompile_macros,
                );
                use crate::interpreter::cpp_bridge::CType;
                let (sigs, struct_defs) = crate::interpreter::cpp_bridge::parse_header_full(
                    &content,
                    &cfg.custom_type_map,
                    &typedefs,
                );
                let mut stmts: Vec<Stmt> = Vec::new();
                // Generate Stmt::ClassDef stubs for C structs so the type checker
                // knows about their fields.
                for sdef in &struct_defs {
                    let field_stmts = sdef
                        .fields
                        .iter()
                        .map(|(fname, fct)| Stmt::Field {
                            name: fname.clone(),
                            kind: FieldKind::Mut,
                            type_ann: ctype_to_tl_str(fct),
                            default: None,
                            access: Accessibility::Public,
                        })
                        .collect();
                    stmts.push(Stmt::ClassDef {
                        name: sdef.name.clone(),
                        template_params: vec![],
                        bases: vec![],
                        decorators: vec![],
                        body: field_stmts,
                    });
                }
                for sig in sigs {
                    let ret_str = ctype_to_tl_str(&sig.ret);
                    let params: Vec<Param> = sig
                        .params
                        .into_iter()
                        .map(|(pname, ct)| {
                            // 書き込み用ポインタ引数（`T*` / `VECTOR*` 等）は `mut` 扱いにする。
                            // これにより型チェッカーの `CallMutParamWithImmutableArg` 検査が
                            // 「不変（`let`）変数を出力ポインタへ渡す」誤りを静的に捕捉する
                            // （P5 — .claude/skills/c-abi-interop/SKILL.md。従来は実行時 TypeError のみ）。
                            let writable_ref = matches!(
                                &ct,
                                CType::Ptr { mutable: true, .. }
                                    | CType::OpaqueStructPtr { mutable: true, .. }
                            );
                            Param::bridge(pname, Some(ctype_to_tl_str(&ct)), writable_ref)
                        })
                        .collect();
                    stmts.push(Stmt::FnDef {
                        name: sig.name,
                        template_params: vec![],
                        params,
                        return_type: Some(ret_str),
                        body: vec![],
                        is_abstract: false,
                        is_static: false,
                        is_class_method: false,
                        decorators: vec![],
                        access: Accessibility::Public,
                    });
                }
                stmts
            })
            .unwrap_or_default();

        let module = vec![file_path];
        Ok(Stmt::Import {
            lang,
            module,
            with_file: None,
            alias,
            body,
        })
    }

    /// `from module import[lang] Name1, Name2 as N2` をパースして `Stmt::FromImport` を返す。
    pub(crate) fn parse_from_import_stmt(&mut self) -> Result<Stmt, String> {
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
                if matches!(
                    self.current(),
                    Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent
                ) {
                    break;
                }
            } else {
                break;
            }
        }

        // モジュールの tl AST を取得
        let body = self.load_module(&lang, &module, None)?;

        Ok(Stmt::FromImport {
            lang,
            module,
            with_file: None,
            names,
            body,
        })
    }

}
