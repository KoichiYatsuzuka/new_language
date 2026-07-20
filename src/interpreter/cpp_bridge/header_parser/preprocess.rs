// header_parser/preprocess.rs — ヘッダ前処理: コメント除去・マクロ展開、typedef/エイリアス解析、括弧マッチング。

use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::interpreter::cpp_bridge::types::{CStructDef, CFnSig, CType},
};
use super::*;

// ── Shared text-parsing utilities (pub(crate) for typedef_loader) ─────────────

/// `seg` が関数ポインタ typedef かどうかを返す。`(…)` グループ内に `*` が現れる場合（`(*name)`, `(__cdecl* name)` 等）を検出する。
pub(crate) fn typedef_contains_fn_ptr(seg: &str) -> bool {
    let mut depth = 0i32;
    for c in seg.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => { depth -= 1; }
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
    if alias.is_empty() || !alias.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
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

