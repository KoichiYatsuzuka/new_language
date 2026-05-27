// header_parser.rs — C/C++ header parsing: function declarations, struct
// definitions, type resolution, and text preprocessing utilities.
//
// Public API:
//   parse_header_full        — parse sigs + struct defs from a header string
//   parse_header             — parse sigs only
//   collect_included_headers — find local #include paths in raw header text
//
// Internal utilities re-used by typedef_loader (pub(crate)):
//   typedef_contains_fn_ptr, extract_alias_token, parse_alias_list
//   strip_and_preprocess, strip_comments, find_matching_brace

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::types::{CStructDef, CFnSig, CType};

const DEFAULTPARAM_MACRO: &str = "DEFAULTPARAM";

// ── Header parser ────────────────────────────────────────────────────────────

/// C/C++ ヘッダから関数宣言と構造体定義を解析する。`(functions, structs)` を返す。
/// 全フィールドがプリミティブ `CType` に解決できる構造体のみ出力する。
/// `custom` は C 型名から tl プリミティブ型へのマッピング、`typedefs` は `load_system_typedefs` で構築済みのエイリアスマップ。
pub fn parse_header_full(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> (Vec<CFnSig>, Vec<CStructDef>) {
    let stripped = strip_comments(content);
    let mut decls: Vec<(String, Option<String>)> = Vec::new();
    scan_scope(&stripped, None, &mut decls);

    let mut sigs = Vec::new();
    for (decl, ns) in &decls {
        if let Ok(sig) = parse_fn_decl_ns(decl, ns.clone(), custom, typedefs) {
            sigs.push(sig);
        }
    }

    let structs = parse_struct_bodies(&stripped, custom, typedefs);
    (sigs, structs)
}

/// C/C++ ヘッダから関数シグネチャのみを解析して返す（構造体定義は無視）。
#[allow(dead_code)]
pub fn parse_header(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CFnSig> {
    parse_header_full(content, custom, typedefs).0
}

/// ヘッダの生テキストからローカル `#include "filename.h"` ディレクティブを検索し、`header_dir` からの相対パスとして存在するパスを返す。
pub fn collect_included_headers(raw_content: &str, header_dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for line in raw_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#include") {
            continue;
        }
        let after = trimmed["#include".len()..].trim_start();
        // Only quoted includes (local headers), not angle-bracket system headers
        if !after.starts_with('"') {
            continue;
        }
        let inner = &after[1..];
        if let Some(end) = inner.find('"') {
            let fname = &inner[..end];
            // Only simple filenames (no path separators) resolved relative to header_dir
            let candidate = header_dir.join(fname);
            if candidate.exists() && !result.contains(&candidate) {
                result.push(candidate);
            }
        }
    }
    result
}

// ── Shared text-parsing utilities (pub(crate) for typedef_loader) ─────────────

/// `seg` が関数ポインタ typedef かどうかを返す。`(…)` グループ内に `*` が現れる場合（`(*name)`, `(__cdecl* name)` 等）を検出する。
pub(crate) fn typedef_contains_fn_ptr(seg: &str) -> bool {
    let mut depth = 0i32;
    for c in seg.chars() {
        match c {
            '(' => depth += 1,
            ')' => { if depth > 0 { depth -= 1; } }
            '*' if depth > 0 => return true,
            _ => {}
        }
    }
    false
}

/// `"*PVOID"` や `"LPDWORD"` のような生エイリアストークンから `(識別子, ポインタ数)` を取り出す。有効な識別子でない場合は `("", 0)` を返す。
pub(crate) fn extract_alias_token(raw: &str) -> (&str, usize) {
    let leading = raw.chars().take_while(|&c| c == '*').count();
    let without_leading = &raw[leading..];
    let alias = without_leading.trim_end_matches('*');
    let trailing = without_leading.len() - alias.len();
    if alias.is_empty() || !alias.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_') {
        return ("", 0);
    }
    (alias, leading + trailing)
}

