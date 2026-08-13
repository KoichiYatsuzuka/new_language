// vm/mod.rs — バイトコード VM（Phase V）。公開 API: compile_fn / run。
//
// 解釈経路の高速実行形。Phase R で解決済みの AST（`Resolution::Local` 等）を入力に、
// トップレベル関数を Chunk へコンパイルして専用スタックマシンで実行する。
// 非対応構文はコンパイル時に弾かれ、呼び出し側がツリーウォークにフォールバックする
// （デュアルモード, D2）。VM 実行モードは `Interpreter::vm_mode`（Off/Auto/Force）で制御する。

pub mod chunk;
pub mod compiler;
pub mod disasm;
pub mod op;
pub mod peephole;
pub mod run;

pub use chunk::Chunk;
pub use compiler::{compile_debug, compile_fn, compile_toplevel_stmt};
pub use run::run;

/// VM 実行モード（CLI `--vm=off|auto|force`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VmMode {
    /// VM を使わず常にツリーウォーク。
    Off,
    /// コンパイルできた関数のみ VM、できなければツリーウォーク（既定）。
    #[default]
    Auto,
    /// コンパイル対象（トップレベル・リーフ関数）で失敗したら穴として可視化する。
    /// V-A では Auto と同じ挙動（フォールバック）だが、将来ここで失敗を報告する。
    Force,
}
