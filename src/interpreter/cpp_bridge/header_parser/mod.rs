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
/// `custom` は C 型名から tl プリミティブ型へのマッピング、`typedefs` は `load_system_typedefs` で構築済みのエイリアスマップ。
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

    let structs = parse_struct_bodies(&stripped, custom, typedefs);
    (sigs, structs)
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
            if candidate.exists() && !result.contains(&candidate) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Vec<CStructDef> {
        let custom = HashMap::new();
        let typedefs = HashMap::new();
        parse_header_full(src, &custom, &typedefs).1
    }

    /// typedef struct（C スタイル）: 完全 + raw レイアウト（float×3 = オフセット 0,4,8）
    #[test]
    fn test_c_typedef_struct_raw_layout() {
        let defs = parse("typedef struct tagVEC { float x; float y; float z; } VEC;");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "VEC");
        assert!(defs[0].complete);
        let layout = defs[0].raw_layout().expect("raw layout expected");
        let offs: Vec<usize> = layout.fields.iter().map(|d| d.byte_offset).collect();
        assert_eq!(offs, vec![0, 4, 8]);
        assert_eq!(layout.total_bytes, 16); // 12 → 8 の倍数へ切り上げ
    }

    /// C++ class（メソッド・アクセス指定子つき）: フィールドのみ抽出、完全
    #[test]
    fn test_cpp_simple_class() {
        let src = "class Point {\npublic:\n    int x;\n    int y;\n    double w;\n    int sum();\nprivate:\n    void helper(int a);\n};";
        let defs = parse(src);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Point");
        assert!(defs[0].complete);
        assert_eq!(defs[0].fields.len(), 3);
        let layout = defs[0].raw_layout().expect("raw layout expected");
        // int(4) int(4) double(8, アラインメント 8) → 0,4,8
        let offs: Vec<usize> = layout.fields.iter().map(|d| d.byte_offset).collect();
        assert_eq!(offs, vec![0, 4, 8]);
        assert_eq!(layout.total_bytes, 16);
    }

    /// C アラインメント: int(4) + double(8) → double は 8 境界へパディング
    #[test]
    fn test_c_alignment_padding() {
        let defs = parse("struct Mixed { int a; double b; int c; };");
        assert_eq!(defs.len(), 1);
        let layout = defs[0].raw_layout().expect("raw layout expected");
        let offs: Vec<usize> = layout.fields.iter().map(|d| d.byte_offset).collect();
        assert_eq!(offs, vec![0, 8, 16]); // a=0, (pad 4), b=8, c=16
        assert_eq!(layout.total_bytes, 24);
    }

    /// virtual メンバ関数を持つクラスは除外される（vtable でレイアウトが変わる）
    #[test]
    fn test_virtual_class_rejected() {
        let defs = parse("class Shape {\npublic:\n    int kind;\n    virtual void draw();\n};");
        assert!(defs.is_empty(), "virtual class must be rejected");
    }

    /// friend を持つクラスは除外される
    #[test]
    fn test_friend_class_rejected() {
        let defs = parse("class Secret {\npublic:\n    int v;\n    friend class Admin;\n};");
        assert!(defs.is_empty(), "friend class must be rejected");
    }

    /// ビットフィールド → complete=false → raw レイアウトなし
    #[test]
    fn test_bitfield_incomplete() {
        let defs = parse("struct Flags { int a; int b : 3; };");
        assert_eq!(defs.len(), 1);
        assert!(!defs[0].complete);
        assert!(defs[0].raw_layout().is_none());
    }

    /// 配列フィールド → complete=false（フィールドがスキップされた）
    #[test]
    fn test_array_field_incomplete() {
        let defs = parse("struct Mat { float m[16]; float scale; };");
        assert_eq!(defs.len(), 1);
        assert!(!defs[0].complete);
        assert!(defs[0].raw_layout().is_none());
    }

    /// 継承つきクラス → complete=false（基底部分のレイアウトが不明）
    #[test]
    fn test_inheritance_incomplete() {
        let defs = parse("class Derived : public Base {\npublic:\n    int extra;\n};");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Derived");
        assert!(!defs[0].complete);
    }

    /// union → complete=false（フィールドが重なる）
    #[test]
    fn test_union_incomplete() {
        let defs = parse("typedef union { int i; float f; } Num;");
        assert_eq!(defs.len(), 1);
        assert!(!defs[0].complete);
    }

    /// static メンバ・ネスト enum はレイアウトに寄与しない（complete のまま）
    #[test]
    fn test_static_and_nested_enum_ignored() {
        let src = "class Cfg {\npublic:\n    static int counter;\n    enum Mode { A, B };\n    int width;\n    int height;\n};";
        let defs = parse(src);
        assert_eq!(defs.len(), 1);
        assert!(defs[0].complete);
        assert_eq!(defs[0].fields.len(), 2);
        let layout = defs[0].raw_layout().expect("raw layout expected");
        assert_eq!(layout.fields[0].byte_offset, 0);
        assert_eq!(layout.fields[1].byte_offset, 4);
    }

    /// C の long は環境依存幅のため raw レイアウト対象外（構造体自体は保持）
    #[test]
    fn test_long_field_no_raw_layout() {
        let defs = parse("struct L { long v; };");
        assert_eq!(defs.len(), 1);
        assert!(defs[0].raw_layout().is_none());
    }

    /// DxLib.h の実フォーマット（タブ・複数エイリアス・改行ブレース）での VECTOR パース
    #[test]
    fn test_dxlib_vector_snippet() {
        let src = "typedef struct tagVECTOR
{
	float					x, y, z ;
} VECTOR, *LPVECTOR, FLOAT3, *LPFLOAT3 ;
";
        let defs = parse(src);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"VECTOR"), "got: {names:?}");
        let v = defs.iter().find(|d| d.name == "VECTOR").unwrap();
        assert!(v.complete, "VECTOR should be complete");
        assert!(v.raw_layout().is_some());
    }

    /// 実 DxLib.h からの構造体抽出（回帰デバッグ用）
    #[test]
    fn test_real_dxlib_header_structs() {
        let raw = std::fs::read("examples/DxLib/DxLib.h").expect("header");
        let content = String::from_utf8_lossy(&raw);
        let defs = parse(&content);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"VECTOR"), "structs found: {} — {:?}", defs.len(), &names[..names.len().min(20)]);
    }
}

