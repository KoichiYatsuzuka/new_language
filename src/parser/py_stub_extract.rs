// parser/py_stub_extract.rs — Python ソース（`.py` / `.pyi`）から型スタブを**行ベース**で抽出する。
//
// # なぜ独立したモジュールなのか
//
// 元は `parser/imports/mod.rs` の中にあったが、そこは `editor` feature では
// 拡張用の構文だけを読む実装（`imports_editor.rs`・削除済み）に差し替えられて**丸ごと消える**
// モジュールだった。そのため VS Code 拡張（wasm）からも使えるようにここへ出した。
// （今は拡張も `parser/imports/` を使う・editor_import_resolution_plan.md 3-2。）
//
// ⚠ ここは 1 行ずつ見るだけの近似抽出で、`python_converter`（＝本物の Python パーサ）は使わない
//    （`python_converter` が取りこぼした宣言を補う側）。⚠ 以前は「rustpython は wasm32 に載らない」
//    ことも理由だったが、載ることを確かめた（2-2・拡張の crate も `python_converter` を取り込む）
//    （`crates/arrow-frontend/Cargo.toml` の「ネイティブ依存を足さないこと」）。
//
// ⚠ CLI の `.pyi` 経路は **`python_converter` の結果をここで補完する**形（`py_modules.rs`）。
//    エディタは補完側だけを使うので、**取れる型は CLI の部分集合**になる。
//    ⇒ 型が少なく出ることはあっても、CLI と違う型が出ることはない
//    （`compare_wasm_frontend.ps1` が守る不変条件と同じ向き）。


