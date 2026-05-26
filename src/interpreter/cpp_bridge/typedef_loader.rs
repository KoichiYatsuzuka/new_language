// typedef_loader.rs — Load and resolve `typedef` aliases from system/SDK headers.
//
// Public API:
//   load_system_typedefs — read typedef aliases from header files,
//                          resolve them transitively, return alias map.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::header_parser::{
    extract_alias_token, find_matching_brace, parse_alias_list, strip_and_preprocess,
    typedef_contains_fn_ptr,
};

// ── Public entry point ────────────────────────────────────────────────────────

/// Load `typedef` aliases from the given system/SDK header files and resolve
/// them transitively to C++ primitive type strings.
///
/// The returned map covers:
/// - Plain `typedef`: `typedef unsigned long DWORD` → `"DWORD"` → `"unsigned long"`
/// - Multi-alias: `typedef unsigned long DWORD, *LPDWORD` → both aliases
/// - Struct typedef: `typedef struct tag { … } A, *B` → `A → tag`, `B → tag*`
/// - `DECLARE_HANDLE(X)` macro → `"X"` → `"void*"` (opaque pointer)
///
/// `macros` is the list of predefined macros (from `precompile_macros`) used to
/// resolve `#ifdef`/`#elif defined(…)` branches.
///
/// Each listed header and its angle-bracket sub-headers are followed recursively.
/// Transitive aliases are fully resolved (e.g. `DWORD_PTR` → `ULONG_PTR` → `unsigned __int64`).
pub fn load_system_typedefs(header_paths: &[String], macros: &[String]) -> HashMap<String, String> {
    let mut raw: HashMap<String, String> = HashMap::new();
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for p in header_paths {
        collect_typedefs_from_file(Path::new(p), &mut raw, &mut visited, macros);
    }
    resolve_typedef_map(&mut raw);
    raw
}

// ── File-level collection ─────────────────────────────────────────────────────

fn collect_typedefs_from_file(
    path: &Path,
    out: &mut HashMap<String, String>,
    visited: &mut std::collections::HashSet<PathBuf>,
    macros: &[String],
) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let header_dir = path.parent().unwrap_or(Path::new("."));
    for inc in collect_typedef_includes(&content, header_dir) {
        collect_typedefs_from_file(&inc, out, visited, macros);
    }
    // Use macro-aware preprocessor so #ifdef _WIN64 blocks are resolved correctly.
    let stripped = strip_and_preprocess(&content, macros);
    collect_declare_handles(&stripped, out);
    collect_typedefs_from_text(&stripped, out);
}

