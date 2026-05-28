/// Native module loader for `import[rs]` — reads crate source directly from the
/// cargo registry (or a local path), auto-discovers compatible `pub fn`, and
/// generates call-through ABI wrappers.  No function bodies are written by the user.
///
/// # Config format (`hv_crates.json` in the source directory)
///
/// ```json
/// {
///   "libm": "0.2",
///   "my_utils": { "path": "./rust/my_utils" }
/// }
/// ```
///
/// - Simple string value  → registry crate, import name = crate name
/// - `{ "version": "x" }` → registry crate (with optional `"crate"` key for a
///                           different crate name)
/// - `{ "path": "..." }`  → local Cargo crate directory
///
/// # Compatibility rules for auto-discovery
///
/// A `pub fn` is wrapped when ALL of the following hold:
/// - No generic type parameters (`<` in the signature → skipped)
/// - Every parameter type is a tl primitive: `i*`, `u*`, `f32`, `f64`, `bool`,
///   `String`, `&str`
/// - The return type is a tl primitive or `()` / absent

use std::path::{Path, PathBuf};

use crate::ast::{Accessibility, Param, Stmt};

use super::codegen::FnExport;
use super::module_compiler::{cache_native, native_lib_ext};

// ── Internal types ────────────────────────────────────────────────────────────

/// Rust クレートから解析した関数シグネチャ情報。
struct RsFnSig {
    name: String,
    params: Vec<RsParam>,
    return_type: Option<String>,
}

/// Rust 関数の単一パラメータ情報（名前と Rust 型）。
struct RsParam {
    name: String,
    rust_type: String,
}

/// クレートのソース種別。レジストリ（crates.io）またはローカルパスを表す。
enum CrateSource {
    Registry { crate_name: String, version_req: String },
    LocalPath { crate_name: String, path: PathBuf },
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Find `hv_crates.json` in `search_dirs`, resolve the crate, parse compatible
/// `pub fn` from its source, compile a call-through wrapper DLL, cache it, and
/// return `Stmt::FnDef` stubs.
pub fn load(module_name: &str, search_dirs: &[PathBuf], version: Option<&str>) -> Result<Vec<Stmt>, String> {
    let source = match version {
        Some(v) => CrateSource::Registry {
            crate_name: module_name.to_string(),
            version_req: v.to_string(),
        },
        None => find_config(module_name, search_dirs)?,
    };

    let stem = module_name.replace(['.', '-'], "_");
    let tmp = std::env::temp_dir().join(format!("hv_rs_{stem}"));

    // Build wrapper skeleton + run cargo metadata to resolve / download the dep.
    let (crate_src_dir, crate_ident) = prepare_wrapper(&source, &stem, &tmp)?;

    // Scan all .rs files in the crate's src/ directory.
    let sigs = scan_src_dir(&crate_src_dir);

    if sigs.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "import[rs] `{module_name}`: no compatible pub fn found \
             (functions must use only primitive types: int, float, bool, str)"
        ));
    }

    // eprintln!(
    //     "RsLoader: found {} compatible fn(s) in `{}`: {}",
    //     sigs.len(),
    //     module_name,
    //     sigs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
    // );

    // Write generated call-through lib.rs and compile.
    let wrapper_src = lib_rs(&sigs, &crate_ident);
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
    let dll_name = format!("{lib_prefix}hv_rs_{stem}.{ext}");
    let dll_path = tmp.join("target").join("release").join(&dll_name);

    let dll_bytes =
        std::fs::read(&dll_path).map_err(|e| format!("cannot read DLL {dll_name}: {e}"))?;
    let _ = std::fs::remove_dir_all(&tmp);

    let exports: Vec<FnExport> = sigs
        .iter()
        .map(|s| FnExport { name: s.name.clone(), n_params: s.params.len() })
        .collect();

    cache_native(module_name, exports, dll_bytes);
    Ok(make_stubs(&sigs))
}

// ── Config parsing ────────────────────────────────────────────────────────────

