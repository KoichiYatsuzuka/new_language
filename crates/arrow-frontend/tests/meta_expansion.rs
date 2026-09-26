//! エディタもメタ関数を展開してから型検査する（設計書 D5・タスク 5-0）ことの回帰テスト。
//!
//! # ⚠ ここが守る性質
//!
//! 1. **置いたコードの誤りがエディタにも出る** — 以前のエディタは展開しなかったので、
//!    メタ関数が置いたコードの型エラーは CLI にだけ出た。
//! 2. **展開に失敗したら `MetaError` だけを出す** — CLI は展開の失敗で止まり型検査へ進まない。
//!    展開前の AST の型エラーを出すと、CLI に無い誤りをエディタだけが出す
//!    （`compare_wasm_frontend.ps1` の INVENTED）。
//! 3. **終わらないメタ関数で解析が固まらない** — 展開中の VM は命令数に上限がある。
//!
//! ⚠ このクレートはワークスペースから `exclude` されている。**ルートの `cargo test` では
//!    走らない。** `cd crates/arrow-frontend && cargo test` で実行すること。

use arrow_frontend::analyze::analyze_json;
use serde_json::Value;

fn diagnostics(source: &str) -> Vec<(String, String, Value)> {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("analyze_json must return valid JSON");
    assert_eq!(
        v["ok"],
        Value::Bool(true),
        "source failed to parse: {}",
        v["parseError"]
    );
    v["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .map(|d| {
            (
                d["source"].as_str().unwrap_or_default().to_string(),
                d["message"].as_str().unwrap_or_default().to_string(),
                d["at"].clone(),
            )
        })
        .collect()
}

#[test]
fn an_error_in_placed_code_is_reported() {
    let d = diagnostics(concat!(
        "exprconst !fn with_int() -> None:\n",
        "    quote code:\n",
        "        let n: int = \"not an int\"\n",
        "\n",
        "with_int()\n",
    ));
    assert!(
        d.iter().any(|(src, msg, _)| src == "StaticTypeError"
            && msg.contains("'n' is declared 'int' but initialized with 'str'")),
        "{d:?}"
    );
}

#[test]
fn a_failed_expansion_reports_only_the_meta_error_at_the_call() {
    let d = diagnostics(concat!(
        "exprconst !fn broken() -> None:\n",
        "    compile_error(\"nope\")\n",
        "\n",
        "let bad: int = \"s\"\n",
        "broken()\n",
    ));
    assert_eq!(d.len(), 1, "{d:?}");
    let (src, msg, at) = &d[0];
    assert_eq!(src, "MetaError");
    assert!(msg.contains("nope"), "{msg}");
    // 位置は `broken()` の呼び出し（5 行目・VS Code の 0 始まりで 4）。
    assert_eq!(at["line"], Value::from(4), "{at}");
}

#[test]
fn an_endless_metafunction_does_not_hang_the_editor() {
    let d = diagnostics(concat!(
        "exprconst !fn spin() -> None:\n",
        "    mut i = 0\n",
        "    while True:\n",
        "        i += 1\n",
        "    quote code:\n",
        "        print(1)\n",
        "\n",
        "spin()\n",
    ));
    assert!(
        d.iter()
            .any(|(src, msg, _)| src == "MetaError" && msg.contains("did not finish within")),
        "{d:?}"
    );
}