/// Python ソースから型スタブを直接抽出して `Stmt::Let(name, type_ann, Expr::None)` のリストを返す。
///
/// - トップレベルの `def` / `async def`:
///   アノテーション付き → `function->ReturnType`、なし → ボディ推論（フォールバックは `Any`）
/// - トップレベルの `class`:
///   **`function->Any`**（呼び出すと `Any` を返す）
///
/// ⚠⚠ **クラス名を戻り値型にしてはいけない**（タスク 7.8）。以前は `function->ClassName` と
/// していたが、そのクラスは **Arrow のクラスではない**（レジストリにメンバーが無い）。
/// 名前だけあってメンバーが空の `NamedInstance` になり、**嘘の型**が下流へ流れていた:
///
/// ```text
/// let h = pmod.Holder(1)
/// let s: str = h.n        # 旧: 黙って通る（Python の int が str 変数に入る）
/// ```
///
/// ⚠ `Any` は Arrow では「何でも通る」ではなく「**明示ダウンキャストを要求する**」。
/// だから `Any` にすることが**型検査を走らせる**ことになる。
/// ⚠ 同じモジュールの**関数**と**モジュール変数**は既に `Any` になっており、
/// クラス経路だけが規則から漏れていた（実測）。
///
/// ⚠⚠ **`Unresolved` にしてはいけない。** `type_check/stmt/check.rs` の `Stmt::Import` に
/// 先人の実測記録がある: `editor` ビルドで未知メンバを `Any` にすると
/// `OperationOnAny` の偽陽性が出る。2 つの「判らない」を混同しないこと:
/// **解決を試みていない**（editor・未読込）は `Unresolved`、
/// **解決を試みたが外部言語なので判らない**は `Any`。ここは後者。
/// - 複数行シグネチャ（ブラケット深さを追跡）に対応
/// - `_` で始まるプライベート関数は除外
pub(crate) fn extract_py_type_stubs(source: &str) -> Vec<crate::ast::Stmt> {
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
                // ⚠⚠ 戻り値は `Any`（doc の「クラス名を戻り値型にしてはいけない」を参照）。
                //    ⚠ スタブ（`.ars`）を読んだ C# / JS のクラスは**通常の Arrow 宣言**として
                //      別経路で登録されるので、ここを通らない。触ってよいのは
                //      「メンバーが判らない Python のクラス」だけ。
                stmts.push(crate::ast::Stmt::Let(
                    class_name.clone(),
                    Some("function->Any".to_string()),
                    crate::ast::Expr::None,
                    crate::token::Span::unknown(),
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
                        crate::token::Span::unknown(),
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

    // `-> ReturnType:` の形式から戻り値アノテーションを取り出す。
    //
    // ⚠⚠ **シグネチャと同じ行に本体が続く形がある。** `.pyi` の標準形は
    //    `def sqrt(x: float) -> float: ...` で、以前は行末の `:` だけを
    //    `trim_end_matches(':')` で削っていたため、戻り値型が `"float: ..."` になり
    //    `py_type_to_arrow` の catch-all で **`Any` に落ちていた**
    //    （＝同梱スタブを整備しても型予測が効かない）。
    // ⇒ **深さ 0 の `:` まで**を型として切り出す。`-> dict[str, int]: ...` の
    //    ように型の中に `:` が入る形があるので、行末からではなく前から探す。
    let ret_ann = if let Some(s) = after.strip_prefix("->") {
        let s = s.trim();
        let mut depth = 0i32;
        let mut end = s.len();
        for (k, ch) in s.char_indices() {
            match ch {
                '[' | '(' | '{' => depth += 1,
                ']' | ')' | '}' => depth -= 1,
                ':' if depth <= 0 => {
                    end = k;
                    break;
                }
                _ => {}
            }
        }
        let s = s[..end].trim();
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
///
/// ⚠ 角括弧の中は入れ子を数えて区切る（ffi_value_exchange_plan.md 7-8）。以前は前方一致で処理していて、`dict[K, V]` /
///   `tuple[...]` は要素型を捨て（`set[tuple[int, int]]` → `set[tuple]`）、`frozenset[T]` は `Any`、`Union[...]` は中を変換
///   しないまま、角括弧を含む `X | None` は `Any` になっていた。要素型が落ちると境界の照合（`ffi_boundary`）が要素を見ない。
/// ⚠ Arrow が表せない型（`Sequence[T]` / `Callable[...]` / `Literal[...]` / 可変長の `tuple[int, ...]` の要素など）は
///   検査しない側（`Any` / 素の `tuple`）に倒す（Python の注釈は強制されないので、狭く読むと正しい値を拒む）。
fn py_type_to_arrow(ann: &str) -> String {
    let a = ann.trim();
    // PEP 604: 最上位（角括弧の外）の `|`
    let alternatives = split_top_level(a, '|');
    if alternatives.len() > 1 {
        return union_of(alternatives.iter().map(|t| py_type_to_arrow(t)).collect());
    }
    // ジェネリクス `Base[args]`（`typing.List[int]` の `typing.` は落とす）
    if let Some((base, inner)) = a.strip_suffix(']').and_then(|s| s.split_once('[')) {
        let base = base.trim().rsplit('.').next().unwrap_or("").trim();
        let raw_args = split_top_level(inner, ',');
        let args: Vec<String> = raw_args.iter().map(|t| py_type_to_arrow(t)).collect();
        return match (base, args.as_slice()) {
            ("Optional", [t]) => union_of(vec![t.clone(), "None".to_string()]),
            ("Union", ts) if !ts.is_empty() => union_of(ts.to_vec()),
            ("List" | "list", [t]) => format!("list[{t}]"),
            // frozenset も実行時は Arrow の set で届く（`py_to_tl`・2-6）
            ("Set" | "set" | "FrozenSet" | "frozenset", [t]) => format!("set[{t}]"),
            ("Dict" | "dict", [k, v]) => format!("dict[{k}, {v}]"),
            // 可変長（`tuple[int, ...]`）と空（`tuple[()]`）は要素の並びを Arrow で書けないので素の `tuple`（形だけ照合）
            ("Tuple" | "tuple", _) if raw_args.iter().any(|t| matches!(t.trim(), "..." | "()")) => "tuple".to_string(),
            ("Tuple" | "tuple", ts) if !ts.is_empty() => format!("tuple[{}]", ts.join(", ")),
            // 知らないジェネリクス（`Sequence[T]` / `Callable[...]` / 利用者のクラス `Box[int]` など）は検査しない
            _ => "Any".to_string(),
        };
    }
    match a {
        "str" | "int" | "float" | "bool" | "None" | "Any" | "bytes"
        | "list" | "dict" | "tuple" | "set" => a.to_string(),
        "frozenset" => "set".to_string(),
        // Python のバイト列はどれも Arrow の bytes として受け取る（ffi_value_exchange_plan.md 5-3）。要素が 1 バイトでない
        // memoryview は Python のオブジェクトのまま届き、境界の照合で FfiTypeError になる
        "bytearray" | "memoryview" => "bytes".to_string(),
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

/// 型の選択肢を Arrow の型にする: `None` との 2 択は `Option[T]`、`Any` を含めば `Any`（どの値も入る）。
fn union_of(types: Vec<String>) -> String {
    if types.iter().any(|t| t == "Any") {
        return "Any".to_string();
    }
    let others: Vec<&String> = types.iter().filter(|t| *t != "None").collect();
    if types.len() == 2 && others.len() == 1 {
        return format!("Option[{}]", others[0]);
    }
    format!("Union[{}]", types.join(", "))
}

/// `sep` で区切る。角括弧・丸括弧の中の `sep` では区切らない（`dict[str, tuple[int, int]]` の中の `,` など）。
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Python のバイト列の注釈はどれも `bytes`（ffi_value_exchange_plan.md 5-3）。
    #[test]
    fn byte_sequences_are_bytes() {
        assert_eq!(py_type_to_arrow("bytes"), "bytes");
        assert_eq!(py_type_to_arrow("bytearray"), "bytes");
        assert_eq!(py_type_to_arrow("Optional[memoryview]"), "Option[bytes]");
        assert_eq!(py_type_to_arrow("list[bytearray]"), "list[bytes]");
    }

    /// 要素型を落とさない（7-8）。以前は `dict[K, V]` → `dict`・`set[tuple[int, int]]` → `set[tuple]`・`frozenset[T]` → `Any`・
    /// `Union[List[int], None]` は中を変換しないまま・`list[int] | None` → `Any` だった。
    #[test]
    fn element_types_are_kept() {
        assert_eq!(py_type_to_arrow("set[tuple[int, int]]"), "set[tuple[int, int]]");
        assert_eq!(py_type_to_arrow("frozenset[str]"), "set[str]");
        assert_eq!(py_type_to_arrow("FrozenSet[int]"), "set[int]");
        assert_eq!(py_type_to_arrow("dict[str, list[int]]"), "dict[str, list[int]]");
        assert_eq!(py_type_to_arrow("Dict[str, Tuple[int, float]]"), "dict[str, tuple[int, float]]");
        assert_eq!(py_type_to_arrow("Union[List[int], None]"), "Option[list[int]]");
        assert_eq!(py_type_to_arrow("Union[int, str, None]"), "Union[int, str, None]");
        assert_eq!(py_type_to_arrow("list[int] | None"), "Option[list[int]]");
        assert_eq!(py_type_to_arrow("int | str"), "Union[int, str]");
        assert_eq!(py_type_to_arrow("typing.Optional[typing.List[str]]"), "Option[list[str]]");
        // Arrow が表せないものは検査しない側
        assert_eq!(py_type_to_arrow("tuple[int, ...]"), "tuple");
        assert_eq!(py_type_to_arrow("Sequence[int]"), "Any");
        assert_eq!(py_type_to_arrow("Callable[[int], str]"), "Any");
        assert_eq!(py_type_to_arrow("Optional[Any]"), "Any");
        assert_eq!(py_type_to_arrow("Person"), "Person");
    }
}
