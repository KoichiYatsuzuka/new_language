// header_parser.rs — C/C++ header parsing: function declarations, struct
// definitions, type resolution, and text preprocessing utilities.
//
// Public API:
//   parse_header_full        — parse sigs + struct defs from a header string
//   parse_header             — parse sigs only
//   collect_included_headers — find local #include paths in raw header text
//
// Internal utilities re-used by typedef_loader (pub(crate)):
//   typedef_contains_fn_ptr, extract_alias_token, parse_alias_list
//   strip_and_preprocess, strip_comments, find_matching_brace

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::types::{CStructDef, CFnSig, CType};

const DEFAULTPARAM_MACRO: &str = "DEFAULTPARAM";

// ── Header parser ────────────────────────────────────────────────────────────

/// C/C++ ヘッダから関数宣言と構造体定義を解析する。`(functions, structs)` を返す。
/// 全フィールドがプリミティブ `CType` に解決できる構造体のみ出力する。
/// `custom` は C 型名から tl プリミティブ型へのマッピング、`typedefs` は `load_system_types` で構築済みのエイリアスマップ（`SystemTypes::typedefs`）。
pub fn parse_header_full(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> (Vec<CFnSig>, Vec<CStructDef>) {
    let stripped = strip_comments(content);
    let mut decls: Vec<(String, Option<String>)> = Vec::new();
    scan_scope(&stripped, None, &mut decls);

    let mut sigs = Vec::new();
    for (decl, ns) in &decls {
        if let Ok(sig) = parse_fn_decl_ns(decl, ns.clone(), custom, typedefs) {
            sigs.push(sig);
        }
    }

    let mut structs = parse_struct_bodies(&stripped, custom, typedefs);
    // 前方宣言だけの構造体（`typedef struct Db Db;` / `struct Db;`）も「構造体の名前」として持つ（フィールドは空・
    // 不完全・4-3）。ポインタの引数・戻り値で `Ptr[Db]` を受け渡せるのは、ヘッダが構造体と宣言した名前だけにする
    // （`CFnSig::unsupported_reason`）。⚠ 解決できない typedef（`TCHAR` / `HWND` …）も構造体ポインタに見えるので、
    //   それを `Ptr[TCHAR]` にしてしまわないため（`const TCHAR*` は実は C の文字列）。
    for name in forward_declared_structs(&stripped) {
        if !structs.iter().any(|d| d.name == name) {
            structs.push(CStructDef { name, fields: Vec::new(), complete: false });
        }
    }
    // 共有ヘッダの `ArPtrProtocol` そのもの（取り込んだヘッダ・写した定義）は Arrow のクラスにしない。
    // 受け渡しは型として扱う（`CType::PtrProtocol`・4-2）
    structs.retain(|d| d.name != "ArPtrProtocol");
    (sigs, structs)
}

/// ヘッダから集めた関数と構造体（`collect_header_decls`）。
pub struct HeaderDecls {
    pub sigs: Vec<CFnSig>,
    pub structs: Vec<CStructDef>,
    /// 取り込んだヘッダと、そこから足した関数の数（実行時の表示用）。
    pub included: Vec<(PathBuf, usize)>,
}

/// cpp のヘッダから関数と構造体を集める。**型スタブ（`parser/imports/cpp.rs` の `parse_cpp_import`）と実行時
/// （`exec/modules.rs` の `load_cpp_module`）が同じこれを使う**（ffi_value_exchange_plan.md 7-28）:
///   1. 本体のヘッダ（`ar_config.json` の `cpp.system_headers` の typedef で型を解決する）
///   2. `cpp-lib` で `precompile_macros` があるときは、本体が取り込む同じ場所のヘッダ（DxLib の `DxFunctionWin.h`）も読み、
///      まだ無い関数・構造体を足す（マクロで有効になる宣言がそこにある）
///   3. 参照されるシステムヘッダの構造体（`RECT` など・`add_system_structs`・7-13）
/// ⚠ 以前は 2 を実行時だけが行い、型スタブには取り込んだヘッダの関数が無かった（使うと型検査で「名前が無い」）。
/// 取り込んだヘッダが読めなければ誤り（黙って飛ばさない）。
pub fn collect_header_decls(
    raw: &str,
    header_dir: &Path,
    lang: &str,
    cfg: &crate::cpp_header::CppBuildConfig,
) -> Result<HeaderDecls, String> {
    let system = crate::cpp_header::load_system_types(&cfg.system_headers, &cfg.precompile_macros);
    let typedefs = &system.typedefs;
    let (mut sigs, mut structs) = parse_header_full(raw, &cfg.custom_type_map, typedefs);
    let mut included = Vec::new();
    if lang == "cpp-lib" && !cfg.precompile_macros.is_empty() {
        let mut known_names: std::collections::HashSet<String> = sigs.iter().map(|s| s.name.clone()).collect();
        let mut known_structs: std::collections::HashSet<String> = structs.iter().map(|d| d.name.clone()).collect();
        for inc_path in collect_included_headers(raw, header_dir) {
            let inc_raw = crate::import_fs::read(&inc_path).map_err(|e| {
                format!("import[{lang}]: cannot read included header '{}' ({})", inc_path.display(), e.kind())
            })?;
            let inc_str = String::from_utf8_lossy(&inc_raw);
            let (inc_sigs, inc_structs) = parse_header_full(&inc_str, &cfg.custom_type_map, typedefs);
            let mut added = 0usize;
            for s in inc_sigs {
                if known_names.insert(s.name.clone()) {
                    sigs.push(s);
                    added += 1;
                }
            }
            for d in inc_structs {
                if known_structs.insert(d.name.clone()) {
                    structs.push(d);
                }
            }
            included.push((inc_path, added));
        }
    }
    add_system_structs(&sigs, &mut structs, &system.structs, &cfg.custom_type_map, typedefs);
    Ok(HeaderDecls { sigs, structs, included })
}

/// ヘッダの関数・構造体が参照する構造体のうち、ヘッダに定義が無く、システムヘッダ（`ar_config.json` の
/// `cpp.system_headers`）に定義があるもの（`RECT` など）を `structs` に足す（ffi_value_exchange_plan.md 7-13）。
/// 足した構造体のフィールドが参照する構造体も足す。システムヘッダに定義があるが読めない構造体（入れ子の union など）は
/// 中身の見えない構造体（`Ptr[T]` で受け渡す・4-3）として足す。
/// ⚠ 以前はシステムヘッダの構造体を知らず、`const RECT*` の引数を持つ関数（DxLib の `ClearDrawScreen`）は「構造体と
///   宣言されていない名前」として使えなかった。
pub fn add_system_structs(
    sigs: &[CFnSig],
    structs: &mut Vec<CStructDef>,
    system: &HashMap<String, String>,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) {
    fn referred(ct: &CType, out: &mut Vec<String>) {
        if let CType::OpaqueStructPtr { type_name, .. } | CType::ByValueStruct { type_name } = ct {
            out.push(type_name.clone());
        }
    }
    loop {
        let mut wanted: Vec<String> = Vec::new();
        for s in sigs {
            referred(&s.ret, &mut wanted);
            for (_, p) in &s.params {
                referred(p, &mut wanted);
            }
        }
        for d in structs.iter() {
            for (_, f) in &d.fields {
                referred(f, &mut wanted);
            }
        }
        wanted.retain(|n| system.contains_key(n) && !structs.iter().any(|d| &d.name == n));
        wanted.sort();
        wanted.dedup();
        if wanted.is_empty() {
            return;
        }
        for name in wanted {
            let def = parse_header_full(&system[&name], custom, typedefs)
                .1
                .into_iter()
                .find(|d| d.name == name)
                .unwrap_or(CStructDef { name, fields: Vec::new(), complete: false });
            structs.push(def);
        }
    }
}

/// 本体の無い構造体の宣言（`typedef struct Tag Alias;` → `Tag` と `Alias`、`struct Tag;` → `Tag`）の名前を集める（4-3）。
/// ⚠ 本体のある定義（`typedef struct { … } Rec;`）は `parse_struct_bodies` が扱うので、ここでは `{` / `}` の後ろだけを見る。
fn forward_declared_structs(stripped: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in stripped.split(';') {
        let tail = seg.rsplit(|c| c == '{' || c == '}').next().unwrap_or("");
        let words: Vec<&str> = tail
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .flat_map(|l| l.split_whitespace())
            .collect();
        let ident = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || c == '_');
        let names: Vec<&str> = match words.as_slice() {
            ["typedef", "struct", tag, alias] if ident(tag) && ident(alias) => vec![tag, alias],
            ["struct", tag] if ident(tag) => vec![tag],
            _ => vec![],
        };
        for n in names {
            if !out.iter().any(|o| o == n) {
                out.push(n.to_string());
            }
        }
    }
    out
}

