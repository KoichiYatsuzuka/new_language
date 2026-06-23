/// Native module loader for `import[rs]` — reads crate source directly from the
/// cargo registry (or a local path), auto-discovers compatible `pub fn` and
/// `pub struct` + `impl` blocks, and generates call-through ABI wrappers.
///
/// # Config (`ar_config.json` — `rust.crates_path`)
///
/// ```json
/// {
///   "rust": {
///     "crates_path": "/path/to/cargo/registry/src/index.crates.io-..."
///   }
/// }
/// ```
///
/// # Compatibility rules
///
/// **Free functions** — wrapped when ALL of the following hold:
/// - No generic type parameters
/// - Every parameter and return type is a Arrow primitive: `i*`, `u*`,
///   `f32`, `f64`, `bool`, `String`, `&str`
///
/// **Structs** — wrapped when ALL of the following hold:
/// - No generic type parameters on the struct
/// - All `pub` fields have ABI-compatible types
/// - Constructor: either `pub fn new(...) -> Self` exists, or all fields are `pub`
///
/// **Struct methods** — wrapped when ALL of the following hold:
/// - `&self` or `&mut self` receiver (no generic params)
/// - All parameter and return types are ABI-compatible

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt};

use super::llvm_codegen::FnExport;
use super::module_compiler::{cache_native, native_lib_ext};

// ── Internal types ────────────────────────────────────────────────────────────

/// Free function signature from a Rust crate.
struct RsFnSig {
    name: String,
    params: Vec<RsParam>,
    return_type: Option<String>,
    /// When `Some(TypeName)`, this is a synthesised one-shot Digest wrapper for
    /// a `pub type TypeName = ...` that implements the RustCrypto `Digest` trait.
    /// The generated wrapper calls `crate::TypeName::digest(input.as_bytes())`
    /// and returns the result as a lowercase hex `String`.
    digest_type: Option<String>,
}

/// Single parameter (name + Rust type string).
#[derive(Clone)]
struct RsParam {
    name: String,
    rust_type: String,
}

/// Struct definition parsed from Rust source.
struct RsStructSig {
    name: String,
    /// Public ABI-compatible fields.
    fields: Vec<RsField>,
    /// Methods from `impl Name { pub fn ... }`.
    methods: Vec<RsMethodSig>,
    /// Constructor params: from `pub fn new(...)` if present, else field order.
    ctor_params: Vec<RsParam>,
    /// If true, use `Name::new(...)` for construction; if false, use struct literal.
    use_new_fn: bool,
}

struct RsField {
    name: String,
    rust_type: String,
}

#[derive(Clone)]
struct RsMethodSig {
    name: String,
    params: Vec<RsParam>,
    self_mutable: bool,
    return_type: Option<String>,
    /// Set when the return type is a struct defined in the same crate.
    return_struct: Option<String>,
}

/// Where the crate source lives.
enum CrateSource {
    Registry { crate_name: String, version_req: String },
    LocalPath { crate_name: String, path: PathBuf },
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Find the crate, parse compatible `pub fn`/`pub struct`, compile a
/// call-through wrapper DLL, cache it, and return `Stmt` stubs.
pub fn load(module_name: &str, search_dirs: &[PathBuf], version: Option<&str>) -> Result<Vec<Stmt>, String> {
    let source = find_config(module_name, version, search_dirs)?;

    let stem = module_name.replace(['.', '-'], "_");
    let tmp = std::env::temp_dir().join(format!("ar_rs_{stem}"));

    let (crate_src_dir, crate_ident) = prepare_wrapper(&source, &stem, &tmp)?;

    let (fns, structs) = scan_all_sigs(&crate_src_dir, &crate_ident);

    if fns.is_empty() && structs.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "import[rs] `{module_name}`: no compatible pub fn or pub struct found \
             (only primitive types: int, float, bool, str, &[u8], Vec<u8>, [u8;N] are supported)"
        ));
    }

    // If digest-pattern wrappers were synthesised, ensure the `digest` crate is
    // a direct dependency of the wrapper project.  We read the version from the
    // target crate's own Cargo.toml so the versions always match.
    if fns.iter().any(|f| f.digest_type.is_some()) {
        let digest_ver = detect_digest_version(&crate_src_dir);
        patch_cargo_toml_digest(&tmp.join("Cargo.toml"), &digest_ver)
            .map_err(|e| format!("cannot patch Cargo.toml: {e}"))?;
    }

    let wrapper_src = lib_rs(&fns, &structs, &crate_ident);
    let src_dir = tmp.join("src");
    std::fs::write(src_dir.join("lib.rs"), &wrapper_src)
        .map_err(|e| format!("cannot write lib.rs: {e}"))?;

    let output = std::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&tmp)
        .output();

    match &output {
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("cargo build failed:\n{stderr}"));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err("cargo not found in PATH".to_string());
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("cannot run cargo: {e}"));
        }
        Ok(_) => {}
    }

    let lib_prefix = if cfg!(target_os = "windows") { "" } else { "lib" };
    let ext = native_lib_ext();
    let dll_name = format!("{lib_prefix}ar_rs_{stem}.{ext}");
    let dll_path = tmp.join("target").join("release").join(&dll_name);

    let dll_bytes =
        std::fs::read(&dll_path).map_err(|e| format!("cannot read DLL {dll_name}: {e}"))?;
    let _ = std::fs::remove_dir_all(&tmp);

    // Build FnExport list: free functions + struct method exports
    let mut exports: Vec<FnExport> = fns
        .iter()
        .map(|s| FnExport { name: s.name.clone(), n_params: s.params.len(), class_name: None })
        .collect();
    for st in &structs {
        // __init__: self + ctor params
        exports.push(FnExport {
            name: "__init__".to_string(),
            n_params: 1 + st.ctor_params.len(),
            class_name: Some(st.name.clone()),
        });
        // drop: self only
        exports.push(FnExport {
            name: "drop".to_string(),
            n_params: 1,
            class_name: Some(st.name.clone()),
        });
        // field getters/setters
        for field in &st.fields {
            let getter = format!("get_{}", field.name);
            exports.push(FnExport { name: getter, n_params: 1, class_name: Some(st.name.clone()) });
            let setter = format!("set_{}", field.name);
            exports.push(FnExport { name: setter, n_params: 2, class_name: Some(st.name.clone()) });
        }
        // methods: self + params
        for m in &st.methods {
            exports.push(FnExport {
                name: m.name.clone(),
                n_params: 1 + m.params.len(),
                class_name: Some(st.name.clone()),
            });
        }
    }

    cache_native(module_name, exports, dll_bytes);
    Ok(make_stubs(&fns, &structs))
}

