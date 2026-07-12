// rs_loader/parse.rs — Rust ソースからのシグネチャ抽出: 再エクスポート追跡、free fn / struct / impl メソッド解析、ABI 互換判定、型変換、パラメータ解析。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt},
    crate::partial_compiler::llvm_codegen::FnExport,
    crate::partial_compiler::module_compiler::{cache_native, native_lib_ext},
};
#[allow(unused_imports)]
use super::*;

// ── Source scanning ───────────────────────────────────────────────────────────

/// Scan all `.rs` files under `src_dir` and collect compatible free functions
/// and struct definitions.
// ── Re-export whitelist ───────────────────────────────────────────────────────

pub(crate) fn collect_reexports(src_dir: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let lib_rs = src_dir.join("lib.rs");
    let Ok(text) = std::fs::read_to_string(&lib_rs) else { return set };
    for line in text.lines() {
        if line.starts_with("pub fn ") {
            if let Some(sig) = parse_single_line_sig(line.trim()) {
                set.insert(sig.name);
            }
        }
        if line.starts_with("pub struct ") {
            if let Some(name) = extract_struct_name(line.trim()) {
                set.insert(name);
            }
        }
    }
    follow_pub_use(&text, src_dir, &mut set, 0);
    set
}

pub(crate) fn follow_pub_use(
    source: &str,
    module_dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 6 { return; }
    for raw in source.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("pub use ") else { continue };
        let rest = rest.trim_end_matches(';');
        if rest.ends_with("::*") {
            let mod_path = rest
                .trim_end_matches("::*")
                .trim_start_matches("self::")
                .replace("::", "/");
            let candidates = [
                module_dir.join(&mod_path).join("mod.rs"),
                module_dir.join(format!("{mod_path}.rs")),
            ];
            for candidate in &candidates {
                if let Ok(text) = std::fs::read_to_string(candidate) {
                    let next_dir = candidate.parent().unwrap_or(module_dir).to_path_buf();
                    for mline in text.lines() {
                        if mline.starts_with("pub fn ") {
                            if let Some(sig) = parse_single_line_sig(mline.trim()) {
                                out.insert(sig.name);
                            }
                        }
                        if mline.starts_with("pub struct ") {
                            if let Some(name) = extract_struct_name(mline.trim()) {
                                out.insert(name);
                            }
                        }
                    }
                    follow_pub_use(&text, &next_dir, out, depth + 1);
                    break;
                }
            }
        } else {
            if let Some(name) = rest.rsplit("::").next() {
                if name.starts_with('{') {
                    let inner = name.trim_start_matches('{').trim_end_matches('}');
                    for item in inner.split(',') {
                        let item = item.trim();
                        if !item.is_empty() && item != ".." {
                            out.insert(item.to_string());
                        }
                    }
                } else if !name.contains('}') && !name.is_empty() {
                    out.insert(name.to_string());
                }
            }
        }
    }
}

pub(crate) fn collect_sigs(
    dir: &Path,
    fns: &mut Vec<RsFnSig>,
    structs: &mut Vec<RsStructSig>,
    seen_fns: &mut std::collections::HashSet<String>,
    seen_structs: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sigs(&path, fns, structs, seen_fns, seen_structs);
        } else if path.extension().map_or(false, |e| e == "rs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                for sig in parse_fn_sigs(&text) {
                    if seen_fns.insert(sig.name.clone()) {
                        fns.push(sig);
                    }
                }
                for st in parse_struct_sigs(&text) {
                    if seen_structs.insert(st.name.clone()) {
                        structs.push(st);
                    }
                }
            }
        }
    }
}

