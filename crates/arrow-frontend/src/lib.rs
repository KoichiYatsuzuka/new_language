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
//! # import 先の読み込み
//!
//! import 先も CLI と**同じ処理**（`src/parser/imports/`）で読む（editor_import_resolution_plan.md 3-2）。
//! 違うのは次の 2 つだけ:
//! - ファイルの読み方: wasm はファイルを持たないので、`src/import_fs.rs` の wasm 版がホスト
//!   （`vscode-extension/src/wasm_host.ts`）に頼む
//! - 実行時のためだけの処理をしない（`import[rs]` の `cargo build`・`.arc` の DLL の登録・`.ars` の書き出し）。
//!   `editor` feature（既定で有効）の `#[cfg]` で分ける
//!
//! 読み込んだモジュールは解析をまたいで保持し、ホストがファイルの変更を知らせたときだけ捨てる
//! （`analyze::invalidate_modules`）。

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
// テンプレートの AST 置換（タスク 2-7）。単相化（2-8）を展開器・型検査から呼ぶため。
#[path = "../../../src/template_subst.rs"]
pub mod template_subst;
// `import[py-int] time` / `math` の同梱型スタブ。`include_str!` なので fs も syscall も
// 使わず、そのまま wasm に載る。`src/py_stubs.rs` 冒頭 doc が「置き場所が `src/` 直下
// なのはこの crate が取り込むため」と書いている、その取り込み。
#[path = "../../../src/py_stubs.rs"]
pub mod py_stubs;
// import の探索規則とモジュールの同一性（相対 import）。パーサと実行時が共有する。
#[path = "../../../src/module_path.rs"]
pub mod module_path;
// import の処理が外界（ファイル・環境変数・Python の場所）に触る唯一の窓口
// （editor_import_resolution_plan.md 2-1）。`module_path` が使う。
#[path = "../../../src/import_fs.rs"]
pub mod import_fs;
// `ar_config.json`（Python の探索先・C# の DLL の置き場など）の読み取り。import の処理が使う。
#[path = "../../../src/ar_config.rs"]
pub mod ar_config;
// コンパイル済みモジュール（`.arc`）の形式の読み取り（埋め込みソースから型を読む・3-2）。
#[path = "../../../src/arc_format.rs"]
pub mod arc_format;
// C/C++ のヘッダから型の情報を読む部分（`import[cpp-*]` の型の出所・2-2）。
#[path = "../../../src/cpp_header/mod.rs"]
pub mod cpp_header;
// Rust crate のソースから型の情報を読む部分（`import[rs]` の型の出所・2-2）。
#[path = "../../../src/rs_crate/mod.rs"]
pub mod rs_crate;
// Python のソース（`.py` / `.pyi`）を Arrow へ写す変換器（`import[py]` / `import[py-int]` の型の出所・2-2）。
#[path = "../../../src/python_converter/mod.rs"]
pub mod python_converter;
#[path = "../../../src/lexer/mod.rs"]
pub mod lexer;
#[path = "../../../src/parser/mod.rs"]
pub mod parser;
#[path = "../../../src/type_check/mod.rs"]
pub mod type_check;
// ── 評価コア（メタ関数の展開時評価に要る）────────────────────────────────
// ⚠ ルート crate を `--no-default-features`（= `native` 無効）でビルドしたときと
//   同じ構成。FFI・Python 相互運用・別プロセスブリッジは同名スタブに差し替わる。
//   この crate は `native` feature を持たないので、常にスタブ側が選ばれる。
//   作業分割は implementation_logs/eval_core_extraction_plan.md を参照。
#[path = "../../../src/vm/mod.rs"]
pub mod vm;
#[path = "../../../src/interpreter.rs"]
pub mod interpreter;
// ⚠ VM が `Op::MakeCode` で参照する（タスク 2-0）ので、こちらにも載せる。
#[path = "../../../src/meta_expand/mod.rs"]
pub mod meta_expand;

// ── このクレート固有のコード ─────────────────────────────────────────────────
pub mod analyze;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