/// 構造体 typedef の `}` の後に続くカンマ区切りエイリアスリスト（`A, *B, C` の部分）を指定ベース型に対してパースする。
pub(crate) fn parse_alias_list(aliases_str: &str, base_type: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for part in aliases_str.split(',') {
        let part_words: Vec<&str> = part.split_whitespace().collect();
        let extra = part_words.iter().filter(|&&w| w == "*").count();
        let alias_raw = match part_words.iter().find(|&&w| w != "*") {
            Some(w) => w,
            None => continue,
        };
        let (alias, alias_stars) = extract_alias_token(alias_raw);
        if alias.is_empty() {
            continue;
        }
        let stars = alias_stars + extra;
        let mut type_expr = base_type.to_string();
        if stars > 0 {
            type_expr.push_str(&"*".repeat(stars));
        }
        results.push((alias.to_string(), type_expr));
    }
    results
}

/// `src` から C/C++ の行コメントおよびブロックコメントを除去する。
pub(crate) fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    // line comment — skip to end of line
                    for c2 in chars.by_ref() {
                        if c2 == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    // block comment — skip to */
                    while let Some(c2) = chars.next() {
                        if c2 == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                        if c2 == '\n' {
                            out.push('\n');
                        }
                    }
                }
                _ => out.push(c),
            }
        } else if c == '#' {
            // preprocessor directive — skip to end of line
            for c2 in chars.by_ref() {
                if c2 == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// C/C++ コメントを除去し、`#ifdef`/`#ifndef`/`#if defined(…)`/`#elif defined(…)`/`#else`/`#endif` プリプロセッサ条件を解決する。
/// 偽の条件で除外された行は空行に置き換える（行数を保持）。その他の `#` ディレクティブも空行にする。
pub(crate) fn strip_and_preprocess(src: &str, macros: &[String]) -> String {
    // Stack frames: (branch_active, any_branch_taken, parent_active)
    let mut cond_stack: Vec<(bool, bool, bool)> = Vec::new();
    let mut in_block_comment = false;
    let mut out = String::with_capacity(src.len());

    let currently_active = |stack: &[(bool, bool, bool)]| stack.iter().all(|(a, _, _)| *a);
    let macro_defined = |name: &str| macros.iter().any(|m| m == name);

    // Parse `defined(MACRO)` or `!defined(MACRO)` from a condition expression.
    // Returns `Some((name, negated))` for simple single-macro conditions.
    let parse_defined_cond = |expr: &str| -> Option<(String, bool)> {
        let expr = expr.trim();
        let (expr, negated) = if let Some(rest) = expr.strip_prefix('!') {
            (rest.trim(), true)
        } else {
            (expr, false)
        };
        let rest = expr.strip_prefix("defined")?.trim();
        let name = if let Some(inner) = rest.strip_prefix('(') {
            inner.split(')').next()?.trim().to_string()
        } else {
            rest.split_whitespace().next()?.to_string()
        };
        if name.is_empty() { return None; }
        Some((name, negated))
    };

    // Evaluate a condition expression (handles `||`/`&&`-joined `defined()` clauses).
    // Unknown sub-expressions default to `true` so that content gated by unrecognised
    // conditions is still included — matching the old `strip_comments` behaviour.
    let eval_cond = |expr: &str| -> bool {
        if expr.contains("||") {
            return expr.split("||").any(|part| {
                if let Some((name, neg)) = parse_defined_cond(part.trim()) {
                    macro_defined(&name) != neg
                } else { true } // unknown sub-term → include
            });
        }
        if expr.contains("&&") {
            return expr.split("&&").all(|part| {
                if let Some((name, neg)) = parse_defined_cond(part.trim()) {
                    macro_defined(&name) != neg
                } else { true } // unknown sub-term → include
            });
        }
        if let Some((name, neg)) = parse_defined_cond(expr) {
            macro_defined(&name) != neg
        } else {
            true // unknown condition → include
        }
    };

    for line in src.lines() {
        // Strip block comment spans from the visible part of the line.
        let (visible, still_in_comment) = {
            let mut s = String::new();
            let mut in_cmt = in_block_comment;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if in_cmt {
                    if c == '*' && chars.peek() == Some(&'/') { chars.next(); in_cmt = false; }
                } else if c == '/' && chars.peek() == Some(&'*') {
                    chars.next(); in_cmt = true;
                } else if c == '/' && chars.peek() == Some(&'/') {
                    break; // rest is a line comment
                } else {
                    s.push(c);
                }
            }
            (s, in_cmt)
        };
        in_block_comment = still_in_comment;

        let trimmed = visible.trim_start();
        if trimmed.starts_with('#') {
            let dir = trimmed[1..].trim_start();
            if dir.starts_with("ifdef") {
                let name = dir["ifdef".len()..].split_whitespace().next().unwrap_or("");
                let parent = currently_active(&cond_stack);
                let active = parent && macro_defined(name);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("ifndef") {
                let name = dir["ifndef".len()..].split_whitespace().next().unwrap_or("");
                let parent = currently_active(&cond_stack);
                let active = parent && !macro_defined(name);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("if ") || dir == "if" {
                let expr = dir["if".len()..].trim();
                let parent = currently_active(&cond_stack);
                let active = parent && eval_cond(expr);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("elif") {
                let expr = dir["elif".len()..].trim();
                if let Some(frame) = cond_stack.last_mut() {
                    let (_, any_taken, parent) = *frame;
                    if any_taken {
                        frame.0 = false;
                    } else {
                        let active = parent && eval_cond(expr);
                        frame.0 = active;
                        if active { frame.1 = true; }
                    }
                }
            } else if dir.starts_with("else") {
                if let Some(frame) = cond_stack.last_mut() {
                    let (_, any_taken, parent) = *frame;
                    let active = !any_taken && parent;
                    frame.0 = active;
                    if active { frame.1 = true; }
                }
            } else if dir.starts_with("endif") {
                cond_stack.pop();
            }
            out.push('\n'); // always emit blank line for any # directive
            continue;
        }

        if currently_active(&cond_stack) {
            out.push_str(&visible);
        }
        out.push('\n');
    }
    out
}

/// `s` の位置 0 にある開き `{` に対応する閉じ `}` の位置を返す。
pub(crate) fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

// ── Struct body parsing ───────────────────────────────────────────────────────

/// ストリップ済みソースから `typedef struct/union Tag { … } Alias;` 定義を走査し、
/// 全フィールドがプリミティブ `CType` に解決できるエイリアス（非ポインタのみ）ごとに `CStructDef` を返す。
pub(crate) fn parse_struct_bodies(
    stripped: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CStructDef> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;

    while i < stripped.len() {
        if stripped[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&stripped[i..]) {
                let seg_before = stripped[seg_start..i].trim_start();
                let is_struct_typedef = seg_before.starts_with("typedef")
                    && (seg_before.contains(" struct ") || seg_before.contains(" union "));

                // `class Name { … }` or `struct Name { … }` (not typedef)
                let class_name = if !is_struct_typedef {
                    let w: Vec<&str> = seg_before.split_whitespace().collect();
                    if matches!(w.first().copied(), Some("class") | Some("struct"))
                        && w.len() >= 2
                        && w[1].chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        Some(w[1].to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };

                if is_struct_typedef {
                    let body = &stripped[i + 1..i + brace_end];
                    let rest = &stripped[i + brace_end + 1..];
                    if let Some(semi_pos) = rest.find(';') {
                        let aliases_str = rest[..semi_pos].trim();
                        if !aliases_str.is_empty() {
                            let fields = parse_struct_field_decls(body, custom, typedefs);
                            if !fields.is_empty() {
                                for (alias, ptr_suffix) in parse_alias_list(aliases_str, "") {
                                    if !ptr_suffix.contains('*') {
                                        result.push(CStructDef {
                                            name: alias,
                                            fields: fields.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(name) = class_name {
                    let body = &stripped[i + 1..i + brace_end];
                    let fields = parse_struct_field_decls(body, custom, typedefs);
                    if !fields.is_empty() {
                        result.push(CStructDef { name, fields });
                    }
                }
                i += brace_end + 1;
                seg_start = i;
                continue;
            }
        }

        if stripped[i..].starts_with(';') {
            seg_start = i + 1;
        }
        i += 1;
    }
    result
}

/// 構造体本体のフィールド宣言をパースする。`float x, y, z;` → 3 フィールド、配列宣言やネスト構造体はスキップ。プリミティブ `CType` に解決できるフィールドのみ返す。
fn parse_struct_field_decls(
    body: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<(String, CType)> {
    let mut fields = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;

    while i < body.len() {
        if body[i..].starts_with('{') {
            if let Some(end) = find_matching_brace(&body[i..]) {
                i += end + 1;
                seg_start = i;
                continue;
            }
        }
        if body[i..].starts_with(';') {
            let seg = body[seg_start..i].trim();
            if !seg.is_empty() {
                parse_field_segment(seg, custom, typedefs, &mut fields);
            }
            i += 1;
            seg_start = i;
            continue;
        }
        i += 1;
    }
    fields
}

/// `;` 区切りの 1 フィールドセグメント（`float x, y, z` や `int flags` など）をパースし、解決済みの `(name, CType)` ペアを `out` に追加する。
fn parse_field_segment(
    seg: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
    out: &mut Vec<(String, CType)>,
) {
    // Skip constructor / method declarations (contain parentheses).
    if seg.contains('(') {
        return;
    }
    let all_words: Vec<&str> = seg.split_whitespace().collect();
    // Strip a leading access-control specifier ("public:" / "private:" / "protected:").
    let start = if all_words
        .first()
        .map(|w| matches!(w.trim_end_matches(':'), "public" | "private" | "protected" | "virtual"))
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let words = &all_words[start..];
    if words.len() < 2 {
        return;
    }

    // Split by comma: `float x, y, z` → `["float x", " y", " z"]`
    let seg = words.join(" ");
    let parts: Vec<&str> = seg.split(',').collect();
    let first_words: Vec<&str> = parts[0].split_whitespace().collect();
    if first_words.len() < 2 {
        return;
    }

    // Last word of the first part is the field name (possibly prefixed with `*`)
    let raw_last = *first_words.last().unwrap();
    // Skip array fields like `m[4][4]`
    if raw_last.contains('[') || raw_last.contains('(') {
        return;
    }
    let (first_name, first_stars) = extract_alias_token(raw_last);
    if first_name.is_empty() {
        return;
    }

    let type_words: Vec<&str> = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w != "*")
        .copied()
        .collect();
    let standalone_stars = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w == "*")
        .count();
    let base_str = type_words.join(" ");
    let total_stars = first_stars + standalone_stars;
    let type_str = if total_stars > 0 {
        format!("{}{}", base_str, "*".repeat(total_stars))
    } else {
        base_str.clone()
    };

    match parse_c_type_str(&type_str, custom, typedefs) {
        Ok(ctype) => {
            out.push((first_name.to_string(), ctype.clone()));
            for part in &parts[1..] {
                let pw: Vec<&str> = part.split_whitespace().collect();
                let extra_stars = pw.iter().filter(|&&w| w == "*").count();
                let raw_name = match pw.iter().find(|&&w| w != "*") {
                    Some(w) => w,
                    None => continue,
                };
                if raw_name.contains('[') || raw_name.contains('(') {
                    continue;
                }
                let (alias, alias_stars) = extract_alias_token(raw_name);
                if alias.is_empty() {
                    continue;
                }
                let stars = alias_stars + extra_stars;
                if stars == 0 {
                    out.push((alias.to_string(), ctype.clone()));
                } else if let Ok(ptr_ct) = parse_c_type_str(
                    &format!("{}{}", base_str, "*".repeat(stars)),
                    custom,
                    typedefs,
                ) {
                    out.push((alias.to_string(), ptr_ct));
                }
            }
        }
        Err(_) => {} // skip fields with unparseable types
    }
}

// ── Scope scanner ─────────────────────────────────────────────────────────────

/// C/C++ スコープを再帰的に走査して関数宣言を収集する。
/// `extern "C" { ... }` は同じ名前空間で再帰し、`namespace X { ... }` は `ns = Some("X")` で再帰し、それ以外の `{ ... }` はスキップ（struct/class/union 本体）、`;` はフラッシュして宣言候補として格納する。
pub(crate) fn scan_scope(text: &str, ns: Option<String>, decls: &mut Vec<(String, Option<String>)>) {
    let mut i = 0;
    let mut seg_start = 0; // start of the current `;`-delimited declaration

    while i < text.len() {
        // Check whether we are at a word boundary (needed for keyword detection)
        let at_boundary = i == 0 || {
            let prev = text[..i].chars().last().unwrap_or(' ');
            !prev.is_alphanumeric() && prev != '_'
        };

        if at_boundary {
            // ── `extern "C" { ... }` ──────────────────────────────────────────
            if text[i..].starts_with("extern") {
                let after_kw = text[i + 6..].trim_start();
                if after_kw.starts_with('"') {
                    if let Some(qclose) = after_kw[1..].find('"') {
                        let rest = after_kw[2 + qclose..].trim_start();
                        if rest.starts_with('{') {
                            if let Some(brace_end) = find_matching_brace(rest) {
                                // Recurse: extern "C" is linkage-only, inherits namespace
                                scan_scope(&rest[1..brace_end], ns.clone(), decls);
                                let consumed = (text.len() - rest.len()) - i + brace_end + 1;
                                i += consumed;
                                seg_start = i;
                                continue;
                            }
                        }
                    }
                }
            }

            // ── `namespace X { ... }` ─────────────────────────────────────────
            if text[i..].starts_with("namespace") {
                let after_kw = text[i + 9..].trim_start();
                let name_end = after_kw
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(after_kw.len());
                let ns_name = &after_kw[..name_end];
                if !ns_name.is_empty() {
                    let rest = after_kw[name_end..].trim_start();
                    if rest.starts_with('{') {
                        if let Some(brace_end) = find_matching_brace(rest) {
                            scan_scope(&rest[1..brace_end], Some(ns_name.to_string()), decls);
                            let consumed = (text.len() - rest.len()) - i + brace_end + 1;
                            i += consumed;
                            seg_start = i;
                            continue;
                        }
                    }
                }
            }
        }

        // ── `{ ... }` that is not extern/namespace → skip (struct / class body) ──
        if text[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&text[i..]) {
                i += brace_end + 1;
                seg_start = i;
                continue;
            }
        }

        // ── `;` → end of a declaration, emit if it looks like a function ────────
        if text[i..].starts_with(';') {
            let segment = &text[seg_start..i];
            let normalized = segment.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty()
                && normalized.contains('(')
                && normalized.contains(')')
                && !normalized.split_whitespace().any(|w| w == "typedef")
            {
                decls.push((normalized, ns.clone()));
            }
            i += 1;
            seg_start = i;
            continue;
        }

        i += 1;
    }
}

// ── Function declaration parser ───────────────────────────────────────────────

/// 先頭の `extern` キーワード、デフォルト引数マクロ（`DEFAULTPARAM(= NULL)` など）、および名前空間を含む可能性のある関数宣言をパースする。
fn parse_fn_decl_ns(
    decl: &str,
    namespace: Option<String>,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Result<CFnSig, String> {
    // Strip leading `extern` keyword
    let decl = decl.trim();
    let decl = if decl.starts_with("extern") {
        let after = decl["extern".len()..].trim_start();
        // Don't strip if it's `extern "C"` — those are handled by the block parser
        if after.starts_with('"') {
            decl
        } else {
            after
        }
    } else {
        decl
    };

    // Find outer parentheses (function name is before first `(`)
    let paren_open = decl.find('(').ok_or("no '('")?;
    let paren_close = decl.rfind(')').ok_or("no ')'")?;
    if paren_close < paren_open {
        return Err("')' before '('".to_string());
    }

    let before_paren = decl[..paren_open].trim();
    let params_raw = &decl[paren_open + 1..paren_close];

    // Strip parameter-position macros like `DEFAULTPARAM(= NULL)`
    let params_clean = strip_parameter_macros(params_raw);

    let (ret_str, name) = split_type_and_name(before_paren)?;
    // Skip names that look like C++ operators or are empty
    if name.is_empty() || name.contains("operator") || name.starts_with('~') {
        return Err("not a plain function".to_string());
    }
    let ret = parse_c_type_str(ret_str.trim(), custom, typedefs)?;

    let mut params: Vec<(String, CType)> = Vec::new();
    // Track which raw (pre-strip) params have DEFAULTPARAM to compute n_required
    let raw_param_list = split_params(params_raw);
    let params_str = params_clean.trim();
    if !params_str.is_empty() && params_str != "void" {
        for (idx, p) in split_params(params_str).iter().enumerate() {
            let p = p.trim();
            if p.is_empty() || p == "..." {
                continue;
            }
            // Detect function pointer parameters before attempting type/name split
            if typedef_contains_fn_ptr(p) {
                params.push((format!("_p{idx}"), CType::FnPtr));
                continue;
            }
            let (type_str, pname) = match split_type_and_name(p) {
                Ok(r) => r,
                Err(_) => (p.to_string(), format!("_p{idx}")),
            };
            match parse_c_type_str(type_str.trim(), custom, typedefs) {
                Ok(ct) => params.push((pname, ct)),
                Err(e) => return Err(format!("param {pname}: {e}")),
            }
        }
    }

    // n_required = index of first param that has DEFAULTPARAM in the raw declaration
    let n_required = raw_param_list
        .iter()
        .position(|p| p.contains(DEFAULTPARAM_MACRO))
        .unwrap_or(params.len());
    // n_required must not exceed actual parsed params count
    let n_required = n_required.min(params.len());

    Ok(CFnSig {
        name,
        params,
        ret,
        namespace,
        n_required,
    })
}

/// `,` でパラメータリストを分割する。ネストした `()` は考慮して分割する。
pub(crate) fn split_params(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '[' | '<' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '>' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                result.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// パラメータ文字列から `IDENTIFIER(...)` 形式のマクロ呼び出しパターンを除去する。`DEFAULTPARAM(= NULL)` などのデフォルト引数マクロの除去に使用する。
fn strip_parameter_macros(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphabetic() || c == '_' {
            let mut ident = c.to_string();
            while matches!(chars.peek(), Some(nc) if nc.is_alphanumeric() || *nc == '_') {
                ident.push(chars.next().unwrap());
            }
            // Skip whitespace between identifier and possible `(`
            let mut spaces = String::new();
            while matches!(chars.peek(), Some(' ') | Some('\t')) {
                spaces.push(chars.next().unwrap());
            }
            if chars.peek() == Some(&'(') {
                // This looks like a macro call — skip the balanced `(...)`
                chars.next(); // consume `(`
                let mut depth = 1usize;
                for mc in chars.by_ref() {
                    match mc {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                // The macro call is gone — don't emit anything
            } else {
                result.push_str(&ident);
                result.push_str(&spaces);
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ── Type string parser ────────────────────────────────────────────────────────

/// `"int *foo"` や `"const int* bar"` のような C 宣言を `("int*", "foo")` に分割する。名前に付いた `*` は型文字列に移動し `parse_c_type_str` が完全なポインタ装飾を確認できるようにする。
fn split_type_and_name(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("cannot split type/name from '{s}'"));
    }

    // Last word may be `*foo`, `**foo`, or just `foo`
    let raw_last = *parts.last().unwrap();
    let star_count = raw_last.chars().take_while(|&c| c == '*').count();
    let name = &raw_last[star_count..];
    if name.is_empty() {
        return Err(format!("cannot extract name from '{s}'"));
    }

    // Stars from the name token belong to the type
    let type_base = parts[..parts.len() - 1].join(" ");
    let type_str = if star_count > 0 {
        format!("{}{}", type_base, "*".repeat(star_count))
    } else {
        type_base
    };

    Ok((type_str, name.to_string()))
}

/// C 型文字列（末尾に `*` を含む可能性あり）を `CType` にマップする。
/// 解決順: (1) `custom`（ユーザー上書き）→ (2) `typedefs`（システム typedef エイリアス）→ (3) 組み込み C/C++ プリミティブ名。
/// 解決できない型は `Err` を返す。
pub(crate) fn parse_c_type_str(
    s: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Result<CType, String> {
    let s = s.trim();

    // Count and strip trailing '*'
    let ptr_count = s.chars().rev().take_while(|&c| c == '*').count();
    let base = s[..s.len() - ptr_count].trim();

    // Detect 'const' qualifier
    let is_const = base.split_whitespace().any(|t| t == "const");

    // Filter qualifiers and MIDL annotations (e.g. `[public]`)
    let tokens: Vec<&str> = base
        .split_whitespace()
        .filter(|t| {
            !matches!(*t, "const" | "volatile" | "restrict" | "__restrict")
                && !t.starts_with('[') // strip MIDL annotations like [public], [unique]
        })
        .collect();

    // Filter signed/unsigned for primitive lookup
    let core: Vec<&str> = tokens
        .iter()
        .filter(|t| !matches!(**t, "unsigned" | "signed"))
        .copied()
        .collect();

    let core_str = core.join(" ");

    // ── 1. User custom type mappings ─────────────────────────────────────────
    if let Some(tl_type) = custom.get(&core_str) {
        let base_ct = match tl_type.as_str() {
            "int" => CType::Int,
            "long" => CType::Long,
            "float" => CType::Float,
            "double" => CType::Double,
            "bool" => CType::Bool,
            "void" => CType::Void,
            other => return Err(format!("custom_type_map: unknown tl type '{other}'")),
        };
        return Ok(if ptr_count > 0 {
            CType::Ptr { inner: Box::new(base_ct), mutable: !is_const }
        } else {
            base_ct
        });
    }

    // ── 2. System typedef aliases ────────────────────────────────────────────
    // Look up the base type name in the pre-resolved typedef map.
    // The resolved value may itself carry a trailing `*` (e.g. HWND → "void*").
    if let Some(resolved) = typedefs.get(&core_str) {
        let resolved_ptrs = resolved.chars().rev().take_while(|&c| c == '*').count();
        let total_ptrs = ptr_count + resolved_ptrs;
        if total_ptrs > 1 {
            return Err("multi-level pointer".to_string());
        }
        let resolved_base = resolved[..resolved.len() - resolved_ptrs].trim();
        let combined = if total_ptrs == 1 {
            format!("{} *", resolved_base)
        } else {
            resolved_base.to_string()
        };
        // Recurse with empty typedefs to avoid infinite loops
        return parse_c_type_str(combined.trim(), custom, &HashMap::new());
    }

    // ── 3. C/C++ built-in primitives ─────────────────────────────────────────
    if ptr_count > 0 {
        if ptr_count > 1 {
            return Err("multi-level pointer".to_string());
        }
        // char* / wchar_t* → tl str
        if matches!(core.as_slice(), ["char"] | ["wchar_t"]) {
            return Ok(CType::CharPtr);
        }
        // void* → opaque integer handle
        if core == ["void"] {
            return Ok(CType::VoidPtr);
        }
        let inner = match core.as_slice() {
            ["bool"] | ["_Bool"] => CType::Bool,
            ["float"] => CType::Float,
            ["double"] | ["long", "double"] => CType::Double,
            ["char"] | ["short"] | ["int"]
            | ["int8_t"] | ["int16_t"] | ["int32_t"]
            | ["uint8_t"] | ["uint16_t"] | ["uint32_t"] => CType::Int,
            ["long"] | ["long", "int"] | ["long", "long"] | ["long", "long", "int"]
            | ["int64_t"] | ["uint64_t"]
            | ["size_t"] | ["ptrdiff_t"] | ["intptr_t"] | ["uintptr_t"]
            | ["__int64"] | ["__int3264"] => CType::Long,
            other => {
                let type_name = other.join(" ");
                if type_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ') {
                    return Ok(CType::OpaqueStructPtr { type_name, mutable: !is_const });
                }
                return Err(format!("unknown pointer base type '{}'", type_name));
            }
        };
        return Ok(CType::Ptr { inner: Box::new(inner), mutable: !is_const });
    }

    match core.as_slice() {
        ["void"] => Ok(CType::Void),
        ["bool"] | ["_Bool"] => Ok(CType::Bool),
        ["float"] => Ok(CType::Float),
        ["double"] | ["long", "double"] => Ok(CType::Double),
        ["char"] | ["short"] | ["int"] | ["wchar_t"]
        | ["int8_t"] | ["int16_t"] | ["int32_t"]
        | ["uint8_t"] | ["uint16_t"] | ["uint32_t"] => Ok(CType::Int),
        ["long"] | ["long", "int"] | ["long", "long"] | ["long", "long", "int"]
        | ["int64_t"] | ["uint64_t"]
        | ["size_t"] | ["ptrdiff_t"] | ["intptr_t"] | ["uintptr_t"]
        | ["__int64"] | ["__int3264"] => Ok(CType::Long),
        other => {
            let type_name = other.join(" ");
            if type_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ') {
                // Struct/union passed by value: bridge via pointer dereferencing in the shim
                return Ok(CType::ByValueStruct { type_name });
            }
            eprintln!(
                "CppBridge: unknown C type '{}', skipping function",
                type_name
            );
            Err(format!("unknown C type '{}'", type_name))
        }
    }
}
