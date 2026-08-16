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

/// VM 実行モード（CLI `--vm=off|auto|force`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VmMode {
    /// VM を使わず常にツリーウォーク。
    Off,
    /// コンパイルできた関数のみ VM、できなければツリーウォーク（既定）。
    #[default]
    Auto,
    /// **フォールバック禁止**（#25）。VM に載せられない箇所に来たら `VmForceError` で止める。
    ///
    /// #3（強制バイトコード）へ進めるかを判定する**唯一のゲート**。
    /// `AR_TW_STATS` は件数を数えるだけで失敗させないので、「本当に 0 件か」は確かめられない。
    ///
    /// ⚠ **定義文（`fn`/`class`/`import` 等）は対象外**。あれらは制御フローを持たず
    /// TLS も使わないので、設計上インタプリタが実行する（`parse_ar` と同じ扱い）。
    /// 対象に含めると永久に 0 件にならない（判断の根拠は #10-d・実装ログ）。
    Force,
}