// ── Config parsing ────────────────────────────────────────────────────────────

fn find_config(module_name: &str, version: Option<&str>, search_dirs: &[PathBuf]) -> Result<CrateSource, String> {
    let cwd = std::env::current_dir().ok();
    let extra: &[PathBuf] = cwd.as_ref().map(std::slice::from_ref).unwrap_or(&[]);
    for dir in search_dirs.iter().chain(extra.iter()) {
        let p = dir.join("ar_config.json");
        if !p.exists() { continue; }

        let json = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        let root: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| format!("{}: JSON parse error: {e}", p.display()))?;

        // `crates_path` may be a single string or an array of strings.
        // Each path is searched in order; the first match wins.
        let crates_val = match root.get("rust").and_then(|r| r.get("crates_path")) {
            Some(v) => v,
            None => continue,
        };
        let crates_paths: Vec<String> = if let Some(s) = crates_val.as_str() {
            vec![s.to_string()]
        } else if let Some(arr) = crates_val.as_array() {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        } else {
            continue;
        };

        let base = p.parent().unwrap_or(Path::new("."));

        for crates_path_str in &crates_paths {
        let crates_root = base.join(crates_path_str);
        let prefix = format!("{module_name}-");

        let mut candidates: Vec<_> = std::fs::read_dir(&crates_root)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .collect();

        let exact = crates_root.join(module_name);
        if exact.exists() && candidates.is_empty() {
            return Ok(CrateSource::LocalPath {
                crate_name: module_name.to_string(),
                path: exact,
            });
        }

        if candidates.is_empty() { continue; }

        let chosen = if let Some(ver) = version {
            candidates.iter()
                .find(|e| e.file_name().to_string_lossy().contains(ver))
                .or_else(|| candidates.iter().max_by_key(|e| e.file_name()))
        } else {
            candidates.iter().max_by_key(|e| e.file_name())
        };

        if let Some(entry) = chosen {
            return Ok(CrateSource::LocalPath {
                crate_name: module_name.to_string(),
                path: entry.path(),
            });
        }
        } // end for crates_path_str
    }

    Err(format!(
        "import[rs] '{module_name}': crate directory not found under \
         rust.crates_path in ar_config.json (searched: {})",
        search_dirs
            .iter()
            .map(|d| format!("'{}'", d.display()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

// ── Digest dependency helpers ─────────────────────────────────────────────────

/// Read the target crate's Cargo.toml and return the version string it requires
/// for the `digest` crate (e.g. `"0.10.7"`).  Falls back to `"0.10"`.
fn detect_digest_version(crate_src_dir: &Path) -> String {
    let cargo_toml = crate_src_dir
        .parent()
        .map(|p| p.join("Cargo.toml"))
        .unwrap_or_default();
    if let Ok(text) = std::fs::read_to_string(&cargo_toml) {
        // Parse the version from `[dependencies.digest]` section or inline form.
        let mut in_digest = false;
        for line in text.lines() {
            let t = line.trim();
            if t == "[dependencies.digest]" {
                in_digest = true;
                continue;
            }
            if in_digest {
                if t.starts_with('[') { break; } // new section
                if let Some(ver_str) = t.strip_prefix("version") {
                    // `version = "0.10.7"` → extract the string value
                    if let Some(v) = ver_str
                        .trim_start_matches(|c: char| c == ' ' || c == '=')
                        .trim()
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        // Return major.minor only for the dep requirement
                        let parts: Vec<&str> = v.splitn(3, '.').collect();
                        return if parts.len() >= 2 {
                            format!("{}.{}", parts[0], parts[1])
                        } else {
                            v.to_string()
                        };
                    }
                }
            }
        }
    }
    "0.10".to_string()
}

/// Append `digest = "version"` to the wrapper project's Cargo.toml.
fn patch_cargo_toml_digest(cargo_toml: &Path, version: &str) -> std::io::Result<()> {
    let mut text = std::fs::read_to_string(cargo_toml).unwrap_or_default();
    if !text.contains("digest") {
        text.push_str(&format!("digest=\"{version}\"\n"));
        std::fs::write(cargo_toml, &text)?;
    }
    Ok(())
}

// ── Wrapper project preparation ───────────────────────────────────────────────

fn prepare_wrapper(
    source: &CrateSource,
    stem: &str,
    tmp: &Path,
) -> Result<(PathBuf, String), String> {
    let src_dir = tmp.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| format!("cannot create temp dir: {e}"))?;
    std::fs::write(src_dir.join("lib.rs"), "")
        .map_err(|e| format!("cannot write placeholder lib.rs: {e}"))?;

    let (cargo_toml_content, crate_name) = match source {
        CrateSource::Registry { crate_name, version_req } => {
            let toml = format!(
                "[package]\nname=\"ar_rs_{stem}\"\nversion=\"0.0.0\"\nedition=\"2021\"\n\
                 [lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\n{crate_name}=\"{version_req}\"\n"
            );
            (toml, crate_name.clone())
        }
        CrateSource::LocalPath { crate_name, path } => {
            let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
            let raw = abs.display().to_string();
            let stripped = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
            let path_str = stripped.replace('\\', "/");
            let toml = format!(
                "[package]\nname=\"ar_rs_{stem}\"\nversion=\"0.0.0\"\nedition=\"2021\"\n\
                 [lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\n\
                 {crate_name}={{path=\"{path_str}\"}}\n"
            );
            (toml, crate_name.clone())
        }
    };

    std::fs::write(tmp.join("Cargo.toml"), &cargo_toml_content)
        .map_err(|e| format!("cannot write Cargo.toml: {e}"))?;

    let meta_out = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(tmp.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;

    if !meta_out.status.success() {
        let stderr = String::from_utf8_lossy(&meta_out.stderr);
        return Err(format!("cargo metadata failed:\n{stderr}"));
    }

    let meta: serde_json::Value = serde_json::from_slice(&meta_out.stdout)
        .map_err(|e| format!("cannot parse cargo metadata output: {e}"))?;

    let crate_src_dir = meta["packages"]
        .as_array()
        .and_then(|pkgs| {
            pkgs.iter().find(|p| p["name"].as_str() == Some(&crate_name))
        })
        .and_then(|p| p["manifest_path"].as_str())
        .map(|mp| {
            PathBuf::from(mp)
                .parent()
                .map(|p| p.join("src"))
                .unwrap_or_else(|| PathBuf::from("src"))
        })
        .ok_or_else(|| {
            format!("crate `{crate_name}` not found in cargo metadata output")
        })?;

    let crate_ident = crate_name.replace('-', "_");
    Ok((crate_src_dir, crate_ident))
}

// ── Source scanning ───────────────────────────────────────────────────────────

/// Scan all `.rs` files under `src_dir` and collect compatible free functions
/// and struct definitions.
// ── Re-export whitelist ───────────────────────────────────────────────────────

fn collect_reexports(src_dir: &Path) -> std::collections::HashSet<String> {
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

fn follow_pub_use(
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

fn collect_sigs(
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
fn scan_all_sigs(
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

fn parse_fn_sigs(source: &str) -> Vec<RsFnSig> {
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

fn parse_single_line_sig(line: &str) -> Option<RsFnSig> {
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
fn extract_struct_name(line: &str) -> Option<String> {
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
fn parse_struct_sigs(source: &str) -> Vec<RsStructSig> {
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
fn collect_digest_fns(src_dir: &Path, crate_ident: &str) -> Vec<RsFnSig> {
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
fn source_exports_digest(source: &str) -> bool {
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
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn collect_struct_fields(source: &str) -> HashMap<String, Vec<RsField>> {
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
fn parse_struct_field_line(line: &str) -> Option<RsField> {
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
fn collect_impl_methods(source: &str, known_structs: &[String]) -> HashMap<String, Vec<RsMethodSig>> {
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
fn parse_method_line(line: &str, known_structs: &[String]) -> Option<RsMethodSig> {
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
fn parse_self_params(params: &str) -> Option<(bool, bool, &str)> {
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

fn is_abi_compatible(t: &str) -> bool {
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
fn is_fixed_byte_array(t: &str) -> bool {
    if let Some(inner) = t.strip_prefix("[u8; ").and_then(|s| s.strip_suffix(']')) {
        return inner.trim().chars().all(|c| c.is_ascii_digit());
    }
    false
}

fn find_matching(s: &str, open: char, close: char) -> Option<usize> {
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

fn parse_params(s: &str) -> Vec<RsParam> {
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

fn parse_one_param(s: &str) -> Option<RsParam> {
    let s = s.trim();
    if matches!(s, "self" | "&self" | "&mut self" | "mut self") { return None; }
    let colon = s.find(':')?;
    let raw_name = s[..colon].trim();
    let name = raw_name.trim_start_matches("mut").trim().to_string();
    let rust_type = s[colon + 1..].trim().to_string();
    if name.is_empty() || rust_type.is_empty() { return None; }
    Some(RsParam { name, rust_type })
}

fn rust_type_to_ar(rt: &str) -> &'static str {
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

// ── Stub generation ───────────────────────────────────────────────────────────

fn make_stubs(fns: &[RsFnSig], structs: &[RsStructSig]) -> Vec<Stmt> {
    let mut stmts: Vec<Stmt> = Vec::new();

    // Free function stubs
    for sig in fns {
        let params: Vec<Param> = sig.params.iter()
            .map(|p| Param {
                name: p.name.clone(),
                mutable: false,
                type_ann: Some(rust_type_to_ar(&p.rust_type).to_string()),
                default: None,
                variadic: false,
            })
            .collect();
        stmts.push(Stmt::FnDef {
            name: sig.name.clone(),
            template_params: vec![],
            params,
            return_type: sig.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string()),
            body: vec![],
            is_abstract: true,
            is_static: false,
            is_class_method: false,
            decorators: vec![],
            access: Accessibility::Public,
        });
    }

    // Class stubs for each struct
    for st in structs {
        let mut class_body: Vec<Stmt> = Vec::new();

        // Internal handle field (private, mutable, default 0)
        class_body.push(Stmt::Field {
            name: "__rs_handle__".to_string(),
            kind: FieldKind::Mut,
            type_ann: "int".to_string(),
            default: Some(Expr::Int(0)),
            access: Accessibility::Public,
        });

        // Public fields mirroring the Rust struct
        for field in &st.fields {
            class_body.push(Stmt::Field {
                name: field.name.clone(),
                kind: FieldKind::Mut,
                type_ann: rust_type_to_ar(&field.rust_type).to_string(),
                default: None,
                access: Accessibility::Public,
            });
        }

        // __init__ stub: (mut self, ctor_param0, ctor_param1, ...)
        {
            let mut params = vec![Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
                variadic: false,
            }];
            for cp in &st.ctor_params {
                params.push(Param {
                    name: cp.name.clone(),
                    mutable: false,
                    type_ann: Some(rust_type_to_ar(&cp.rust_type).to_string()),
                    default: None,
                    variadic: false,
                });
            }
            class_body.push(Stmt::FnDef {
                name: "__init__".to_string(),
                template_params: vec![],
                params,
                return_type: None,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // drop stub
        class_body.push(Stmt::FnDef {
            name: "drop".to_string(),
            template_params: vec![],
            params: vec![Param { name: "self".to_string(), mutable: true, type_ann: None, default: None, variadic: false }],
            return_type: None,
            body: vec![],
            is_abstract: true,
            is_static: false,
            is_class_method: false,
            decorators: vec![],
            access: Accessibility::Public,
        });

        // Field getter stubs: get_{field}(let self) -> T
        for field in &st.fields {
            let getter_name = format!("get_{}", field.name);
            class_body.push(Stmt::FnDef {
                name: getter_name,
                template_params: vec![],
                params: vec![Param { name: "self".to_string(), mutable: false, type_ann: None, default: None, variadic: false }],
                return_type: Some(rust_type_to_ar(&field.rust_type).to_string()),
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // Field setter stubs: set_{field}(mut self, val: T)
        for field in &st.fields {
            let setter_name = format!("set_{}", field.name);
            class_body.push(Stmt::FnDef {
                name: setter_name,
                template_params: vec![],
                params: vec![
                    Param { name: "self".to_string(), mutable: true, type_ann: None, default: None, variadic: false },
                    Param { name: "val".to_string(), mutable: false, type_ann: Some(rust_type_to_ar(&field.rust_type).to_string()), default: None, variadic: false },
                ],
                return_type: None,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // Method stubs
        for m in &st.methods {
            let mut params = vec![Param {
                name: "self".to_string(),
                mutable: m.self_mutable,
                type_ann: None,
                default: None,
                variadic: false,
            }];
            for p in &m.params {
                params.push(Param {
                    name: p.name.clone(),
                    mutable: false,
                    type_ann: Some(rust_type_to_ar(&p.rust_type).to_string()),
                    default: None,
                    variadic: false,
                });
            }
            // Return type: primitive or a struct class name
            let ret_type_str = m.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string())
                .or_else(|| m.return_struct.clone());
            class_body.push(Stmt::FnDef {
                name: m.name.clone(),
                template_params: vec![],
                params,
                return_type: ret_type_str,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        stmts.push(Stmt::ClassDef {
            name: st.name.clone(),
            template_params: vec![],
            bases: vec![],
            decorators: vec![],
            body: class_body,
        });
    }

    stmts
}

// ── Call-through lib.rs generation ───────────────────────────────────────────

const ABI_HEADER: &str = r#"// Auto-generated — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_imports, unused_mut,
         clippy::missing_safety_doc)]

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicI64, Ordering};
use std::collections::HashMap;

const TL_NONE:  i64 = 0;
const TL_TRUE:  i64 = 1;
const TL_FALSE: i64 = 2;

#[repr(C)]
struct ArCallbacks {
    make_int:      unsafe extern "C" fn(i64) -> i64,
    make_float:    unsafe extern "C" fn(f64) -> i64,
    make_bool:     unsafe extern "C" fn(i32) -> i64,
    make_str:      unsafe extern "C" fn(*const u8, i32) -> i64,
    make_list:     unsafe extern "C" fn(*const i64, i32) -> i64,
    make_tuple:    unsafe extern "C" fn(*const i64, i32) -> i64,
    make_dict:     unsafe extern "C" fn(*const i64, *const i64, i32) -> i64,
    make_none:     unsafe extern "C" fn() -> i64,
    is_truthy:     unsafe extern "C" fn(i64) -> i32,
    binop:         unsafe extern "C" fn(i32, i64, i64) -> i64,
    unop:          unsafe extern "C" fn(i32, i64) -> i64,
    call_fn:       unsafe extern "C" fn(i64, *const i64, i32) -> i64,
    get_attr:      unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    set_attr:      unsafe extern "C" fn(i64, *const u8, i32, i64),
    subscript:     unsafe extern "C" fn(i64, i64) -> i64,
    get_global:    unsafe extern "C" fn(*const u8, i32) -> i64,
    iter_from:     unsafe extern "C" fn(i64) -> i64,
    iter_next:     unsafe extern "C" fn(i64) -> i64,
    is_type:       unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    arena_save:    unsafe extern "C" fn() -> u64,
    arena_compact: unsafe extern "C" fn(i64, u64) -> i64,
    compact_many:  unsafe extern "C" fn(*const i64, i32, u64, *mut i64),
    to_int:        unsafe extern "C" fn(i64) -> i64,
    to_float:      unsafe extern "C" fn(i64) -> f64,
    deep_copy:     unsafe extern "C" fn(i64) -> i64,
    to_cstr:       unsafe extern "C" fn(i64) -> *const u8,
    write_handle:  unsafe extern "C" fn(i64, i64),
    list_append:   unsafe extern "C" fn(i64, i64) -> i64,
    raise_exc:     unsafe extern "C" fn(i64, i64) -> i64,
    make_cell:     unsafe extern "C" fn(i64) -> i64,
    get_cell:      unsafe extern "C" fn(i64) -> i64,
    set_cell:      unsafe extern "C" fn(i64, i64),
    call_method:   unsafe extern "C" fn(i64, *const u8, i32, *const i64, i32) -> i64,
}

static mut CB: *const ArCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn ar_init(cb: *const ArCallbacks) { CB = cb; }

#[inline(always)] unsafe fn cb_make_int(n: i64) -> i64   { ((*CB).make_int)(n) }
#[inline(always)] unsafe fn cb_make_float(f: f64) -> i64  { ((*CB).make_float)(f) }
#[inline(always)] unsafe fn cb_make_str(p: *const u8, l: i32) -> i64 { ((*CB).make_str)(p, l) }
#[inline(always)] unsafe fn cb_to_int(h: i64) -> i64     { ((*CB).to_int)(h) }
#[inline(always)] unsafe fn cb_to_float(h: i64) -> f64   { ((*CB).to_float)(h) }
#[inline(always)] unsafe fn cb_to_cstr(h: i64) -> *const u8 { ((*CB).to_cstr)(h) }

#[inline(always)]
unsafe fn cb_get_attr(obj_h: i64, name: &[u8]) -> i64 {
    ((*CB).get_attr)(obj_h, name.as_ptr(), name.len() as i32)
}

#[inline(always)]
unsafe fn cb_set_attr(obj_h: i64, name: &[u8], val_h: i64) {
    ((*CB).set_attr)(obj_h, name.as_ptr(), name.len() as i32, val_h)
}

unsafe fn handle_to_string(h: i64) -> String {
    let ptr = cb_to_cstr(h);
    if ptr.is_null() { return String::new(); }
    std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned()
}

"#;

fn lib_rs(fns: &[RsFnSig], structs: &[RsStructSig], crate_ident: &str) -> String {
    let mut out = ABI_HEADER.to_string();

    // Struct arenas (one static arena + counter per struct)
    for st in structs {
        out.push_str(&struct_arena_decl(&st.name, crate_ident));
    }

    // Free function wrappers
    for sig in fns {
        out.push_str(&fn_wrapper(sig, crate_ident));
    }

    // Struct wrappers
    for st in structs {
        out.push_str(&struct_wrappers(st, crate_ident, structs));
    }

    out
}

/// Generate the static arena declaration for a struct using OnceLock for lazy init.
fn struct_arena_decl(struct_name: &str, crate_ident: &str) -> String {
    let lock_ident = arena_lock_ident(struct_name);
    let getter = arena_getter_fn(struct_name);
    let counter = counter_ident(struct_name);
    format!(
        "static {lock_ident}: OnceLock<Mutex<HashMap<i64, {crate_ident}::{struct_name}>>> = OnceLock::new();\n\
         fn {getter}() -> &'static Mutex<HashMap<i64, {crate_ident}::{struct_name}>> {{\n    \
         {lock_ident}.get_or_init(|| Mutex::new(HashMap::new()))\n}}\n\
         static {counter}: AtomicI64 = AtomicI64::new(1);\n\n"
    )
}

/// The static OnceLock identifier for the arena.
fn arena_lock_ident(struct_name: &str) -> String {
    format!("ARENA_LOCK_{}", struct_name.to_uppercase())
}

/// The getter function name that lazily initialises the arena.
fn arena_getter_fn(struct_name: &str) -> String {
    format!("get_arena_{}", struct_name.to_lowercase())
}

fn counter_ident(struct_name: &str) -> String {
    format!("COUNTER_{}", struct_name.to_uppercase())
}

/// Generate all wrappers for a struct: __init__, drop, field getters/setters, methods.
fn struct_wrappers(st: &RsStructSig, crate_ident: &str, all_structs: &[RsStructSig]) -> String {
    let mut out = String::new();
    out.push_str(&struct_init_wrapper(st, crate_ident));
    out.push_str(&struct_drop_wrapper(&st.name));
    for field in &st.fields {
        out.push_str(&struct_getter_wrapper(&st.name, field));
        out.push_str(&struct_setter_wrapper(&st.name, field));
    }
    for m in &st.methods {
        out.push_str(&struct_method_wrapper(&st.name, m, crate_ident, all_structs));
    }
    out
}

/// `{StructName}____init___tl` — constructor wrapper.
fn struct_init_wrapper(st: &RsStructSig, crate_ident: &str) -> String {
    let name = &st.name;
    let arena = format!("{}()", arena_getter_fn(name));
    let counter = counter_ident(name);
    // Symbol: method_symbol(name, "__init__") = "{name}____init__", + "_tl"
    let sym = format!("{name}____init___tl");

    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n"
    );
    out.push_str("    let self_h = *args.add(0);\n");

    // Decode constructor args starting at index 1
    for (i, p) in st.ctor_params.iter().enumerate() {
        out.push_str(&param_conversion(i + 1, &p.name, &p.rust_type));
    }

    // Build the struct instance
    if st.use_new_fn {
        let args: Vec<String> = st.ctor_params.iter().map(|p| p.name.clone()).collect();
        out.push_str(&format!(
            "    let _instance = {crate_ident}::{name}::new({});\n",
            args.join(", ")
        ));
    } else {
        let field_inits: Vec<String> = st.ctor_params.iter()
            .map(|p| format!("{}: {}", p.name, p.name))
            .collect();
        out.push_str(&format!(
            "    let _instance = {crate_ident}::{name} {{ {} }};\n",
            field_inits.join(", ")
        ));
    }

    // Store in arena
    out.push_str(&format!(
        "    let _key = {counter}.fetch_add(1, Ordering::SeqCst);\n"
    ));
    out.push_str(&format!(
        "    {arena}.lock().unwrap().insert(_key, _instance);\n"
    ));

    // Store __rs_handle__ in the HV instance
    out.push_str("    let _key_h = cb_make_int(_key);\n");
    out.push_str("    cb_set_attr(self_h, b\"__rs_handle__\", _key_h);\n");

    // Also populate HV field cache from the arena
    for field in &st.fields {
        let fname = &field.name;
        let ftype = &field.rust_type;
        out.push_str(&format!(
            "    {{\n        let _arena = {arena}.lock().unwrap();\n        \
             if let Some(_obj) = _arena.get(&_key) {{\n            \
             let _fh = {};\n            \
             cb_set_attr(self_h, b\"{fname}\", _fh);\n        }}\n    }}\n",
            rust_value_to_handle("_obj", fname, ftype)
        ));
    }

    out.push_str("    TL_NONE\n}\n\n");
    out
}

/// `{StructName}__drop_tl` — destructor wrapper.
fn struct_drop_wrapper(struct_name: &str) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let sym = format!("{struct_name}__drop_tl");
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         {arena}.lock().unwrap().remove(&_key);\n    \
         TL_NONE\n}}\n\n"
    )
}

/// `{StructName}__get_{field}_tl` — field getter.
fn struct_getter_wrapper(struct_name: &str, field: &RsField) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let fname = &field.name;
    let sym = format!("{struct_name}__get_{fname}_tl");
    let result_expr = rust_value_to_handle("_obj", fname, &field.rust_type);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let _arena = {arena}.lock().unwrap();\n    \
         match _arena.get(&_key) {{\n        \
         Some(_obj) => {result_expr},\n        \
         None => TL_NONE,\n    \
         }}\n}}\n\n"
    )
}

/// `{StructName}__set_{field}_tl` — field setter.
fn struct_setter_wrapper(struct_name: &str, field: &RsField) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let fname = &field.name;
    let sym = format!("{struct_name}__set_{fname}_tl");
    let decode = param_conversion(1, "_val", &field.rust_type);
    let val_handle = rust_value_to_handle_of("_val", &field.rust_type);
    format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n    \
         let self_h = *args.add(0);\n{decode}    \
         let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n    \
         let _key = cb_to_int(_key_h);\n    \
         let mut _arena = {arena}.lock().unwrap();\n    \
         if let Some(_obj) = _arena.get_mut(&_key) {{\n        \
         _obj.{fname} = _val;\n    }}\n    \
         drop(_arena);\n    \
         let _fh = {val_handle};\n    \
         cb_set_attr(self_h, b\"{fname}\", _fh);\n    \
         TL_NONE\n}}\n\n"
    )
}

/// `{StructName}__{method_name}_tl` — method wrapper.
/// `all_structs` is needed to look up the return struct's ctor params.
fn struct_method_wrapper(struct_name: &str, m: &RsMethodSig, _crate_ident: &str, all_structs: &[RsStructSig]) -> String {
    let arena = format!("{}()", arena_getter_fn(struct_name));
    let mname = &m.name;
    let sym = format!("{struct_name}__{mname}_tl");

    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {sym}(args: *const i64, _n: i32) -> i64 {{\n"
    );
    out.push_str("    let self_h = *args.add(0);\n");
    out.push_str("    let _key_h = cb_get_attr(self_h, b\"__rs_handle__\");\n");
    out.push_str("    let _key = cb_to_int(_key_h);\n");

    for (i, p) in m.params.iter().enumerate() {
        out.push_str(&param_conversion(i + 1, &p.name, &p.rust_type));
    }

    let args_call: Vec<String> = m.params.iter().map(|p| p.name.clone()).collect();
    let call_expr = format!("_obj.{mname}({})", args_call.join(", "));

    if let Some(ret_struct_name) = &m.return_struct {
        // Returns a struct — extract field values before releasing the arena lock,
        // then call cb_call_fn to construct the HV instance.
        let ret_struct = all_structs.iter().find(|s| &s.name == ret_struct_name);
        let ctor_params: &[RsParam] = ret_struct
            .map(|s| s.ctor_params.as_slice())
            .unwrap_or(&[]);

        // Open arena block
        let lock_kw = if m.self_mutable { "mut " } else { "" };
        out.push_str(&format!("    let {lock_kw}_arena = {arena}.lock().unwrap();\n"));
        out.push_str(&format!("    let _found = _arena.{borrow};\n",
            borrow = if m.self_mutable { "get_mut(&_key)" } else { "get(&_key)" }
        ));
        out.push_str("    if _found.is_none() { return TL_NONE; }\n");
        out.push_str("    let _obj = _found.unwrap();\n");
        out.push_str(&format!("    let _rval = {call_expr};\n"));

        // Extract each ctor field from the result before lock is dropped
        for (i, p) in ctor_params.iter().enumerate() {
            let t = &p.rust_type;
            let fname = &p.name;
            out.push_str(&format!("    let _ret_{i}: {t} = _rval.{fname};\n"));
        }
        out.push_str("    drop(_arena);\n"); // explicit drop = release lock before callbacks

        // Build HV instance via cb_get_global + cb_call_fn
        out.push_str(&format!(
            "    let _cls_h = ((*CB).get_global)(\"{ret_struct_name}\".as_ptr(), {n});\n",
            n = ret_struct_name.len()
        ));

        // Build ctor arg handles
        let handle_exprs: Vec<String> = ctor_params.iter().enumerate()
            .map(|(i, p)| rust_value_to_handle_of(&format!("_ret_{i}"), &p.rust_type))
            .collect();
        if handle_exprs.is_empty() {
            out.push_str("    let _ctor: [i64; 0] = [];\n");
        } else {
            out.push_str(&format!("    let _ctor = [{}];\n", handle_exprs.join(", ")));
        }
        out.push_str("    ((*CB).call_fn)(_cls_h, _ctor.as_ptr(), _ctor.len() as i32)\n");
        out.push_str("}\n\n");
    } else {
        // Primitive / void return — original path
        let obj_borrow = if m.self_mutable { "get_mut(&_key)" } else { "get(&_key)" };
        if m.self_mutable {
            out.push_str(&format!("    let mut _arena = {arena}.lock().unwrap();\n"));
        } else {
            out.push_str(&format!("    let _arena = {arena}.lock().unwrap();\n"));
        }
        out.push_str(&format!("    match _arena.{obj_borrow} {{\n"));
        out.push_str("        None => TL_NONE,\n");
        out.push_str(&format!(
            "        Some(_obj) => {{\n{}\n        }},\n    }}\n}}\n\n",
            return_conversion_expr(&call_expr, m.return_type.as_deref())
        ));
    }

    out
}

/// Generate a Rust expression that converts an arena object's field to an i64 handle.
/// `obj_var` is the variable holding a reference to the struct.
fn rust_value_to_handle(obj_var: &str, field: &str, rust_type: &str) -> String {
    let field_expr = format!("{obj_var}.{field}");
    rust_value_to_handle_of(&field_expr, rust_type)
}

/// Generate a Rust expression that converts a value expression to an i64 handle.
fn rust_value_to_handle_of(expr: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("cb_make_int({expr})"),
        "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128" =>
            format!("cb_make_int({expr} as i64)"),
        "f64" => format!("cb_make_float({expr})"),
        "f32" => format!("cb_make_float({expr} as f64)"),
        "bool" => format!("if {expr} {{ TL_TRUE }} else {{ TL_FALSE }}"),
        "String" => format!("cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"),
        "&str" | "&String" => format!("cb_make_str({expr}.as_ptr(), {expr}.len() as i32)"),
        _ => format!("cb_make_int({expr} as i64)"),
    }
}

fn param_conversion(i: usize, name: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("    let {name}: i64 = cb_to_int(*args.add({i}));\n"),
        t @ ("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") =>
            format!("    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n"),
        "f64" => format!("    let {name}: f64 = cb_to_float(*args.add({i}));\n"),
        "f32" => format!("    let {name}: f32 = cb_to_float(*args.add({i})) as f32;\n"),
        "bool" => format!("    let {name}: bool = *args.add({i}) == TL_TRUE;\n"),
        "String" =>
            format!("    let {name}: String = handle_to_string(*args.add({i}));\n"),
        "&str" | "&String" =>
            format!(
                "    let _owned_{name}: String = handle_to_string(*args.add({i}));\n    let {name}: &str = &_owned_{name};\n"
            ),
        // &[u8]: decode str handle → String → take bytes slice
        "&[u8]" =>
            format!(
                "    let _bytes_{name}: String = handle_to_string(*args.add({i}));\n    let {name}: &[u8] = _bytes_{name}.as_bytes();\n"
            ),
        _ => format!("    let {name}: i64 = *args.add({i});\n"),
    }
}

fn return_conversion(call: &str, rust_type: Option<&str>) -> String {
    format!("    {}\n", return_conversion_expr(call, rust_type))
}

fn return_conversion_expr(call: &str, rust_type: Option<&str>) -> String {
    match rust_type.map(str::trim) {
        None | Some("()") =>
            format!("{call};\n            TL_NONE"),
        Some("i64" | "isize") =>
            format!("cb_make_int({call})"),
        Some("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") =>
            format!("cb_make_int(({call}) as i64)"),
        Some("f64") =>
            format!("cb_make_float({call})"),
        Some("f32") =>
            format!("cb_make_float(({call}) as f64)"),
        Some("bool") =>
            format!("if {call} {{ TL_TRUE }} else {{ TL_FALSE }}"),
        Some("String") => {
            format!("{{ let _r: String = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}")
        }
        Some("&str") => {
            format!("{{ let _r: &str = {call};\n            cb_make_str(_r.as_ptr(), _r.len() as i32) }}")
        }
        // Byte-array output: hex-encode → return as str handle
        Some("Vec<u8>") => {
            format!("{{ let _r: Vec<u8> = {call};\n            let _hex = _r.iter().map(|b| format!(\"{{:02x}}\", b)).collect::<String>();\n            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}")
        }
        Some(t) if is_fixed_byte_array(t) => {
            format!("{{ let _r = {call};\n            let _hex = _r.iter().map(|b| format!(\"{{:02x}}\", b)).collect::<String>();\n            cb_make_str(_hex.as_ptr(), _hex.len() as i32) }}")
        }
        Some(_) =>
            format!("{{ let _r: i64 = {call} as i64;\n            cb_make_int(_r) }}"),
    }
}

fn fn_wrapper(sig: &RsFnSig, crate_ident: &str) -> String {
    // Digest-pattern: synthesised one-shot hash wrapper.
    if let Some(type_path) = &sig.digest_type {
        return digest_wrapper(&sig.name, type_path);
    }

    let name = &sig.name;
    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {name}_tl(args: *const i64, _n: i32) -> i64 {{\n"
    );

    for (i, p) in sig.params.iter().enumerate() {
        out.push_str(&param_conversion(i, &p.name, &p.rust_type));
    }

    let args: Vec<String> = sig.params.iter().map(|p| p.name.clone()).collect();
    let call = format!("{crate_ident}::{name}({})", args.join(", "));
    out.push_str(&return_conversion(&call, sig.return_type.as_deref()));
    out.push_str("}\n\n");
    out
}

/// Generate a one-shot hash wrapper for a RustCrypto Digest type alias.
/// Calls `TypePath::digest(input.as_bytes())` and hex-encodes the result.
fn digest_wrapper(fn_name: &str, type_path: &str) -> String {
    format!(
        "#[no_mangle]\n\
         pub unsafe extern \"C\" fn {fn_name}_tl(args: *const i64, _n: i32) -> i64 {{\n    \
         let _s: String = handle_to_string(*args.add(0));\n    \
         use {type_path} as _HvHasher;\n    \
         use digest::Digest as _HvDigest;\n    \
         let _result = _HvHasher::digest(_s.as_bytes());\n    \
         let _hex: String = _result.iter().map(|b| format!(\"{{:02x}}\", b)).collect();\n    \
         cb_make_str(_hex.as_ptr(), _hex.len() as i32)\n\
         }}\n\n"
    )
}
