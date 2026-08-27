// parser/imports_editor.rs — `editor` feature 版の import 解析（ファイルシステムに触れない）。
//
// 通常ビルド（`parser/imports/`）は import 文をパースする**その場で**対象モジュールを
// 読み込む: `.ar` を開き、Python サブプロセスを起動し、C++ シムをビルドし、.NET
// アセンブリをロードする。バッチ実行では正しいが、エディタでは 1 打鍵ごとに走るため
// 使えない（`examples/interop/importation.ar` は実測 7.7 秒かかる）。
//
// ここでは**構文だけを解釈して `body: vec![]` を返す**。狙いは 2 つ:
//   1. import 文が構文エラーにならない（＝以降の行の解析が生き残る）
//   2. fs / プロセス / DLL に一切触れない（＝wasm32 に載る、副作用が無い）
//
// 代償は「import したモジュールのメンバ型が分からない」こと。ただし
// `InferredType::Namespace` の未知メンバは `Unresolved` を返すだけで**エラーにはならない**
// （[type_check/types.rs] の doc 参照）ので、**偽陽性の診断は出ない**。
// 型が付かないだけで、間違った型は付かない。
//
// ⚠ このファイルは `parser/imports/` と**同じ構文**を受理し続けなければならない。
//    受理する構文がずれると、エディタだけが構文エラーを出す（またはその逆）。

use {
    crate::ast::Stmt,
    crate::parser::Parser,
    crate::token::Token,
};

impl Parser {
    /// `import[lang] module.sub as alias` をパースして `Stmt::Import` を返す。
    /// 本体は読み込まない（`body` は常に空）。
    pub(crate) fn parse_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `import` を消費

        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "ar-auto".to_string()
        };

        // cpp-dll / cpp-lib はモジュールパスではなくヘッダパスを取る。
        if lang == "cpp-dll" || lang == "cpp-lib" {
            return self.parse_cpp_import_syntax(lang);
        }

        let module = self.parse_module_path()?;
        // モジュール名の位置はここでしか正しく取れない。この後 `[0.2]` や `as x` を
        // 読むと `prev_pos()` が `]` や別名を指してしまう。
        let module_pos = self.prev_pos();

        // `[version]` — `import[rs] libm[0.2]`（rs のみ）
        let version_present = lang == "rs" && *self.current() == Token::LBracket;
        if version_present {
            let _ = self.parse_version_bracket()?;
        }

        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        // エディタ索引: 束縛される名前（`as` があればその別名、無ければ末尾セグメント）。
        // 位置は最後に読んだ識別子＝まさにその名前を指す。
        #[cfg(feature = "editor")]
        {
            // `as x` があればその別名の位置（＝直前に読んだ識別子）、無ければモジュール名の位置。
            let (bind, pos) = match &alias {
                Some(a) => (a.clone(), self.prev_pos()),
                None => (module.last().cloned().unwrap_or_default(), module_pos),
            };
            let h = self.note_def_at(&bind, crate::parser::editor_hooks::EditorKind::Module, pos);
            let sig = format!("import[{lang}] {}", module.join("."));
            self.note_signature(h, &sig);
        }

        Ok(Stmt::Import {
            lang,
            module,
            with_file: None,
            alias,
            body: Vec::new(),
        })
    }

    /// `from module import[lang] Name1, Name2 as N2` をパースして `Stmt::FromImport` を返す。
    /// 本体は読み込まない（`body` は常に空）。
    pub(crate) fn parse_from_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `from` を消費

        let module = self.parse_module_path()?;

        self.eat(&Token::Import)?;
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "ar-auto".to_string()
        };

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
                let sig = format!("from {} import[{lang}] {name}", module.join("."));
                self.note_signature(h, &sig);
            }
            names.push((name, alias));
            if *self.current() == Token::Comma {
                self.advance();
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

        Ok(Stmt::FromImport {
            lang,
            module,
            with_file: None,
            names,
            body: Vec::new(),
        })
    }

    /// `import[cpp-dll] Dir.Header as alias` の構文だけを読む。
    /// 通常ビルドと違いヘッダファイルは開かないので `with_file` は `None`。
    fn parse_cpp_import_syntax(&mut self, lang: String) -> Result<Stmt, String> {
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

        let alias = if *self.current() == Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };

        Ok(Stmt::Import {
            lang,
            module: parts,
            with_file: None,
            alias,
            body: Vec::new(),
        })
    }

    // ─── 構文ヘルパ（`imports/dispatch.rs` と同一の受理規則）────────────────

    /// `[lang]` トークン列をパースして言語識別子文字列を返す。
    fn parse_lang_bracket(&mut self) -> Result<String, String> {
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
    fn parse_module_path(&mut self) -> Result<Vec<String>, String> {
        let mut segments = vec![self.expect_ident()?];
        while *self.current() == Token::Dot {
            self.advance();
            segments.push(self.expect_ident()?);
        }
        Ok(segments)
    }

    /// `[version]` を読み飛ばして文字列として返す（`import[rs] libm[0.2]`）。
    fn parse_version_bracket(&mut self) -> Result<String, String> {
        self.eat(&Token::LBracket)?;
        let mut out = String::new();
        // バージョンは `0.2` のように数値・ドット・識別子が混ざる。
        // `]` まで貪欲に読む（受理のみが目的で、値は使わない）。
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            out.push_str(&format!("{}", self.current()));
            self.advance();
        }
        self.eat(&Token::RBracket)?;
        Ok(out)
    }
}
