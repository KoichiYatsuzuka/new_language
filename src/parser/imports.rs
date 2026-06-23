// imports.rs — import statement parsing and module loading for the tl parser.

use super::Parser;
use crate::ast::{Accessibility, FieldKind, Param, Stmt};
use crate::token::Token;
use crate::lexer;
use crate::python_converter;
use std::path::PathBuf;

/// ar_config.json の JSON テキストから `csharp.lib_paths` を取り出す。
/// 依存クレートなしの簡易パーサー（正規表現・serde 不使用）。
fn parse_cs_lib_paths(json: &str, base: &std::path::Path) -> Option<Vec<std::path::PathBuf>> {
    // "lib_paths" キーを探す
    let key = "\"lib_paths\"";
    let start = json.find(key)?;
    let after_key = &json[start + key.len()..];
    let arr_start = after_key.find('[')?;
    let arr_end = after_key[arr_start..].find(']')?;
    let arr = &after_key[arr_start + 1..arr_start + arr_end];
    let mut paths = Vec::new();
    for part in arr.split(',') {
        let trimmed = part.trim().trim_matches('"');
        if !trimmed.is_empty() {
            let p = std::path::PathBuf::from(trimmed);
            paths.push(if p.is_absolute() { p } else { base.join(p) });
        }
    }
    if paths.is_empty() { None } else { Some(paths) }
}

/// C 型を対応する tl プリミティブ型名に変換する（静的型検査スタブ生成用）。
///
/// `CType::Void` → `"None"`, `CType::Bool` → `"bool"`,
/// `CType::Float` / `CType::Double` → `"float"`, `CType::CharPtr` → `"str"`,
/// その他の整数・ポインタ型 → `"int"`, `CType::FnPtr` → `"function"` として扱う。
fn ctype_to_tl_str(ct: &crate::interpreter::cpp_bridge::CType) -> &'static str {
    use crate::interpreter::cpp_bridge::CType;
    match ct {
        CType::Void => "None",
        CType::Bool => "bool",
        CType::Float | CType::Double => "float",
        CType::CharPtr => "str",
        // int / long / opaque pointer / mutable pointer → すべて int として扱う
        CType::Int
        | CType::Long
        | CType::VoidPtr
        | CType::OpaqueStructPtr { .. }
        | CType::ByValueStruct { .. }
        | CType::Ptr { .. } => "int",
        CType::FnPtr => "function",
    }
}

