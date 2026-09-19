//! `analyze_json` が出す `typeRefs` の回帰テスト。
//!
//! # なぜ JSON の水準で試すのか
//!
//! `typeRefs` は **VS Code 拡張との契約**であって、パーサの内部表現ではない。
//! 拡張はこの配列を「名前引きより先に見る唯一の根拠」として使うので、壊れると
//! `let x: int` の `int` が静かにキャスト関数として着色される — テストが無いと
//! 誰も気付かない種類の退行になる。だから `EditorIndex` を直接見るのではなく、
//! 拡張が実際に読む JSON を検査する。
//!
//! ⚠ このクレートはワークスペースから `exclude` されている（ルートの `[profile.release]`
//!    を波及させないため）。**ルートの `cargo test` では走らない。**
//!    `cd crates/arrow-frontend && cargo test` で実行すること。

use arrow_frontend::analyze::analyze_json;
use serde_json::Value;

/// `(line, col, name)` の集合として `typeRefs` を取り出す。位置は 0 始まり。
fn type_refs(source: &str) -> Vec<(u64, u64, String)> {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("analyze_json must return valid JSON");
    assert_eq!(
        v["ok"], Value::Bool(true),
        "source failed to parse: {}", v["parseError"],
    );
    v["typeRefs"]
        .as_array()
        .expect("typeRefs must be an array")
        .iter()
        .map(|r| {
            (
                r["at"]["line"].as_u64().unwrap(),
                r["at"]["col"].as_u64().unwrap(),
                r["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn has(refs: &[(u64, u64, String)], line: u64, col: u64, name: &str) -> bool {
    refs.iter().any(|(l, c, n)| *l == line && *c == col && n == name)
}

/// 型注釈の `int` は拾い、**呼び出しの `int(n)` は拾わない**。
///
/// これが本テストの主眼。両者は同じ綴りの同じ識別子で、区別できるのは
/// 「パーサが型位置として読んだか」だけ。
#[test]
fn annotation_yes_call_no() {
    //                     0123456789012345678
    let src = "let a: int = 5\n\
               fn f(let n: int) -> str:\n\
               \x20   let v: int = int(n)\n\
               \x20   return str(v)\n";
    let refs = type_refs(src);

    // 型注釈位置
    assert!(has(&refs, 0, 7, "int"), "let a: int — {refs:?}");
    assert!(has(&refs, 1, 12, "int"), "fn param annotation — {refs:?}");
    assert!(has(&refs, 1, 20, "str"), "return annotation — {refs:?}");
    assert!(has(&refs, 2, 11, "int"), "let v: int — {refs:?}");

    // 呼び出し位置（キャスト関数としての `int` / `str`）
    assert!(!has(&refs, 2, 17, "int"), "int(n) は型参照ではない — {refs:?}");
    assert!(!has(&refs, 3, 11, "str"), "str(v) は型参照ではない — {refs:?}");
}

/// コレクション型の**内側**も型位置。`list[int]` の `int` を取りこぼさない。
#[test]
fn nested_type_arguments() {
    //                 0123456789012345678901234
    let src = "let xs: list[int] = [1, 2]\n\
               let d: dict[str, int] = {\"a\": 1}\n";
    let refs = type_refs(src);

    assert!(has(&refs, 0, 8, "list"), "list — {refs:?}");
    assert!(has(&refs, 0, 13, "int"), "list[int] の int — {refs:?}");
    assert!(has(&refs, 1, 7, "dict"), "dict — {refs:?}");
    assert!(has(&refs, 1, 12, "str"), "dict の key 型 — {refs:?}");
    assert!(has(&refs, 1, 17, "int"), "dict の value 型 — {refs:?}");
}

/// 型ガード `x is int` の右辺も型位置。式位置と同じ扱いにすると関数色になる。
#[test]
fn type_guard_right_hand_side() {
    let src = "fn g(let x: any) -> bool:\n\
               \x20   if x is int:\n\
               \x20       return True\n\
               \x20   return False\n";
    let refs = type_refs(src);

    assert!(has(&refs, 1, 12, "int"), "x is int の int — {refs:?}");
}

/// **このファイルで宣言されていない名前**の型引数（`Box[int]`）も型位置。
///
/// ⚠⚠ **この形は 2 度壊れている。**
/// - タスク 9.7 より前 … `parse_type_expr` が `[...]` を**読み飛ばして捨てて**いた。
///   それでも読み飛ばしループが `note_type_ref` を呼んでいたので、このテストは通っていた
///   （テスト名 `skipped_…` はその頃の名残）。
/// - タスク 9.7 … 「テンプレート名以外の `[...]` は弾く」にしたため**パースエラー**になり、
///   このテストが落ちた。さらに `from tmod import[ar] Box` + `let b: Box[int]` という
///   **モジュールを跨ぐテンプレート注釈が書けなくなっていた**（パーサは import 先を
///   知らないので「テンプレートでない」と誤判定する）。
/// - 2026-09-20 … 判定を型検査へ移し、パーサは型引数を**保ったまま**返すようにした。
///   `[int]` は `parse_type_expr` で型式としてパースされるので `note_type_ref` も鳴る。
///
/// ⇒ このテストは「パーサが型引数を捨てない／弾かない」ことの網。**消さないこと。**
#[test]
fn generic_arguments_of_unknown_base() {
    //                 0123456789012345678901
    let src = "fn h(let b: Box[int]) -> None:\n\
               \x20   pass\n";
    let refs = type_refs(src);

    assert!(has(&refs, 0, 12, "Box"), "Box — {refs:?}");
    assert!(has(&refs, 0, 16, "int"), "読み飛ばす型引数の int — {refs:?}");
}

/// alias は**使用箇所**を記録し、展開先（定義側）の位置は記録しない。
///
/// 記録してしまうと、使用箇所と無関係な行に型色が付く。import 経由の alias なら
/// 他ファイルの行番号がこのファイルの索引に紛れ込む。
#[test]
fn alias_records_use_site_only() {
    //                 0123456789012345
    let src = "alias Num: int\n\
               let z: Num = 1\n";
    let refs = type_refs(src);

    assert!(has(&refs, 1, 7, "Num"), "使用箇所の Num — {refs:?}");
    assert!(
        !has(&refs, 0, 11, "int"),
        "alias 定義側の int を記録してはいけない — {refs:?}",
    );
}

/// 構文エラーでも `typeRefs` キー自体は必ず存在する（拡張が `?? []` に頼らずに済む）。
#[test]
fn parse_error_still_has_key() {
    let raw = analyze_json("let a: int =\n", "test.ar");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["ok"], Value::Bool(false), "この入力は構文エラーのはず");
    assert!(v["typeRefs"].is_array(), "typeRefs キーが無い: {raw}");
}
