// parser/py_import_links.rs — Python のモジュールの import の表（型の再エクスポートをたどるため・pandas_import_plan.md の 1-3）。
//
// `import[py]` の型スタブ（`py_stub_extract`）は 1 つのファイルの宣言しか読まないので、`pandas/__init__.py` の
// `from pandas.core.api import DataFrame` のような**再エクスポート**と、注釈に書いた**よそのモジュールのクラス**
// （`if TYPE_CHECKING:` の中の `from pandas.core.frame import DataFrame`）が型にならず、すべて `Any` になっていた。
// ここは「このモジュールの名前がどのモジュールの何を指すか」だけを集める。たどって宣言を写すのは `py_modules.rs`。
//
// ⚠ 集めるのはモジュールの本体の直下と、その `if` / `try` の中（`if TYPE_CHECKING:`・`try: import x`）だけ。関数・クラスの
//   本体の import はモジュールの名前を束縛しないので集めない。
// ⚠ 行単位で読む（`py_import_links` の doc・速さのため）。

/// あるモジュールの型スタブの元（再エクスポート・注釈のクラス・ほかのファイルの基底をたどるときに、モジュールごとに 1 度だけ作る）。
#[derive(Debug)]
pub(crate) struct PyRawStub {
    /// 絶対のモジュール名（`pandas.core.frame` の `["pandas", "core", "frame"]`）。
    pub(crate) module: Vec<String>,
    /// パッケージ（`__init__`）か（相対の import の起点が変わる）。
    pub(crate) is_package: bool,
    /// このファイルの宣言（`extract_py_type_stubs_raw`・よその名前を `Any` にする前）。
    pub(crate) decls: Vec<crate::ast::Stmt>,
    /// このファイルの import の表。
    pub(crate) links: Vec<PyImportLink>,
    /// ほかのファイルの基底を持つクラスの、その基底の名前（`py_class_outside_bases`）。
    pub(crate) outside_bases: std::collections::HashMap<String, Vec<String>>,
}

/// このモジュールの名前 `local` が指す物。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PyImportLink {
    /// このモジュールで束縛される名前（`as` の別名があればそれ）。
    pub(crate) local: String,
    /// 相対の段数（`from .x import y` は 1・`from ..x` は 2・絶対は 0）。
    pub(crate) level: u32,
    /// 取り込むモジュール（`from a.b import c` の `["a", "b"]`。`from . import c` は空）。
    pub(crate) module: Vec<String>,
    /// 取り込む名前（`from a import c` の `c`）。`import a.b as m` のようにモジュールそのものを束縛するときは `None`。
    pub(crate) name: Option<String>,
}

/// `source` のモジュールの import の表（書いた順）。
///
/// ⚠ **行単位で読む**（`py_stub_extract` と同じやり方）。以前は Python の構文木（rustpython）を作っていたが、pandas を読むと
///   142 ファイルで 357 ms かかった（`frame.py` は 1 万行を超える）。import の文の形は単純なので、行で十分に読める。
/// 拾う: インデント 0 の `import` / `from .. import ..` と、インデント 0 の `if` / `elif` / `else` / `try` / `except` /
/// `finally` の中のもの（`if TYPE_CHECKING:` など）。`def` / `class` の中のもの（それらの中の `if` の中も）は拾わない。
/// 括弧で複数の行にわたる形（`from a import (\n b,\n c,\n)`）と、行末の `\` の続きも読む。
pub(crate) fn py_import_links(source: &str, _filename: &str) -> Vec<PyImportLink> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    // 今いる、インデント 0 の複合文が import を拾う場所か（`if` / `try` など）。`def` / `class` の中は拾わない
    let mut in_conditional = false;
    // 条件の中で見つけた `def` / `class` のインデント（その中は拾わない）
    let mut skip_deeper_than: Option<usize> = None;
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            i += 1;
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            skip_deeper_than = None;
            let head = t.split(|c: char| c.is_whitespace() || c == ':' || c == '(').next().unwrap_or("");
            in_conditional = matches!(head, "if" | "elif" | "else" | "try" | "except" | "finally");
        } else {
            if !in_conditional {
                i += 1;
                continue;
            }
            if let Some(d) = skip_deeper_than {
                if indent > d {
                    i += 1;
                    continue;
                }
                skip_deeper_than = None;
            }
            if t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("class ") || t.starts_with('@') {
                skip_deeper_than = Some(indent);
                i += 1;
                continue;
            }
        }
        if !(t.starts_with("import ") || t.starts_with("from ")) {
            i += 1;
            continue;
        }
        // 文を 1 行にまとめる（括弧が閉じるまで・行末の `\`）
        let mut stmt = strip_comment(t).to_string();
        let mut depth: i32 = stmt.matches('(').count() as i32 - stmt.matches(')').count() as i32;
        while (depth > 0 || stmt.ends_with('\\')) && i + 1 < lines.len() {
            if stmt.ends_with('\\') {
                stmt.pop();
            }
            i += 1;
            let more = strip_comment(lines[i].trim());
            depth += more.matches('(').count() as i32 - more.matches(')').count() as i32;
            stmt.push(' ');
            stmt.push_str(more);
        }
        parse_import(&stmt, &mut out);
        i += 1;
    }
    out
}

/// 行のコメント（`#` から後ろ）を落とす。⚠ import の文には文字列が出てこないので、引用符は見ない。
fn strip_comment(t: &str) -> &str {
    t.split('#').next().unwrap_or("").trim_end()
}