impl Parser {
    /// `import[lang] module.sub as alias` をパースして `Stmt::Import` を返す。
    ///
    /// - `import[py] math as m`
    /// - `import[py] os.path as p`
    pub(super) fn parse_import_stmt(&mut self) -> Result<Stmt, String> {
        self.advance(); // `import` を消費

        // `[lang]` を読む。省略時は "tl-auto" (auto-select: prefer .arc over .ar)
        let lang = if *self.current() == Token::LBracket {
            self.parse_lang_bracket()?
        } else {
            "tl-auto".to_string()
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

    /// `import[cpp-lib] Dir.Name as alias` をパースする。
    /// `import[cpp-dll] Dir.Name as alias` も同様。
    ///
    /// ドット区切り識別子をヘッダファイルパスに解決する:
    ///   `DxLib.DxLib` → `{source_dir}/DxLib/DxLib.h`
    ///   最後のコンポーネントに `.h` 拡張子が付く。
    ///
    /// ヘッダが存在する場合は静的型情報として Stmt::FnDef スタブを body に積む。
    fn parse_cpp_import(&mut self, lang: String) -> Result<Stmt, String> {
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
        let body = std::fs::read_to_string(&resolved)
            .ok()
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
                            type_ann: ctype_to_tl_str(fct).to_string(),
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
                    let ret_str = ctype_to_tl_str(&sig.ret).to_string();
                    let params = sig
                        .params
                        .into_iter()
                        .map(|(pname, ct)| {
                            let mutable = matches!(&ct, CType::Ptr { mutable: true, .. });
                            Param {
                                name: pname,
                                mutable,
                                type_ann: Some(ctype_to_tl_str(&ct).to_string()),
                                default: None,
                                variadic: false,
                            }
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
    pub(super) fn parse_from_import_stmt(&mut self) -> Result<Stmt, String> {
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
    /// 例: `os.path` → `["os", "path"]`
    fn parse_module_path(&mut self) -> Result<Vec<String>, String> {
        let mut segments = vec![self.expect_ident()?];
        while *self.current() == Token::Dot {
            self.advance();
            segments.push(self.expect_ident()?);
        }
        Ok(segments)
    }

    /// モジュールを検索・読み込み・変換して tl AST を返す。キャッシュを使用する。
    fn load_module(
        &mut self,
        lang: &str,
        module: &[String],
        version: Option<&str>,
    ) -> Result<Vec<Stmt>, String> {
        match lang {
            // default (no bracket): prefer .arc, fall back to .ar
            "tl-auto" | "ar-auto" => self.load_tl_module(module),
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
            other => Err(format!("unknown import language '{other}'")),
        }
    }

    /// `import[rs] name[version]` — クレートバインディングをコンパイル・キャッシュし、
    /// 型チェッカ・インタプリタが使う `Stmt::FnDef` スタブを返す。
    /// `version` が `Some` ならそれを直接使用し、`None` なら `ar_crates.json` を参照する。
    fn load_rs_module(
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
    fn parse_version_bracket(&mut self) -> Result<String, String> {
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

    /// `.ar` / `.arc` モジュールをロードして AST を返す。
    ///
    /// 各検索ディレクトリ (`source_dir` → `root_dir`) に対して以下の優先順で試す:
    /// 1. `module.arc`         — コンパイル済みモジュール（埋め込みソース付きバイナリ）
    /// 2. `module.ar`          — ソースファイルモジュール
    /// 3. `module/__init__.ar` — パッケージモジュール
    fn load_tl_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        // 検索ディレクトリリスト（source_dir と root_dir が同じなら重複させない）
        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        // (パス, コンパイル済みか) の候補リスト — .arc が .ar より先になる
        let candidates: Vec<(PathBuf, bool)> = search_dirs
            .iter()
            .flat_map(|dir| {
                [
                    (dir.join(&tlc_rel), true),
                    (dir.join(&file_rel), false),
                    (dir.join(&init_rel), false),
                ]
            })
            .collect();

        let (abs_path, is_compiled) = candidates
            .iter()
            .find(|(p, _)| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|(p, _)| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find module '{}' (looked at {})",
                    module.join("."),
                    paths
                )
            })?;

        let cache_key = ("ar-auto".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        // ソースを取得: .arc はバイナリから埋め込みソースを抽出、.ar は直読み
        let (source, filename) = if is_compiled {
            let (mod_name, src) = crate::partial_compiler::load_tlc(&abs_path)
                .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
            let label = format!("<compiled:{mod_name}>");
            (src, label)
        } else {
            let src = std::fs::read_to_string(&abs_path)
                .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
            (src, abs_path.to_string_lossy().into_owned())
        };

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut sub = Parser::new(tokens, Some(module_dir));
        // 親のキャッシュ・循環検出セット・ルートディレクトリを引き継ぐ
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;

        // 子パーサが生成したキャッシュエントリを親にマージする
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[ar]`: `.ar` ソースのみをロードする。`.arc` があっても無視する。
    fn load_tl_source_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let file_rel = module_base.with_extension("ar");
        let init_rel = module_base.join("__init__.ar");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        let candidates: Vec<PathBuf> = search_dirs
            .iter()
            .flat_map(|dir| [dir.join(&file_rel), dir.join(&init_rel)])
            .collect();

        let abs_path = candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find source module '{}' (looked at {})",
                    module.join("."),
                    paths
                )
            })?;

        let cache_key = ("ar".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        let source = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("cannot read module '{}': {e}", module.join(".")))?;
        let filename = abs_path.to_string_lossy().into_owned();

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[arc]`: `.arc` コンパイル済みモジュールのみをロードする。`.ar` があっても無視する。
    fn load_tlc_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let tlc_rel = module_base.with_extension("arc");

        let search_dirs: Vec<PathBuf> = {
            let a = self.source_dir.clone();
            let b = self.root_dir.clone();
            if a == b { vec![a] } else { vec![a, b] }
        };

        let candidates: Vec<PathBuf> =
            search_dirs.iter().map(|dir| dir.join(&tlc_rel)).collect();

        let abs_path = candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| {
                let paths = candidates
                    .iter()
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "cannot find compiled module '{}' (looked at {}; compile with: cargo run --release -- --compile <source.ar>)",
                    module.join("."), paths
                )
            })?;

        let cache_key = ("arc".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        let (mod_name, source) = crate::partial_compiler::load_tlc(&abs_path)
            .map_err(|e| format!("cannot load compiled module '{}': {e}", module.join(".")))?;
        let filename = format!("<compiled:{mod_name}>");

        self.loading.insert(abs_path.clone());

        let tokens = lexer::Lexer::new(&source, filename.as_str()).tokenize();
        let module_dir = abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut sub = Parser::new(tokens, Some(module_dir));
        sub.module_cache = self.module_cache.clone();
        sub.loading = self.loading.clone();
        sub.root_dir = self.root_dir.clone();

        let body = sub.parse_program()?;
        self.module_cache.extend(sub.module_cache);
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// Python モジュールを検索・変換する（キャッシュ込み）。
    fn load_python_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        // .py ファイルパスを解決する
        let rel_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("py");
        let abs_path = self.source_dir.join(&rel_path);
        let cache_key = ("py".to_string(), abs_path.clone());

        // キャッシュに存在する場合はそのまま返す
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // 循環 import 検出
        if self.loading.contains(&abs_path) {
            return Err(format!(
                "circular import detected: '{}'",
                abs_path.display()
            ));
        }

        // ファイルが存在しない場合はエラー
        let source = std::fs::read_to_string(&abs_path).map_err(|_| {
            format!(
                "cannot find module '{}' (looked at '{}')",
                module.join("."),
                abs_path.display()
            )
        })?;

        // 読み込み中マーカーをセット
        self.loading.insert(abs_path.clone());

        // Python → tl AST 変換
        let filename = abs_path.to_string_lossy().to_string();
        let body = python_converter::convert_python_source(&source, &filename)?;

        // 読み込み中マーカーを解除してキャッシュに登録
        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[py-int]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// body は型検査専用（実行時は PyO3 経由で別ロジックが動く）。
    fn load_python_interface_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let pyi_rel = module_base.with_extension("pyi");
        let py_rel = module_base.with_extension("py");

        // .pyi → .py の順に search_dirs で探す
        let search_dirs = self.python_search_dirs();
        for dir in &search_dirs {
            let pyi_abs = dir.join(&pyi_rel);
            if pyi_abs.exists() {
                return self.load_pyi_file(module, &pyi_abs);
            }
        }
        for dir in &search_dirs {
            let py_abs = dir.join(&py_rel);
            if py_abs.exists() {
                return self.load_pyi_file(module, &py_abs);
            }
        }

        // 見つからなければ空の body を返す（型検査スキップ、実行時は PyO3 が担当）
        Ok(vec![])
    }

    /// .pyi または .py ファイルをパースして型検査用 body を返す。
    /// 変換エラーが発生したステートメントは無視する（ベストエフォート）。
    fn load_pyi_file(
        &mut self,
        module: &[String],
        abs_path: &PathBuf,
    ) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py-int".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        let source = std::fs::read_to_string(abs_path).map_err(|_| {
            format!(
                "cannot read interface file for module '{}'",
                module.join(".")
            )
        })?;
        self.loading.insert(abs_path.clone());
        let filename = abs_path.to_string_lossy().to_string();
        // 変換エラーは無視して空 body を返す（.pyi は実行不要）
        let body = python_converter::convert_python_source(&source, &filename).unwrap_or_default();
        self.loading.remove(abs_path);
        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// `import[cs-dll]` / `import[cs-proc]` — .NET アセンブリから型スタブを生成する。
    ///
    /// DLL の検索順:
    ///   1. source_dir / path/to/LastSegment.dll
    ///   2. source_dir / LastSegment.dll
    ///   3. root_dir  / LastSegment.dll
    ///   4. ar_config.json の csharp.lib_paths に列挙されたディレクトリ
    ///
    /// DLL が見つからない場合は警告を出して空スタブを返す（型なし・実行時に解決）。
    /// `is_proc` は IPC サブプロセス方式かを示すが、型スタブは両方式で共通。
    fn load_cs_module(&mut self, module: &[String], is_proc: bool) -> Result<Vec<Stmt>, String> {
        let last = module.last().cloned().unwrap_or_default();
        let dll_name = format!("{last}.dll");

        // キャッシュキー
        let cache_key = (if is_proc { "cs-proc" } else { "cs-dll" }.to_string(),
                         std::path::PathBuf::from(&dll_name));
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        // 候補パスを順番に試す
        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("dll");
        let candidates: Vec<PathBuf> = vec![
            self.source_dir.join(&sub_path),
            self.source_dir.join(&dll_name),
            self.root_dir.join(&dll_name),
        ];

        let mut dll_path: Option<PathBuf> = None;
        for c in &candidates {
            if c.exists() {
                dll_path = Some(c.clone());
                break;
            }
        }

        // ar_config.json の csharp.lib_paths も検索
        if dll_path.is_none() {
            if let Some(extra) = self.load_cs_lib_paths() {
                for dir in extra {
                    let p = dir.join(&dll_name);
                    if p.exists() {
                        dll_path = Some(p);
                        break;
                    }
                }
            }
        }

        let body = match dll_path {
            Some(path) => {
                match super::cs_assembly::load_cs_assembly(&path) {
                    Ok(stmts) => stmts,
                    Err(e) => {
                        eprintln!("Warning: import[cs-*]: {e}; falling back to empty stubs");
                        vec![]
                    }
                }
            }
            None => {
                eprintln!(
                    "Warning: import[cs-*]: cannot find '{dll_name}' for module '{}'; \
                     no type stubs available (add the DLL path to ar_config.json csharp.lib_paths)",
                    module.join(".")
                );
                vec![]
            }
        };

        self.module_cache.insert(cache_key, body.clone());
        Ok(body)
    }

    /// ar_config.json の `csharp.lib_paths` を読んでパスリストを返す。
    fn load_cs_lib_paths(&self) -> Option<Vec<PathBuf>> {
        // Walk up from source_dir looking for ar_config.json
        let mut dir = self.source_dir.clone();
        loop {
            let cfg = dir.join("ar_config.json");
            if cfg.exists() {
                if let Ok(text) = std::fs::read_to_string(&cfg) {
                    return parse_cs_lib_paths(&text, &dir);
                }
            }
            if !dir.pop() { break; }
        }
        None
    }

    /// Python モジュールの検索ディレクトリリストを返す。
    /// source_dir を先頭に、PYTHONPATH 環境変数、Python site-packages を続ける。
    fn python_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.source_dir.clone()];
        if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
            for p in std::env::split_paths(&pythonpath) {
                dirs.push(p);
            }
        }
        // Python インタープリタの sys.prefix から site-packages を推測
        if let Ok(prefix) = std::env::var("PYTHONHOME") {
            dirs.push(PathBuf::from(&prefix).join("Lib").join("site-packages"));
        }
        dirs
    }
}

