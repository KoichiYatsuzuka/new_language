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
    // DxLib の SDK は git の対象外（利用者の環境）。場所は `ar_config.json` の `test.dxlib_header`（7-33）。書いていなければ
    // このテストは飛ばす
    let Some(path) = crate::ar_config::require_test_resource("dxlib_header", "the real DxLib header (DxLib.h)") else {
        return;
    };
    let raw = std::fs::read(&path).expect("header");
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
    // ⚠ 2445 → 2447（7-29）: `void (* cb )( void )` の宣言子をマクロとして消していたので、callback の引数が `( void )` の
    //   `SetRestoreShredPoint` / `SetRestoreGraphCallback` を捨てていた（ほかの宣言は全件の写しの比較で同じ）
    assert_eq!(sigs.len(), 2447, "declaration count drifted");
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
