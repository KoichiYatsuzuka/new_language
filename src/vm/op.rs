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
    /// グローバル `names[name_idx]` の値を push する（呼び先の解決）。未定義なら NameError。
    LoadGlobal(u32),
    /// pop して locals[slot] へそのまま書き込む（const / 代入 / let-from-immutable / リテラル let）。
    StoreLocal(u16),
    /// pop して deep_copy してから locals[slot] へ（`mut` 宣言: exec は常に deep_copy_value）。
    StoreLocalDeepCopy(u16),
    /// pop して deep_copy + freeze してから locals[slot] へ（`let` = mut ソースからの束縛）。
    StoreLocalCopyFreeze(u16),
    /// pop し、Instance のときのみ deep_copy + freeze してから locals[slot] へ
    /// （`let` = 非識別子式からの束縛。exec_let の非 ident 分岐に一致）。
    StoreLocalFreezeInstance(u16),
    /// スタックトップを1つ捨てる。
    Pop,
    /// 二項演算: pop b, pop a, push apply_binop_dyn(op, a, b)。
    Bin(BinOp),
    /// 単項演算: pop a, push apply_unary_dyn(op, a)。
    Un(UnaryOp),
    /// 属性（フィールド）読み: pop obj, push get_attr_val(obj, names[name_idx], attr_caches[cache_idx])。
    GetAttr(u32, u32),
    /// 属性（フィールド）書き: スタックは `[obj, value]`。value・obj を pop し、
    /// `attr_assign_evaled(obj, names[name_idx], value)` で代入する（値は push しない）。
    /// obj が Instance であることはコンパイル時に保証済み（`self` または instance 型注釈）。
    SetAttr(u32),
    /// スタックトップ2つを入れ替える（複合属性代入で rhs を先に評価しつつ演算順を保つため）。
    Swap,
    /// 型判定: pop v, push Bool(value_is_type(v, names[name_idx]))（match の `is TypeName` パターン）。
    IsType(u32),
    /// 純粋・共通な組み込み呼び出し（print/range/len）: argc 個の評価済み引数を pop し、
    /// `eval_builtin_evaled(names[name_idx], args)` で実行して結果を push する。
    CallBuiltin(u32, u16),
    /// イテレータ取得: pop iterable, push make_for_iterator(iterable)（`for` ループの入口）。
    GetIter,
    /// イテレータ前進: `locals[iter_slot]` の `.next()` を呼ぶ。
    /// `EndOfIteration` なら `exit_ip` へジャンプ、要素があれば `locals[target_slot]` へ束縛して継続。
    /// フィールドは (iter_slot, target_slot, exit_ip)。
    ForIter(u16, u16, u32),
    /// 無条件ジャンプ（絶対 index）。
    Jump(u32),
    /// pop した値が偽ならジャンプ（if/while の条件分岐）。
    JumpIfFalse(u32),
    /// スタックトップが偽ならジャンプ（値を残す）、真なら pop して継続（`and` 短絡）。
    JumpIfFalseOrPop(u32),
    /// スタックトップが真ならジャンプ（値を残す）、偽なら pop して継続（`or` 短絡）。
    JumpIfTrueOrPop(u32),
    /// 関数呼び出し: スタックは `[callee, arg0, .., argN-1]`。args を argc 個・callee を pop し、
    /// `mut_mask`（bit i = arg i の is_mutable）付きでディスパッチして結果を push する。
    Call(u16, u32),
    /// インスタンスメソッド呼び出し: スタックは `[obj, arg0, .., argN-1]`。args を argc 個・obj を pop し、
    /// `names[name_idx]` のメソッドを `mut_mask` 付きでディスパッチして結果を push する。
    /// obj が Instance であることはコンパイル時の型注釈で保証済み。
    CallMethod(u32, u16, u32),
    /// スタックトップを関数戻り値として返す。
    Return,
    /// `None` を関数戻り値として返す（本体末尾のフォールオフ）。
    ReturnNil,
}
