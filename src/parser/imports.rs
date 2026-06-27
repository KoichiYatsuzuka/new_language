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

/// ar_config.json の JSON テキストから `python.search_paths` を取り出す。
/// 依存クレートなしの簡易パーサー（正規表現・serde 不使用）。
fn parse_python_search_paths(json: &str, base: &std::path::Path) -> Vec<std::path::PathBuf> {
    // "python" キーを探す
    let python_key = "\"python\"";
    let start = match json.find(python_key) {
        Some(s) => s,
        None => return vec![],
    };
    // "search_paths" キーを python オブジェクト内で探す
    let after_python = &json[start + python_key.len()..];
    let key = "\"search_paths\"";
    let key_start = match after_python.find(key) {
        Some(s) => s,
        None => return vec![],
    };
    let after_key = &after_python[key_start + key.len()..];
    let arr_start = match after_key.find('[') {
        Some(s) => s,
        None => return vec![],
    };
    let arr_end = match after_key[arr_start..].find(']') {
        Some(s) => s,
        None => return vec![],
    };
    let arr = &after_key[arr_start + 1..arr_start + arr_end];
    let mut paths = Vec::new();
    for part in arr.split(',') {
        let trimmed = part.trim().trim_matches('"');
        if !trimmed.is_empty() {
            let p = std::path::PathBuf::from(trimmed);
            paths.push(if p.is_absolute() { p } else { base.join(p) });
        }
    }
    paths
}

/// Python プロセスを実行して標準ライブラリと site-packages のパスを取得する。
/// OnceLock でキャッシュするので初回のみサブプロセスが起動する。
fn python_lib_dirs() -> &'static Vec<std::path::PathBuf> {
    use std::sync::OnceLock;
    static DIRS: OnceLock<Vec<std::path::PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let script = concat!(
            "import sysconfig; ",
            "paths = [sysconfig.get_path('stdlib'), sysconfig.get_path('purelib')]; ",
            "print('\\n'.join(p for p in paths if p))"
        );
        #[cfg(windows)]
        let candidates = ["py", "python", "python3"];
        #[cfg(not(windows))]
        let candidates = ["python3", "python"];
        for exe in candidates {
            let Ok(out) = std::process::Command::new(exe)
                .args(["-c", script])
                .output()
            else { continue };
            if !out.status.success() { continue; }
            if let Ok(s) = String::from_utf8(out.stdout) {
                let dirs: Vec<std::path::PathBuf> = s
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(std::path::PathBuf::from)
                    .collect();
                if !dirs.is_empty() {
                    return dirs;
                }
            }
        }
        vec![]
    })
}

// ─── Python 型スタブ抽出 ───────────────────────────────────────────────────────

/// Python ソースから型スタブを直接抽出して `Stmt::Let(name, type_ann, Expr::None)` のリストを返す。
///
/// - トップレベルの `def` / `async def`:
///   アノテーション付き → `function->ReturnType`、なし → ボディ推論（フォールバックは `Any`）
/// - トップレベルの `class`:
///   `function->ClassName`（呼び出すとインスタンスを返す型として扱う）
/// - 複数行シグネチャ（ブラケット深さを追跡）に対応
/// - `_` で始まるプライベート関数は除外
fn extract_py_type_stubs(source: &str) -> Vec<crate::ast::Stmt> {
    let lines: Vec<&str> = source.lines().collect();
    let mut stmts: Vec<crate::ast::Stmt> = Vec::new();
    let mut i = 0usize;
    // (class_name, class_indent) — None のときはトップレベル
    let mut current_class: Option<(String, usize)> = None;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let trimmed = trimmed.trim_end();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            i += 1;
            continue;
        }

        let line_indent = py_line_indent(line);

        // クラス内 → インデントが下がったらトップレベルに戻る
        if let Some((_, class_indent)) = &current_class {
            if line_indent <= *class_indent {
                current_class = None;
            }
        }

        // トップレベルの class 定義
        if line_indent == 0 {
            if let Some(class_name) = py_parse_class_name(trimmed) {
                current_class = Some((class_name.clone(), 0));
                stmts.push(crate::ast::Stmt::Let(
                    class_name.clone(),
                    Some(format!("function->{}", class_name)),
                    crate::ast::Expr::None,
                ));
                i += 1;
                continue;
            }
        }

        // 関数定義（トップレベルのみ対象）
        if current_class.is_none()
            && line_indent == 0
            && (trimmed.starts_with("def ") || trimmed.starts_with("async def "))
        {
            if let Some((name, ret_ann, body_line, func_indent)) =
                py_parse_func_def(&lines, i)
            {
                if !name.starts_with('_') {
                    let arrow_ret = if let Some(ann) = ret_ann {
                        py_type_to_arrow(&ann)
                    } else {
                        py_infer_body_ret(&lines, body_line, func_indent)
                    };
                    stmts.push(crate::ast::Stmt::Let(
                        name,
                        Some(format!("function->{}", arrow_ret)),
                        crate::ast::Expr::None,
                    ));
                }
                i = body_line;
                continue;
            }
        }

        i += 1;
    }

    stmts
}

