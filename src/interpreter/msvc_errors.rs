// msvc_errors.rs — Parse MSVC compiler / linker diagnostic output.
//
// Strategy: instead of matching specific error codes (C2039, C2733, C2664, LNK2019, …),
// scan every MSVC diagnostic line with three extraction patterns in priority order:
//   1. Single-quoted identifier  'Name'  — C-series type / member / overload errors
//   2. Mangled C++ symbol        (?Name@…)  — LNK-series unresolved-external
//   3. "referenced in function"  — English-locale LNK fallback
//
// Extracted names are intersected with the caller's compiled-function list by the
// caller (`effective_sigs.retain`), so false positives (type names, namespace names)
// are silently discarded without removing valid functions.

use std::collections::HashSet;

/// Extract C identifiers of functions that caused MSVC errors.
/// The caller should remove any returned name that matches a compiled function
/// and retry compilation.
pub fn extract_bad_fn_names(err: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for line in err.lines() {
        if !is_msvc_diagnostic(line) {
            continue;
        }
        extract_from_line(line, &mut names);
    }
    names
}

/// MSVC コンパイラまたはリンカの診断行かどうかを判定する。
/// `error C` または `error LNK` を含む行を診断行とみなす。
fn is_msvc_diagnostic(line: &str) -> bool {
    line.contains(": error C")
        || line.contains(": error LNK")
        || line.contains(": fatal error C")
        || line.contains(": fatal error LNK")
}

/// MSVC 診断行から問題のある関数・シンボル名を抽出して `names` セットに追加する。
/// シングルクォート囲みの識別子、マングル済み C++ シンボル、`referenced in function` 形式の3パターンを試みる。
fn extract_from_line(line: &str, names: &mut HashSet<String>) {
    // Pattern 1: single-quoted identifier 'Name'
    // Appears in C2039 ("not a member"), C2733 ("overload in extern C"),
    // C2664 ("cannot convert argument") and similar diagnostics.
    //
    // C2664 error format (Japanese locale):
    //   error C2664: 'ReturnType DxLib::FunctionName(params)': 引数 N が...
    // The single-quoted section contains the full C++ signature, not just a name.
    // We extract the function name: last identifier before the first '(' inside the quoted string.
    let mut scan = line;
    while let Some(q1) = scan.find('\'') {
        let after = &scan[q1 + 1..];
        match after.find('\'') {
            Some(q2) => {
                let candidate = &after[..q2];
                if is_valid_c_ident(candidate) {
                    names.insert(candidate.to_string());
                } else if let Some(paren) = candidate.find('(') {
                    // C2664-style: extract function name before first '('
                    // e.g. "int DxLib::SetASyncLoadFinishCallback" → "SetASyncLoadFinishCallback"
                    let before_paren = &candidate[..paren];
                    let fn_name = before_paren
                        .split("::")
                        .last()
                        .and_then(|s| s.trim().split_whitespace().last())
                        .unwrap_or("");
                    if is_valid_c_ident(fn_name) {
                        names.insert(fn_name.to_string());
                    }
                }
                scan = &after[q2 + 1..];
            }
            None => break,
        }
    }

    // Pattern 2: mangled C++ name (?FunctionName@…)
    // LNK2019 unresolved-external symbols use decorated names; the plain
    // function name follows the leading `(?`.
    if let Some(mq) = line.find("(?") {
        let rest = &line[mq + 2..];
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }

    // Pattern 3: English-locale "referenced in function X" fallback
    // Used when the system locale renders LNK2019 body text in English.
    if let Some(pos) = line.rfind("referenced in function ") {
        let rest = &line[pos + "referenced in function ".len()..];
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }
}

/// 文字列が有効な C 識別子かどうかを判定する。
/// 先頭がアルファベットまたはアンダースコアで、残りがすべて英数字またはアンダースコアであれば `true`。
fn is_valid_c_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .map_or(false, |c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}