/// `hv_crates.json` を検索ディレクトリから探し、該当モジュールの [`CrateSource`] を返す。
fn find_config(module_name: &str, search_dirs: &[PathBuf]) -> Result<CrateSource, String> {
    for dir in search_dirs {
        let p = dir.join("hv_crates.json");
        if !p.exists() {
            continue;
        }
        let json = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        let root: serde_json::Value = serde_json::from_str(&json)
            .map_err(|e| format!("{}: JSON parse error: {e}", p.display()))?;

        let entry = match root.get(module_name) {
            Some(e) => e,
            None => continue,
        };

        let source = if let Some(ver) = entry.as_str() {
            // "libm": "0.2"
            CrateSource::Registry {
                crate_name: module_name.to_string(),
                version_req: ver.to_string(),
            }
        } else if let Some(path_str) = entry.get("path").and_then(|p| p.as_str()) {
            // "my_utils": { "path": "./rust/my_utils" }
            let crate_name = entry
                .get("crate")
                .and_then(|c| c.as_str())
                .unwrap_or(module_name)
                .to_string();
            let base = p.parent().unwrap_or(Path::new("."));
            CrateSource::LocalPath { crate_name, path: base.join(path_str) }
        } else if let Some(ver) = entry.get("version").and_then(|v| v.as_str()) {
            // "libm": { "version": "0.2" }  or  "math": { "crate": "libm", "version": "0.2" }
            let crate_name = entry
                .get("crate")
                .and_then(|c| c.as_str())
                .unwrap_or(module_name)
                .to_string();
            CrateSource::Registry { crate_name, version_req: ver.to_string() }
        } else {
            return Err(format!(
                "`{module_name}` in hv_crates.json: expected a version string or \
                 an object with `path` or `version`"
            ));
        };

        return Ok(source);
    }

    Err(format!(
        "no entry for `{module_name}` in hv_crates.json (searched: {})",
        search_dirs
            .iter()
            .map(|d| format!("'{}'", d.display()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

// ── Wrapper project preparation ───────────────────────────────────────────────

/// Create the temp Cargo project skeleton, run `cargo metadata` to resolve and
/// download the dependency, and return the crate's `src/` directory plus the
/// Rust identifier for the crate (hyphens → underscores).
fn prepare_wrapper(
    source: &CrateSource,
    stem: &str,
    tmp: &Path,
) -> Result<(PathBuf, String), String> {
    let src_dir = tmp.join("src");
    std::fs::create_dir_all(&src_dir)
        .map_err(|e| format!("cannot create temp dir: {e}"))?;
    // Placeholder so cargo doesn't error before we write the real lib.rs.
    std::fs::write(src_dir.join("lib.rs"), "")
        .map_err(|e| format!("cannot write placeholder lib.rs: {e}"))?;

    let (cargo_toml_content, crate_name) = match source {
        CrateSource::Registry { crate_name, version_req } => {
            let toml = format!(
                "[package]\nname=\"hv_rs_{stem}\"\nversion=\"0.0.0\"\nedition=\"2021\"\n\
                 [lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\n{crate_name}=\"{version_req}\"\n"
            );
            (toml, crate_name.clone())
        }
        CrateSource::LocalPath { crate_name, path } => {
            let abs = path.canonicalize().unwrap_or_else(|_| path.clone());
            let path_str = abs.display().to_string().replace('\\', "/");
            let toml = format!(
                "[package]\nname=\"hv_rs_{stem}\"\nversion=\"0.0.0\"\nedition=\"2021\"\n\
                 [lib]\ncrate-type=[\"cdylib\"]\n[dependencies]\n\
                 {crate_name}={{path=\"{path_str}\"}}\n"
            );
            (toml, crate_name.clone())
        }
    };

    std::fs::write(tmp.join("Cargo.toml"), &cargo_toml_content)
        .map_err(|e| format!("cannot write Cargo.toml: {e}"))?;

    // `cargo metadata` resolves and fetches the dependency; parse the output to
    // locate the crate's source directory.
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

/// Recursively scan all `.rs` files under `src_dir` and collect every
/// non-generic `pub fn` whose parameter and return types are ABI-compatible.
/// Uses the `pub use` re-export whitelist from `lib.rs` to skip functions that
/// are not actually accessible at the crate root.
fn scan_src_dir(src_dir: &Path) -> Vec<RsFnSig> {
    let whitelist = collect_reexports(src_dir);

    let mut sigs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_sigs(src_dir, &mut sigs, &mut seen);

    // When we have a non-empty whitelist, keep only crate-root-accessible names.
    // Empty whitelist (lib.rs unreadable / no pub use lines) → no filtering.
    if !whitelist.is_empty() {
        sigs.retain(|s| whitelist.contains(&s.name));
    }
    sigs
}

// ── Re-export whitelist ───────────────────────────────────────────────────────

/// Walk the `pub use` chain from `src/lib.rs` and collect every name that is
/// actually re-exported at the crate root.
fn collect_reexports(src_dir: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let lib_rs = src_dir.join("lib.rs");
    let Ok(text) = std::fs::read_to_string(&lib_rs) else { return set };
    // pub fn at the top level of lib.rs are directly exported.
    for line in text.lines() {
        if line.starts_with("pub fn ") {
            if let Some(sig) = parse_single_line_sig(line.trim()) {
                set.insert(sig.name);
            }
        }
    }
    follow_pub_use(&text, src_dir, &mut set, 0);
    set
}

/// Recursively follow `pub use` lines and collect exported names into `out`.
/// `module_dir` is the directory that contains the module file currently being
/// parsed (so relative paths like `self::sub::*` resolve correctly).
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
            // Glob re-export — follow that module recursively.
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
                    // Collect pub fn defined directly in the glob-followed module.
                    for mline in text.lines() {
                        if mline.starts_with("pub fn ") {
                            if let Some(sig) = parse_single_line_sig(mline.trim()) {
                                out.insert(sig.name);
                            }
                        }
                    }
                    follow_pub_use(&text, &next_dir, out, depth + 1);
                    break;
                }
            }
        } else {
            // Named re-export — extract the last path segment.
            if let Some(name) = rest.rsplit("::").next() {
                if name.starts_with('{') {
                    // Grouped: pub use path::{A, B, C};
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

/// ディレクトリを再帰的に走査して、ABI 互換な `pub fn` シグネチャを収集する。
fn collect_sigs(
    dir: &Path,
    out: &mut Vec<RsFnSig>,
    seen: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sigs(&path, out, seen);
        } else if path.extension().map_or(false, |e| e == "rs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                for sig in parse_fn_sigs(&text) {
                    if seen.insert(sig.name.clone()) {
                        out.push(sig);
                    }
                }
            }
        }
    }
}

// ── Signature parsing ─────────────────────────────────────────────────────────

/// ソーステキストからトップレベルの `pub fn` シグネチャを解析して返す。
fn parse_fn_sigs(source: &str) -> Vec<RsFnSig> {
    let mut sigs = Vec::new();
    for line in source.lines() {
        // Only match pub fn at column 0 — impl-block methods are always indented.
        if line.starts_with("pub fn ") {
            if let Some(sig) = parse_single_line_sig(line.trim()) {
                sigs.push(sig);
            }
        }
    }
    sigs
}

/// 1行の `pub fn ...` 宣言を解析して [`RsFnSig`] を返す。ジェネリックや非互換型の場合は `None`。
fn parse_single_line_sig(line: &str) -> Option<RsFnSig> {
    let rest = line.strip_prefix("pub fn ")?;

    // Function name.
    let name_end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    let name = rest[..name_end].to_string();
    let rest = rest[name_end..].trim_start();

    // Skip generic params — any `<` makes the function incompatible.
    if rest.starts_with('<') {
        return None;
    }

    // Parameter list.
    if !rest.starts_with('(') {
        return None;
    }
    let paren_end = find_matching(rest, '(', ')')?;
    let params_str = &rest[1..paren_end];
    let rest = rest[paren_end + 1..].trim_start();

    // Return type.
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

    // Reject if return type is incompatible.
    if let Some(ref rt) = return_type {
        if !is_abi_compatible(rt) {
            return None;
        }
    }

    let params = parse_params(params_str);

    // Reject if any param type is incompatible.
    for p in &params {
        if !is_abi_compatible(&p.rust_type) {
            return None;
        }
    }

    Some(RsFnSig { name, params, return_type })
}

/// Rust 型文字列が tl ABI と互換性があるかを判定する（整数・浮動小数点・bool・文字列型）。
fn is_abi_compatible(t: &str) -> bool {
    matches!(
        t.trim(),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize"
        | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
        | "f32" | "f64"
        | "bool"
        | "String" | "&str" | "&String"
    )
}

/// 開きブラケットに対応する閉じブラケットの位置を返す。見つからない場合は `None`。
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

/// パラメータリスト文字列を解析して [`RsParam`] のリストを返す。
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

/// `name: Type` 形式の1パラメータ文字列を解析する。`self` 系パラメータは `None` を返す。
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

// ── Type mapping ──────────────────────────────────────────────────────────────

/// Rust 型文字列を対応する tl 型名に変換する（`i64` → `"int"` など）。
fn rust_type_to_tl(rt: &str) -> &str {
    match rt.trim() {
        "i8" | "i16" | "i32" | "i64" | "i128"
        | "u8" | "u16" | "u32" | "u64" | "u128"
        | "isize" | "usize" => "int",
        "f32" | "f64" => "float",
        "bool" => "bool",
        "String" | "&str" | "&String" => "str",
        _ => "Any",
    }
}

// ── Stub generation ───────────────────────────────────────────────────────────

/// Rust 関数シグネチャから tl の `Stmt::FnDef` スタブリストを生成する（型注釈のみ・本体なし）。
fn make_stubs(sigs: &[RsFnSig]) -> Vec<Stmt> {
    sigs.iter()
        .map(|sig| {
            let params: Vec<Param> = sig
                .params
                .iter()
                .map(|p| Param {
                    name: p.name.clone(),
                    mutable: false,
                    type_ann: Some(rust_type_to_tl(&p.rust_type).to_string()),
                    default: None,
                })
                .collect();
            Stmt::FnDef {
                name: sig.name.clone(),
                template_params: vec![],
                params,
                return_type: sig.return_type.as_deref().map(|r| rust_type_to_tl(r).to_string()),
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            }
        })
        .collect()
}

// ── Call-through lib.rs generation ───────────────────────────────────────────

const ABI_HEADER: &str = r#"// Auto-generated — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_imports, unused_mut,
         clippy::missing_safety_doc)]

