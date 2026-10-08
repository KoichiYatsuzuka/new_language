//! 組み込み関数の宣言（`src/built_in_stab/builtins.ars`）がエディタに届くことの回帰テスト。
//!
//! # ⚠ ここが守る性質
//!
//! 1. **組み込みの呼び出しの結果に型が付く** — `let f = open(..)` の `f` は `FileObject`。
//!    以前は型検査器が `range` / `len` 以外の組み込みの型を知らず、`Unresolved` だった。
//! 2. **拡張が受け取る宣言の本文が解析できる** — 拡張は wasm の `ar_builtins` から
//!    `type_check::builtins::SOURCE` を受け取り、`analyze_json` で hover・補完用の表にする。
//!    以前の拡張用 `builtins.ars` は `code`（2026-09-22 から予約語）を仮引数名に使っていて
//!    丸ごと読めず、組み込みの hover・補完が消えていたのに誰も気づかなかった。
//!
//! ⚠ このクレートはワークスペースから `exclude` されている。**ルートの `cargo test` では
//!    走らない。** `cd crates/arrow-frontend && cargo test` で実行すること。

use arrow_frontend::analyze::analyze_json;
use serde_json::Value;

fn analyze(source: &str) -> Value {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("analyze_json must return valid JSON");
    assert_eq!(v["ok"], Value::Bool(true), "source failed to parse: {}", v["parseError"]);
    v
}

fn inferred_of(v: &Value, name: &str) -> Option<String> {
    v["symbols"]
        .as_array()?
        .iter()
        .find(|s| s["name"] == name)?
        .get("inferred")?
        .as_str()
        .map(str::to_string)
}

#[test]
fn builtin_call_results_have_declared_types() {
    let v = analyze(concat!(
        "let f = open(\"a.txt\", FileOpenMode.read)\n",
        "let r = repr(1)\n",
        "let e = getenv(\"HOME\")\n",
        "close(f)\n",
    ));
    assert_eq!(inferred_of(&v, "f").as_deref(), Some("FileObject"));
    assert_eq!(inferred_of(&v, "r").as_deref(), Some("str"));
    assert_eq!(inferred_of(&v, "e").as_deref(), Some("str"));
}

#[test]
fn builtins_source_is_analyzable_for_the_prelude() {
    let v = analyze(arrow_frontend::type_check::builtins::SOURCE);
    // 拡張は最上位（スコープ 0）の宣言を組み込みとして取り込む（`wasm_providers.ts` の `loadPrelude`）。
    let names: Vec<&str> = v["symbols"]
        .as_array()
        .expect("symbols")
        .iter()
        .filter(|s| s["scope"] == 0 && s["kind"] == "function")
        .filter_map(|s| s["name"].as_str())
        .collect();
    for n in ["print", "len", "range", "open", "close", "repr", "getenv"] {
        assert!(names.contains(&n), "prelude に '{n}' が無い: {names:?}");
    }
    // 実行時に無い関数を並べていないこと（全体の突き合わせは本体側の `builtins_ars_matches_the_runtime`）。
    for n in ["ord", "abs", "isinstance", "exec"] {
        assert!(!names.contains(&n), "実行時に無い '{n}' が prelude に残っている");
    }
}

/// 組み込みの型（`enum` / `class`）のメンバが拡張の `.` 補完・hover 用の表（`members`）に載る。
/// 以前は定義が無く、`f.` にも `FileOpenMode.` にも何も出なかった。
#[test]
fn builtin_types_reach_the_member_table() {
    let v = analyze(arrow_frontend::type_check::builtins::SOURCE);
    let names = |ty: &str| -> Vec<String> {
        v["members"][ty]["members"]
            .as_array()
            .unwrap_or_else(|| panic!("members に {ty} が無い"))
            .iter()
            .filter_map(|m| m["name"].as_str().map(str::to_string))
            .collect()
    };
    let file = names("FileObject");
    for m in ["read", "read_line", "read_letter", "write", "write_line"] {
        assert!(file.iter().any(|n| n == m), "FileObject.{m} が無い: {file:?}");
    }
    let mode = names("FileOpenMode");
    for m in ["write", "rewrite", "read", "make_and_write"] {
        assert!(mode.iter().any(|n| n == m), "FileOpenMode.{m} が無い: {mode:?}");
    }
}

/// 組み込みの型のメソッドの結果の型も束縛の型として届く（`write` → `None`）。
#[test]
fn builtin_method_results_have_declared_types() {
    let v = analyze(concat!(
        "let f = open(\"a.txt\", FileOpenMode.rewrite)\n",
        "let n = f.write(\"x\")\n",
        "let m: FileOpenMode = FileOpenMode.read\n",
        "close(f)\n",
    ));
    assert_eq!(inferred_of(&v, "n").as_deref(), Some("None"));
}