/// 1 つの import の文（1 行にまとめたもの）を表に足す。
fn parse_import(stmt: &str, out: &mut Vec<PyImportLink>) {
    let words = |s: &str| s.split(',').map(|p| p.trim().trim_matches(|c| c == '(' || c == ')').trim().to_string()).filter(|p| !p.is_empty()).collect::<Vec<_>>();
    if let Some(rest) = stmt.strip_prefix("import ") {
        for item in words(rest) {
            let (target, alias) = split_as(&item);
            let module: Vec<String> = target.split('.').map(str::to_string).collect();
            if module.iter().any(|m| m.is_empty()) {
                continue;
            }
            // `import a.b` は `a` を束縛し、`import a.b as m` は `m` に `a.b` を束縛する（CPython）
            let (local, module) = match alias {
                Some(a) => (a, module),
                None => (module[0].clone(), vec![module[0].clone()]),
            };
            out.push(PyImportLink { local, level: 0, module, name: None });
        }
        return;
    }
    let Some(rest) = stmt.strip_prefix("from ") else { return };
    let Some((src, names)) = rest.split_once(" import ") else { return };
    let src = src.trim();
    let level = src.chars().take_while(|c| *c == '.').count() as u32;
    let module: Vec<String> =
        src[level as usize..].split('.').filter(|m| !m.is_empty()).map(str::to_string).collect();
    for item in words(names) {
        let (name, alias) = split_as(&item);
        // ⚠ `from x import *` は名前が分からないので表に入れない（その名前は `Any`）
        if name == "*" || name.is_empty() {
            continue;
        }
        let local = alias.unwrap_or_else(|| name.clone());
        out.push(PyImportLink { local, level, module: module.clone(), name: Some(name) });
    }
}

/// `x as y` を（`x`, `Some(y)`）に分ける。
fn split_as(item: &str) -> (String, Option<String>) {
    match item.split_once(" as ") {
        Some((a, b)) => (a.trim().to_string(), Some(b.trim().to_string())),
        None => (item.trim().to_string(), None),
    }
}

/// 相対の import（`level` 段・`module`）を、それを書いたモジュール（`importer`・`importer_is_package` は `__init__` か）から見た
/// 絶対のモジュール名にする。上へ行き過ぎたら `None`（CPython の `ImportError: attempted relative import beyond top-level package`）。
pub(crate) fn absolute_module(importer: &[String], importer_is_package: bool, level: u32, module: &[String]) -> Option<Vec<String>> {
    if level == 0 {
        return Some(module.to_vec());
    }
    // パッケージ（`__init__`）ならそれ自身、モジュールなら親が起点（CPython の `__package__`）
    let mut base: Vec<String> = if importer_is_package { importer.to_vec() } else { importer[..importer.len().saturating_sub(1)].to_vec() };
    for _ in 1..level {
        base.pop()?;
    }
    if base.is_empty() && importer.is_empty() {
        return None;
    }
    base.extend(module.iter().cloned());
    Some(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_module_level_imports_including_type_checking_blocks() {
        let src = "from __future__ import annotations\n\
                   import numpy as np\n\
                   import os.path\n\
                   from pandas.core.frame import DataFrame\n\
                   from .series import Series as S\n\
                   from typing import TYPE_CHECKING\n\
                   if TYPE_CHECKING:\n    from pandas.core.indexes.base import Index\n\
                   try:\n    from x import y\nexcept ImportError:\n    y = None\n\
                   def f():\n    from z import w\n\
                   class C:\n    from q import r\n";
        let links = py_import_links(src, "m.py");
        let got: Vec<(String, u32, Vec<String>, Option<String>)> =
            links.into_iter().map(|l| (l.local, l.level, l.module, l.name)).collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        {
            assert_eq!(
                got,
                vec![
                    ("annotations".into(), 0, s(&["__future__"]), Some("annotations".into())),
                    ("np".into(), 0, s(&["numpy"]), None),
                    ("os".into(), 0, s(&["os"]), None),
                    ("DataFrame".into(), 0, s(&["pandas", "core", "frame"]), Some("DataFrame".into())),
                    ("S".into(), 1, s(&["series"]), Some("Series".into())),
                    ("TYPE_CHECKING".into(), 0, s(&["typing"]), Some("TYPE_CHECKING".into())),
                    ("Index".into(), 0, s(&["pandas", "core", "indexes", "base"]), Some("Index".into())),
                    ("y".into(), 0, s(&["x"]), Some("y".into())),
                ]
            );
        }
    }

    #[test]
    fn relative_modules_become_absolute() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // pandas/core/frame.py の `from .series import Series` → pandas.core.series
        assert_eq!(absolute_module(&s(&["pandas", "core", "frame"]), false, 1, &s(&["series"])), Some(s(&["pandas", "core", "series"])));
        // pandas/core/__init__.py の `from .frame import X` → pandas.core.frame
        assert_eq!(absolute_module(&s(&["pandas", "core"]), true, 1, &s(&["frame"])), Some(s(&["pandas", "core", "frame"])));
        // `from ..api import X`（pandas/core/frame.py）→ pandas.api
        assert_eq!(absolute_module(&s(&["pandas", "core", "frame"]), false, 2, &s(&["api"])), Some(s(&["pandas", "api"])));
        // 上へ行き過ぎ
        assert_eq!(absolute_module(&s(&["m"]), false, 3, &s(&["x"])), None);
    }
}
