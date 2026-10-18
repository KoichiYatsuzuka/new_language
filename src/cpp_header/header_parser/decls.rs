// header_parser/decls.rs — 関数宣言とスコープ走査: 名前空間スコープ走査、関数宣言解析、パラメータ分割、C 型文字列の解析。

use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::cpp_header::types::{CDefault, CStructDef, CFnSig, CType},
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
            // C++ の既定値（`int n = 3`）は型と名前の外へ（値は `param_default_text` が読む・7-13）
            let p = match p.find('=') {
                Some(eq) if !typedef_contains_fn_ptr(p) => p[..eq].trim(),
                _ => p,
            };
            // Detect function pointer parameters before attempting type/name split
            if typedef_contains_fn_ptr(p) {
                let pname = fn_ptr_param_name(p).unwrap_or_else(|| format!("_p{idx}"));
                params.push((pname, CType::FnPtr));
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

    // 既定値（7-13）: 引数ごとに `DEFAULTPARAM( = x )` / `= x` の字句を読み、その引数の型の値にする。省略できるのは、
    // 既定値を**読めた**引数が末尾に続く部分だけ（読めない既定値の引数は省略できない・黙って 0 にしない）
    // ⚠ 以前は最初の DEFAULTPARAM の位置を `n_required` にし、既定値は読まずに省略した引数を 0 / NULL で埋めていた。
    let mut defaults: Vec<Option<CDefault>> = params
        .iter()
        .enumerate()
        .map(|(i, (_, ct))| raw_param_list.get(i).and_then(|raw| param_default_text(raw)).and_then(|t| ct.parse_default(&t)))
        .collect();
    let mut n_required = params.len();
    while n_required > 0 && defaults[n_required - 1].is_some() {
        n_required -= 1;
    }
    for d in defaults.iter_mut().take(n_required) {
        *d = None;
    }

    Ok(CFnSig {
        name,
        params,
        ret,
        namespace,
        n_required,
        defaults,
    })
}

/// 引数 1 つの宣言（マクロを外す前）から既定値の字句を取り出す（7-13）: `DEFAULTPARAM( = x )` の `x`、または C++ の
/// `type name = x` の `x`。既定値が無ければ `None`。
fn param_default_text(raw: &str) -> Option<String> {
    if let Some(pos) = raw.find(DEFAULTPARAM_MACRO) {
        let rest = &raw[pos + DEFAULTPARAM_MACRO.len()..];
        let open = rest.find('(')?;
        let mut depth = 0usize;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let inner = rest[open + 1..open + i].trim();
                        return Some(inner.strip_prefix('=').unwrap_or(inner).trim().to_string());
                    }
                }
                _ => {}
            }
        }
        return None;
    }
    // C++ の既定値（`int n = 3`）。`==` などは引数の宣言に現れない
    raw.find('=').map(|eq| raw[eq + 1..].trim().to_string())
}

/// C / C++ の組み込みの型の名前か（`signed` / `unsigned` を外した形）。typedef でこれを置き換えない（7-13）:
/// Windows SDK の `basetyps.h` などは C のために `typedef unsigned short wchar_t;` を持つが、C++ では `wchar_t` は組み込みの
/// 型（キーワード）。置き換えると `const wchar_t*` が文字列でなく数値へのポインタになり、`WCHAR` / `LPCWSTR` の連鎖も壊れる。
pub(crate) fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "void" | "bool" | "_Bool" | "char" | "wchar_t" | "char8_t" | "char16_t" | "char32_t" | "short" | "short int"
            | "int" | "long" | "long int" | "long long" | "long long int" | "float" | "double" | "long double"
    )
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

/// `ident ( group )` の `group` が関数ポインタ・参照の宣言子か（`* cb` / `&ref` / `__cdecl *cb` / `WINAPI* cb`）。
/// マクロの引数（`= NULL`）とは、先頭（呼び出し規約の語のあと）が `*` / `&` であることで見分ける（7-29）。
fn is_declarator_group(group: &str) -> bool {
    let g = group.trim_start();
    if g.starts_with('*') || g.starts_with('&') {
        return true;
    }
    // 呼び出し規約などの語が 1 つ前にある形（`__cdecl *cb`・`WINAPI* cb`）
    let word_end = g.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(g.len());
    word_end > 0 && g[word_end..].trim_start().starts_with('*')
}