/// 行の先頭インデント幅（スペース/タブ数）を返す。
fn py_line_indent(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// `class Foo` や `class Foo(Base):` からクラス名を取り出す。
fn py_parse_class_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("class ")?;
    let name_end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// `def` / `async def` をパースして `(名前, 戻り値アノテーション, body_start_idx, func_indent)` を返す。
/// 複数行シグネチャはブラケット深さを追跡して結合する（最大 40 行）。
fn py_parse_func_def(
    lines: &[&str],
    start: usize,
) -> Option<(String, Option<String>, usize, usize)> {
    let start_line = lines[start];
    let func_indent = py_line_indent(start_line);
    let trimmed = start_line.trim_start();

    // 関数名を取り出す
    let after_def = trimmed
        .strip_prefix("async def ")
        .or_else(|| trimmed.strip_prefix("def "))?;
    let paren = after_def.find('(')?;
    let name = after_def[..paren].trim().to_string();
    if name.is_empty() { return None; }

    // 複数行シグネチャを1行に結合（括弧が閉じるまで）
    let mut depth = 0i32;
    let mut combined = String::new();
    let mut body_line = start + 1;
    for (j, &line) in lines[start..].iter().enumerate().take(40) {
        let raw = py_strip_comment(line);
        if j > 0 { combined.push(' '); }
        combined.push_str(raw.trim());
        for ch in raw.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            body_line = start + j + 1;
            break;
        }
    }

    // 閉じ括弧の位置を探す
    let open = combined.find('(')?;
    let mut pd = 0i32;
    let mut close = None;
    for (k, ch) in combined[open..].char_indices() {
        match ch {
            '(' => pd += 1,
            ')' => {
                pd -= 1;
                if pd <= 0 { close = Some(open + k); break; }
            }
            _ => {}
        }
    }
    let close = close?;
    let after = combined[close + 1..].trim();

    // `-> ReturnType:` の形式から戻り値アノテーションを取り出す
    let ret_ann = if let Some(s) = after.strip_prefix("->") {
        let s = s.trim().trim_end_matches(':').trim();
        if s.is_empty() { None } else { Some(s.to_string()) }
    } else {
        None
    };

    Some((name, ret_ann, body_line, func_indent))
}

/// 行中のコメント（`#` 以降）を除去する簡易実装（文字列リテラル内の `#` は無視）。
fn py_strip_comment(line: &str) -> &str {
    let mut in_str = false;
    let mut str_char = b' ';
    for (i, &b) in line.as_bytes().iter().enumerate() {
        if in_str {
            if b == str_char { in_str = false; }
        } else {
            if b == b'#' { return &line[..i]; }
            if b == b'"' || b == b'\'' { in_str = true; str_char = b; }
        }
    }
    line
}

