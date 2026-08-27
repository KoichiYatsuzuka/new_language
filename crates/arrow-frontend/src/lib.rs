//! arrow-frontend — Arrow のフロントエンド（字句解析 → 構文解析 → 静的型検査）を
//! エディタ／wasm32 向けに切り出したクレート。
//!
//! # なぜ別クレートなのか
//!
//! ルートパッケージ `arrow` は `pyo3` と `libloading` に依存しており、wasm32 に載らない。
//! しかし調べてみると、その依存は**すべて `src/interpreter/` と `src/partial_compiler/`
//! に閉じている**（`src/lexer` / `src/parser` / `src/type_check` の使用箇所は 0）。
//! そこで「同じソースファイルを `#[path]` で取り込む、依存の軽い別クレート」を用意すれば、
//! **Rust 側のコードを 1 行も複製せずに**フロントエンドだけを wasm 化できる。
//!
//! この構成の要点は、VS Code 拡張が使う解析器が
//! **`cargo run` が使う解析器と同一のソース**であること。TypeScript 側に言語仕様の
//! 判断を一切置かないため、定義上「拡張だけ解釈がずれる」ことが起こらない。
//!
//! # 唯一の差分
//!
//! `editor` feature（既定で有効）により `src/parser/mod.rs` の `#[cfg]` が
//! import 解析を [`parser::imports_editor`] へ差し替える。エディタでは
//! import 先を実際に読み込まない（fs・プロセス・DLL に触れない）。
//! 詳細と代償は `src/parser/imports_editor.rs` の doc に書いてある。

// ── ルート crate と共有するソース ────────────────────────────────────────────
// パスは**このファイルからの相対**。実体はリポジトリ直下の `src/`。
#[path = "../../../src/ast.rs"]
pub mod ast;
#[path = "../../../src/token.rs"]
pub mod token;
#[path = "../../../src/decl_names.rs"]
pub mod decl_names;
#[path = "../../../src/expr_walk.rs"]
pub mod expr_walk;
#[path = "../../../src/stmt_walk.rs"]
pub mod stmt_walk;
#[path = "../../../src/lexer/mod.rs"]
pub mod lexer;
#[path = "../../../src/parser/mod.rs"]
pub mod parser;
#[path = "../../../src/type_check/mod.rs"]
pub mod type_check;

// ── このクレート固有のコード ─────────────────────────────────────────────────
pub mod analyze;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
