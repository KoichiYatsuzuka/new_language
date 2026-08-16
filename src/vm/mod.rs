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

/// VM 実行モード（CLI `--vm=off|on`。`auto`/`force` は `on` の別名）。
///
/// **#3（強制バイトコード・D2）でデュアルモードを畳んだ**。以前は 3 値で、`Auto` が
/// 「コンパイルできなければ黙ってツリーウォークへ落ちる」フォールバックを持っていた。
/// 今は **`On` なら必ず VM で実行し、載せられなければ `VmForceError` で止まる**
/// （以前の `Force` の挙動）。`auto` / `force` は既存スクリプト互換のため受け付ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VmMode {
    /// VM を使わず常にツリーウォーク。
    ///
    /// ⚠ **これは「検証用の出口」であって通常実行の経路ではない**。
    /// 残してある理由は 2 つ:
    /// - [compare_vm_modes.ps1] の off/auto byte-identical 検査が本系列唯一の差分検出網であること
    /// - 退行を疑ったとき「`--vm=off` でも同じ差が出るか」で VM 経路由来かを切り分けられること
    ///   （計画書の判定基準。#10-b・#27・#27-d 段階 2b で実際にこれで誤帰属を防いだ）
    ///
    /// ⇒ ツリーウォークの制御フロー実装と TLS/センチネルは**この経路のために残っている**。
    /// 実測では通常実行（`On`）で 1 度も通らない（`AR_TW_STATS` の `tw_control_flow` が 0）。
    ///
    /// ⚠⚠ **これが `Default`** なのは意図的（#3）。`On` は
    /// 「リゾルバ・型注釈・`toplevel_globals` が揃っている」ことを前提に**載せられなければ
    /// 止まる**ので、**パイプラインを通さない文脈で既定にしてはいけない**。
    /// 実際 `Interpreter::new()` を直接使う **REPL と単体テスト**は解決情報を持たないため、
    /// 正しいコードでも `VmForceError` になる（実装中に踏んだ）。
    /// ⇒ **`On` にするのは `run_program`（＝ファイル実行）だけ**。§2.3 のツール文脈は従来どおり。
    #[default]
    Off,
    /// **常に VM で実行する**。載せられない構文に来たら `VmForceError` で停止する（#3・D2）。
    ///
    /// ⚠ **前提**: リゾルバ・型注釈・`toplevel_globals` を設定済みであること
    /// （`run_program` がやっている）。`force_gate.ps1` はこの挙動をそのまま測っている。
    On,
}
