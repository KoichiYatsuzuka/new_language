// imports/cpp.rs — cpp-dll / cpp-lib import の解析: ヘッダ解決と型スタブ(Stmt::FnDef)生成。

use {
    crate::parser::Parser,
    crate::ast::{Accessibility, FieldKind, Param, Stmt},
    crate::token::Token,
};
use super::*;

impl Parser {
    /// `import[cpp-lib] Dir.Name as alias` をパースする。
    /// `import[cpp-dll] Dir.Name as alias` も同様。
    ///
    /// ドット区切り識別子をヘッダファイルパスに解決する:
    ///   `DxLib.DxLib` → `{source_dir}/DxLib/DxLib.h`
    ///   `..DxLib.DxLib` → `{source_dir}/../DxLib/DxLib.h`（相対 import・[`crate::module_path`]）
    ///   最後のコンポーネントに `.h` 拡張子が付く。
    ///
    /// ヘッダが存在する場合は静的型情報として Stmt::FnDef スタブを body に積む。
    pub(crate) fn parse_cpp_import(&mut self, lang: String) -> Result<Stmt, String> {
        // ヘッダパス: [.]* IDENT ('.' IDENT)*
        if !matches!(self.current(), Token::Ident(_) | Token::Dot | Token::Ellipsis) {
            return Err(format!(
                "import[{lang}]: expected dotted identifier for header path, got `{}`",
                self.current()
            ));
        }
        let (level, parts) = self.parse_module_ref(false)?;
        // ヘッダ名の位置（エディタ索引）。`as x` を読むと `prev_pos()` が別名を指す。
        #[cfg(feature = "editor")]
        let module_pos = self.prev_pos();

        // ドット区切りパーツを探索の起点基準のヘッダパスに解決する
        // 例: DxLib.DxLib → {source_dir}/DxLib/DxLib.h
        let mut resolved = self.import_base(level);
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

        // エディタ索引: 束縛される名前（別名か最後の部分）。位置はその名前を書いた識別子。
        #[cfg(feature = "editor")]
        {
            let (bind, pos) = match &alias {
                Some(a) => (a.clone(), self.prev_pos()),
                None => (parts.last().cloned().unwrap_or_default(), module_pos),
            };
            let h = self.note_def_at(&bind, crate::parser::editor_hooks::EditorKind::Module, pos);
            let sig = format!("import[{lang}] {}", crate::module_path::written_spelling(level, &parts));
            self.note_signature(h, &sig);
        }

        // ヘッダファイルを読み込んで静的型スタブを生成する。
        // ファイルが存在しない場合は空の body になる（実行時に解決される）。
        // 非 UTF-8 バイト（例: DxLib.h の Shift-JIS コメント）を含むヘッダでも
        // スタブを生成できるよう、`read_to_string`（厳格 UTF-8）ではなく
        // `read` + `from_utf8_lossy` を使う（実行時の `load_cpp_module` と一致）。
        // これがないと型チェッカーが空の body を受け取り、`T*` + let の静的検査
        // （P5）が働かない。
        // ⚠⚠ **ヘッダが読めなければエラー**（`editor_import_resolution_plan.md` 1-2）。以前は `.ok()` で
        //    黙って空の body（型なし）にしていた。未解決の import は黙って型情報を落とさない。
        let raw = self.try_import(|_| {
            crate::import_fs::read(&resolved).map_err(|e| {
                // ⚠ 理由は `ErrorKind`（`entity not found` など）。OS のエラー文はロケールで変わり（日本語の
                //   Windows では日本語）、拡張（wasm）の文面と食い違う。
                format!(
                    "import[{lang}]: cannot read header '{file_path}' ({}); the header provides the types of this import",
                    e.kind()
                )
            })
        })?;
        let body = raw
            .map(|raw| String::from_utf8_lossy(&raw).into_owned())
            .map(|content| {
                let cfg = crate::cpp_header::load_cpp_config(
                    resolved.parent().unwrap_or(std::path::Path::new(".")),
                );
                let typedefs = crate::cpp_header::load_system_typedefs(
                    &cfg.system_headers,
                    &cfg.precompile_macros,
                );
                use crate::cpp_header::CType;
                let (sigs, struct_defs) = crate::cpp_header::parse_header_full(
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
                        .map(|(fname, fct)| Stmt::Field { src: None,
                            name: fname.clone(),
                            kind: FieldKind::Mut,
                            type_ann: ctype_to_tl_str(fct),
                            default: None,
                            access: Accessibility::Public,
                        })
                        .collect();
                    stmts.push(Stmt::ClassDef { src: None,
                        name: sdef.name.clone(),
                        template_params: vec![],
                        bases: vec![],
                        // ⚠ C の構造体スタブは trait を継承しない（タスク 9.9）。
                        base_args: vec![],
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
                    stmts.push(Stmt::FnDef { src: None,
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

        // ⚠ `module` は**解決済みヘッダパス**（実行時に読み直すのがこれ）。原文の
        //   ドット表記はここで失われるので、エディタ用スタブの鍵のために別途残す
        //   （`Stmt::Import::source_module` の doc）。
        let module = vec![file_path];
        // 束縛は別名か**ヘッダのファイル名（stem）**（cpp 系は従来どおり・パッケージではない）。
        let bind_name = alias.clone().unwrap_or_else(|| {
            std::path::Path::new(&module[0])
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("lib")
                .to_string()
        });
        Ok(Stmt::Import {
            bind: Some(crate::ast::ImportBind { name: bind_name, module: module.clone() }),
            lang,
            module,
            source_module: Some(crate::module_path::written_spelling(level, &parts)),
            alias,
            body,
            origin: self.import_origin(level),
        })
    }

    /// `from module import[lang] Name1, Name2 as N2` をパースして `Stmt::FromImport` を返す。
    pub(crate) fn parse_from_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `from` を消費

        // モジュール指定（`..a.b`・相対 import）。`from . import x` のように名前の無い形も受ける。
        let (level, module) = self.parse_module_ref(true)?;

        // `import[lang]` または `import`（省略時は "ar-auto"）
        self.eat(&Token::Import)?;
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "ar-auto".to_string()
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
            // エディタ索引: 束縛される名前。位置は最後に読んだ識別子。
            #[cfg(feature = "editor")]
            {
                let bind = alias.clone().unwrap_or_else(|| name.clone());
                let h = self.note_def(&bind, crate::parser::editor_hooks::EditorKind::Module);
                let sig = format!(
                    "from {} import[{lang}] {name}",
                    crate::module_path::written_spelling(level, &module)
                );
                self.note_signature(h, &sig);
            }
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

        let file_dir = self.source_dir.clone();

        // `from . import x` / `from .. import x`（CPython の書き方）: x はサブモジュールか、
        // このディレクトリのパッケージの名前（`packages.rs`）。
        if module.is_empty() {
            let resolved = self.try_import(|p| p.from_dots_import(&lang, &file_dir, level, names.clone()))?;
            let Some(mut stmts) = resolved else {
                // 読み込めなかった（拡張だけ・`try_import`）: 名前は取り込んだことにして続ける。
                return Ok(Stmt::FromImport {
                    lang,
                    module: Vec::new(),
                    source_module: Some(".".repeat(level as usize)),
                    names,
                    body: Vec::new(),
                    origin: self.import_origin(level),
                });
            };
            let last = stmts.pop().expect("from_dots_import は 1 つ以上の文を返す");
            self.pending_stmts.extend(stmts);
            return Ok(last);
        }

        // モジュールの tl AST を取得する。CPython と同じく、先にパッケージの連鎖（`a` → `a.b`）を読み込み、
        // 取り込む名前がサブモジュールならそれも読み込む（`packages.rs`）。
        let resolved = self.try_import(|p| {
            let loaded = p.load_module(&lang, level, &module, None)?;
            if Self::has_packages(&lang) {
                let chain = p.package_chain(&lang, &file_dir, level, &module, &loaded)?;
                let subs = p.from_import_submodules(&lang, &file_dir, level, &module, &names, &loaded)?;
                p.pending_stmts.extend(chain.into_iter().map(|(_, st)| st));
                p.pending_stmts.extend(subs);
            }
            Ok(loaded)
        })?;
        let loaded = resolved.unwrap_or_else(|| Self::unresolved_import(&lang, &module, None).0);

        Ok(Stmt::FromImport {
            source_module: Self::written_if_renamed(level, &module, &loaded.name),
            lang,
            module: loaded.name,
            names,
            body: loaded.body,
            origin: self.import_origin(level),
        })
    }

}
