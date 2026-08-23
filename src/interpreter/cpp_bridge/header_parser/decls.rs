// header_parser/decls.rs — 関数宣言とスコープ走査: 名前空間スコープ走査、関数宣言解析、パラメータ分割、C 型文字列の解析。

use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::interpreter::cpp_bridge::types::{CStructDef, CFnSig, CType},
};
use super::*;

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

        // ⚠ #60: `extern "C" { … }` / `namespace X { … }` への降下は
        // `try_enter_scope` へ切り出した（以前はここに 5 段の入れ子で書かれていた）。
        if at_boundary {
            if let Some(consumed) = try_enter_scope(text, i, &ns, decls) {
                i += consumed;
                seg_start = i;
                continue;
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

/// 位置 `i` が `extern "C" {` / `namespace X {` の始まりなら**内部へ降下**して
/// 消費バイト数を返す（#60 で `scan_scope` から切り出し）。`None` = 降下しない。
///
/// ⚠ `extern` と `namespace` は同じ位置で両方に一致しえないので、
/// 片方が `None` を返したときにもう片方を試さなくてよい（切り出し前と同じ挙動）。
fn try_enter_scope(
    text: &str,
    i: usize,
    ns: &Option<String>,
    decls: &mut Vec<(String, Option<String>)>,
) -> Option<usize> {
    if text[i..].starts_with("extern") {
        return try_enter_extern_c(text, i, ns, decls);
    }
    if text[i..].starts_with("namespace") {
        return try_enter_namespace(text, i, decls);
    }
    None
}

/// `extern "C" { … }`（#60）。**リンケージ指定だけ**なので名前空間は引き継ぐ。
fn try_enter_extern_c(
    text: &str,
    i: usize,
    ns: &Option<String>,
    decls: &mut Vec<(String, Option<String>)>,
) -> Option<usize> {
    let after_kw = text[i + 6..].trim_start();
    let after_quote = after_kw.strip_prefix('"')?;
    let qclose = after_quote.find('"')?;
    let rest = after_quote[qclose + 1..].trim_start();
    descend_scope(text, i, rest, ns.clone(), decls)
}

/// `namespace X { … }`（#60）。内部の宣言には `X` を付ける。
fn try_enter_namespace(
    text: &str,
    i: usize,
    decls: &mut Vec<(String, Option<String>)>,
) -> Option<usize> {
    let after_kw = text[i + 9..].trim_start();
    let name_end = after_kw
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after_kw.len());
    let ns_name = &after_kw[..name_end];
    if ns_name.is_empty() {
        return None;
    }
    let rest = after_kw[name_end..].trim_start();
    descend_scope(text, i, rest, Some(ns_name.to_string()), decls)
}

/// `rest`（キーワードの後ろ）が `{ … }` なら再帰して**消費バイト数**を返す（#60）。
///
/// ⚠ 消費量は `text` 先頭からの位置で数える — `rest` は `trim_start()` 済みなので
/// `(text.len() - rest.len())` が `rest` の絶対オフセットになる。
fn descend_scope(
    text: &str,
    i: usize,
    rest: &str,
    child_ns: Option<String>,
    decls: &mut Vec<(String, Option<String>)>,
) -> Option<usize> {
    if !rest.starts_with('{') {
        return None;
    }
    let brace_end = find_matching_brace(rest)?;
    scan_scope(&rest[1..brace_end], child_ns, decls);
    Some((text.len() - rest.len()) - i + brace_end + 1)
}

// ── Function declaration parser ───────────────────────────────────────────────

/// 先頭の `extern` キーワード、デフォルト引数マクロ（`DEFAULTPARAM(= NULL)` など）、および名前空間を含む可能性のある関数宣言をパースする。
pub(crate) fn parse_fn_decl_ns(
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
pub(crate) fn strip_parameter_macros(s: &str) -> String {
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
pub(crate) fn split_type_and_name(s: &str) -> Result<(String, String), String> {
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

