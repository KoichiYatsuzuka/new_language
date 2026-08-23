// header_parser/structs.rs — C/C++ struct/class 定義の解析: struct 本体・フィールド宣言・メンバ分類(MemberKind)・raw レイアウト判定。

use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::interpreter::cpp_bridge::types::{CStructDef, CFnSig, CType},
};
use super::*;

// ── Struct body parsing ───────────────────────────────────────────────────────

/// ストリップ済みソースから `typedef struct/union Tag { … } Alias;` 定義を走査し、
/// 全フィールドがプリミティブ `CType` に解決できるエイリアス（非ポインタのみ）ごとに `CStructDef` を返す。
pub(crate) fn parse_struct_bodies(
    stripped: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CStructDef> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;

    while i < stripped.len() {
        // ⚠ #60: **成功パスを早期 continue へ反転**した（以前は最大ネスト 11 ＝ src 最悪）。
        // `{` 以外の位置は「区切りを覚えて 1 バイト進むだけ」。
        if !stripped[i..].starts_with('{') {
            if stripped[i..].starts_with(';') {
                seg_start = i + 1;
            }
            i += 1;
            continue;
        }

        // ⚠ `trim()` と `trim_start()` を**取り違えないこと**。
        // 分類側（`classify_struct_head`）は `trim_start()` でなければならない —
        // `.trim()` すると `"typedef union "` の**末尾の空白が消えて** `" union "` の
        // 部分文字列判定が外れ、union が struct として扱われる。
        let seg_raw = &stripped[seg_start..i];

        // `namespace X { … }` / `extern "C" { … }` はスコープブロックなので
        // スキップせず内部に降下する（DxLib.h は全体が namespace DxLib で包まれている）。
        if is_scope_block(seg_raw.trim()) {
            i += 1;
            seg_start = i;
            continue;
        }

        let Some(brace_end) = find_matching_brace(&stripped[i..]) else {
            // 対応する `}` が無い（打ち切られたヘッダ等）。1 バイト進めて探索を続ける。
            i += 1;
            continue;
        };

        let body = &stripped[i + 1..i + brace_end];
        match classify_struct_head(seg_raw.trim_start()) {
            StructHead::Typedef { is_union } => push_typedef_structs(
                body,
                &stripped[i + brace_end + 1..],
                is_union,
                custom,
                typedefs,
                &mut result,
            ),
            StructHead::Class { name, inherits } => {
                push_class_struct(&name, body, inherits, custom, typedefs, &mut result)
            }
            StructHead::Other => {}
        }
        i += brace_end + 1;
        seg_start = i;
    }
    result
}

/// `{` の直前のセグメントが**スコープブロック**（`namespace X` / `extern "C"`）か（#60）。
///
/// スコープブロックは**スキップせず内部へ降下する**（DxLib.h は全体が `namespace DxLib`）。
fn is_scope_block(seg_before: &str) -> bool {
    let w: Vec<&str> = seg_before.split_whitespace().collect();
    matches!(w.first().copied(), Some("namespace"))
        || (w.contains(&"extern") && seg_before.contains("\"C\""))
}

/// `{` の直前のセグメントから**何の定義か**を判定した結果（#60）。
enum StructHead {
    /// `typedef struct/union Tag { … } Alias;` — 名前は `}` の**後ろ**にある。
    Typedef { is_union: bool },
    /// `class Name { … }` / `struct Name { … }`（typedef ではない）。
    /// `inherits` は継承の有無（基底部分がレイアウトに入るので complete=false になる）。
    Class { name: String, inherits: bool },
    /// 構造体定義ではない（関数本体・初期化子など）。
    Other,
}

/// `{` の直前のセグメントを分類する（#60 で `parse_struct_bodies` から切り出し）。
///
/// ⚠ 引数は **`trim_start()` したもの**を渡すこと（`trim()` だと union 判定が壊れる）。
fn classify_struct_head(seg_before: &str) -> StructHead {
    let is_union = seg_before.starts_with("typedef") && seg_before.contains(" union ");
    if seg_before.starts_with("typedef") && (seg_before.contains(" struct ") || is_union) {
        return StructHead::Typedef { is_union };
    }
    let w: Vec<&str> = seg_before.split_whitespace().collect();
    if !matches!(w.first().copied(), Some("class") | Some("struct")) || w.len() < 2 {
        return StructHead::Other;
    }
    let name = w[1].trim_end_matches(':');
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return StructHead::Other;
    }
    // 継承（`class D : public B`）はレイアウトに基底部分が含まれるため complete=false。
    StructHead::Class {
        name: name.to_string(),
        inherits: seg_before.contains(':'),
    }
}

