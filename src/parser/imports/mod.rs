// imports/mod.rs — import 文の解析とモジュール読み込みサブシステムのモジュール束ね。
//
// Python 型スタブ抽出・設定(ar_config.json)解析・C 型→Arrow 型変換などの自由ヘルパーを保持し、
// `impl Parser` の import 解析メソッドを役割別サブモジュール(dispatch/cpp/ar_modules/py_modules/
// cs_js_modules)へ分割して宣言する。

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
    // List[T] → list[T]
    if let Some(inner) = a.strip_prefix("List[").and_then(|s| s.strip_suffix(']')) {
        return format!("list[{}]", py_type_to_arrow(inner));
    }
    // Set[T] → set[T]
    if let Some(inner) = a.strip_prefix("Set[").and_then(|s| s.strip_suffix(']')) {
        return format!("set[{}]", py_type_to_arrow(inner));
    }
    // ── PEP 585: 小文字の組み込みジェネリクス（Python 3.9+）──
    // `typing.List[T]` 等の旧表記だけを見ていると、現代的な `-> list[int]` が
    // 下の catch-all で `Any` に落ち、**スタブが要素型の情報を失う**。
    // そうなると FFI 境界検査（`ffi_boundary`）も `Any` は検査不能として素通しするため、
    // 「スタブを整備するほど検査が効く」という設計が成立しない。ここで拾う。
    if let Some(inner) = a.strip_prefix("list[").and_then(|s| s.strip_suffix(']')) {
        return format!("list[{}]", py_type_to_arrow(inner));
    }
    if let Some(inner) = a.strip_prefix("set[").and_then(|s| s.strip_suffix(']')) {
        return format!("set[{}]", py_type_to_arrow(inner));
    }
    // Dict / dict → dict（キー・値の型は保守的に落とす）
    if a.starts_with("Dict[") || a.starts_with("dict[") { return "dict".to_string(); }
    // Tuple / tuple → tuple
    if a.starts_with("Tuple[") || a.starts_with("tuple[") { return "tuple".to_string(); }
    // ── PEP 604: `X | None` / `X | Y`（Python 3.10+）──
    // 角括弧を含む場合は入れ子の区切りと紛れるため対象外にする（保守的）。
    if !a.contains('[') {
        if let Some((l, r)) = a.split_once('|') {
            let (l, r) = (l.trim(), r.trim());
            if !l.is_empty() && !r.is_empty() {
                return if r == "None" {
                    format!("Option[{}]", py_type_to_arrow(l))
                } else if l == "None" {
                    format!("Option[{}]", py_type_to_arrow(r))
                } else {
                    format!("Union[{}, {}]", py_type_to_arrow(l), py_type_to_arrow(r))
                };
            }
        }
    }
    // プリミティブ型はそのまま
    match a {
        "str" | "int" | "float" | "bool" | "None" | "Any" | "bytes"
        | "list" | "dict" | "tuple" | "set" => a.to_string(),
        // Union[...] はそのまま（from_ann で処理）
        _ if a.starts_with("Union[") => a.to_string(),
        // 大文字始まりのクラス名はそのまま（NamedInstance として解釈される）
        _ if a.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && a.chars().all(|c| c.is_alphanumeric() || c == '_') =>
        {
            a.to_string()
        }
        // 不明・複雑な型 → Any
        _ => "Any".to_string(),
    }
}

// ─── C 型スタブ ───────────────────────────────────────────────────────────────

/// C 型を対応する tl 型名に変換する（静的型検査スタブ生成専用 — 実行時の
/// マーシャリングは `NativeFnRef` 解決時に C シグネチャから別途決まる）。
///
/// - `CType::Void` → `"None"`, `CType::Bool` → `"bool"`,
///   `CType::Float` / `CType::Double` → `"float"`, `CType::CharPtr` → `"str"`,
///   整数・`void*` → `"int"`, `CType::FnPtr` → `"function"`。
/// - プリミティブポインタ（`int*` / `double*` 等）は**ポインティ型**で注釈する
///   （`double*` → `"float"`）。実行時の write-back が書き戻す値型と一致させるため。
/// - 構造体ポインタ・by-value 構造体は `"Any"` とする。名義型（構造体名）で縛ると
///   構造互換な別名クラスのシャドウ変換（`MyVec` → `VECTOR*`、SKILL.md P3）と
///   int ハンドル経路の両方を静的に壊すため。可変性検査（`mut`）は型注釈と
///   独立に `Param::mutable` で機能する。
pub(crate) fn ctype_to_tl_str(ct: &crate::interpreter::cpp_bridge::CType) -> String {
    use crate::interpreter::cpp_bridge::CType;
    match ct {
        CType::Void => "None".to_string(),
        CType::Bool => "bool".to_string(),
        CType::Float | CType::Double => "float".to_string(),
        CType::CharPtr => "str".to_string(),
        CType::Int | CType::Long | CType::VoidPtr => "int".to_string(),
        CType::Ptr { inner, .. } => ctype_to_tl_str(inner),
        CType::OpaqueStructPtr { .. } | CType::ByValueStruct { .. } => "Any".to_string(),
        CType::FnPtr => "function".to_string(),
    }
}


mod dispatch;
mod cpp;
mod ar_modules;
mod py_modules;
mod cs_js_modules;