/// 関数ポインタの引数（`void (* cb )( void )`）の名前（`cb`）。宣言子の括弧の中の最後の識別子。無ければ `None`（7-29）。
fn fn_ptr_param_name(p: &str) -> Option<String> {
    let open = p.find('(')?;
    let close = open + p[open..].find(')')?;
    let inner = &p[open + 1..close];
    let name = inner
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .last()?;
    // 呼び出し規約だけ（名前の無い `void (*)(int)`）なら名前は無い
    if matches!(name, "__cdecl" | "__stdcall" | "__fastcall" | "WINAPI" | "CALLBACK" | "APIENTRY") {
        return None;
    }
    Some(name.to_string())
}

/// パラメータ文字列から `IDENTIFIER(...)` 形式のマクロ呼び出しパターンを除去する。`DEFAULTPARAM(= NULL)` などのデフォルト引数マクロの除去に使用する。
/// 関数ポインタの宣言子（`void (* cb )`）は残す（`is_declarator_group`・7-29）。
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
                // This looks like a macro call — read the balanced `(...)`
                chars.next(); // consume `(`
                let mut depth = 1usize;
                let mut group = String::new();
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
                    group.push(mc);
                }
                let mut trailing = String::new();
                while matches!(chars.peek(), Some(' ') | Some('\t')) {
                    trailing.push(chars.next().unwrap());
                }
                // ⚠ 関数ポインタの宣言子（`void (* cb )( void )` の `(* cb )`・`(__cdecl *cb)`）はマクロではないので残す
                //   （ffi_value_exchange_plan.md 7-29）。以前は `void (* cb )` をマクロの呼び出しとして消し、残った `( void )`
                //   が型として読めずに関数ごと黙って捨てていた（DxLib の callback を取る 7 関数）。残せば `typedef_contains_fn_ptr`
                //   が関数ポインタの引数と見分け、型スタブで「callback は未対応」の理由つきの静的エラーになる。
                //   宣言子のあとには必ず引数の並び `(` か配列の `[` が続く（SAL の `_Out_writes_to_(*pcch, ..) LPWSTR p` は続かない）
                if is_declarator_group(&group) && matches!(chars.peek(), Some('(') | Some('[')) {
                    result.push_str(&ident);
                    result.push_str(&spaces);
                    result.push('(');
                    result.push_str(&group);
                    result.push(')');
                }
                // otherwise the macro call is gone — don't emit it
                result.push_str(&trailing);
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