/// `typedef struct/union Tag { … } A, B, *P;` の**エイリアスごと**に `CStructDef` を積む（#60）。
///
/// ⚠ **ポインタ別名（`*P`）は積まない**（raw レイアウトを持たないため）。
fn push_typedef_structs(
    body: &str,
    rest: &str,
    is_union: bool,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
    out: &mut Vec<CStructDef>,
) {
    let Some(semi_pos) = rest.find(';') else {
        return;
    };
    let aliases_str = rest[..semi_pos].trim();
    if aliases_str.is_empty() {
        return;
    }
    let Some((fields, fields_complete)) = parse_struct_field_decls(body, custom, typedefs) else {
        return;
    };
    if fields.is_empty() {
        return;
    }
    // union はフィールドが重なるためレイアウト不完全扱い
    let complete = fields_complete && !is_union;
    for (alias, ptr_suffix) in parse_alias_list(aliases_str, "") {
        if !ptr_suffix.contains('*') {
            out.push(CStructDef {
                name: alias,
                fields: fields.clone(),
                complete,
            });
        }
    }
}

/// `class Name { … }` / `struct Name { … }` を 1 件積む（#60）。
fn push_class_struct(
    name: &str,
    body: &str,
    has_inheritance: bool,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
    out: &mut Vec<CStructDef>,
) {
    let Some((fields, fields_complete)) = parse_struct_field_decls(body, custom, typedefs) else {
        return;
    };
    if fields.is_empty() {
        return;
    }
    out.push(CStructDef {
        name: name.to_string(),
        fields,
        complete: fields_complete && !has_inheritance,
    });
}

