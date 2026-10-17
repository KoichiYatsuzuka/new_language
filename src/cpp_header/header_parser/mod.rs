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

    /// 4 バイト未満の整数フィールドも幅と符号つきで読み、C と同じレイアウトになる（ffi_value_exchange_plan.md 1-6）。
    /// ⚠ 1-3 では `CType::Int` が幅を持たなかったので、不完全（レイアウト不明）として止めていた
    #[test]
    fn test_narrow_int_fields_have_exact_layout() {
        use crate::interpreter::value::RawWidth;
        let layout_of = |defs: &[CStructDef]| {
            assert_eq!(defs.len(), 1);
            assert!(defs[0].complete);
            let l = defs[0].raw_layout().expect("raw layout expected");
            let offs: Vec<usize> = l.fields.iter().map(|d| d.byte_offset).collect();
            let widths: Vec<RawWidth> = l.fields.iter().map(|d| d.width).collect();
            (offs, widths)
        };
        let (offs, widths) = layout_of(&parse("typedef struct { unsigned char r, g, b, a; } RGBA8;"));
        assert_eq!(offs, vec![0, 1, 2, 3]);
        assert!(widths.iter().all(|w| matches!(w, RawWidth::U8)));
        let (offs, widths) = layout_of(&parse("struct S { short a; int b; };"));
        assert_eq!(offs, vec![0, 4]);
        assert!(matches!(widths[0], RawWidth::I16));
        let (offs, _) = layout_of(&parse("struct U { uint16_t lo; uint16_t hi; };"));
        assert_eq!(offs, vec![0, 2]);
        // typedef で解決した幅も見る（DxLib の BYTE など）
        let custom = HashMap::new();
        let mut typedefs = HashMap::new();
        typedefs.insert("BYTE".to_string(), "unsigned char".to_string());
        let (offs, widths) = layout_of(&parse_header_full("struct C { BYTE r; BYTE g; };", &custom, &typedefs).1);
        assert_eq!(offs, vec![0, 1]);
        assert!(matches!(widths[0], RawWidth::U8));
    }

    /// ポインタは幅が決まっているので narrow ではない（4 バイト以上の整数も complete のまま）
    #[test]
    fn test_pointer_and_int32_fields_stay_complete() {
        let defs = parse("struct N { int id; const char* name; };");
        assert_eq!(defs.len(), 1);
        assert!(defs[0].complete);
        let defs = parse("struct P { int32_t a; uint32_t b; float c; };");
        assert_eq!(defs.len(), 1);
        assert!(defs[0].complete);
        assert!(defs[0].raw_layout().is_some());
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

    /// C の long はプラットフォームの幅で並べる（Windows は LLP64 で 4 バイト・他は 8 バイト）。
    /// 1-6 より前は幅が分からないとして raw レイアウトの対象外にしていた。
    #[test]
    fn test_long_field_follows_platform_width() {
        use crate::interpreter::value::RawWidth;
        let defs = parse("struct L { long v; int w; };");
        assert_eq!(defs.len(), 1);
        let l = defs[0].raw_layout().expect("raw layout expected");
        if cfg!(windows) {
            assert!(matches!(l.fields[0].width, RawWidth::I32));
            assert_eq!(l.fields[1].byte_offset, 4);
        } else {
            assert!(matches!(l.fields[0].width, RawWidth::I64));
            assert_eq!(l.fields[1].byte_offset, 8);
        }
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
        let custom = HashMap::new();
        let typedefs = HashMap::new();
        let (sigs, defs) = parse_header_full(&content, &custom, &typedefs);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"VECTOR"), "structs found: {} — {:?}", defs.len(), &names[..names.len().min(20)]);

        // ⚠⚠ #60 で**件数を固定した**。それ以前は「VECTOR が居る」だけの検査で、
        // `parse_struct_bodies` / `scan_scope` を丸ごと書き換えても素通りしていた
        // （#60 の実作業では一時的な全ダンプ差分で担保したが、恒久の網が要る）。
        // ⚠ パーサを**改良**して数が変わったら、差分を目で見てからこの値を更新すること。
        assert_eq!(sigs.len(), 2445, "declaration count drifted");
        // ⚠ 65 → 61（7-13）: 別名の並び `NEAR *NPWAVEFORMATEX` の最初の語を別名にしていたので、`NEAR` / `FAR` という
        //   偽の構造体が `WAVEFORMAT` / `WAVEFORMATEX` から 2 つずつできていた（宣言子は最後の語・`parse_alias_list`）
        assert_eq!(defs.len(), 61, "struct count drifted");

        // 代表的な struct の raw レイアウトも固定する（オフセット計算の回帰検知）。
        let vector = defs.iter().find(|d| d.name == "VECTOR").expect("VECTOR");
        let layout = vector.raw_layout().expect("VECTOR raw layout");
        assert_eq!(
            layout.fields.iter().map(|f| f.byte_offset).collect::<Vec<_>>(),
            vec![0, 4, 8]
        );
        assert_eq!(layout.total_bytes, 16);
    }
}


