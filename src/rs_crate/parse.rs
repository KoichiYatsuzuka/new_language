// rs_crate/parse.rs — Rust ソースからのシグネチャ抽出: 再エクスポート追跡、free fn / struct / impl メソッド解析、ABI 互換判定、型変換、パラメータ解析。

use {
    std::collections::HashMap, std::path::Path,
};
use super::*;

// ── Source scanning ───────────────────────────────────────────────────────────

/// Scan all `.rs` files under `src_dir` and collect compatible free functions
/// and struct definitions.
// ── Re-export whitelist ───────────────────────────────────────────────────────

pub(crate) fn collect_reexports(src_dir: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let lib_rs = src_dir.join("lib.rs");
    let Ok(text) = crate::import_fs::read_to_string(&lib_rs) else { return set };
    for line in text.lines() {
        // ⚠ 呼べない関数の名前も載せる（未対応として報告するため・2-5）
        if line.starts_with("pub fn ") {
            if let Some(name) = fn_name_of(line) {
                set.insert(name);
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
                if let Ok(text) = crate::import_fs::read_to_string(candidate) {
                    let next_dir = candidate.parent().unwrap_or(module_dir).to_path_buf();
                    for mline in text.lines() {
                        if mline.starts_with("pub fn ") {
                            if let Some(name) = fn_name_of(mline) {
                                out.insert(name);
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
    unsupported: &mut Vec<RsUnsupported>,
    seen_fns: &mut std::collections::HashSet<String>,
    seen_structs: &mut std::collections::HashSet<String>,
) {
    let entries = match crate::import_fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for path in entries {
        if crate::import_fs::is_dir(&path) {
            collect_sigs(&path, fns, structs, unsupported, seen_fns, seen_structs);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(text) = crate::import_fs::read_to_string(&path) {
                let (ok, bad) = parse_fn_sigs(&text);
                for sig in ok {
                    if seen_fns.insert(sig.name.clone()) {
                        fns.push(sig);
                    }
                }
                for u in bad {
                    if seen_fns.insert(u.name.clone()) {
                        unsupported.push(u);
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
/// The third element is the free functions Arrow cannot call yet, with the reason (ffi_value_exchange_plan.md 2-5:
/// they used to disappear silently at import).
pub(crate) fn scan_all_sigs(
    src_dir: &Path,
    crate_ident: &str,
) -> (Vec<RsFnSig>, Vec<RsStructSig>, Vec<RsUnsupported>) {
    let whitelist = collect_reexports(src_dir);
    let mut fns: Vec<RsFnSig> = Vec::new();
    let mut structs: Vec<RsStructSig> = Vec::new();
    let mut unsupported: Vec<RsUnsupported> = Vec::new();
    let mut seen_fns: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_structs: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_sigs(src_dir, &mut fns, &mut structs, &mut unsupported, &mut seen_fns, &mut seen_structs);

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
        unsupported.retain(|u| whitelist.contains(&u.name));
    }
    (fns, structs, unsupported)
}

// ── Signature text ────────────────────────────────────────────────────────────

/// `pub fn` から始まるシグネチャを、複数行に分かれていても 1 行にまとめる（2-5）。
/// 括弧が閉じ、本体の `{` か `;` が現れるまでの行をつなぐ。戻り値は（まとめた文字列、消費した行数）。
/// ⚠ 以前は 1 行に収まるシグネチャしか読めず、`rustfmt` が折り返した関数は黙って消えていた。
pub(crate) fn signature_text(lines: &[&str], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut depth = 0i32;
    let mut opened = false;
    let mut n = 0;
    for line in &lines[start..] {
        n += 1;
        let t = line.trim();
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(t);
        for c in t.chars() {
            match c {
                '(' => { depth += 1; opened = true; }
                ')' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 && (t.contains('{') || t.ends_with(';')) {
            break;
        }
        if n > 40 {
            break;
        }
    }
    (text, n)
}

/// `pub fn name` の関数名（シグネチャが読めなくても名前だけは取る）。
pub(crate) fn fn_name_of(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("pub fn ")?;
    let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
}

/// 型が渡せないときの説明（`parameter `x` is …` / `it returns …` に続ける）。
fn type_problem(t: &str) -> String {
    let t = t.trim();
    if t.starts_with("&mut ") {
        format!("`{t}`, a mutable reference, which cannot cross the bridge")
    } else {
        format!("`{t}`, which cannot cross the bridge yet")
    }
}

/// 戻り値の型の文字列（`-> T` の `T`。`where` 節と本体の `{` の手前まで）。
fn return_type_of(after_params: &str) -> Option<String> {
    let ret = after_params.trim_start().strip_prefix("->")?.trim_start();
    // 括弧の外の `{` / `;` で切る（5-4）。⚠ 以前は `[u8; 4]` の中の `;` で切って `[u8` と読み、`[u8; N]` を返す関数は
    //   型の分からない関数として未対応になっていた
    let mut depth = 0i32;
    let mut end = ret.len();
    for (i, c) in ret.char_indices() {
        match c {
            '[' | '(' | '<' => depth += 1,
            ']' | ')' | '>' => depth -= 1,
            '{' | ';' if depth <= 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    if let Some(w) = ret.find(" where ") {
        end = end.min(w);
    }
    let t = ret[..end].trim().to_string();
    (!t.is_empty()).then_some(t)
}

// ── Signature parsing — free functions ───────────────────────────────────────

/// 最上位の `pub fn` を、Arrow から呼べるもの（`Ok`）と呼べないもの（理由つき）に分ける。
pub(crate) fn parse_fn_sigs(source: &str) -> (Vec<RsFnSig>, Vec<RsUnsupported>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("pub fn ") {
            let (text, n) = signature_text(&lines, i);
            match classify_fn_sig(&text) {
                Some(Ok(sig)) => ok.push(sig),
                Some(Err(u)) => bad.push(u),
                None => {}
            }
            i += n;
        } else {
            i += 1;
        }
    }
    (ok, bad)
}

/// 1 行にまとめたシグネチャを読む。名前すら読めなければ `None`。
pub(crate) fn classify_fn_sig(text: &str) -> Option<Result<RsFnSig, RsUnsupported>> {
    let name = fn_name_of(text)?;
    let unsupported = |params: Vec<RsParam>, ret: Option<String>, reason: String| {
        Some(Err(RsUnsupported { name: name.clone(), params, return_type: ret, reason }))
    };
    let rest = text.trim().strip_prefix("pub fn ")?[name.len()..].trim_start();
    if rest.starts_with('<') {
        // 引数の個数は型スタブに出す（型は `Any`）。出さないと「引数の個数が違う」という偽の誤りが重なる。
        let after = find_matching(rest, '<', '>').map_or("", |e| rest[e + 1..].trim_start());
        let params = find_matching(after, '(', ')').map(|e| parse_params(&after[1..e])).unwrap_or_default();
        let params = params.into_iter().filter(|p| !matches!(p.name.as_str(), "self")).collect();
        return unsupported(params, None, "it is generic".to_string());
    }
    let Some(paren_end) = rest.starts_with('(').then(|| find_matching(rest, '(', ')')).flatten() else {
        return unsupported(vec![], None, "its signature could not be read".to_string());
    };
    let params = parse_params(&rest[1..paren_end]);
    let return_type = return_type_of(&rest[paren_end + 1..]);
    if let Some(rt) = &return_type {
        if !is_return_type(rt) {
            let why = format!("it returns {}", type_problem(rt));
            return unsupported(params, return_type.clone(), why);
        }
    }
    for p in &params {
        if !is_param_type(&p.rust_type) {
            let why = format!("parameter `{}` is {}", p.name, type_problem(&p.rust_type));
            return unsupported(params.clone(), return_type, why);
        }
    }
    Some(Ok(RsFnSig { name, params, return_type, digest_type: None }))
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

/// `pub struct Name { ... }` のフィールドの読み取り結果。
pub(crate) struct StructFields {
    /// Arrow から読み書きできる `pub` フィールド
    pub(crate) fields: Vec<RsField>,
    /// private フィールド、または Arrow が渡せない型の `pub` フィールドがある（構造体リテラルで作れない）
    pub(crate) literal_blocked: bool,
}

/// Parse all `pub struct Name { ... }` and `impl Name { ... }` blocks from source.
///
/// 構築の方法: `pub fn new(..) -> Self` があればそれ、無ければ（全フィールドが `pub` で渡せる型なら）構造体リテラル。
/// どちらも使えなければ構築を未対応として印を付ける（2-5）。⚠ 以前は `new` を見つける処理が `self` の無い関数を
/// 捨てていたので `new` は一度も使われず、private フィールドを持つ構造体は中継 DLL のビルドが失敗して import 全体が
/// 落ちていた。`pub` フィールドが 1 つも無い構造体（`new` とメソッドだけで使う型）も黙って捨てていた。
pub(crate) fn parse_struct_sigs(source: &str) -> Vec<RsStructSig> {
    let struct_fields = collect_struct_fields(source);
    let known_names: Vec<String> = struct_fields.keys().cloned().collect();
    let mut impl_items = collect_impl_items(source, &known_names);

    let mut result = Vec::new();
    for (struct_name, sf) in struct_fields {
        let items = impl_items.remove(&struct_name).unwrap_or_default();
        let literal_ok = !sf.literal_blocked;
        let (ctor_params, use_new_fn, ctor_unsupported) = match items.new_fn {
            Some(Ok(params)) => (params, true, None),
            Some(Err(_)) | None if literal_ok => (
                sf.fields.iter().map(|f| RsParam { name: f.name.clone(), rust_type: f.rust_type.clone() }).collect(),
                false,
                None,
            ),
            Some(Err(why)) => (vec![], false, Some(format!("its `new` cannot be called: {why}"))),
            None => (
                vec![],
                false,
                Some("it has private fields (or fields Arrow cannot pass) and no `pub fn new(..) -> Self`".to_string()),
            ),
        };
        result.push(RsStructSig {
            name: struct_name,
            fields: sf.fields,
            methods: items.methods,
            ctor_params,
            use_new_fn,
            ctor_unsupported,
            unsupported_methods: items.unsupported,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

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
    let source = match crate::import_fs::read_to_string(&lib_rs_path) {
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

/// Scan source for `pub struct Name { ... }` definitions (every brace-delimited, non-generic one).
pub(crate) fn collect_struct_fields(source: &str) -> HashMap<String, StructFields> {
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
                    let mut literal_blocked = false;
                    let mut depth = line.chars().filter(|&c| c == '{').count() as i32
                        - line.chars().filter(|&c| c == '}').count() as i32;
                    i += 1;
                    while i < lines.len() && depth > 0 {
                        let fline = lines[i].trim();
                        if depth == 1 && is_field_line(fline) {
                            match parse_struct_field_line(fline) {
                                Some(field) => fields.push(field),
                                // private, or a pub field of a type Arrow cannot pass
                                None => literal_blocked = true,
                            }
                        }
                        depth += fline.chars().filter(|&c| c == '{').count() as i32;
                        depth -= fline.chars().filter(|&c| c == '}').count() as i32;
                        i += 1;
                    }
                    result.insert(name, StructFields { fields, literal_blocked });
                    continue;
                }
            }
        }
        i += 1;
    }
    result
}

/// フィールドの行か（`name: Type,`・`pub name: Type,`）。属性・コメント・閉じ括弧は除く。
fn is_field_line(line: &str) -> bool {
    !line.is_empty()
        && !line.starts_with("//")
        && !line.starts_with('#')
        && !line.starts_with('}')
        && line.contains(':')
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
    if !is_field_type(&type_str) { return None; }
    Some(RsField { name, rust_type: type_str })
}

/// `impl Name { ... }` から拾ったもの。
#[derive(Default)]
pub(crate) struct ImplItems {
    pub(crate) methods: Vec<RsMethodSig>,
    pub(crate) unsupported: Vec<RsUnsupported>,
    /// `pub fn new(..) -> Self`（引数が渡せる型なら `Ok`、渡せなければ理由）。
    pub(crate) new_fn: Option<Result<Vec<RsParam>, String>>,
}

/// Scan source for `impl Name { pub fn ... }` blocks.
/// `known_structs` lists struct names defined in the same source so that
/// struct-valued return types are accepted (not filtered out).
pub(crate) fn collect_impl_items(source: &str, known_structs: &[String]) -> HashMap<String, ImplItems> {
    let mut result: HashMap<String, ImplItems> = HashMap::new();
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
                let mt = lines[i].trim();
                if depth == 1 && mt.starts_with("pub fn ") {
                    let (text, n) = signature_text(&lines, i);
                    let items = result.entry(struct_name.clone()).or_default();
                    match classify_method(&text, &struct_name, known_structs) {
                        Some(MethodKind::Method(m)) => items.methods.push(m),
                        Some(MethodKind::New(r)) => items.new_fn = Some(r),
                        Some(MethodKind::Unsupported(u)) => items.unsupported.push(u),
                        None => {}
                    }
                    // the signature lines may contain braces (the body's `{` on the last line)
                    for l in &lines[i..i + n] {
                        let t = l.trim();
                        depth += t.chars().filter(|&c| c == '{').count() as i32;
                        depth -= t.chars().filter(|&c| c == '}').count() as i32;
                    }
                    i += n;
                    continue;
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

/// `impl` の中の `pub fn` の分類。
pub(crate) enum MethodKind {
    Method(RsMethodSig),
    /// `pub fn new(..) -> Self`
    New(Result<Vec<RsParam>, String>),
    Unsupported(RsUnsupported),
}

/// Parse a method signature (already joined into one line) like `pub fn method(&self, x: f64) -> f64 {`.
/// `-> Self` は構造体自身（2-5。以前は捨てていた）。`self` の無い関数は `new` 以外未対応、`self` を値で受ける
/// （消費する）メソッドも未対応（中継 DLL はオブジェクトを表から借りて呼ぶので、取り出せない）。
pub(crate) fn classify_method(text: &str, struct_name: &str, known_structs: &[String]) -> Option<MethodKind> {
    let name = fn_name_of(text)?;
    let unsupported = |params: Vec<RsParam>, ret: Option<String>, reason: String| {
        Some(MethodKind::Unsupported(RsUnsupported { name: name.clone(), params, return_type: ret, reason }))
    };
    let rest = text.trim().strip_prefix("pub fn ")?[name.len()..].trim_start();
    if rest.starts_with('<') {
        // 引数の個数は型スタブに出す（型は `Any`）。出さないと「引数の個数が違う」という偽の誤りが重なる。
        let after = find_matching(rest, '<', '>').map_or("", |e| rest[e + 1..].trim_start());
        let params = find_matching(after, '(', ')').map(|e| parse_params(&after[1..e])).unwrap_or_default();
        let params = params.into_iter().filter(|p| !matches!(p.name.as_str(), "self")).collect();
        return unsupported(params, None, "it is generic".to_string());
    }
    let Some(paren_end) = rest.starts_with('(').then(|| find_matching(rest, '(', ')')).flatten() else {
        return unsupported(vec![], None, "its signature could not be read".to_string());
    };
    let params_str = &rest[1..paren_end];
    let return_type = return_type_of(&rest[paren_end + 1..]);
    let returns_self = return_type.as_deref().is_some_and(|t| t == "Self" || t == struct_name);

    let Some((self_mutable, by_value, params_without_self)) = parse_self_params(params_str) else {
        // no `self`: an associated function
        let params = parse_params(params_str);
        if name == "new" && returns_self {
            return Some(MethodKind::New(match params.iter().find(|p| !is_param_type(&p.rust_type)) {
                None => Ok(params),
                Some(p) => Err(format!("parameter `{}` is {}", p.name, type_problem(&p.rust_type))),
            }));
        }
        return unsupported(
            params,
            return_type,
            "it is an associated function (no `self`), which cannot be called yet".to_string(),
        );
    };
    let params = parse_params(params_without_self);
    if by_value {
        return unsupported(params, return_type, "it takes `self` by value (it consumes the object)".to_string());
    }
    for p in &params {
        if !is_param_type(&p.rust_type) {
            let why = format!("parameter `{}` is {}", p.name, type_problem(&p.rust_type));
            return unsupported(params.clone(), return_type, why);
        }
    }
    let (ret_prim, return_struct) = match return_type.as_deref() {
        None | Some("()") => (None, None),
        Some(_) if returns_self => (None, Some(struct_name.to_string())),
        Some(t) if is_return_type(t) => (Some(t.to_string()), None),
        Some(t) if known_structs.iter().any(|k| k == t) => (None, Some(t.to_string())),
        Some(t) => {
            let why = format!("it returns {}", type_problem(t));
            return unsupported(params, return_type.clone(), why);
        }
    };
    Some(MethodKind::Method(RsMethodSig { name, params, self_mutable, return_type: ret_prim, return_struct }))
}

/// Split `&self, x: f64, y: f64` into (is_mut, by_value, "x: f64, y: f64").
/// Returns None if the params string has no self at all.
pub(crate) fn parse_self_params(params: &str) -> Option<(bool, bool, &str)> {
    let trimmed = params.trim();
    if trimmed.is_empty() { return None; }

    // (prefix, mutable, by value)
    for (prefix, mutable, by_value) in &[
        ("&mut self", true, false),
        ("&self", false, false),
        ("mut self", true, true),
        ("self", false, true),
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            // `selfish: T` is not `self`
            if rest.is_empty() || rest.trim_start().starts_with(',') {
                let rest = rest.trim_start().trim_start_matches(',').trim();
                return Some((*mutable, *by_value, rest));
            }
        }
    }
    None
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// 注意: `&mut T` は**意図的に**非対応（`is_param_type` が拒否 → 該当関数/メソッドは
/// 未対応として印付きでスタブに載る・2-5）。将来対応する場合は、stubs.rs 側で該当パラメータを
/// `Param::bridge(…, writable_ref=true)` で `mut` マーキングし、cpp/C# ブリッジと
/// 同じ静的可変性検査（CallMutParamWithImmutableArg）を維持すること。
/// 中継 DLL が**引数**として受け取れる型（2-5・5-4）。`&[u8]` / `Vec<u8>` は bytes、数値の `&[T]` / `Vec<T>` は `Ptr[T]` か
/// list（コピー）、`Vec<String>` / `&[String]` / `&[&str]` は list[str]、`Option<T>` は None か T。
pub(crate) fn is_param_type(t: &str) -> bool {
    let t = t.trim();
    is_scalar_type(t)
        || matches!(t, "&str" | "&String" | "&[u8]" | "Vec<u8>" | "Vec<String>" | "&[String]" | "&[&str]")
        || generic_inner(t, "Vec<").or_else(|| slice_inner(t)).is_some_and(|e| num_elem(e).is_some())
        || generic_inner(t, "Option<").is_some_and(is_scalar_type)
}

/// 中継 DLL が**戻り値**として返せる型（2-5・5-4）。`Vec<u8>` / `[u8; N]` は bytes（以前は 16 進文字列）、数値の `Vec<T>` は
/// 所有の `Ptr[T]`（D-16）、`Vec<String>` / `Vec<bool>` は list、`Option<T>` は None か T。⚠ `&[u8]` / `&String` は返せない。
pub(crate) fn is_return_type(t: &str) -> bool {
    let t = t.trim();
    is_scalar_type(t)
        || matches!(t, "&str" | "Vec<u8>" | "Vec<String>" | "Vec<bool>")
        || is_fixed_byte_array(t)
        || generic_inner(t, "Vec<").is_some_and(|e| num_elem(e).is_some())
        || generic_inner(t, "Option<").is_some_and(is_scalar_type)
}

/// 数値・bool・`String`（`Option<T>` の中身にもなる）。
fn is_scalar_type(t: &str) -> bool {
    matches!(
        t.trim(),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "f32" | "f64" | "bool" | "String"
    )
}

/// 数値の要素型 → `ArPtrProtocol` の要素の型名（`Vec<T>` / `&[T]`・5-4）。`u8` は bytes（D-9）、`bool` は list
/// （`Ptr` の要素に bool は無い）、`i128` / `u128` は C の型に無いので対象外。
pub(crate) fn num_elem(t: &str) -> Option<&'static str> {
    Some(match t.trim() {
        "i8" => "int8",
        "i16" => "int16",
        "i32" => "int32",
        "i64" | "isize" => "int64",
        "u16" => "uint16",
        "u32" => "uint32",
        "u64" | "usize" => "uint64",
        "f32" => "float32",
        "f64" => "float64",
        _ => return None,
    })
}

/// `Vec<T>` / `Option<T>` の `T`（`head` は `"Vec<"` などの頭）。
pub(crate) fn generic_inner<'a>(t: &'a str, head: &str) -> Option<&'a str> {
    t.trim().strip_prefix(head)?.strip_suffix('>').map(str::trim)
}

/// `&[T]` の `T`。
pub(crate) fn slice_inner(t: &str) -> Option<&str> {
    t.trim().strip_prefix("&[")?.strip_suffix(']').map(str::trim)
}

/// 構造体の `pub` フィールドとして読み書きできる型（ゲッターとセッターの両方が要る・2-5）。
pub(crate) fn is_field_type(t: &str) -> bool {
    matches!(
        t.trim(),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "f32" | "f64" | "bool" | "String"
    )
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

/// 戻り値・フィールドの Arrow の型（引数は [`rust_param_type_to_ar`]）。
pub(crate) fn rust_type_to_ar(rt: &str) -> String {
    let t = rt.trim();
    if let Some(inner) = generic_inner(t, "Option<") {
        return format!("Option[{}]", rust_type_to_ar(inner));
    }
    if let Some(e) = generic_inner(t, "Vec<").or_else(|| slice_inner(t)) {
        if let Some(n) = num_elem(e) {
            return format!("Ptr[{n}]");
        }
        match e {
            // ⚠ 5-4 より前は 16 進文字列（`"6869"`）だった
            "u8" => return "bytes".to_string(),
            "String" | "&str" => return "list[str]".to_string(),
            "bool" => return "list[bool]".to_string(),
            _ => {}
        }
    }
    match t {
        "i8" | "i16" | "i32" | "i64" | "i128"
        | "u8" | "u16" | "u32" | "u64" | "u128"
        | "isize" | "usize" => "int",
        "f32" | "f64" => "float",
        "bool" => "bool",
        "String" | "&str" | "&String" => "str",
        _ if is_fixed_byte_array(t) => "bytes",
        _ => "Any",
    }
    .to_string()
}

/// 引数の Arrow の型（5-4）。数値の `&[T]` / `Vec<T>` は `Ptr[T]`（Rust や C から受け取ったもの）か list（コピー）を受ける。
pub(crate) fn rust_param_type_to_ar(rt: &str) -> String {
    let t = rt.trim();
    if let Some(inner) = generic_inner(t, "Option<") {
        return format!("Option[{}]", rust_param_type_to_ar(inner));
    }
    if let Some(n) = generic_inner(t, "Vec<").or_else(|| slice_inner(t)).and_then(num_elem) {
        let list = if n.starts_with("float") { "float" } else { "int" };
        return format!("Union[Ptr[{n}], list[{list}]]");
    }
    rust_type_to_ar(t)
}


#[cfg(test)]
mod probe_tests {
    use super::*;

    fn probe_src() -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|d| d.join("examples/interop/rs_crates/ffi_probe_rs/src/lib.rs"))
            .find(|p| p.exists())
            .expect("ffi_probe_rs/src/lib.rs");
        std::fs::read_to_string(p).unwrap()
    }

    /// `new()` があればそれで、無ければ構造体リテラルで、どちらも無理なら構築を未対応にする（2-5）。
    #[test]
    fn construction_follows_new_or_literal() {
        let structs = parse_struct_sigs(&probe_src());
        let get = |n: &str| structs.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("{n}"));
        assert!(get("Account").use_new_fn && get("Account").ctor_unsupported.is_none());
        assert!(get("Person").use_new_fn);
        assert!(!get("Counter").use_new_fn && get("Counter").ctor_unsupported.is_none());
        // `pub` フィールドが無くても型として拾う
        assert!(get("Secret").use_new_fn && get("Secret").fields.is_empty());
        assert!(get("Sealed").ctor_unsupported.as_deref().is_some_and(|w| w.contains("private fields")));
    }

    /// `-> Self` は構造体自身。`self` の無い関連関数と `self` を消費するメソッドは理由つきで未対応（2-5）。
    #[test]
    fn methods_are_classified() {
        let structs = parse_struct_sigs(&probe_src());
        let counter = structs.iter().find(|s| s.name == "Counter").unwrap();
        let doubled = counter.methods.iter().find(|m| m.name == "doubled").expect("doubled");
        assert_eq!(doubled.return_struct.as_deref(), Some("Counter"));
        let seal = counter.methods.iter().find(|m| m.name == "seal").expect("seal");
        assert_eq!(seal.return_struct.as_deref(), Some("Sealed"));
        let why = |n: &str| counter.unsupported_methods.iter().find(|u| u.name == n).map(|u| u.reason.clone());
        assert!(why("zero").is_some_and(|w| w.contains("associated function")));
        assert!(why("into_n").is_some_and(|w| w.contains("by value")));
    }

    /// 複数行のシグネチャも読む。呼べない関数は理由つきで返す（以前は黙って消えた）。
    #[test]
    fn free_functions_are_classified() {
        let (ok, bad) = parse_fn_sigs(&probe_src());
        assert!(ok.iter().any(|f| f.name == "add_three" && f.params.len() == 3));
        let why = |n: &str| bad.iter().find(|u| u.name == n).map(|u| u.reason.clone()).unwrap_or_default();
        // 5-4: Vec<String> / &[i64] / Vec<u8> / Option<String> は受け渡せる
        for n in ["words", "sum", "count_bytes", "maybe"] {
            assert!(ok.iter().any(|f| f.name == n), "{n}");
        }
        assert!(why("pair").starts_with("it returns `(i32, String)`"));
        assert!(why("bump").contains("mutable reference"));
        assert_eq!(why("ident"), "it is generic");
        assert_eq!(bad.iter().find(|u| u.name == "ident").unwrap().params.len(), 1);
        assert!(ok.iter().all(|f| !["pair", "bump", "ident"].contains(&f.name.as_str())));
    }

    #[test]
    fn types_are_accepted_by_position() {
        assert!(is_param_type("&[u8]") && !is_return_type("&[u8]"));
        assert!(is_return_type("Vec<u8>") && is_param_type("Vec<u8>"));
        assert!(is_return_type("[u8; 32]") && !is_param_type("[u8; 32]"));
        assert!(is_field_type("String") && !is_field_type("&str") && !is_field_type("Vec<u8>"));
        // 5-4: 数値の Vec / slice・Vec<String>・Option<T>
        assert!(is_param_type("&[i32]") && is_param_type("Vec<f64>") && is_return_type("Vec<u32>"));
        assert!(!is_return_type("&[i32]") && !is_param_type("Vec<i128>") && !is_param_type("Vec<bool>"));
        assert!(is_param_type("&[&str]") && is_return_type("Vec<String>") && is_return_type("Vec<bool>"));
        assert!(is_param_type("Option<String>") && is_return_type("Option<i64>") && !is_param_type("Option<&str>"));
    }

    /// 型スタブの型（5-4・D-16）。数値の Vec は `Ptr[T]`、引数は `Ptr[T]` か list を受ける。
    #[test]
    fn arrow_types_of_collections() {
        assert_eq!(rust_type_to_ar("Vec<u8>"), "bytes");
        assert_eq!(rust_type_to_ar("[u8; 4]"), "bytes");
        assert_eq!(rust_type_to_ar("Vec<i32>"), "Ptr[int32]");
        assert_eq!(rust_type_to_ar("Vec<String>"), "list[str]");
        assert_eq!(rust_type_to_ar("Option<String>"), "Option[str]");
        assert_eq!(rust_param_type_to_ar("&[f64]"), "Union[Ptr[float64], list[float]]");
        assert_eq!(rust_param_type_to_ar("Vec<usize>"), "Union[Ptr[uint64], list[int]]");
        assert_eq!(rust_param_type_to_ar("&[u8]"), "bytes");
    }
}
