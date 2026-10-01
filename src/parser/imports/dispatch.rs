// imports/dispatch.rs — import 文の解析とモジュール読み込みの振り分け: parse_import_stmt / lang・version ブラケット / load_module / load_rs_module。

use {
    crate::parser::Parser,
    crate::ast::{ImportOrigin, Stmt},
    crate::module_path,
    crate::token::Token,
    std::path::{Path, PathBuf},
};

/// 読み込んだモジュール（[`Parser::load_module`] の結果）。
pub(crate) struct LoadedModule {
    pub(crate) body: Vec<Stmt>,
    /// AST の `module` に入れる名前。
    ///
    /// ⚠ Arrow / py のモジュールは**ファイルごとに一意な名前**（[`crate::module_path`]）。
    ///   それ以外（py-int / rs / cs / js）は書いた綴りのまま（実行時がその名前で外部へ問い合わせる）。
    pub(crate) name: Vec<String>,
}

impl LoadedModule {
    /// 名前を書き換えない言語用（書いた綴りをそのまま `module` にする）。
    pub(crate) fn as_written(body: Vec<Stmt>, module: &[String]) -> Self {
        LoadedModule { body, name: module.to_vec() }
    }
}

impl Parser {
    /// 探索の起点（`level` を適用済み・[`module_path::search_base`]）。
    pub(crate) fn import_base(&self, level: u32) -> PathBuf {
        module_path::search_base(&self.source_dir, level)
    }

    /// import 文に載せる探索の起点（実行時の探索もこれを使う・[`ImportOrigin`]）。
    ///
    /// ⚠ ファイルから読んでいないソース（`Parser::new(_, None)`）では起点を載せない
    ///   （`Parser::has_source_dir` の doc）。
    pub(crate) fn import_origin(&self, level: u32) -> ImportOrigin {
        ImportOrigin {
            level,
            base_dir: self.has_source_dir.then(|| self.import_base(level)),
        }
    }

    /// 見つけたモジュールファイルに名前を付ける（Arrow / py・[`module_path::ModuleNames`]）。
    ///
    /// - `local` … import 文のファイルの近く（起点）で見つけた。名前はエントリのディレクトリからの相対。
    /// - それ以外（Python の検索パスなど外部で見つけた）… 書いた綴り（Python と同じ名前の付け方）。
    pub(crate) fn name_module_file(
        &self,
        file: &Path,
        written: &[String],
        local: bool,
    ) -> Result<Vec<String>, String> {
        let identity = module_path::module_identity(file);
        let preferred = if local {
            module_path::root_relative_name(&identity, &self.root_dir)
                .unwrap_or_else(|| written.to_vec())
        } else {
            written.to_vec()
        };
        self.module_names.borrow_mut().assign(&identity, preferred)
    }

    /// `module` が書いた綴りと違うときだけ、書いた綴りを残す（`Stmt::Import::source_module`）。
    pub(crate) fn written_if_renamed(level: u32, written: &[String], name: &[String]) -> Option<String> {
        (level != 0 || written != name).then(|| module_path::written_spelling(level, written))
    }

    /// `import[lang] module.sub as alias` をパースして `Stmt::Import` を返す。
    ///
    /// - `import[py] math as m`
    /// - `import[py] os.path as p`
    /// - `import ..lib.helper as h`（相対 import・[`crate::module_path`]）
    pub(crate) fn parse_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `import` を消費