/// C/C++ ヘッダから関数シグネチャのみを解析して返す（構造体定義は無視）。
#[allow(dead_code)]
pub fn parse_header(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CFnSig> {
    parse_header_full(content, custom, typedefs).0
}

/// ヘッダの生テキストからローカル `#include "filename.h"` ディレクティブを検索し、`header_dir` からの相対パスとして存在するパスを返す。
pub fn collect_included_headers(raw_content: &str, header_dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for line in raw_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#include") {
            continue;
        }
        let after = trimmed["#include".len()..].trim_start();
        // Only quoted includes (local headers), not angle-bracket system headers
        if !after.starts_with('"') {
            continue;
        }
        let inner = &after[1..];
        if let Some(end) = inner.find('"') {
            let fname = &inner[..end];
            // Only simple filenames (no path separators) resolved relative to header_dir
            let candidate = header_dir.join(fname);
            if crate::import_fs::exists(&candidate) && !result.contains(&candidate) {
                result.push(candidate);
            }
        }
    }
    result
}


mod preprocess;
mod structs;
mod decls;
pub(crate) use preprocess::*;
pub(crate) use structs::*;
pub(crate) use decls::*;

// ── Tests ─────────────────────────────────────────────────────────────────────

// ⚠ `native` 限定: 結果を `raw_layout`（実行時のメソッド・`interpreter/cpp_bridge/types.rs`）で確かめる。
//   拡張の crate（`crates/arrow-frontend`）は `native` を持たないので、そちらではビルドしない
//   （ルート crate の `cargo test` では従来どおり走る）。
#[cfg(all(test, feature = "native"))]
mod tests;


#[cfg(test)]
mod ptr_protocol_tests;

// ffi_value_exchange_plan.md 7-13（DxLib の実用例で見つかったもの）
#[cfg(test)]
mod system_types_tests;

// ffi_value_exchange_plan.md 7-29: 関数ポインタの引数を持つ関数を黙って捨てない
#[cfg(test)]
mod fn_ptr_param_tests;