const TL_NONE:  i64 = 0;
const TL_TRUE:  i64 = 1;
const TL_FALSE: i64 = 2;

#[repr(C)]
struct HvCallbacks {
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
}

static mut CB: *const HvCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn hv_init(cb: *const HvCallbacks) { CB = cb; }

#[inline(always)] unsafe fn cb_make_int(n: i64) -> i64   { ((*CB).make_int)(n) }
#[inline(always)] unsafe fn cb_make_float(f: f64) -> i64  { ((*CB).make_float)(f) }
#[inline(always)] unsafe fn cb_make_str(p: *const u8, l: i32) -> i64 { ((*CB).make_str)(p, l) }
#[inline(always)] unsafe fn cb_to_int(h: i64) -> i64     { ((*CB).to_int)(h) }
#[inline(always)] unsafe fn cb_to_float(h: i64) -> f64   { ((*CB).to_float)(h) }
#[inline(always)] unsafe fn cb_to_cstr(h: i64) -> *const u8 { ((*CB).to_cstr)(h) }

unsafe fn handle_to_string(h: i64) -> String {
    let ptr = cb_to_cstr(h);
    if ptr.is_null() { return String::new(); }
    std::ffi::CStr::from_ptr(ptr as *const i8).to_string_lossy().into_owned()
}

