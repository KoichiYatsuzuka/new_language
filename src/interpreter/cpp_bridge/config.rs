// config.rs — Build configuration loaded from `tl_config.json`.
//
// Public API:
//   CppBuildConfig  — struct holding all build options
//   load_cpp_config — search for and parse tl_config.json

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Default constants ─────────────────────────────────────────────────────────

const CONFIG_FILE_NAME: &str = "tl_config.json";

pub(crate) const DEFAULT_SYSTEM_LIBS: &[&str] = &[
    "winmm.lib",
    "imm32.lib",
    "ws2_32.lib",
    "dxguid.lib",
    "d3d9.lib",
    "d3d11.lib",
    "dxgi.lib",
    "dinput8.lib",
    "d3dcompiler.lib",
];
// Ordered by preference: more specific (versioned) first.
pub(crate) const DEFAULT_LIB_PATTERNS: &[&str] = &["_vs2015_x64_md.lib", "_x64.lib"];
pub(crate) const DEFAULT_TARGET_ARCH: &str = "amd64";

// ── Build config ──────────────────────────────────────────────────────────────

/// Build configuration loaded from `tl_config.json`.
pub struct CppBuildConfig {
    /// Explicit path to `vcvarsall.bat`. When `None`, auto-detection is used.
    pub msvc: Option<PathBuf>,
    /// Additional search paths tried before `MSVC_CANDIDATES` when `msvc` is `None`.
    /// Configurable via `"msvc_search_paths": [...]` in `tl_config.json`.
    pub msvc_search_paths: Vec<String>,
    /// Preprocessor macros to `#define` before the header `#include` in the shim.
    pub precompile_macros: Vec<String>,
    /// Target architecture passed to `vcvarsall.bat` (default: `"amd64"`).
    pub target_arch: String,
    /// Extra flags appended to the `cl.exe` invocation (after the fixed base flags).
    pub cl_extra_flags: Vec<String>,
    /// Extra flags appended to the `/link` section of the `cl.exe` invocation.
    pub link_extra_flags: Vec<String>,
    /// System / SDK libraries to link. Defaults to `DEFAULT_SYSTEM_LIBS`.
    pub system_libs: Vec<String>,
    /// Whether to emit `#define WIN32_LEAN_AND_MEAN` in the shim (default: `true`).
    pub win32_lean_and_mean: bool,
    /// Additional C type → tl primitive mappings checked before the built-in table.
    /// Keys are C type names (no `*`, no qualifiers); values are tl primitive names
    /// (`"int"` / `"long"` / `"float"` / `"double"` / `"bool"` / `"void"`).
    pub custom_type_map: HashMap<String, String>,
    /// Suffix patterns used to discover library files next to the header.
    /// Ordered by preference (most specific first). Defaults to `DEFAULT_LIB_PATTERNS`.
    pub lib_patterns: Vec<String>,
    /// OS/SDK system headers to load for typedef alias resolution.
    /// E.g. `["C:/Program Files (x86)/Windows Kits/10/Include/10.0.22621.0/um/Windows.h"]`.
    pub system_headers: Vec<String>,
}

impl Default for CppBuildConfig {
    fn default() -> Self {
        CppBuildConfig {
            msvc: None,
            msvc_search_paths: Vec::new(),
            precompile_macros: Vec::new(),
            target_arch: DEFAULT_TARGET_ARCH.to_string(),
            cl_extra_flags: Vec::new(),
            link_extra_flags: Vec::new(),
            system_libs: DEFAULT_SYSTEM_LIBS.iter().map(|s| s.to_string()).collect(),
            win32_lean_and_mean: true,
            custom_type_map: HashMap::new(),
            lib_patterns: DEFAULT_LIB_PATTERNS.iter().map(|s| s.to_string()).collect(),
            system_headers: Vec::new(),
        }
    }
}

// ── Config loader ─────────────────────────────────────────────────────────────

/// Search for `tl_config.json` starting at `start_dir` and walking up to
/// parent directories, then the current working directory.
pub fn load_cpp_config(start_dir: &Path) -> CppBuildConfig {
    let mut config = CppBuildConfig::default();

    let mut search: Vec<PathBuf> = Vec::new();
    let canon = std::fs::canonicalize(start_dir).unwrap_or_else(|_| start_dir.to_path_buf());
    let mut d = canon;
    loop {
        search.push(d.clone());
        match d.parent() {
            Some(p) if p != d => d = p.to_path_buf(),
            _ => break,
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if !search.contains(&cwd) {
            search.push(cwd);
        }
    }

    for dir in &search {
        let cfg_path = dir.join(CONFIG_FILE_NAME);
        if cfg_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&cfg_path) {
                parse_tl_config_json(&text, &mut config);
            }
            break;
        }
    }

    config
}

