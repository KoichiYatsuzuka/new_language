// config.rs — Build configuration loaded from `hv_config.json`.
//
// Public API:
//   CppBuildConfig  — struct holding all build options
//   load_cpp_config — search for and parse hv_config.json

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Default constants ─────────────────────────────────────────────────────────

const CONFIG_FILE_NAME: &str = "hv_config.json";

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

/// `hv_config.json` から読み込まれる C++ ビルド設定。
pub struct CppBuildConfig {
    /// `vcvarsall.bat` への明示パス。`None` の場合は自動検出を使用する。
    pub msvc: Option<PathBuf>,
    /// `msvc` が `None` の場合に `MSVC_CANDIDATES` より先に試みる追加検索パス。`"msvc_search_paths"` キーで設定する。
    pub msvc_search_paths: Vec<String>,
    /// シムの `#include` 前に `#define` するプリプロセッサマクロ。
    pub precompile_macros: Vec<String>,
    /// `vcvarsall.bat` に渡すターゲットアーキテクチャ（デフォルト: `"amd64"`）。
    pub target_arch: String,
    /// `cl.exe` 呼び出しに追加するフラグ（固定フラグの後に追記）。
    pub cl_extra_flags: Vec<String>,
    /// `cl.exe` の `/link` セクションに追加するフラグ。
    pub link_extra_flags: Vec<String>,
    /// リンクするシステム / SDK ライブラリ。デフォルトは `DEFAULT_SYSTEM_LIBS`。
    pub system_libs: Vec<String>,
    /// シムに `#define WIN32_LEAN_AND_MEAN` を出力するかどうか（デフォルト: `true`）。
    pub win32_lean_and_mean: bool,
    /// 組み込みテーブルより先に参照する追加 C 型 → tl プリミティブ マッピング。
    /// キーは C 型名（`*` や修飾子なし）、値は tl プリミティブ名（`"int"` / `"long"` / `"float"` / `"double"` / `"bool"` / `"void"`）。
    pub custom_type_map: HashMap<String, String>,
    /// ヘッダ隣接ライブラリを検出するサフィックスパターン（優先度の高い順）。デフォルトは `DEFAULT_LIB_PATTERNS`。
    pub lib_patterns: Vec<String>,
    /// typedef エイリアス解決のためにロードする OS/SDK システムヘッダ。
    pub system_headers: Vec<String>,
}

impl Default for CppBuildConfig {
    /// `CppBuildConfig` のデフォルト値を返す。各フィールドはデフォルト定数で初期化される。
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

/// `start_dir` から親ディレクトリへと遡りながら `hv_config.json` を検索してロードする。
/// 見つからなければ現在のワーキングディレクトリも確認する。
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

/// `hv_config.json` の内容を最小限の JSON パーサで解析し `config` に設定値を反映する。外部依存なし。
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

/// 指定キーに対応する JSON 文字列値の内容を取り出して返す。
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

/// 指定キーに対応する JSON 真偽値を取り出して返す。
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

/// 指定キーに対応する JSON 文字列配列を取り出して返す。
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

/// 指定キーに対応する JSON オブジェクト `{ "key": "value", ... }` を `HashMap<String, String>` として取り出す。文字列値のキーのみ抽出し、非文字列値はスキップする。
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

/// 指定キーに対応する JSON オブジェクト `{ ... }` の内容文字列を取り出して返す。
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