#[cfg(test)]
mod ptr_protocol_tests {
    use super::*;

    /// `ArPtrProtocol` を引数・戻り値・構造体のフィールドで認識する（ffi_value_exchange_plan.md 4-2）。
    /// 引数は 4-4、値返しは 4-5 から使える（ポインタを返すものは使えない）。共有ヘッダの定義を写してあってもクラスにしない。
    #[test]
    fn recognises_ptr_protocol() {
        let header = r#"
            #include "ar_ptr_protocol.h"
            typedef struct ArPtrProtocol { uint64_t address; const char* elem_type; } ArPtrProtocol;
            ArPtrProtocol make_squares(int n);
            void free_it(ArPtrProtocol* p);
            int count(const struct ArPtrProtocol* p);
            int pass(ArPtrProtocol p);
            ArPtrProtocol* bad(void);
            typedef struct { int n; ArPtrProtocol data; } Holder;
        "#;
        let (sigs, structs) = parse_header_full(header, &HashMap::new(), &HashMap::new());
        let get = |name: &str| sigs.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(get("make_squares").ret, CType::PtrProtocol { by_pointer: false, mutable: false });
        assert_eq!(get("free_it").params[0].1, CType::PtrProtocol { by_pointer: true, mutable: true });
        assert_eq!(get("count").params[0].1, CType::PtrProtocol { by_pointer: true, mutable: false });
        assert_eq!(get("pass").params[0].1, CType::PtrProtocol { by_pointer: false, mutable: false });
        assert_eq!(get("bad").ret, CType::PtrProtocol { by_pointer: true, mutable: true });
        // ポインタを返すものは使えない。引数（4-4）と値返し（4-5）は使える
        let why = get("bad").unsupported_reason(&structs).unwrap_or_default();
        assert!(why.contains("ArPtrProtocol"), "bad: {why}");
        for name in ["make_squares", "free_it", "count", "pass"] {
            assert_eq!(get(name).unsupported_reason(&structs), None, "{name}");
        }
        assert!(structs.iter().all(|d| d.name != "ArPtrProtocol"), "the protocol itself is not a class");
        let holder = structs.iter().find(|d| d.name == "Holder").expect("Holder");
        assert_eq!(holder.fields[1].1, CType::PtrProtocol { by_pointer: false, mutable: false });
        assert!(!holder.has_exact_layout(), "a struct with an ArPtrProtocol field is expanded in 4-6");
    }

    /// 本体の無い構造体の宣言を「構造体の名前」として持つ（4-3）。解決できない typedef（`TCHAR`）へのポインタは
    /// 構造体ではないので使えないまま。
    #[test]
    fn forward_declared_structs_are_known() {
        let header = r#"
            typedef struct Db Db;
            struct Window;
            typedef struct { int x; } Point;
            Db* open_db(const char* name);
            int count(Db* db);
            void show(struct Window* w);
            int draw(const TCHAR* s);
        "#;
        let (sigs, structs) = parse_header_full(header, &HashMap::new(), &HashMap::new());
        for name in ["Db", "Window"] {
            let d = structs.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("{name}"));
            assert!(d.fields.is_empty() && !d.complete);
        }
        assert_eq!(structs.iter().filter(|d| d.name == "Point").count(), 1, "a struct with a body is not added twice");
        let get = |name: &str| sigs.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(get("open_db").unsupported_reason(&structs), None);
        assert_eq!(get("count").unsupported_reason(&structs), None);
        assert_eq!(get("show").params[0].1, CType::OpaqueStructPtr { type_name: "Window".to_string(), mutable: true });
        assert_eq!(get("show").unsupported_reason(&structs), None);
        let why = get("draw").unsupported_reason(&structs).unwrap_or_default();
        assert!(why.contains("TCHAR") && why.contains("does not declare as a struct"), "{why}");
    }
}

// ffi_value_exchange_plan.md 7-13（DxLib の実用例で見つかったもの）
#[cfg(test)]
mod system_types_tests {
    use super::*;
    use crate::cpp_header::types::{CDefault, IntWidth};

    /// `const T*` は T が typedef でも `const` のまま（以前は外れて、`const TCHAR*` が書き込み用の `char*` になっていた）
    #[test]
    fn const_survives_typedef_resolution() {
        let mut td = HashMap::new();
        td.insert("TCHAR".to_string(), "char".to_string());
        td.insert("PCSTR".to_string(), "const char*".to_string());
        let c = |s: &str| parse_c_type_str(s, &HashMap::new(), &td).unwrap();
        assert_eq!(c("const TCHAR *"), CType::CharPtr { wide: false, mutable: false });
        assert_eq!(c("TCHAR *"), CType::CharPtr { wide: false, mutable: true });
        assert_eq!(c("PCSTR"), CType::CharPtr { wide: false, mutable: false });
        // 組み込みの型は typedef で置き換えない（Windows SDK の C 向けの `typedef unsigned short wchar_t;`）
        td.insert("wchar_t".to_string(), "unsigned short".to_string());
        let c = |s: &str| parse_c_type_str(s, &HashMap::new(), &td).unwrap();
        assert_eq!(c("const wchar_t *"), CType::CharPtr { wide: true, mutable: false });
    }

