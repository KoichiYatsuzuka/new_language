//! ホスト供給の型スタブ（`stub_registry`）が `analyze_json` の結果に効くことの回帰テスト。
//!
//! # なぜ JSON の水準で試すのか
//!
//! スタブ機構は **VS Code 拡張との契約**で、拡張が見るのは `exprTypes` / `symbols` /
//! `diagnostics` だけ。パーサの内部で body が埋まっていても、そこへ届いていなければ
//! 利用者にとっては何も直っていない。だから `Stmt` を直接見ずに JSON を検査する。
//!
//! # ⚠ ここが守る 2 つの性質
//!
//! 1. **スタブがあると型が付く** — 無ければ付かない。これが機構の目的。
//! 2. **スタブがあっても診断は増えない**（計画 D-1）。スタブは実モジュールの部分集合で
//!    しかも古くなりうるので、「読めた」と見なして未知型名の検査を走らせると
//!    **CLI が出さないエラーをエディタだけが出す**。`compare_wasm_frontend.ps1` の
//!    「wasm は少なく報告してよいが、多く報告してはならない」を破る形。
//!
//! ⚠ このクレートはワークスペースから `exclude` されている。**ルートの `cargo test` では
//!    走らない。** `cd crates/arrow-frontend && cargo test` で実行すること。
//!
//! ⚠ `stub_registry` は thread_local。テストはスレッドごとに走るので他のテストとは
//!    干渉しないが、**1 つのテスト内では必ず `clear_stubs()` から始める**。

use arrow_frontend::analyze::analyze_json;
use arrow_frontend::parser::stub_registry;
use serde_json::Value;

fn analyze(source: &str) -> Value {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("analyze_json must return valid JSON");
    assert_eq!(
        v["ok"],
        Value::Bool(true),
        "source failed to parse: {}",
        v["parseError"]
    );
    v
}

/// `let <name> = ...` に付いた推論型を `symbols` から引く。
fn inferred_of(v: &Value, name: &str) -> Option<String> {
    v["symbols"]
        .as_array()?
        .iter()
        .find(|s| s["name"] == name)?
        .get("inferred")?
        .as_str()
        .map(str::to_string)
}

fn diagnostic_count(v: &Value) -> usize {
    v["diagnostics"].as_array().map_or(0, Vec::len)
}

const SRC: &str = "import[cpp-dll] SakuraCore.SakuraEdit as sakura\n\
                   \n\
                   fn main() -> None:\n\
                   \x20   let n = sakura.get_line_count(0)\n\
                   \x20   print(n)\n";

/// スタブが無ければ従来どおり型は付かない（そして誤診断も出ない）。
#[test]
fn without_stub_no_type_and_no_diagnostics() {
    stub_registry::clear_stubs();

    let v = analyze(SRC);
    assert_eq!(
        inferred_of(&v, "n").as_deref(),
        None,
        "スタブが無いのに型が付いている — どこかが推測している"
    );
    assert_eq!(diagnostic_count(&v), 0, "スタブ無しで診断が出てはいけない");
}

/// スタブを積むと戻り値型が届く。
#[test]
fn with_stub_return_type_reaches_the_editor() {
    stub_registry::clear_stubs();
    stub_registry::set_stub(
        "cpp-dll:SakuraCore.SakuraEdit".to_string(),
        "fn get_line_count(ed: int) -> int:\n    ...\n".to_string(),
    );

    let v = analyze(SRC);
    assert_eq!(
        inferred_of(&v, "n").as_deref(),
        Some("int"),
        "スタブを積んだのに戻り値型が届いていない"
    );
    assert_eq!(
        diagnostic_count(&v),
        0,
        "スタブを積んだだけで診断が増えた（計画 D-1 違反）"
    );
}

/// **D-1 の要**: スタブが定義していない型名を注釈に書いても、エディタは黙る。
///
/// スタブは実モジュールの部分集合なので、「レジストリに無い＝存在しない」と
/// 断定できない。ここが落ちるときは `has_unloaded_import` が editor で
/// 「読めた」と答えるようになっている（＝CLI が出さないエラーを出し始める）。
#[test]
fn stub_does_not_enable_unknown_type_name_errors() {
    stub_registry::clear_stubs();
    stub_registry::set_stub(
        "cpp-dll:SakuraCore.SakuraEdit".to_string(),
        "fn get_line_count(ed: int) -> int:\n    ...\n".to_string(),
    );

    // `V3` はスタブにもこのファイルにも無い型名。CLI は実ヘッダを読むので知っている。
    let src = "import[cpp-dll] SakuraCore.SakuraEdit as sakura\n\
               \n\
               fn f(v: V3) -> None:\n\
               \x20   print(v)\n";
    let v = analyze(src);
    assert_eq!(
        diagnostic_count(&v),
        0,
        "スタブに無い型名でエディタだけがエラーを出した: {}",
        v["diagnostics"]
    );
}

/// 自分自身を import するスタブでも無限再帰しない（`with_stub` の循環検出）。
#[test]
fn self_referential_stub_terminates() {
    stub_registry::clear_stubs();
    stub_registry::set_stub(
        "ar-auto:loopy".to_string(),
        "import loopy\nfn f() -> int:\n    ...\n".to_string(),
    );

    let v = analyze("import loopy\n\nfn main() -> None:\n    let x = loopy.f()\n    print(x)\n");
    assert_eq!(inferred_of(&v, "x").as_deref(), Some("int"));
}

/// 壊れた `.ars` はエディタごと壊さない（握り潰して空 body に倒れる）。
#[test]
fn broken_stub_falls_back_to_empty_body() {
    stub_registry::clear_stubs();
    stub_registry::set_stub(
        "cpp-dll:SakuraCore.SakuraEdit".to_string(),
        "fn ( this is not arrow ->->\n".to_string(),
    );

    let v = analyze(SRC);
    assert_eq!(inferred_of(&v, "n").as_deref(), None);
    assert_eq!(diagnostic_count(&v), 0, "壊れたスタブで診断が出た");
}
