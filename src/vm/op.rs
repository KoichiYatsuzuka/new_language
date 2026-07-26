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
    /// フィールド: (argc, mut_mask, name_idx, span_idx)。name_idx=呼び出し元名（トレースバック用・
    /// `names`）、span_idx=呼び出し位置（`spans`）。
    Call(u16, u32, u32, u32),
    /// インスタンスメソッド呼び出し: スタックは `[obj, arg0, .., argN-1]`。args を argc 個・obj を pop し、
    /// `names[name_idx]` のメソッドを `mut_mask` 付きでディスパッチして結果を push する。
    /// obj が Instance であることはコンパイル時の型注釈で保証済み。フィールド: (name_idx, argc, mut_mask)。
    /// メソッド呼び出しはツリーウォークが呼び出し位置 span を渡さない（フレームが degraded）ため、
    /// VM も call_span=None で一致させる（byte-identical）。
    CallMethod(u32, u16, u32),
    /// スタックトップを関数戻り値として返す。
    Return,
    /// `None` を関数戻り値として返す（本体末尾のフォールオフ）。
    ReturnNil,
    // ── 例外処理（Phase V-C） ──
    /// try 本体の入口: 例外ハンドラ（handler_ip）と現在のオペランドスタック深さを
    /// VM のハンドラスタックに push する。
    SetupTry(u32),
    /// try 本体が正常終了したときにハンドラを1つ pop する。
    PopTry,
    /// `raise expr`: pop した例外値に `spans[idx]` を焼き、current_exception を設定して伝播する。
    Raise(u32),
    /// bare `raise`（再送出）／どの except にもマッチしなかったときの再伝播。
    /// current_exception を用いて伝播する（スタックには触れない）。
    Reraise,
    /// スタックトップを複製する（except 節の型照合で例外値を残すため）。
    Dup,
    /// pop した例外値が `names[name_idx]` 型にマッチするか（`exc_matches`）を Bool で push する。
    ExcMatch(u32),
    // ── ブロック式（Phase V-C） ──
    /// 空の `Value::List` を push する（loop_yield の蓄積先の初期化）。
    BuildEmptyList,
    /// pop した値を `locals[slot]` のリスト（accumulator）へ追加する（`loop_yield`）。
    ListAppendLocal(u16),
    /// pop したリストが空なら `None`、非空ならそのリストを push する
    /// （for/while/block 式の値: 蓄積が空なら None）。
    ListOrNone,
    // ── デバッガ REPL（V-E。停止スコープの生変数へ名前でアクセス） ──
    /// 現在のスコープから `names[name_idx]` の値を名前引きで push する（`get_val`）。未定義なら NameError。
    LoadName(u32),
    /// pop した値を `let dbg::name` として現在のスコープへ宣言する（不変・`let` 意味論）。
    DeclareName(u32),
    // ── 添字・コレクションリテラル（タスク #5） ──
    /// 添字読み: pop key, pop obj, push `eval_subscript(obj, key)`（`obj[key]`）。
    Subscript,
    /// 添字書き: pop value, pop key, pop obj, `eval_setitem(obj, key, value)`（`obj[key] = value`）。
    SetIndex,
    /// リテラルリスト構築: 末尾 N 要素を pop して `Value::List` を push（`[a, b, ..]`）。
    BuildList(u16),
    /// リテラルタプル構築: 末尾 N 要素を pop して `Value::Tuple`（要素型名収集）を push（`(a, b, ..)`）。
    BuildTuple(u16),
    /// リテラル集合構築: 末尾 N 要素を pop して `Value::Set`（`set_insert` で重複排除）を push（`{a, b, ..}`）。
    BuildSet(u16),
    /// リテラル辞書構築: 末尾 2N 要素（k0,v0,k1,v1,..）を pop して `Value::Dict` を push（`{k: v, ..}`）。
    BuildDict(u16),
    // ── ジェネレータ（タスク #8） ──
    /// `yield expr`: pop した値をジェネレータの yield 収集バッファ（`GENERATOR_YIELDS`）へ追加する
    /// （eager 収集・ツリーウォークの `Stmt::Yield` と同一意味論。値は産出するだけで制御は継続）。
    Yield,
    // ── async（タスク #9） ──
    /// `target <- async->T: body`: pop した AsyncManager に `chunk.async_blocks[idx]` のタスクを投入する。
    /// 捕捉変数は frame から読み、`vm_async_submit`（capture_env 経由）で env を組む（ツリーウォーク一致）。
    AsyncSubmit(u32),
}