/// Top-level scan: collects free functions, structs, AND digest-pattern wrappers.
pub(crate) fn scan_all_sigs(
    src_dir: &Path,
    crate_ident: &str,
) -> (Vec<RsFnSig>, Vec<RsStructSig>) {
    let whitelist = collect_reexports(src_dir);
    let mut fns: Vec<RsFnSig> = Vec::new();
    let mut structs: Vec<RsStructSig> = Vec::new();
    let mut seen_fns: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_structs: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_sigs(src_dir, &mut fns, &mut structs, &mut seen_fns, &mut seen_structs);

    // Detect Digest-trait pattern and synthesise one-shot hash functions.
    for dig_fn in collect_digest_fns(src_dir, crate_ident) {
        if seen_fns.insert(dig_fn.name.clone()) {
            fns.push(dig_fn);
        }
    }

    if !whitelist.is_empty() {
        // For digest-synthesised functions the whitelist check is skipped —
        // they're generated, not directly re-exported.
        let digest_names: std::collections::HashSet<String> = fns.iter()
            .filter(|f| f.digest_type.is_some())
            .map(|f| f.name.clone())
            .collect();
        fns.retain(|s| whitelist.contains(&s.name) || digest_names.contains(&s.name));
        structs.retain(|s| whitelist.contains(&s.name));
    }
    (fns, structs)
}

// ── Signature parsing — free functions ───────────────────────────────────────

pub(crate) fn parse_fn_sigs(source: &str) -> Vec<RsFnSig> {
    let mut sigs = Vec::new();
    for line in source.lines() {
        if line.starts_with("pub fn ") {
            if let Some(sig) = parse_single_line_sig(line.trim()) {
                sigs.push(sig);
            }
        }
    }
    sigs
}

pub(crate) fn parse_single_line_sig(line: &str) -> Option<RsFnSig> {
    let rest = line.strip_prefix("pub fn ")?;

    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = rest[..name_end].to_string();
    let rest = rest[name_end..].trim_start();

    if rest.starts_with('<') {
        return None;
    }
    if !rest.starts_with('(') {
        return None;
    }
    let paren_end = find_matching(rest, '(', ')')?;
    let params_str = &rest[1..paren_end];
    let rest = rest[paren_end + 1..].trim_start();

    let return_type = if rest.starts_with("->") {
        let ret = rest[2..].trim_start();
        let end = ret
            .find(|c: char| c == '{' || c == ';' || c == 'w')
            .unwrap_or(ret.len());
        let t = ret[..end].trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    } else {
        None
    };

    if let Some(ref rt) = return_type {
        if !is_abi_compatible(rt) {
            return None;
        }
    }

    let params = parse_params(params_str);
    for p in &params {
        if !is_abi_compatible(&p.rust_type) {
            return None;
        }
    }

    Some(RsFnSig { name, params, return_type, digest_type: None })
}

// ── Signature parsing — structs and impl blocks ───────────────────────────────

/// Extract struct name from `pub struct Name {` or `pub struct Name(` line.
/// Returns `None` if the line has generic params.
pub(crate) fn extract_struct_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("pub struct ")?;
    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = rest[..name_end].to_string();
    let after = rest[name_end..].trim_start();
    // Skip generic structs
    if after.starts_with('<') { return None; }
    Some(name)
}

/// Parse all `pub struct Name { ... }` and `impl Name { ... }` blocks from source.
/// Returns a list of `RsStructSig` for each struct that has at least one
/// ABI-compatible field or is fully constructible.
pub(crate) fn parse_struct_sigs(source: &str) -> Vec<RsStructSig> {
    // Collect struct field definitions and impl methods separately,
    // then combine by struct name.
    let struct_fields = collect_struct_fields(source);
    let known_names: Vec<String> = struct_fields.keys().cloned().collect();
    let impl_methods = collect_impl_methods(source, &known_names);

    let mut result = Vec::new();
    for (struct_name, fields) in struct_fields {
        // Need at least one pub field
        if fields.is_empty() { continue; }

        let methods = impl_methods
            .get(&struct_name)
            .cloned()
            .unwrap_or_default();

        // Determine constructor params: prefer fn new(), fall back to field order
        let (ctor_params, use_new_fn) = if let Some(new_method) =
            methods.iter().find(|m| m.name == "new" && m.return_type.as_deref() == Some("Self"))
        {
            (new_method.params.iter().map(|p| RsParam { name: p.name.clone(), rust_type: p.rust_type.clone() }).collect(), true)
        } else {
            (fields.iter().map(|f| RsParam { name: f.name.clone(), rust_type: f.rust_type.clone() }).collect(), false)
        };

        // Filter out the special `new` function from methods (it becomes __init__)
        let methods: Vec<RsMethodSig> = methods
            .into_iter()
            .filter(|m| m.name != "new")
            .collect();

        result.push(RsStructSig {
            name: struct_name,
            fields,
            methods,
            ctor_params,
            use_new_fn,
        });
    }
    result
}