        // `[lang]` を読む。省略時は "ar-auto" (auto-select: prefer .arc over .ar)
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "ar-auto".to_string()
        };

        // cpp-dll / cpp-lib: `import[cpp-dll] Dir.Name with stub as alias`
        if lang == "cpp-dll" || lang == "cpp-lib" {
            return self.parse_cpp_import(lang);
        }

        // モジュール指定 (`..a.b.c`)
        let (level, module) = self.parse_module_ref()?;

        // `[version]` — `import[rs] libm[0.2]` のバージョン指定（rs のみ）
        let version = if lang == "rs" && *self.current() == Token::LBracket {
            Some(self.parse_version_bracket()?)
        } else {
            None
        };

        // `as alias` (省略可)
        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        // モジュールの tl AST を取得（キャッシュ込み）
        let loaded = self.load_module(&lang, level, &module, version.as_deref())?;

        Ok(Stmt::Import {
            source_module: Self::written_if_renamed(level, &module, &loaded.name),
            lang,
            module: loaded.name,
            alias,
            body: loaded.body,
            origin: self.import_origin(level),
        })
    }

    /// `[lang]` トークン列をパースして言語識別子文字列を返す。
    pub(crate) fn parse_lang_bracket(&mut self) -> Result<String, String> {
        self.eat(&Token::LBracket)?;
        let mut lang = match self.current().clone() {
            Token::Ident(s) => {
                self.advance();
                s
            }
            other => return Err(format!("expected language identifier, got `{other}`")),
        };
        // ハイフン区切りの識別子を許容（例: `py-int`）
        while *self.current() == Token::Minus {
            self.advance();
            match self.current().clone() {
                Token::Ident(s) => {
                    self.advance();
                    lang = format!("{lang}-{s}");
                }
                other => {
                    return Err(format!(
                        "expected identifier after '-' in lang tag, got `{other}`"
                    ))
                }
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(lang)
    }

    /// モジュールを検索・変換して tl AST を返す。キャッシュを使用する。
    ///
    /// `level` は先頭のドットの数。探索の規則は**全言語で** [`crate::module_path`] に従う。
    pub(crate) fn load_module(
        &mut self,
        lang: &str,
        level: u32,
        module: &[String],
        version: Option<&str>,
    ) -> Result<LoadedModule, String> {
        match lang {
            // default (no bracket): prefer .arc, fall back to .ar
            // ("tl-auto" は旧名の別名。既存ソース互換のため受理し続ける)
            "ar-auto" | "tl-auto" => self.load_tl_module(level, module),
            // import[ar]: force .ar source, skip .arc
            "tl" | "ar" => self.load_tl_source_module(level, module),
            // import[arc]: force .arc, error if not found
            "tlc" | "arc" => self.load_tlc_module(level, module),
            "py" => {
                let dir = self.source_dir.clone();
                self.load_python_module(&dir, level, module)
            }
            // py-int: .pyi を優先し、なければ .py にフォールバック
            // body は型検査専用（実行時は PyO3 経由）
            "py-int" => self
                .load_python_interface_module(level, module)
                .map(|b| LoadedModule::as_written(b, module)),
            // import[rs]: クレートバインディングをコンパイル・キャッシュし stubs を返す
            // ⚠ クレートはファイルではないので相対の書き方は受けない。
            "rs" if level > 0 => Err(format!(
                "import[rs] takes a crate name, not a file path: `{}` cannot be relative",
                module_path::written_spelling(level, module)
            )),
            "rs" => self
                .load_rs_module(module, version)
                .map(|b| LoadedModule::as_written(b, module)),
            // import[cs-dll]: .NET NativeAOT DLL — アセンブリから型スタブを生成
            "cs-dll" => self
                .load_cs_module(level, module, false)
                .map(|b| LoadedModule::as_written(b, module)),
            // import[cs-proc]: .NET IPC サブプロセス — 型情報は cs-dll と同一
            "cs-proc" => self
                .load_cs_module(level, module, true)
                .map(|b| LoadedModule::as_written(b, module)),
            // import[js-proc]: Node.js IPC サブプロセス — .ars スタブが存在すれば読み込む
            "js-proc" => self
                .load_js_module(level, module)
                .map(|b| LoadedModule::as_written(b, module)),
            other => Err(format!("unknown import language '{other}'")),
        }
    }

    /// `import[rs] name[version]` — クレートバインディングをコンパイル・キャッシュし、
    /// 型チェッカ・インタプリタが使う `Stmt::FnDef` スタブを返す。
    /// `version` が `Some` ならそれを直接使用し、`None` なら `ar_crates.json` を参照する。
    pub(crate) fn load_rs_module(
        &mut self,
        module: &[String],
        version: Option<&str>,
    ) -> Result<Vec<Stmt>, String> {
        let module_name = module.last().cloned().unwrap_or_default();
        let cache_key = ("rs".to_string(), PathBuf::from(&module_name));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // `ar_config.json`（`rust.crates_path`）を探すディレクトリ: import 文のファイルの
        // ディレクトリから**祖先へ**（`python.search_paths` / `csharp.lib_paths` と同じ方針）。
        // ⚠ 以前は `source_dir` と `root_dir`（エントリのディレクトリ）の 2 箇所だった。
        let search_dirs: Vec<PathBuf> = module_path::absolute(&self.source_dir)
            .ancestors()
            .map(Path::to_path_buf)
            .collect();

        let body = crate::partial_compiler::rs_loader::load(&module_name, &search_dirs, version)
            .map_err(|e| format!("import[rs] '{}': {e}", module.join(".")))?;

        // Write .ars stub so the VS Code extension can provide hover/completion
        let stub_text = crate::partial_compiler::stub_gen::generate_stub(&body);
        let stub_path = self.source_dir.join(format!("{module_name}.ars"));
        let _ = std::fs::write(&stub_path, &stub_text);

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `[X.Y.Z]` 形式のバージョンブラケットをパースして文字列で返す。
    /// 例: `[0.2]` → `"0.2"`, `[1]` → `"1"`, `[1.2.3]` → `"1.2.3"`
    pub(crate) fn parse_version_bracket(&mut self) -> Result<String, String> {
        self.eat(&Token::LBracket)?;
        let mut ver = String::new();
        loop {
            match self.current().clone() {
                Token::RBracket => { self.advance(); break; }
                Token::Eof | Token::Newline => {
                    return Err("unterminated version bracket in import[rs]".to_string());
                }
                Token::Int(n) => { ver.push_str(&n.to_string()); self.advance(); }
                Token::Float(f) => { ver.push_str(&format!("{f}")); self.advance(); }
                Token::Dot => { ver.push('.'); self.advance(); }
                Token::Ident(s) => { ver.push_str(&s); self.advance(); }
                Token::Str(s) => { ver.push_str(&s); self.advance(); }
                other => return Err(format!("unexpected token `{other}` in version bracket")),
            }
        }
        if ver.is_empty() {
            return Err("version bracket cannot be empty in import[rs]".to_string());
        }
        Ok(ver)
    }

}
