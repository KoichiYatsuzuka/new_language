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
pub use compiler::{
    compile_async_body, compile_debug, compile_fn, compile_toplevel_stmt,
    is_toplevel_compile_target,
};
pub use run::run;