/// 関数ボディをスキャンして `return` 文から戻り値型を推論する。
/// ネストした `def` / `class` はスキップ。`yield` があれば即 `"Any"` を返す。
fn py_infer_body_ret(lines: &[&str], body_start: usize, func_indent: usize) -> String {
    let mut types: Vec<&'static str> = Vec::new();
    let mut has_return = false;
    let mut i = body_start;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { i += 1; continue; }

        let indent = py_line_indent(line);
        if indent <= func_indent { break; }

        // ネストされた def/class をスキップ
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") || trimmed.starts_with("class ") {
            let nested_indent = indent;
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if !t.is_empty() && py_line_indent(lines[i]) <= nested_indent { break; }
                i += 1;
            }
            continue;
        }

        if trimmed.starts_with("yield ") || trimmed == "yield" { return "Any".to_string(); }

        if let Some(rest) = trimmed.strip_prefix("return") {
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                has_return = true;
                types.push(py_literal_type(rest.trim()));
            }
        }

        i += 1;
    }

    if !has_return { return "None".to_string(); }
    let non_none: Vec<_> = types.iter().filter(|&&t| t != "None").cloned().collect();
    if non_none.is_empty() { return "None".to_string(); }
    if types.len() == 1 { return types[0].to_string(); }
    "Any".to_string()
}

/// Python のリテラル式から Arrow の型名を推論する。
fn py_literal_type(expr: &str) -> &'static str {
    let e = expr.trim();
    if e.is_empty() || e == "None" { return "None"; }
    if e == "True" || e == "False" { return "bool"; }
    if e.starts_with('"') || e.starts_with('\'') || e.starts_with("b\"") || e.starts_with("b'")
        || e.starts_with("f\"") || e.starts_with("f'")
    {
        return "str";
    }
    let numeric: String = e.chars().filter(|&c| c != '_').collect();
    if numeric.parse::<i64>().is_ok() { return "int"; }
    if numeric.parse::<f64>().is_ok() { return "float"; }
    if e.starts_with('[') { return "list"; }
    if e.starts_with('(') { return "tuple"; }
    if e == "{}" || (e.starts_with('{') && e.contains(':')) { return "dict"; }
    if e.starts_with('{') { return "set"; }
    "Any"
}

/// Python 型アノテーション文字列を Arrow の `from_ann` が解釈できる文字列に変換する。
fn py_type_to_arrow(ann: &str) -> String {
    let a = ann.trim();
    // Optional[T] → Option[T]
    if let Some(inner) = a.strip_prefix("Optional[").and_then(|s| s.strip_suffix(']')) {
        return format!("Option[{}]", py_type_to_arrow(inner));
    }
    // List[T] → list (Arrow は List 型パラメータなし)
    if let Some(inner) = a.strip_prefix("List[").and_then(|s| s.strip_suffix(']')) {
        return format!("list[{}]", py_type_to_arrow(inner));
    }
    // Dict → dict
    if a.starts_with("Dict[") { return "dict".to_string(); }
    // Tuple → tuple
    if a.starts_with("Tuple[") { return "tuple".to_string(); }
    // プリミティブ型はそのまま
    match a {
        "str" | "int" | "float" | "bool" | "None" | "Any" | "bytes"
        | "list" | "dict" | "tuple" | "set" => a.to_string(),
        // Union[...] はそのまま（from_ann で処理）
        _ if a.starts_with("Union[") => a.to_string(),
        // 大文字始まりのクラス名はそのまま（NamedInstance として解釈される）
        _ if a.chars().next().map_or(false, |c| c.is_ascii_uppercase())
            && a.chars().all(|c| c.is_alphanumeric() || c == '_') =>
        {
            a.to_string()
        }
        // 不明・複雑な型 → Any
        _ => "Any".to_string(),
    }
}