"#;

/// ラッパー `lib.rs` のソース文字列を生成する。ABI ヘッダと全関数ラッパーを結合して返す。
fn lib_rs(sigs: &[RsFnSig], crate_ident: &str) -> String {
    let mut out = ABI_HEADER.to_string();
    for sig in sigs {
        out.push_str(&fn_wrapper(sig, crate_ident));
    }
    out
}

/// Generate a wrapper that converts handles → Rust types, calls the real crate
/// function, and converts the result back to an i64 handle.
fn fn_wrapper(sig: &RsFnSig, crate_ident: &str) -> String {
    let name = &sig.name;
    let mut out = format!(
        "#[no_mangle]\npub unsafe extern \"C\" fn {name}_tl(args: *const i64, _n: i32) -> i64 {{\n"
    );

    for (i, p) in sig.params.iter().enumerate() {
        out.push_str(&param_conversion(i, &p.name, &p.rust_type));
    }

    // Build call using each param's bound name directly.
    let args: Vec<String> = sig.params.iter().map(|p| p.name.clone()).collect();
    let call = format!("{crate_ident}::{name}({})", args.join(", "));
    out.push_str(&return_conversion(&call, sig.return_type.as_deref()));
    out.push_str("}\n\n");
    out
}

/// 引数インデックス・名前・Rust 型から、ハンドルを Rust 型に変換するコードを生成する。
fn param_conversion(i: usize, name: &str, rust_type: &str) -> String {
    match rust_type.trim() {
        "i64" | "isize" => format!("    let {name}: i64 = cb_to_int(*args.add({i}));\n"),
        t @ ("i32" | "i16" | "i8") =>
            format!("    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n"),
        t @ ("u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128") =>
            format!("    let {name}: {t} = cb_to_int(*args.add({i})) as {t};\n"),
        "f64" => format!("    let {name}: f64 = cb_to_float(*args.add({i}));\n"),
        "f32" => format!("    let {name}: f32 = cb_to_float(*args.add({i})) as f32;\n"),
        "bool" => format!("    let {name}: bool = *args.add({i}) == TL_TRUE;\n"),
        "String" =>
            format!("    let {name}: String = handle_to_string(*args.add({i}));\n"),
        "&str" | "&String" =>
            format!(
                "    let _owned{i}: String = handle_to_string(*args.add({i}));\n    let {name}: &str = &_owned{i};\n"
            ),
        _ => format!("    let {name}: i64 = *args.add({i});\n"),
    }
}

