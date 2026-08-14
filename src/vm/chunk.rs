// vm/chunk.rs — コンパイル済み関数の実行表現（Phase V, V-A）。

use crate::ast::AttrCache;
use crate::ast::Stmt;
use crate::interpreter::Value;
use crate::token::Span;

use super::op::Op;

/// `target <- async->T: body` の VM 表現（タスク #9）。`AsyncSubmit(idx)` op が参照する。
/// `body` は非同期タスクの AST（別スレッドでツリーウォーク実行される）。`captures` は本体が参照する
/// enclosing フレームの `(変数名, slot, is_mutable)`（実行時に frame から値を読んで env を組む）。
/// 入れ子 `fn` 定義 1 件ぶんのデータ（#27・`Op::MakeFn`）。
#[derive(Debug, Clone)]
pub struct ChunkFnDef {
    pub name: String,
    pub params: Vec<crate::ast::Param>,
    pub body: Vec<Stmt>,
    pub return_type: Option<String>,
    /// 生成した関数値を書き込む slot（リゾルバの base slot と同じ番号）。
    pub slot: u16,
    /// キャプチャする外側ローカル（名前, slot）。**不変な変数だけ**（#27）。
    ///
    /// 可変変数のキャプチャはツリーウォークだと `Var::Mutable` → `Var::Cell` へ昇格して
    /// **外側と同じ `Rc<RefCell<Value>>` を共有**する。VM のフラット slot は `Value` 直値なので
    /// 共有セルを表現できず、コンパイラが bail する（対応にはフレーム表現の変更が要る）。
    pub captures: Vec<(String, u16)>,
}

pub struct AsyncBlock {
    pub body: Vec<Stmt>,
    pub captures: Vec<(String, u16, bool)>,
}

/// 文境界の行テーブル（`Chunk::stmt_spans`）の番兵。
///
/// - `NOT_STMT`: その ip は**文の先頭ではない**（大多数の op）。
/// - `STMT_NO_SPAN`: 文の先頭だが、その文は位置情報を持たない
///   （`stmt_location` が `None` を返す種類 — `if`/`while`/`return` 等）。
///   ツリーウォークの `best_span_for` はこの場合 `dbg_last_span` へフォールバックするので、
///   VM も同じ扱いにする（そうしないと transcript が食い違う）。
pub const NOT_STMT: u32 = u32::MAX;
/// 文の先頭だが位置情報なし（`best_span_for` のフォールバックに委ねる）。
pub const STMT_NO_SPAN: u32 = u32::MAX - 1;

/// 1 関数分のコンパイル結果。`Rc<Chunk>` で関数ごとにキャッシュされる。
pub struct Chunk {
    /// 命令列。
    pub code: Vec<Op>,
    /// 定数プール（リテラル値）。
    pub consts: Vec<Value>,
    /// 属性名プール（`GetAttr` が参照）。
    pub names: Vec<String>,
    /// 属性アクセスのインラインキャッシュ（R3 と同じ機構を VM 内でも使う）。
    pub attr_caches: Vec<AttrCache>,
    /// 位置情報プール（`Raise`/`Call`/`CallMethod` が参照。例外・トレースバックの file/line/col）。
    pub spans: Vec<Span>,
    /// **文境界の行テーブル**（#1）。`code` と同じ長さで、`stmt_spans[ip]` は
    /// `NOT_STMT` / `STMT_NO_SPAN` / `spans` への index のいずれか。
    ///
    /// デバッガの文単位ブレークが「この ip は文の先頭か・その位置はどこか」を O(1) で引くために使う。
    /// **通常の実行ループは一切参照しない**（デバッグセッション中の専用ループだけが読む）。
    ///
    /// ⚠ 命令を削除・移動する最適化（[peephole](super::peephole)）は**この表も同時に詰め直す**こと。
    /// ずれると停止位置が別の文にずれる。
    pub stmt_spans: Vec<u32>,
    /// slot → 変数名 のデバッグ名テーブル（V-E。デバッガ REPL 用メタデータ。実行では未使用なので
    /// 現状は保持のみ — デバッガ VM 統合で消費予定）。
    #[allow(dead_code)]
    pub local_names: Vec<String>,
    /// フレームのローカル slot 総数（パラメータ + base ローカル）。
    pub n_locals: usize,
    /// 先頭から何 slot がパラメータか（`self` を含む）。
    /// デバッガが「停止時点で**まだ宣言されていない**ローカル」を隠すのに使う（#1）:
    /// パラメータは入口で束縛済みなので最初から可視、それ以外は Store されるまで不可視。
    pub n_params: usize,
    /// 非同期タスクブロック（`AsyncSubmit(idx)` が参照, タスク #9）。async を含まない関数では空。
    pub async_blocks: Vec<AsyncBlock>,
    /// `LoadGlobal` のグローバル索引キャッシュ（#11）。`(slot_epoch, scopes[0] index)` を焼く。
    pub global_caches: Vec<crate::ast::SlotCache>,
    /// メソッド呼び出しの FFI 境界検査用の表示情報（#27-b）。node_id → (表示名 index, span index)。
    ///
    /// **エラーメッセージのためだけ**に要る（`check_ffi_return` の `callee_name` / `call_span`）。
    /// op のオペランドに足すと `Op` のサイズが `Call` を超えて全命令が太るので、
    /// **参照されるのが外部言語レシーバのときだけ**という性質を使って副表に逃がしてある。
    /// これが無いと `L.get_int` が `get_int`、位置が `<unknown>` になり off/auto が食い違う。
    pub ffi_call_info: std::collections::HashMap<u32, (u32, u32)>,
    /// 入れ子 `fn` 定義（#27）。`Op::MakeFn(idx)` が index で参照する。
    ///
    /// ⚠ **キャプチャを持たない入れ子 `fn` に限る**。コンパイラが
    /// 「自由変数 ∩ 外側の slot = ∅」を確かめてから積むので、生成する `FnValue` の
    /// `captured_env` は**構成上空**になり、ツリーウォークの `capture_env`（外側スコープを
    /// 走査して何も見つからない）と一致する。
    pub fn_defs: Vec<ChunkFnDef>,
}