// ─── C 型スタブ ───────────────────────────────────────────────────────────────

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
            // import[js-proc]: Node.js IPC サブプロセス — .ars スタブが存在すれば読み込む
            "js-proc" => self.load_js_module(module),
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
    /// python_search_dirs() の順に .py または __init__.py を探す。
    fn load_python_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let rel_py   = module_base.with_extension("py");
        let rel_init = module_base.join("__init__.py");
        let search_dirs = self.python_search_dirs();

        // 検索ディレクトリを順に試して最初に見つかった .py / __init__.py を使う
        let abs_path = search_dirs
            .iter()
            .flat_map(|d| [d.join(&rel_py), d.join(&rel_init)])
            .find(|p| p.exists())
            .ok_or_else(|| {
                let looked = search_dirs
                    .iter()
                    .flat_map(|d| [d.join(&rel_py), d.join(&rel_init)])
                    .map(|p| format!("'{}'", p.display()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("cannot find module '{}' (looked at {})", module.join("."), looked)
            })?;

        let cache_key = ("py".to_string(), abs_path.clone());

        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        if self.loading.contains(&abs_path) {
            return Err(format!("circular import detected: '{}'", abs_path.display()));
        }

        let source = std::fs::read_to_string(&abs_path)
            .map_err(|e| format!("cannot read '{}': {e}", abs_path.display()))?;

        self.loading.insert(abs_path.clone());

        let filename = abs_path.to_string_lossy().to_string();
        let body = python_converter::convert_python_source(&source, &filename)?;

        self.loading.remove(&abs_path);
        self.module_cache.insert(cache_key, body.clone());

        Ok(body)
    }

    /// `import[py-int]` 用: .pyi を優先して検索し、なければ .py にフォールバックする。
    /// `__init__.pyi` / `__init__.py` も検索対象に含める。
    /// body は型検査専用（実行時は PyO3 経由で別ロジックが動く）。
    fn load_python_interface_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let module_base: PathBuf = module.iter().collect();
        let search_dirs = self.python_search_dirs();

        // 候補パスを生成: module.pyi, module/__init__.pyi, module.py, module/__init__.py
        let candidates: Vec<(PathBuf, bool)> = {
            let mut v = Vec::new();
            for dir in &search_dirs {
                v.push((dir.join(module_base.with_extension("pyi")), true));
                v.push((dir.join(module_base.join("__init__.pyi")), true));
            }
            for dir in &search_dirs {
                v.push((dir.join(module_base.with_extension("py")), false));
                v.push((dir.join(module_base.join("__init__.py")), false));
            }
            v
        };

        for (abs_path, is_pyi) in candidates {
            if !abs_path.exists() { continue; }
            return self.load_py_type_body(module, &abs_path, is_pyi);
        }

        // 見つからなければ空の body を返す（型検査スキップ、実行時は PyO3 が担当）
        Ok(vec![])
    }

    /// Python ソースファイルから型検査用の body を生成する。
    ///
    /// - `.pyi` ファイル: `python_converter` でベストエフォート変換 + スタブで補完
    /// - `.py` ファイル: `extract_py_type_stubs` でシグネチャを直接抽出（`python_converter` は使わない）
    fn load_py_type_body(
        &mut self,
        module: &[String],
        abs_path: &PathBuf,
        is_pyi: bool,
    ) -> Result<Vec<Stmt>, String> {
        let cache_key = ("py-int".to_string(), abs_path.clone());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }
        if self.loading.contains(abs_path) {
            return Ok(vec![]);
        }
        let source = std::fs::read_to_string(abs_path).map_err(|_| {
            format!("cannot read interface file for module '{}'", module.join("."))
        })?;
        self.loading.insert(abs_path.clone());

        let body = if is_pyi {
            // .pyi: python_converter でベストエフォート変換
            let filename = abs_path.to_string_lossy().to_string();
            let mut converted = python_converter::convert_python_source(&source, &filename)
                .unwrap_or_default();
            // スタブで不足を補完（変換できなかった関数を追加）
            let known: std::collections::HashSet<String> = converted
                .iter()
                .filter_map(|s| if let Stmt::FnDef { name, .. } = s { Some(name.clone()) } else { None })
                .collect();
            for stub in extract_py_type_stubs(&source) {
                if let Stmt::Let(ref name, _, _) = stub {
                    if !known.contains(name.as_str()) {
                        converted.push(stub);
                    }
                }
            }
            converted
        } else {
            // .py: 直接スタブ抽出（python_converter は複雑な構文に対応できないため使わない）
            extract_py_type_stubs(&source)
        };

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
        // 単一セグメント "name" の場合は source_dir/name/name.dll も試す（パッケージディレクトリ規約）。
        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("dll");
        let mut candidates: Vec<PathBuf> = vec![
            self.source_dir.join(&sub_path),
            self.source_dir.join(&dll_name),
            self.root_dir.join(&dll_name),
        ];
        if module.len() == 1 {
            // import[cs-dll] foo → also try source_dir/foo/foo.dll
            candidates.push(self.source_dir.join(&last).join(&dll_name));
            candidates.push(self.root_dir.join(&last).join(&dll_name));
        }

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

    /// `import[js-proc]` — .ars スタブファイルが存在すれば読み込み、なければ空スタブを返す。
    ///
    /// スタブ検索順:
    ///   1. source_dir / path/to/module.ars
    ///   2. root_dir   / path/to/module.ars
    ///
    /// スタブが見つからない場合は空 body を返す（型なし・実行時にブリッジが関数リストを提供）。
    fn load_js_module(&mut self, module: &[String]) -> Result<Vec<Stmt>, String> {
        let cache_key = ("js-proc".to_string(), module.iter().collect::<PathBuf>());
        if let Some(body) = self.module_cache.get(&cache_key) {
            return Ok(body.clone());
        }

        let sub_path: PathBuf = module.iter().collect::<PathBuf>().with_extension("ars");
        let candidates = [
            self.source_dir.join(&sub_path),
            self.root_dir.join(&sub_path),
        ];

        let body = candidates.iter().find_map(|p| -> Option<Vec<Stmt>> {
            if !p.exists() { return None; }
            let src = std::fs::read_to_string(p).ok()?;
            let filename = p.to_string_lossy().to_string();
            let module_dir = p.parent().map(|d| d.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let tokens = lexer::Lexer::new(&src, filename.as_str()).tokenize();
            let mut sub = Parser::new(tokens, Some(module_dir));
            sub.module_cache = self.module_cache.clone();
            sub.loading     = self.loading.clone();
            sub.root_dir    = self.root_dir.clone();
            sub.parse_program().ok()
        }).unwrap_or_default();

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
    /// source_dir を先頭に、ar_config.json の python.search_paths、PYTHONPATH 環境変数、Python site-packages を続ける。
    fn python_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.source_dir.clone()];
        // ar_config.json の python.search_paths を追加する（source_dir → root_dir の順に探す）
        let config_search = if self.source_dir == self.root_dir {
            vec![self.source_dir.clone()]
        } else {
            vec![self.source_dir.clone(), self.root_dir.clone()]
        };
        for config_dir in &config_search {
            let cfg_path = config_dir.join("ar_config.json");
            if cfg_path.exists() {
                if let Ok(text) = std::fs::read_to_string(&cfg_path) {
                    for p in parse_python_search_paths(&text, config_dir) {
                        if !dirs.contains(&p) {
                            dirs.push(p);
                        }
                    }
                }
                break;
            }
        }
        if let Ok(pythonpath) = std::env::var("PYTHONPATH") {
            for p in std::env::split_paths(&pythonpath) {
                dirs.push(p);
            }
        }
        // Python インタープリタの sys.prefix から site-packages を推測
        if let Ok(prefix) = std::env::var("PYTHONHOME") {
            dirs.push(PathBuf::from(&prefix).join("Lib").join("site-packages"));
        }
        // Python プロセスから標準ライブラリと site-packages のパスを取得して追加
        for p in python_lib_dirs() {
            if !dirs.contains(p) {
                dirs.push(p.clone());
            }
        }
        dirs
    }
}