/// 整数の型名（`unsigned` / `signed` を除いた語の並び）を、幅と符号つきの `CType::Int` にする（1-6）。
/// 整数でなければ `None`。`sign_kw` は `unsigned` / `signed` が書かれていたか、`unsigned` はそれが `unsigned` か。
///
/// ⚠ 以前は `char` / `short` / `int` / `intN_t` を符号付き 4 バイトに、`long` / `long long` / `size_t` を 8 バイトに
///   まとめていた（`unsigned` も捨てていた）。`size_t` / `intptr_t` などはポインタの幅（x64 では 8 バイト）とする。
fn c_int_type(core: &[&str], sign_kw: bool, unsigned: bool) -> Option<CType> {
    use crate::cpp_header::types::IntWidth::*;
    let (width, signed) = match core {
        ["char"] | ["__int8"] => (W8, !unsigned),
        ["short"] | ["short", "int"] | ["__int16"] => (W16, !unsigned),
        ["int"] | ["__int32"] => (W32, !unsigned),
        [] if sign_kw => (W32, !unsigned), // `unsigned` / `signed` だけ
        ["long"] | ["long", "int"] => (CLong, !unsigned),
        ["long", "long"] | ["long", "long", "int"] | ["__int64"] => (W64, !unsigned),
        ["int8_t"] => (W8, true),
        ["uint8_t"] => (W8, false),
        ["int16_t"] => (W16, true),
        ["uint16_t"] => (W16, false),
        ["int32_t"] => (W32, true),
        ["uint32_t"] => (W32, false),
        ["int64_t"] => (W64, true),
        ["uint64_t"] => (W64, false),
        ["size_t"] | ["uintptr_t"] => (W64, false),
        ["ptrdiff_t"] | ["intptr_t"] | ["ssize_t"] | ["__int3264"] => (W64, true),
        // 幅と符号は環境で決まる（中継コードの生成で決める）
        ["wchar_t"] => (WChar, false),
        _ => return None,
    };
    Some(CType::Int { width, signed })
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

    // Filter signed/unsigned for primitive lookup（書かれていたかと、`unsigned` かは控える・1-6）
    let sign_kw = tokens.iter().any(|t| matches!(*t, "unsigned" | "signed"));
    let unsigned = tokens.contains(&"unsigned");
    let core: Vec<&str> = tokens
        .iter()
        .filter(|t| !matches!(**t, "unsigned" | "signed"))
        .copied()
        .collect();

    // `struct Name` は `Name` と同じ型（C の構造体タグ・4-3）。前方宣言（`typedef struct Name Name;` / `struct Name;`）や
    // 本体のある定義の名前と突き合わせるので、ここで外す。⚠ 外さないと `struct Window*` が知らない名前になる。
    let core: Vec<&str> = match core.as_slice() {
        ["struct", rest @ ..] if !rest.is_empty() => rest.to_vec(),
        _ => core,
    };
    let core_str = core.join(" ");

    // ── 1. User custom type mappings ─────────────────────────────────────────
    if let Some(tl_type) = custom.get(&core_str) {
        use crate::cpp_header::types::IntWidth;
        let base_ct = match tl_type.as_str() {
            // 対応表の "int" / "long" は従来どおり 4 / 8 バイトの符号付き
            "int" => CType::Int { width: IntWidth::W32, signed: true },
            "long" => CType::Int { width: IntWidth::W64, signed: true },
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

    // ── ArPtrProtocol（ffi_value_exchange_plan.md 4-1・4-2）──────────────────────
    // ⚠ ヘッダは `#include` を辿らずに読むので、共有ヘッダの定義は見えない。**名前で**見分ける（`struct` 付きも）。
    //   利用者の対応表（1.）は先に見る（上書きを許す）。typedef の表より先に見る（ヘッダに定義を写してあっても同じ扱い）。
    if matches!(core.as_slice(), ["ArPtrProtocol"] | ["struct", "ArPtrProtocol"]) {
        if ptr_count > 1 {
            return Err("multi-level pointer".to_string());
        }
        let by_pointer = ptr_count == 1;
        return Ok(CType::PtrProtocol { by_pointer, mutable: by_pointer && !is_const });
    }

    // ── 2. System typedef aliases ────────────────────────────────────────────
    // Look up the base type name in the pre-resolved typedef map.
    // The resolved value may itself carry a trailing `*` (e.g. HWND → "void*").
    // ⚠ 組み込みの型（`wchar_t` など）は typedef で置き換えない（`is_builtin_type_name`・7-13）
    if let Some(resolved) = typedefs.get(&core_str).filter(|_| !is_builtin_type_name(&core_str)) {
        let resolved_ptrs = resolved.chars().rev().take_while(|&c| c == '*').count();
        let total_ptrs = ptr_count + resolved_ptrs;
        if total_ptrs > 1 {
            return Err("multi-level pointer".to_string());
        }
        let resolved_base = resolved[..resolved.len() - resolved_ptrs].trim();
        // ⚠ 書いた `const` を残す（7-13）。以前は外していたので、`const TCHAR*` が書き込み用の `char*`（出力バッファ・
        //   `mut` の bytes を求める）に、`const RECT*` が書き込み用の構造体ポインタになっていた
        let konst = if is_const && !resolved_base.split_whitespace().any(|t| t == "const") { "const " } else { "" };
        let combined = if total_ptrs == 1 {
            format!("{konst}{resolved_base} *")
        } else {
            format!("{konst}{resolved_base}")
        };
        // Recurse with empty typedefs to avoid infinite loops
        return parse_c_type_str(combined.trim(), custom, &HashMap::new());
    }

    // ── 3. C/C++ built-in primitives ─────────────────────────────────────────
    if ptr_count > 0 {
        if ptr_count > 1 {
            return Err("multi-level pointer".to_string());
        }
        // char* / wchar_t* → C 文字列。`wchar_t` か、`const` が無い（出力バッファ）かを持つ（1-2）。
        // ⚠ 以前は両方を区別せず、`wchar_t*` にも UTF-8 を渡し、`char*` の出力バッファへの書き込みを捨てていた。
        // 文字列とみなすのは符号の指定が無い `char` だけ。`unsigned char*` / `signed char*` はバイト（小さい整数）
        // へのポインタ（1-6。以前は文字列とみなしていた）。
        if !sign_kw && matches!(core.as_slice(), ["char"] | ["wchar_t"]) {
            return Ok(CType::CharPtr { wide: core == ["wchar_t"], mutable: !is_const });
        }
        // void* → opaque integer handle
        if core == ["void"] {
            return Ok(CType::VoidPtr);
        }
        let inner = match core.as_slice() {
            ["bool"] | ["_Bool"] => CType::Bool,
            ["float"] => CType::Float,
            ["double"] | ["long", "double"] => CType::Double,
            ints if c_int_type(ints, sign_kw, unsigned).is_some() => {
                c_int_type(ints, sign_kw, unsigned).unwrap()
            }
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
        ints if c_int_type(ints, sign_kw, unsigned).is_some() => Ok(c_int_type(ints, sign_kw, unsigned).unwrap()),
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