/// Scan source for `pub struct Name { pub field: Type, ... }` definitions.
/// Returns a map of struct_name → Vec<RsField> (only pub ABI-compatible fields).
// ── Digest-trait pattern detection ───────────────────────────────────────────

/// Detect crates that follow the RustCrypto `Digest` pattern:
///   - Re-export `Digest` trait  (`pub use digest::...` or `pub use ...::{..., Digest, ...}`)
///   - Define type aliases        (`pub type Sha256 = CoreWrapper<...>`)
///
/// For each type alias found, synthesise a free function
/// `alias_snake(input: &str) -> String` that calls
/// `crate::AliasName::digest(input.as_bytes())` and returns a lowercase hex string.
///
/// This covers sha2, sha3, md-5, blake2, ripemd, and any other RustCrypto hash crate
/// without requiring the user to write a wrapper.
pub(crate) fn collect_digest_fns(src_dir: &Path, crate_ident: &str) -> Vec<RsFnSig> {
    // Only scan lib.rs at the crate root.
    let lib_rs_path = src_dir.join("lib.rs");
    let source = match std::fs::read_to_string(&lib_rs_path) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    // Check whether this file re-exports the `Digest` trait.
    if !source_exports_digest(&source) {
        return vec![];
    }

    // Collect `pub type Name = ...` aliases (no generics on the alias name itself).
    let mut fns = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pub type ") {
            // Expect `Name = ...` — skip if Name contains '<'
            let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let alias_name = &rest[..name_end];
            if alias_name.is_empty() { continue; }
            let after = rest[name_end..].trim_start();
            if !after.starts_with('=') { continue; }
            // alias_name is now a valid type alias (no generics on the alias itself)
            let fn_name = to_snake_case(alias_name);
            fns.push(RsFnSig {
                name: fn_name,
                params: vec![RsParam { name: "input".to_string(), rust_type: "&str".to_string() }],
                return_type: Some("String".to_string()),
                digest_type: Some(format!("{crate_ident}::{alias_name}")),
            });
        }
    }
    fns
}

/// Returns true if `source` re-exports the `Digest` trait in any of the common forms:
///   `pub use digest::Digest`
///   `pub use digest::{self, Digest}`
///   `pub use digest::{Digest, ...}`
///   `pub use sha3::digest::Digest` etc.
pub(crate) fn source_exports_digest(source: &str) -> bool {
    for line in source.lines() {
        let t = line.trim();
        if !t.starts_with("pub use ") { continue; }
        // Check for `Digest` as a bare import or inside a brace group
        let rest = &t["pub use ".len()..];
        let rest = rest.trim_end_matches(';');
        if rest.ends_with("::Digest") { return true; }
        if let Some(brace_start) = rest.find('{') {
            let inner = &rest[brace_start + 1..];
            let inner = inner.trim_end_matches('}');
            if inner.split(',').any(|item| item.trim() == "Digest" || item.trim() == "self, Digest") {
                return true;
            }
        }
    }
    false
}

/// Convert `CamelCase` to `snake_case`.
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

pub(crate) fn collect_struct_fields(source: &str) -> HashMap<String, Vec<RsField>> {
    let mut result = HashMap::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Match `pub struct Name {` at top-level (not indented in the original)
        if lines[i].starts_with("pub struct ") {
            if let Some(name) = extract_struct_name(line) {
                // Only support brace-delimited structs
                if line.contains('{') {
                    let mut fields = Vec::new();
                    let mut depth = line.chars().filter(|&c| c == '{').count() as i32
                        - line.chars().filter(|&c| c == '}').count() as i32;
                    i += 1;
                    while i < lines.len() && depth > 0 {
                        let fline = lines[i].trim();
                        // Parse `pub field_name: Type,`
                        if depth == 1 {
                            if let Some(field) = parse_struct_field_line(fline) {
                                fields.push(field);
                            }
                        }
                        depth += fline.chars().filter(|&c| c == '{').count() as i32;
                        depth -= fline.chars().filter(|&c| c == '}').count() as i32;
                        i += 1;
                    }
                    if !fields.is_empty() {
                        result.insert(name, fields);
                    }
                    continue;
                }
            }
        }
        i += 1;
    }
    result
}

