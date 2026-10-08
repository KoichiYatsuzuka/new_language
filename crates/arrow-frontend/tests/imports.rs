//! 拡張が import 先を CLI と同じ処理（`src/parser/imports/`）で読むことの回帰テスト
//! （editor_import_resolution_plan.md 3-2）。
//!
//! # なぜ JSON の水準で試すのか
//!
//! 拡張が見るのは `exprTypes` / `symbols` / `diagnostics` だけ。パーサの内部で body が埋まっていても、
//! そこへ届いていなければ利用者にとっては何も直っていない。だから `Stmt` を直接見ずに JSON を検査する。
//!
//! # ここが守る性質
//!
//! 1. import 先のモジュールの型が届く（以前は拡張だけ import 先を読まず、型が付かなかった）
//! 2. 読めない import は**誤り**として出し、ファイルの残りの解析は続ける（黙って型を落とさない・D-4）
//! 3. 読んだモジュールは解析をまたいで保持し、`invalidate_modules` までは読み直さない（D-2）
//!
//! ⚠ このクレートはワークスペースから `exclude` されている。**ルートの `cargo test` では
//!    走らない。** `cd crates/arrow-frontend && cargo test` で実行すること
//!    （`compare_wasm_frontend.ps1` が前段で走らせる）。
//!
//! ⚠ ネイティブでビルドしたテストは `import_fs` の CLI 版（`std::fs`）でファイルを読む。wasm 版
//!   （ホストの関数）の経路は `compare_wasm_frontend.ps1` が拡張と同じホストで確かめる。
//! ⚠ 保持しているモジュールは thread_local。テストはスレッドごとに走るので互いに干渉しないが、
//!   各テストは自分専用のディレクトリを使う（同じディレクトリだと保持したものを共有しうる）。

use std::path::PathBuf;

use arrow_frontend::analyze::{analyze_file_json, invalidate_modules};
use serde_json::Value;

/// テスト専用のディレクトリを作り、`files` を書く。
fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("arrow_frontend_imports_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, src) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    dir
}

fn analyze_at(dir: &PathBuf, main: &str) -> Value {
    let path = dir.join(main);
    let source = std::fs::read_to_string(&path).unwrap();
    let raw = analyze_file_json(&source, &path.to_string_lossy());
    let v: Value = serde_json::from_str(&raw).expect("analyze_file_json must return valid JSON");
    assert_eq!(v["ok"], Value::Bool(true), "failed to parse: {}", v["parseError"]);
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

fn diagnostics(v: &Value) -> Vec<(String, Option<(u64, u64)>, String)> {
    v["diagnostics"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| {
                    let at = d["at"].as_object().map(|p| {
                        (p["line"].as_u64().unwrap() + 1, p["col"].as_u64().unwrap() + 1)
                    });
                    (d["source"].as_str().unwrap_or("").to_string(), at, d["message"].as_str().unwrap_or("").to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// import 先の関数の戻り値型が届き、そのモジュールに無いメンバーは CLI と同じく誤りになる。
#[test]
fn module_types_reach_the_editor() {
    let dir = project("types", &[
        ("util.ar", "fn count(n: int) -> int:\n    return n + 1\n"),
        ("main.ar", "import util\n\nlet n = util.count(1)\nlet s: str = util.count(2)\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(inferred_of(&v, "n").as_deref(), Some("int"), "import 先の戻り値型が届いていない");
    let d = diagnostics(&v);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].1, Some((4, 1)), "型の誤り（str に int）がこのファイルの位置に出ていない: {d:?}");
}

/// 読めない import は `ParseError` の診断（CLI が止まる位置）になり、ファイルの残りは解析される。
#[test]
fn unresolved_import_is_reported_and_the_rest_is_analysed() {
    let dir = project("unresolved", &[
        ("main.ar", "import no_such_module_xyz\n\nlet n: int = \"a\"\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    let d = diagnostics(&v);
    assert!(
        d.iter().any(|(src, at, msg)| src == "ParseError" && *at == Some((1, 26)) && msg.contains("no_such_module_xyz")),
        "未解決の import が誤りとして出ていない（黙って型を落としている）: {d:?}"
    );
    assert!(
        d.iter().any(|(src, at, _)| src == "StaticTypeError" && *at == Some((3, 1))),
        "未解決の import の後ろの行が解析されていない: {d:?}"
    );
}

/// import 先の中の未解決の import も、このファイルの import 文の誤りとして出る（import 先の中で
/// 黙って空にしない）。
#[test]
fn unresolved_import_inside_a_module_is_not_swallowed() {
    let dir = project("nested", &[
        ("mid.ar", "import no_such_inner_xyz\n\nfn f() -> int:\n    return 1\n"),
        ("main.ar", "import mid\n\nlet n = mid.f()\n"),
    ]);
    let v = analyze_at(&dir, "main.ar");
    let d = diagnostics(&v);
    assert!(
        d.iter().any(|(src, at, msg)| src == "ParseError" && *at == Some((1, 11)) && msg.contains("no_such_inner_xyz")),
        "import 先の中の未解決の import が消えた: {d:?}"
    );
}

/// 読んだモジュールは保持し、`invalidate_modules` までは読み直さない（D-2）。捨てたら読み直す。
#[test]
fn modules_are_kept_until_invalidated() {
    let dir = project("cache", &[
        ("util.ar", "fn value() -> int:\n    return 1\n"),
        ("main.ar", "import util\n\nlet v = util.value()\n"),
    ]);
    invalidate_modules();
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(inferred_of(&v, "v").as_deref(), Some("int"));

    // 保持している間は、ファイルが変わっても前の型のまま（打鍵ごとには読み直さない）。
    std::fs::write(dir.join("util.ar"), "fn value() -> str:\n    return \"a\"\n").unwrap();
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(inferred_of(&v, "v").as_deref(), Some("int"), "保持しているはずのモジュールを読み直した");

    // ホストが変更を知らせたら読み直す。
    invalidate_modules();
    let v = analyze_at(&dir, "main.ar");
    assert_eq!(inferred_of(&v, "v").as_deref(), Some("str"), "捨てた後に読み直していない");
}

/// 保持したモジュールを使う 2 回目の解析でも、このファイルの式の型が正しい
/// （node-id が保持したモジュールのものと衝突しない・`Parser::reuse_modules`）。
#[test]
fn reused_modules_do_not_collide_with_this_files_node_ids() {
    let dir = project("node_ids", &[
        ("util.ar", "fn s() -> str:\n    return \"a\"\nfn f() -> float:\n    return 1.0\n"),
        ("main.ar", "import util\n\nlet a = 1\nlet b = true\nlet c = util.s()\n"),
    ]);
    invalidate_modules();
    let first = analyze_at(&dir, "main.ar");
    let second = analyze_at(&dir, "main.ar");
    for name in ["a", "b", "c"] {
        assert_eq!(inferred_of(&first, name), inferred_of(&second, name), "{name}");
    }
    assert_eq!(first["exprTypes"].as_array().map(Vec::len), second["exprTypes"].as_array().map(Vec::len));
    let mut t1: Vec<String> = first["exprTypes"].as_array().unwrap().iter().map(|e| e.to_string()).collect();
    let mut t2: Vec<String> = second["exprTypes"].as_array().unwrap().iter().map(|e| e.to_string()).collect();
    t1.sort();
    t2.sort();
    assert_eq!(t1, t2, "保持したモジュールを使った解析で式の型が変わった");
}
