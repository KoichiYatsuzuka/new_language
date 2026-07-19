// rs_loader/loader.rs — import[rs] のロード統括: load 本体、crate 設定探索、digest バージョン検出、Cargo.toml パッチ、ラッパー生成準備。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt},
    crate::partial_compiler::llvm_codegen::FnExport,
    crate::partial_compiler::module_compiler::{cache_native, native_lib_ext},
};
#[allow(unused_imports)]
use super::*;

// ── Public entry point ────────────────────────────────────────────────────────

/// Find the crate, parse compatible `pub fn`/`pub struct`, compile a
/// call-through wrapper DLL, cache it, and return `Stmt` stubs.
pub(crate) fn load(module_name: &str, search_dirs: &[PathBuf], version: Option<&str>) -> Result<Vec<Stmt>, String> {
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

pub(crate) fn find_config(module_name: &str, version: Option<&str>, search_dirs: &[PathBuf]) -> Result<CrateSource, String> {
    let cwd = std::env::current_dir().ok();
    let extra: &[PathBuf] = cwd.as_slice();
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

        let candidates: Vec<_> = std::fs::read_dir(&crates_root)
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
pub(crate) fn detect_digest_version(crate_src_dir: &Path) -> String {
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
                        .trim_start_matches([' ', '='])
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
pub(crate) fn patch_cargo_toml_digest(cargo_toml: &Path, version: &str) -> std::io::Result<()> {
    let mut text = std::fs::read_to_string(cargo_toml).unwrap_or_default();
    if !text.contains("digest") {
        text.push_str(&format!("digest=\"{version}\"\n"));
        std::fs::write(cargo_toml, &text)?;
    }
    Ok(())
}

// ── Wrapper project preparation ───────────────────────────────────────────────

pub(crate) fn prepare_wrapper(
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