/// 関数呼び出し式と戻り値 Rust 型から、結果をハンドルに変換して返すコードを生成する。
fn return_conversion(call: &str, rust_type: Option<&str>) -> String {
    match rust_type.map(str::trim) {
        None | Some("()") =>
            format!("    {call};\n    TL_NONE\n"),
        Some("i64" | "isize") =>
            format!("    let _r: i64 = {call};\n    cb_make_int(_r)\n"),
        Some(t @ ("i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "u128" | "i128")) =>
            format!("    let _r: {t} = {call};\n    cb_make_int(_r as i64)\n"),
        Some("f64") =>
            format!("    let _r: f64 = {call};\n    cb_make_float(_r)\n"),
        Some("f32") =>
            format!("    let _r: f32 = {call};\n    cb_make_float(_r as f64)\n"),
        Some("bool") =>
            format!("    let _r: bool = {call};\n    if _r {{ TL_TRUE }} else {{ TL_FALSE }}\n"),
        Some("String") =>
            format!("    let _r: String = {call};\n    cb_make_str(_r.as_ptr(), _r.len() as i32)\n"),
        Some("&str") =>
            format!("    let _r: &str = {call};\n    cb_make_str(_r.as_ptr(), _r.len() as i32)\n"),
        Some(_) =>
            format!("    let _r: i64 = {call};\n    _r\n"),
    }
}