/// 構造体本体のフィールド宣言をパースする。`float x, y, z;` → 3 フィールド。
///
/// 戻り値:
/// - `None` — 本体に `virtual` / `friend` が含まれる（simple class ではない — 構造体自体を除外）
/// - `Some((fields, complete))` — `complete` は全レイアウトメンバをフィールドとして
///   取り込めたとき `true`（配列・ビットフィールド・ネスト構造体・未解決型を
///   スキップした場合は `false` — raw レイアウトは付与できない）
pub(crate) fn parse_struct_field_decls(
    body: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Option<(Vec<(String, CType)>, bool)> {
    let mut fields = Vec::new();
    let mut complete = true;
    let mut i = 0;
    let mut seg_start = 0;

    while i < body.len() {
        if body[i..].starts_with('{') {
            if let Some(end) = find_matching_brace(&body[i..]) {
                // ネストした型定義（enum / struct / union / メソッド定義本体）。
                // レイアウトメンバではないためスキップ（complete は維持）。
                i += end + 1;
                seg_start = i;
                continue;
            }
        }
        if body[i..].starts_with(';') {
            let seg = body[seg_start..i].trim();
            if !seg.is_empty() {
                match classify_member_segment(seg) {
                    MemberKind::Reject => return None, // virtual / friend
                    MemberKind::Ignore => {}           // メソッド・static・型定義等（レイアウト非寄与）
                    MemberKind::Bitfield => complete = false,
                    MemberKind::Field => {
                        let before = fields.len();
                        parse_field_segment(seg, custom, typedefs, &mut fields);
                        if fields.len() == before {
                            // フィールドのはずがパースできなかった（配列・未解決型など）
                            complete = false;
                        }
                    }
                }
            }
            i += 1;
            seg_start = i;
            continue;
        }
        i += 1;
    }
    Some((fields, complete))
}

/// 構造体本体の1メンバセグメントの種別。
pub(crate) enum MemberKind {
    /// virtual（vtable ポインタでレイアウトが変わる）/ friend — simple class ではない
    Reject,
    /// レイアウトに寄与しないメンバ（メソッド宣言・static・ネスト型定義・using 等）
    Ignore,
    /// ビットフィールド — レイアウト計算不能（complete=false）
    Bitfield,
    /// データフィールド候補
    Field,
}

/// メンバセグメントを分類する。先頭のアクセス指定子（`public:` 等）は除いて判定する。
pub(crate) fn classify_member_segment(seg: &str) -> MemberKind {
    let words: Vec<&str> = seg.split_whitespace().collect();
    // 先頭のアクセス指定子ラベルを除去
    let start = if words
        .first()
        .map(|w| matches!(w.trim_end_matches(':'), "public" | "private" | "protected"))
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let words = &words[start..];
    let Some(&first) = words.first() else {
        return MemberKind::Ignore;
    };
    // virtual メンバ関数 → vtable が挿入されるため simple class ではない
    if first == "virtual" || words.contains(&"virtual") {
        return MemberKind::Reject;
    }
    if first == "friend" {
        return MemberKind::Reject;
    }
    // レイアウトに寄与しないメンバ
    if matches!(first, "static" | "typedef" | "using" | "enum" | "struct" | "class" | "union") {
        return MemberKind::Ignore;
    }
    // メソッド宣言（括弧を含む）
    if seg.contains('(') {
        return MemberKind::Ignore;
    }
    // ビットフィールド: `int flags : 3`
    // （先頭のアクセス指定子 `public:` 等は除去済みの words で判定する）
    if words.iter().any(|w| w.contains(':')) {
        return MemberKind::Bitfield;
    }
    MemberKind::Field
}

/// `;` 区切りの 1 フィールドセグメント（`float x, y, z` や `int flags` など）をパースし、解決済みの `(name, CType)` ペアを `out` に追加する。
pub(crate) fn parse_field_segment(
    seg: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
    out: &mut Vec<(String, CType)>,
) {
    // Skip constructor / method declarations (contain parentheses).
    if seg.contains('(') {
        return;
    }
    let all_words: Vec<&str> = seg.split_whitespace().collect();
    // Strip a leading access-control specifier ("public:" / "private:" / "protected:").
    let start = if all_words
        .first()
        .map(|w| matches!(w.trim_end_matches(':'), "public" | "private" | "protected" | "virtual"))
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let words = &all_words[start..];
    if words.len() < 2 {
        return;
    }

    // Split by comma: `float x, y, z` → `["float x", " y", " z"]`
    let seg = words.join(" ");
    let parts: Vec<&str> = seg.split(',').collect();
    let first_words: Vec<&str> = parts[0].split_whitespace().collect();
    if first_words.len() < 2 {
        return;
    }

    // Last word of the first part is the field name (possibly prefixed with `*`)
    let raw_last = *first_words.last().unwrap();
    // Skip array fields like `m[4][4]`
    if raw_last.contains('[') || raw_last.contains('(') {
        return;
    }
    let (first_name, first_stars) = extract_alias_token(raw_last);
    if first_name.is_empty() {
        return;
    }

    let type_words: Vec<&str> = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w != "*")
        .copied()
        .collect();
    let standalone_stars = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w == "*")
        .count();
    let base_str = type_words.join(" ");
    let total_stars = first_stars + standalone_stars;
    let type_str = if total_stars > 0 {
        format!("{}{}", base_str, "*".repeat(total_stars))
    } else {
        base_str.clone()
    };

    match parse_c_type_str(&type_str, custom, typedefs) {
        Ok(ctype) => {
            out.push((first_name.to_string(), ctype.clone()));
            for part in &parts[1..] {
                let pw: Vec<&str> = part.split_whitespace().collect();
                let extra_stars = pw.iter().filter(|&&w| w == "*").count();
                let raw_name = match pw.iter().find(|&&w| w != "*") {
                    Some(w) => w,
                    None => continue,
                };
                if raw_name.contains('[') || raw_name.contains('(') {
                    continue;
                }
                let (alias, alias_stars) = extract_alias_token(raw_name);
                if alias.is_empty() {
                    continue;
                }
                let stars = alias_stars + extra_stars;
                if stars == 0 {
                    out.push((alias.to_string(), ctype.clone()));
                } else if let Ok(ptr_ct) = parse_c_type_str(
                    &format!("{}{}", base_str, "*".repeat(stars)),
                    custom,
                    typedefs,
                ) {
                    out.push((alias.to_string(), ptr_ct));
                }
            }
        }
        Err(_) => {} // skip fields with unparseable types
    }
}

