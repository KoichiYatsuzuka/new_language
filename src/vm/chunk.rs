// vm/chunk.rs — コンパイル済み関数の実行表現（Phase V, V-A）。

use crate::ast::AttrCache;
use crate::interpreter::Value;
use crate::token::Span;

use super::op::Op;

/// 1 関数分のコンパイル結果。`Rc<Chunk>` で関数ごとにキャッシュされる。
///
/// 注: V-E（デバッガ統合）で行テーブル（op→Span）を追加予定。V-A では未実装
/// （関数内のエラー位置はトレースバックの呼び出し元フレームで代替）。
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
    /// slot → 変数名 のデバッグ名テーブル（V-E。デバッガ REPL 用メタデータ。実行では未使用なので
    /// 現状は保持のみ — デバッガ VM 統合で消費予定）。
    #[allow(dead_code)]
    pub local_names: Vec<String>,
    /// フレームのローカル slot 総数（パラメータ + base ローカル）。
    pub n_locals: usize,
}
