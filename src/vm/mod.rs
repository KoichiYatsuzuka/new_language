// vm/mod.rs — バイトコード VM（Phase V）。公開 API: compile_* / run。
//
// **解釈経路の唯一の実行形**（D2）。Phase R で解決済みの AST（`Resolution::Local` 等）を入力に
// Chunk へコンパイルし、専用スタックマシンで実行する。入口は 6 つ:
// 関数本体（`compile_fn`）／最上位の 1 文（`compile_toplevel_stmt`）／import モジュール本体の
// 1 文（`compile_module_stmt`・#42）／定義文脈の式（`compile_definition_expr`・#41）／
// async 本体（`compile_async_body`）／デバッガ REPL の 1 文（`compile_debug`）。
//
// ⚠ **フォールバックは存在しない**（#3 → #33 で `VmMode`・`--vm`・ツリーウォークの制御フローを
// すべて削除した）。載せられない構文に出会ったらコンパイラは `None` を返し、呼び出し側は
// **`VmForceError` で停止する**。ツリーウォーク（`exec()`）が実行するのは**定義文だけ**（#10-d）。
//
// ⚠ VM は「解決情報が揃っている」前提（#3/#36）。`resolve_program` ＋ `check_and_annotate` ＋
// `set_toplevel_globals` を供給しない入口では、正しいコードでも `VmForceError` になる。
// **入口ごとに配線する責任がある**（`run_program`・REPL・テストヘルパー・モジュール本体）。

pub mod chunk;
pub mod compiler;
pub mod disasm;
pub mod op;
/// op → 宣言順インデックス / 名前表（`--features prof` 専用・自動生成）。
#[cfg(feature = "prof")]
pub mod op_prof;
pub mod peephole;
pub mod run;

pub use chunk::Chunk;
pub use compiler::{
    compile_async_body, compile_debug, compile_definition_expr, compile_fn, compile_module_stmt, compile_toplevel_stmt,
    is_toplevel_compile_target,
};
pub use run::run;

