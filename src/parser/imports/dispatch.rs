// imports/dispatch.rs — import 文の解析とモジュール読み込みの振り分け: parse_import_stmt / lang・version ブラケット / module パス / load_module / load_rs_module。

use {
    crate::parser::Parser,
    crate::ast::Stmt,
    crate::token::Token,
};

impl Parser {
    /// `import[lang] module.sub as alias` をパースして `Stmt::Import` を返す。
    ///
    /// - `import[py] math as m`
    /// - `import[py] os.path as p`
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

        // モジュールパス (`a.b.c`)
        let module = self.parse_module_path()?;

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
        let body = self.load_module(&lang, &module, version.as_deref())?;

        Ok(Stmt::Import {
            lang,
            module,
            with_file: None,
            alias,
            body,
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

    /// ドット区切りのモジュールパスをパースして `Vec<String>` を返す。
    /// 例: `os.path` → `["os", "path"]`
    pub(crate) fn parse_module_path(&mut self) -> Result<Vec<String>, String> {
        let mut segments = vec![self.expect_ident()?];
        while *self.current() == Token::Dot {
            self.advance();
            segments.push(self.expect_ident()?);
        }
        Ok(segments)
    }

    /// モジュールを検索・読み込み・変換して tl AST を返す。キャッシュを使用する。
    pub(crate) fn load_module(
        &mut self,
        lang: &str,
        module: &[String],
        version: Option<&str>,
    ) -> Result<Vec<Stmt>, String> {
        match lang {
            // default (no bracket): prefer .arc, fall back to .ar
            // ("tl-auto" は旧名の別名。既存ソース互換のため受理し続ける)
            "ar-auto" | "tl-auto" => self.load_tl_module(module),
            // import[ar]: force .ar source, skip .arc
            "tl" | "ar" => self.load_tl_source_module(module),
            // import[arc]: force .arc, error if not found
            "tlc" | "arc" => self.load_tlc_module(module),
            "py" => self.load_python_module(module),
            // py-int: .pyi を優先し、なければ .py にフォールバック
            // body は型検査専用（実行時は PyO3 経由）
            "py-int" => self.load_python_interface_module(module),
            // import[rs]: クレートバインディングをコンパイル・キャッシュし stubs を返す
            "rs" => self.load_rs_module(module, version),
            // import[cs-dll]: .NET NativeAOT DLL — アセンブリから型スタブを生成
            "cs-dll" => self.load_cs_module(module, false),
            // import[cs-proc]: .NET IPC サブプロセス — 型情報は cs-dll と同一
            "cs-proc" => self.load_cs_module(module, true),
            // import[js-proc]: Node.js IPC サブプロセス — .ars スタブが存在すれば読み込む
            "js-proc" => self.load_js_module(module),
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
        let cache_key = ("rs".to_string(), std::path::PathBuf::from(&module_name));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        let search_dirs: Vec<std::path::PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

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
