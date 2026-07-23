// vm/op.rs — バイトコード VM のオペコード定義（Phase V, V-A）。
//
// スタックマシン。命令は `Vec<Op>`（chunk.code）に線形に並び、`vm/run.rs` の
// ディスパッチループが `ip`（命令ポインタ）を進めながら実行する。
// ジャンプ先は絶対 index（code 配列内の位置）で持つ（コンパイル時にバックパッチ）。

use crate::ast::{BinOp, UnaryOp};

/// VM オペコード（V-A の最小セット）。
#[derive(Debug, Clone)]
pub enum Op {
    /// consts[idx] を push する。
    Const(u32),
    /// `None` を push する。
    Nil,
    /// locals[slot] を push する（LocalRef / パラメータ / base ローカル読み）。
    LoadLocal(u16),
    /// pop して locals[slot] へ書き込む（let / mut / 代入）。
    StoreLocal(u16),
    /// スタックトップを1つ捨てる。
    Pop,
    /// 二項演算: pop b, pop a, push apply_binop_dyn(op, a, b)。
    Bin(BinOp),
    /// 単項演算: pop a, push apply_unary_dyn(op, a)。
    Un(UnaryOp),
    /// 属性（フィールド）読み: pop obj, push get_attr_val(obj, names[name_idx], attr_caches[cache_idx])。
    GetAttr(u32, u32),
    /// 無条件ジャンプ（絶対 index）。
    Jump(u32),
    /// pop した値が偽ならジャンプ（if/while の条件分岐）。
    JumpIfFalse(u32),
    /// スタックトップが偽ならジャンプ（値を残す）、真なら pop して継続（`and` 短絡）。
    JumpIfFalseOrPop(u32),
    /// スタックトップが真ならジャンプ（値を残す）、偽なら pop して継続（`or` 短絡）。
    JumpIfTrueOrPop(u32),
    /// スタックトップを関数戻り値として返す。
    Return,
    /// `None` を関数戻り値として返す（本体末尾のフォールオフ）。
    ReturnNil,
}