/// Collect headers to follow for typedef loading.
/// Follows both `#include "file.h"` (quoted) and `#include <file.h>` (angle-bracket).
/// Angle-bracket names are resolved relative to `header_dir`; if not found there,
/// sibling directories (one level up from `header_dir`) are also checked.
fn collect_typedef_includes(raw_content: &str, header_dir: &Path) -> Vec<PathBuf> {
    // Sibling dirs: e.g. `um/` and `shared/` both live under the SDK version root.
    let parent = header_dir.parent();
    let mut result = Vec::new();
    for line in raw_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#include") {
            continue;
        }
        let after = trimmed["#include".len()..].trim_start();
        let (fname, require_exists) = if after.starts_with('"') {
            let inner = &after[1..];
            (inner.find('"').map(|e| inner[..e].to_string()), false)
        } else if after.starts_with('<') {
            let inner = &after[1..];
            // Only plain filenames — skip paths with slashes (e.g. <sys/types.h>)
            (inner.find('>').map(|e| inner[..e].to_string()), true)
        } else {
            (None, false)
        };
        let fname = match fname {
            Some(f) if !f.is_empty() => f,
            _ => continue,
        };
        // Try header_dir first, then sibling directories
        let mut found: Option<PathBuf> = None;
        let candidate = header_dir.join(&fname);
        if candidate.exists() {
            found = Some(candidate);
        } else if require_exists {
            if let Some(p) = parent {
                if let Ok(entries) = std::fs::read_dir(p) {
                    for entry in entries.flatten() {
                        let c = entry.path().join(&fname);
                        if c.exists() {
                            found = Some(c);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(p) = found {
            if !result.contains(&p) {
                result.push(p);
            }
        }
    }
    result
}

/// Detect `DECLARE_HANDLE(Name)` macro calls in stripped source and register
/// each `Name` as an opaque pointer (`void*`).
fn collect_declare_handles(stripped: &str, out: &mut HashMap<String, String>) {
    let mut rest = stripped;
    while let Some(pos) = rest.find("DECLARE_HANDLE") {
        rest = &rest[pos + "DECLARE_HANDLE".len()..];
        let after = rest.trim_start();
        if after.starts_with('(') {
            let inner = &after[1..];
            if let Some(close) = inner.find(')') {
                let name = inner[..close].trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    out.entry(name.to_string()).or_insert_with(|| "void*".to_string());
                }
            }
        }
    }
}

/// Scan stripped source for `typedef <type_expr> <Alias>;` statements and
/// populate `out` with `Alias → type_expr` entries (first definition wins).
fn collect_typedefs_from_text(stripped: &str, out: &mut HashMap<String, String>) {
    let mut i = 0;
    let mut seg_start = 0;

    while i < stripped.len() {
        let at_boundary = i == 0 || {
            let prev = stripped[..i].chars().last().unwrap_or(' ');
            !prev.is_alphanumeric() && prev != '_'
        };

        if at_boundary {
            // Skip `extern "C" { ... }` — recurse inside to catch typedefs there too
            if stripped[i..].starts_with("extern") {
                let after_kw = stripped[i + 6..].trim_start();
                if after_kw.starts_with('"') {
                    if let Some(qclose) = after_kw[1..].find('"') {
                        let rest = after_kw[2 + qclose..].trim_start();
                        if rest.starts_with('{') {
                            if let Some(brace_end) = find_matching_brace(rest) {
                                collect_typedefs_from_text(&rest[1..brace_end], out);
                                let consumed = (stripped.len() - rest.len()) - i + brace_end + 1;
                                i += consumed;
                                seg_start = i;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Skip `{ ... }` blocks (struct/union/enum bodies, namespace bodies, etc.)
        if stripped[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&stripped[i..]) {
                // If the segment so far starts with `typedef`, keep seg_start so the full
                // `typedef struct Tag { … } Alias;` is captured when we hit the `;`.
                let seg_before_brace = stripped[seg_start..i].trim_start();
                i += brace_end + 1;
                if !seg_before_brace.starts_with("typedef") {
                    seg_start = i;
                }
                continue;
            }
        }

        if stripped[i..].starts_with(';') {
            let segment = stripped[seg_start..i].trim();
            for (alias, type_expr) in parse_typedef_segments(segment) {
                out.entry(alias).or_insert(type_expr);
            }
            i += 1;
            seg_start = i;
            continue;
        }

        i += 1;
    }
}

// ── Typedef segment parser ────────────────────────────────────────────────────

/// Parse a `typedef` segment and return all `(alias, type_expr)` pairs it declares.
///
/// Handles all three equivalent forms:
///  - `typedef struct Tag { … } A, *B;`  →  `A → Tag`, `B → Tag*`
///  - `typedef unsigned long DWORD, *LPDWORD;`  →  `DWORD → unsigned long`, `LPDWORD → unsigned long*`
///  - `typedef void *PVOID;`  →  `PVOID → void*`
///
/// Function-pointer typedefs (any `*` inside `(…)`) are skipped.
fn parse_typedef_segments(seg: &str) -> Vec<(String, String)> {
    let words: Vec<&str> = seg.split_whitespace().collect();
    if words.first().copied() != Some("typedef") || words.len() < 3 {
        return vec![];
    }
    if typedef_contains_fn_ptr(seg) {
        return vec![];
    }

    // ── Struct/union/enum typedef with body ──────────────────────────────────
    if seg.contains('{') {
        let brace_start = seg.find('{').unwrap();
        let brace_end = match find_matching_brace(&seg[brace_start..]) {
            Some(p) => brace_start + p,
            None => return vec![],
        };
        let aliases_str = seg[brace_end + 1..].trim();
        if aliases_str.is_empty() {
            return vec![];
        }
        // Tag name = first identifier after `struct`/`union`/`enum` keyword, before `{`
        let before_brace_words: Vec<&str> = seg[..brace_start].split_whitespace().collect();
        let tag = before_brace_words
            .iter()
            .skip(1) // skip "typedef"
            .skip_while(|&&w| w == "struct" || w == "union" || w == "enum")
            .find(|&&w| w.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_'))
            .copied();
        let base_type = match tag {
            Some(t) => t.to_string(),
            None => return vec![],
        };
        return parse_alias_list(aliases_str, &base_type);
    }

    // ── Standard typedef: `typedef <type> alias1[, alias2, …]` ──────────────
    let after_typedef = seg.trim().strip_prefix("typedef").unwrap().trim_start();
    let parts: Vec<&str> = after_typedef.split(',').collect();
    if parts.is_empty() {
        return vec![];
    }

    // First comma-group: `<type tokens> first_alias`
    let first_words: Vec<&str> = parts[0].split_whitespace().collect();
    if first_words.len() < 2 {
        return vec![];
    }
    let alias_raw = *first_words.last().unwrap();
    let (alias, alias_stars) = extract_alias_token(alias_raw);
    if alias.is_empty() {
        return vec![];
    }
    // Base type = words before the alias, not counting their pointer decoration
    let type_words: Vec<&str> = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w != "*")
        .copied()
        .collect();
    let standalone_before = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w == "*")
        .count();
    let base_type = type_words.join(" ");
    let first_stars = alias_stars + standalone_before;

    let mut results = Vec::new();
    let mut type_expr = base_type.clone();
    if first_stars > 0 {
        type_expr.push_str(&"*".repeat(first_stars));
    }
    results.push((alias.to_string(), type_expr));

    // Remaining comma-groups: each is just an alias with optional leading `*`
    for part in &parts[1..] {
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
        let mut type_expr = base_type.clone();
        if stars > 0 {
            type_expr.push_str(&"*".repeat(stars));
        }
        results.push((alias.to_string(), type_expr));
    }
    results
}

// ── Alias resolution ──────────────────────────────────────────────────────────

/// Transitively resolve all alias entries until stable (max 32 passes).
/// After this, every value in the map is either a C++ primitive string or an
/// unresolvable type expression (struct name, unknown typedef, etc.).
pub(crate) fn resolve_typedef_map(map: &mut HashMap<String, String>) {
    for _ in 0..32 {
        let mut changed = false;
        let snapshot: HashMap<String, String> = map.clone();
        for val in map.values_mut() {
            let resolved = resolve_one_typedef(val, &snapshot);
            if *val != resolved {
                *val = resolved;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Attempt one resolution step: look up the base type of `expr` in `map`.
/// Handles pointer types: `"DWORD *"` → look up `"DWORD"`, prepend `*` to result.
fn resolve_one_typedef(expr: &str, map: &HashMap<String, String>) -> String {
    // Strip qualifiers for lookup, then restore pointer decoration
    let tokens: Vec<&str> = expr
        .split_whitespace()
        .filter(|t| !matches!(*t, "const" | "volatile" | "restrict" | "__restrict"))
        .collect();
    let has_ptr = tokens.last() == Some(&"*");
    let base_tokens: &[&str] = if has_ptr { &tokens[..tokens.len() - 1] } else { &tokens };
    let base_str = base_tokens.join(" ");
    if let Some(resolved_base) = map.get(&base_str) {
        if has_ptr {
            // Append one `*` to the resolved base (preserving its own trailing `*`)
            return format!("{} *", resolved_base.trim_end_matches('*').trim());
        }
        return resolved_base.clone();
    }
    expr.to_string()
}