    /// 既定値: `DEFAULTPARAM( = x )` と C++ の `= x` を引数の型で読む。読めない既定値の引数は省略できない
    #[test]
    fn default_arguments() {
        let sig = |d: &str| parse_fn_decl_ns(d, None, &HashMap::new(), &HashMap::new()).unwrap();
        let s = sig("int DrawString(int x, const char* s, unsigned int Edge DEFAULTPARAM( = 0 ), double z DEFAULTPARAM( = 1.5f ), void* p DEFAULTPARAM(=NULL))");
        assert_eq!(s.n_required, 2);
        assert_eq!(
            s.defaults,
            vec![None, None, Some(CDefault::Int(0)), Some(CDefault::Float(1.5)), Some(CDefault::Null)]
        );
        // マクロ名の既定値は読めない: その引数は省略できない（後ろの読める既定値だけ省略できる）
        let s = sig("int f(int a, int mode DEFAULTPARAM( = MODE_X ), int b DEFAULTPARAM( = -1 ))");
        assert_eq!(s.n_required, 2);
        assert_eq!(s.defaults, vec![None, None, Some(CDefault::Int(-1))]);
        // C++ の既定値（名前と型も正しく読む）
        let s = sig("int g(int a, int b = 0x10, bool on = true)");
        assert_eq!((s.n_required, s.params[1].0.as_str(), s.params[2].0.as_str()), (1, "b", "on"));
        assert_eq!(s.defaults, vec![None, Some(CDefault::Int(16)), Some(CDefault::Bool(true))]);
        // Windows の `BOOL` の TRUE / FALSE。ポインタの既定値は NULL だけ
        let int = CType::Int { width: IntWidth::W32, signed: true };
        assert_eq!(int.parse_default("TRUE"), Some(CDefault::Int(1)));
        assert_eq!(int.parse_default("( -2 )"), Some(CDefault::Int(-2)));
        assert_eq!(CType::VoidPtr.parse_default("5"), None);
        assert_eq!(CType::Double.parse_default("SOME_MACRO"), None);
    }

    /// 型スタブと実行時が使うヘッダの集め方（7-28）: cpp-lib で `precompile_macros` があるときだけ取り込んだヘッダも読む
    #[test]
    fn included_headers_are_collected_for_cpp_lib() {
        let dir = std::env::temp_dir().join(format!("ar_collect_decls_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("extra.h"), "int extra_fn(int x);
").unwrap();
        let main = "#include \"extra.h\"
int main_fn(int x);
";
        let mut cfg = crate::cpp_header::CppBuildConfig::default();
        cfg.precompile_macros = vec!["WINDOWS_DESKTOP_OS".to_string()];
        let names = |lang: &str, cfg: &crate::cpp_header::CppBuildConfig| -> Vec<String> {
            collect_header_decls(main, &dir, lang, cfg).unwrap().sigs.into_iter().map(|s| s.name).collect()
        };
        assert_eq!(names("cpp-lib", &cfg), vec!["main_fn", "extra_fn"]);
        assert_eq!(names("cpp-dll", &cfg), vec!["main_fn"], "cpp-dll does not read included headers");
        assert_eq!(names("cpp-lib", &crate::cpp_header::CppBuildConfig::default()), vec!["main_fn"], "only with precompile_macros");
        let d = collect_header_decls(main, &dir, "cpp-lib", &cfg).unwrap();
        assert_eq!(d.included.len(), 1);
        assert_eq!(d.included[0].1, 1, "one function added from extra.h");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// システムヘッダの構造体（`RECT`）を、参照されたものだけ足す
    #[test]
    fn system_structs_are_added_when_referred() {
        let mut td = HashMap::new();
        td.insert("LONG".to_string(), "long".to_string());
        let mut system = HashMap::new();
        system.insert("RECT".to_string(), "typedef struct { LONG left; LONG top; LONG right; LONG bottom; } RECT;".to_string());
        system.insert("POINT".to_string(), "typedef struct { LONG x; LONG y; } POINT;".to_string());
        let (sigs, mut structs) =
            parse_header_full("int ClearDrawScreen(const RECT *ClearRect DEFAULTPARAM( = NULL ));", &HashMap::new(), &td);
        assert!(sigs[0].unsupported_reason(&structs).is_some(), "RECT is unknown before");
        add_system_structs(&sigs, &mut structs, &system, &HashMap::new(), &td);
        let names: Vec<&str> = structs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["RECT"], "only the referred struct");
        assert!(structs[0].has_exact_layout());
        assert_eq!(sigs[0].unsupported_reason(&structs), None);
        assert_eq!(sigs[0].params[0].1, CType::OpaqueStructPtr { type_name: "RECT".to_string(), mutable: false });
        assert_eq!((sigs[0].n_required, sigs[0].defaults.clone()), (0, vec![Some(CDefault::Null)]));
    }
}