/// Minimal JSON parser for `tl_config.json` — no external dependencies.
///
/// Expected schema:
/// ```json
/// {
///   "cpp": {
///     "msvc": "...",
///     "msvc_search_paths": ["C:/path/to/vcvarsall.bat"],
///     "precompile_macros": ["MACRO1"],
///     "target_arch": "amd64",
///     "cl_extra_flags": ["/W4"],
///     "link_extra_flags": [],
///     "system_libs": ["winmm.lib"],
///     "win32_lean_and_mean": true,
///     "custom_type_map": { "MY_TYPE": "int" },
///     "lib_patterns": ["_vs2015_x64_md.lib", "_x64.lib"]
///   }
/// }
/// ```
fn parse_tl_config_json(content: &str, config: &mut CppBuildConfig) {
    let cpp_block = match extract_json_object(content, "cpp") {
        Some(b) => b,
        None => return,
    };

    if let Some(v) = extract_json_string(&cpp_block, "msvc") {
        config.msvc = Some(PathBuf::from(v));
    }
    if let Some(v) = extract_json_array(&cpp_block, "msvc_search_paths") {
        config.msvc_search_paths = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "precompile_macros") {
        config.precompile_macros = v;
    }
    if let Some(v) = extract_json_string(&cpp_block, "target_arch") {
        config.target_arch = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "cl_extra_flags") {
        config.cl_extra_flags = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "link_extra_flags") {
        config.link_extra_flags = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "system_libs") {
        config.system_libs = v;
    }
    if let Some(v) = extract_json_bool(&cpp_block, "win32_lean_and_mean") {
        config.win32_lean_and_mean = v;
    }
    if let Some(v) = extract_json_string_map(&cpp_block, "custom_type_map") {
        config.custom_type_map = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "lib_patterns") {
        config.lib_patterns = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "system_headers") {
        config.system_headers = v;
    }
}

// ── Minimal JSON helpers ──────────────────────────────────────────────────────

/// Extract the string content of a JSON string value for the given key.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let inner = &after_colon[1..];
    let end = inner.find('"')?;
    Some(
        inner[..end]
            .replace("\\\\", "\\")
            .replace("\\\"", "\"")
            .replace("\\/", "/"),
    )
}

/// Extract a JSON boolean value for the given key.
fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Extract a JSON array of strings for the given key.
fn extract_json_array(json: &str, key: &str) -> Option<Vec<String>> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('[') {
        return None;
    }
    let bracket_end = after_colon.find(']')?;
    let arr_content = &after_colon[1..bracket_end];
    let items = arr_content
        .split(',')
        .filter_map(|item| {
            let t = item.trim();
            if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                Some(t[1..t.len() - 1].to_string())
            } else {
                None
            }
        })
        .collect();
    Some(items)
}

/// Extract a JSON object `{ "key": "value", ... }` as a `HashMap<String, String>`.
/// Only string-valued keys are extracted; non-string values are skipped.
fn extract_json_string_map(json: &str, key: &str) -> Option<HashMap<String, String>> {
    let block = extract_json_object(json, key)?;
    let mut map = HashMap::new();
    let mut rest = block.as_str();
    while let Some(q1) = rest.find('"') {
        rest = &rest[q1 + 1..];
        let q2 = match rest.find('"') {
            Some(i) => i,
            None => break,
        };
        let k = rest[..q2].to_string();
        rest = &rest[q2 + 1..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => break,
        };
        rest = rest[colon + 1..].trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        rest = &rest[1..];
        let q3 = match rest.find('"') {
            Some(i) => i,
            None => break,
        };
        let v = rest[..q3].to_string();
        rest = &rest[q3 + 1..];
        map.insert(k, v);
    }
    Some(map)
}

/// Extract the object content `{ ... }` for the given key from a JSON string.
fn extract_json_object(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('{') {
        return None;
    }
    // Find matching closing brace
    let mut depth = 0usize;
    for (i, ch) in after_colon.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after_colon[1..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