/// Parse a single struct field line like `pub field_name: Type,`.
/// Returns `None` if the field is not public or the type is not ABI-compatible.
pub(crate) fn parse_struct_field_line(line: &str) -> Option<RsField> {
    // Must start with `pub `
    let rest = line.strip_prefix("pub ")?;
    // Skip `pub(crate)`, `pub(super)`, etc.
    if rest.starts_with('(') { return None; }
    // field_name: Type[,]
    let colon = rest.find(':')?;
    let name = rest[..colon].trim().to_string();
    // Validate name
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let type_str = rest[colon + 1..].trim().trim_end_matches(',').trim().to_string();
    if !is_abi_compatible(&type_str) { return None; }
    Some(RsField { name, rust_type: type_str })
}

/// Scan source for `impl Name { pub fn ... }` blocks.
/// Returns a map of struct_name → Vec<RsMethodSig>.
/// `known_structs` lists struct names defined in the same source so that
/// struct-valued return types are accepted (not filtered out).
pub(crate) fn collect_impl_methods(source: &str, known_structs: &[String]) -> HashMap<String, Vec<RsMethodSig>> {
    let mut result: HashMap<String, Vec<RsMethodSig>> = HashMap::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Match `impl Name {` — no generics, no `impl Trait for Name`
        if line.starts_with("impl ") && !line.contains(" for ") && !line.contains('<') {
            let after_impl = line["impl ".len()..].trim();
            let name_end = after_impl.find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after_impl.len());
            let struct_name = after_impl[..name_end].to_string();
            if struct_name.is_empty() {
                i += 1;
                continue;
            }

            // Track brace depth to stay within this impl block
            let mut depth = trimmed.chars().filter(|&c| c == '{').count() as i32
                - trimmed.chars().filter(|&c| c == '}').count() as i32;
            i += 1;

            while i < lines.len() && depth > 0 {
                let mline = lines[i];
                let mt = mline.trim();

                if depth == 1 && mt.starts_with("pub fn ") {
                    if let Some(msig) = parse_method_line(mt, known_structs) {
                        result.entry(struct_name.clone()).or_default().push(msig);
                    }
                }

                depth += mt.chars().filter(|&c| c == '{').count() as i32;
                depth -= mt.chars().filter(|&c| c == '}').count() as i32;
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    result
}

/// Parse a method signature line like `pub fn method(&self, x: f64) -> f64 {`.
/// Returns `None` for generics, incompatible types, or non-self methods.
/// `known_structs` allows struct-typed return values (wrapped via `cb_call_fn`).
pub(crate) fn parse_method_line(line: &str, known_structs: &[String]) -> Option<RsMethodSig> {
    let rest = line.strip_prefix("pub fn ")?;

    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = rest[..name_end].to_string();
    let rest = rest[name_end..].trim_start();

    // Skip generic methods
    if rest.starts_with('<') { return None; }
    if !rest.starts_with('(') { return None; }

    let paren_end = find_matching(rest, '(', ')')?;
    let params_str = &rest[1..paren_end];
    let rest = rest[paren_end + 1..].trim_start();

    // Determine self kind
    let (self_present, self_mutable, params_without_self) = parse_self_params(params_str)?;
    if !self_present { return None; }

    // Parse remaining params — only primitive types allowed for parameters
    let params = parse_params(params_without_self);
    for p in &params {
        if !is_abi_compatible(&p.rust_type) { return None; }
    }

    // Parse return type — primitive or known struct
    let (return_type, return_struct) = if rest.starts_with("->") {
        let ret = rest[2..].trim_start();
        let end = ret.find(|c: char| c == '{' || c == ';').unwrap_or(ret.len());
        let t = ret[..end].trim().to_string();
        if t == "Self" { return None; }
        if t.is_empty() {
            (None, None)
        } else if is_abi_compatible(&t) {
            (Some(t), None)
        } else if known_structs.contains(&t) {
            // Return type is a struct defined in this crate — constructible via cb_call_fn
            (None, Some(t))
        } else {
            return None;
        }
    } else {
        (None, None)
    };

    Some(RsMethodSig { name, params, self_mutable, return_type, return_struct })
}

/// Split `&self, x: f64, y: f64` into (has_self, is_mut_self, "x: f64, y: f64").
/// Returns None if the params string has no self at all.
pub(crate) fn parse_self_params(params: &str) -> Option<(bool, bool, &str)> {
    let trimmed = params.trim();
    if trimmed.is_empty() { return None; }

    // Check for self variants at the start
    for (prefix, mutable) in &[
        ("&mut self,", true),
        ("&mut self", true),
        ("mut self,", true),
        ("mut self", true),
        ("&self,", false),
        ("&self", false),
        ("self,", false),
        ("self", false),
    ] {
        if trimmed.starts_with(prefix) {
            let rest = trimmed[prefix.len()..].trim_start_matches(',').trim();
            return Some((true, *mutable, rest));
        }
    }
    None
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 注意: `&mut T` は**意図的に**非対応（この関数が拒否 → 該当関数/メソッドは
/// スタブ生成ごと除外される）。将来対応する場合は、stubs.rs 側で該当パラメータを
/// `Param::bridge(…, writable_ref=true)` で `mut` マーキングし、cpp/C# ブリッジと
/// 同じ静的可変性検査（CallMutParamWithImmutableArg）を維持すること。
pub(crate) fn is_abi_compatible(t: &str) -> bool {
    let t = t.trim();
    if matches!(
        t,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "f32" | "f64"
        | "bool"
        | "String" | "&str" | "&String"
        | "&[u8]"   // byte slice input → HV str
        | "Vec<u8>" // byte vec output → HV str (hex)
    ) {
        return true;
    }
    // Fixed-size byte arrays: [u8; N] — output as hex str
    if is_fixed_byte_array(t) {
        return true;
    }
    false
}

/// Returns true for `[u8; N]` where N is a decimal integer.
pub(crate) fn is_fixed_byte_array(t: &str) -> bool {
    if let Some(inner) = t.strip_prefix("[u8; ").and_then(|s| s.strip_suffix(']')) {
        return inner.trim().chars().all(|c| c.is_ascii_digit());
    }
    false
}

pub(crate) fn find_matching(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == open { depth += 1; }
        else if c == close {
            depth -= 1;
            if depth == 0 { return Some(i); }
        }
    }
    None
}

pub(crate) fn parse_params(s: &str) -> Vec<RsParam> {
    if s.trim().is_empty() { return vec![]; }
    let mut params = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if let Some(p) = parse_one_param(&s[start..i]) { params.push(p); }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(p) = parse_one_param(&s[start..]) { params.push(p); }
    params
}

pub(crate) fn parse_one_param(s: &str) -> Option<RsParam> {
    let s = s.trim();
    if matches!(s, "self" | "&self" | "&mut self" | "mut self") { return None; }
    let colon = s.find(':')?;
    let raw_name = s[..colon].trim();
    let name = raw_name.trim_start_matches("mut").trim().to_string();
    let rust_type = s[colon + 1..].trim().to_string();
    if name.is_empty() || rust_type.is_empty() { return None; }
    Some(RsParam { name, rust_type })
}

pub(crate) fn rust_type_to_ar(rt: &str) -> &'static str {
    match rt.trim() {
        "i8" | "i16" | "i32" | "i64" | "i128"
        | "u8" | "u16" | "u32" | "u64" | "u128"
        | "isize" | "usize" => "int",
        "f32" | "f64" => "float",
        "bool" => "bool",
        "String" | "&str" | "&String" => "str",
        // Byte types: pass as / return as str (input = raw bytes; output = hex)
        "&[u8]" | "Vec<u8>" => "str",
        _ if is_fixed_byte_array(rt.trim()) => "str",
        _ => "Any",
    }
}

