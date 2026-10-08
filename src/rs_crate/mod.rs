// rs_crate/mod.rs — `import[rs]` の**型の情報**（Rust crate のソースから `pub fn` / `pub struct` を拾う部分）。
//
// 実行時の DLL を `cargo build` で作る `partial_compiler/rs_loader` から切り出した。crate のソースと
// `ar_config.json` を読むだけで、ファイルへのアクセスは `crate::import_fs` を通すので、VS Code 拡張（wasm）
// でも CLI と同じ型の情報が得られる（editor_import_resolution_plan.md 2-2）。

use std::path::{Path, PathBuf};

use crate::ast::Stmt;

// ── Internal types ────────────────────────────────────────────────────────────

/// Free function signature from a Rust crate.
pub(crate) struct RsFnSig {
    pub(crate) name: String,
    pub(crate) params: Vec<RsParam>,
    pub(crate) return_type: Option<String>,
    /// When `Some(TypeName)`, this is a synthesised one-shot Digest wrapper for
    /// a `pub type TypeName = ...` that implements the RustCrypto `Digest` trait.
    /// The generated wrapper calls `crate::TypeName::digest(input.as_bytes())`
    /// and returns the result as a lowercase hex `String`.
    pub(crate) digest_type: Option<String>,
}

/// Single parameter (name + Rust type string).
#[derive(Clone)]
pub(crate) struct RsParam {
    pub(crate) name: String,
    pub(crate) rust_type: String,
}

/// Struct definition parsed from Rust source.
pub(crate) struct RsStructSig {
    pub(crate) name: String,
    /// Public ABI-compatible fields.
    pub(crate) fields: Vec<RsField>,
    /// Methods from `impl Name { pub fn ... }`.
    pub(crate) methods: Vec<RsMethodSig>,
    /// Constructor params: from `pub fn new(...)` if present, else field order.
    pub(crate) ctor_params: Vec<RsParam>,
    /// If true, use `Name::new(...)` for construction; if false, use struct literal.
    pub(crate) use_new_fn: bool,
}

pub(crate) struct RsField {
    pub(crate) name: String,
    pub(crate) rust_type: String,
}

#[derive(Clone)]
pub(crate) struct RsMethodSig {
    pub(crate) name: String,
    pub(crate) params: Vec<RsParam>,
    pub(crate) self_mutable: bool,
    pub(crate) return_type: Option<String>,
    /// Set when the return type is a struct defined in the same crate.
    pub(crate) return_struct: Option<String>,
}

/// Where the crate source lives.
pub(crate) enum CrateSource {
    /// TODO(reserved): crates.io 依存としてのロード。`prepare_wrapper` 側の生成は
    /// 実装済みだが、`find_config` はまだ `LocalPath` しか返さないため未構築。
    #[allow(dead_code)]
    Registry { crate_name: String, version_req: String },
    LocalPath { crate_name: String, path: PathBuf },
}

mod parse;
mod stubs;

pub(crate) use parse::*;
pub(crate) use stubs::*;

// ── Config parsing ────────────────────────────────────────────────────────────

pub(crate) fn find_config(module_name: &str, version: Option<&str>, search_dirs: &[PathBuf]) -> Result<CrateSource, String> {
    let cwd = crate::import_fs::current_dir();
    let extra: &[PathBuf] = cwd.as_slice();
    for dir in search_dirs.iter().chain(extra.iter()) {
        let p = dir.join("ar_config.json");
        if !crate::import_fs::exists(&p) { continue; }

        let json = crate::import_fs::read_to_string(&p)
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

        let candidates: Vec<PathBuf> = crate::import_fs::read_dir(&crates_root)
            .unwrap_or_default()
            .into_iter()
            .filter(|p| crate::import_fs::is_dir(p))
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)))
            .collect();

        let exact = crates_root.join(module_name);
        if crate::import_fs::exists(&exact) && candidates.is_empty() {
            return Ok(CrateSource::LocalPath {
                crate_name: module_name.to_string(),
                path: exact,
            });
        }

        if candidates.is_empty() { continue; }

        let chosen = if let Some(ver) = version {
            candidates.iter()
                .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains(ver)))
                .or_else(|| candidates.iter().max_by_key(|p| p.file_name().map(|n| n.to_os_string())))
        } else {
            candidates.iter().max_by_key(|p| p.file_name().map(|n| n.to_os_string()))
        };

        if let Some(entry) = chosen {
            return Ok(CrateSource::LocalPath {
                crate_name: module_name.to_string(),
                path: entry.clone(),
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

/// ローカルの crate（`CrateSource::LocalPath`）のソースのディレクトリと、Rust での crate の名前。
///
/// ⚠ CLI はまだ使わない（拡張が `import[rs]` を読むようになる 4-1 で使う）。
///
/// `cargo metadata` に尋ねたときと同じ値（パス依存の crate の `manifest_path` の親の `src`・
/// 名前は `-` を `_` に替えたもの）。外部プロセスを使わないので拡張でも使える。
#[allow(dead_code)]
pub(crate) fn local_crate_src(source: &CrateSource) -> Option<(PathBuf, String)> {
    match source {
        CrateSource::LocalPath { crate_name, path } => {
            Some((path.join("src"), crate_name.replace('-', "_")))
        }
        CrateSource::Registry { .. } => None,
    }
}

/// 互換な `pub fn` / `pub struct` が 1 つも無いときの誤り（`rs_loader::load` と共有）。
pub(crate) fn no_compatible_items(module_name: &str) -> String {
    format!(
        "import[rs] `{module_name}`: no compatible pub fn or pub struct found \
         (only primitive types: int, float, bool, str, &[u8], Vec<u8>, [u8;N] are supported)"
    )
}

/// `import[rs]` の**型の情報だけ**（crate のソースを走査して型スタブを作る）。
///
/// 実行時の DLL は作らない（`cargo build` しない）。VS Code 拡張（wasm）はこれを使う
/// （CLI の `rs_loader::load` は同じ走査をしたうえで DLL も作る）。
#[allow(dead_code)] // 拡張（4-1）が使う
pub(crate) fn load_types(
    module_name: &str,
    search_dirs: &[PathBuf],
    version: Option<&str>,
) -> Result<Vec<Stmt>, String> {
    let source = find_config(module_name, version, search_dirs)?;
    let Some((crate_src_dir, crate_ident)) = local_crate_src(&source) else {
        return Err(format!("import[rs] `{module_name}`: only local crates (rust.crates_path) are supported"));
    };
    let (fns, structs) = scan_all_sigs(&crate_src_dir, &crate_ident);
    if fns.is_empty() && structs.is_empty() {
        return Err(no_compatible_items(module_name));
    }
    Ok(make_stubs(&fns, &structs))
}

