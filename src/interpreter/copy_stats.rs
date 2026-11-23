// interpreter/copy_stats.rs — 診断フック `AR_COPY_STATS=1`（copy_alloc_speed_plan.md 1-2）。
//
// **値の複製がどの入口で何回起き、どれだけの大きさを写したか**を数える。速度の調査で、時間の大半が
// `let self` / `let` 仮引数で受けるときと格納のときの深いコピーだと分かった（同 §0）。どの複製を省けるかは
// ベンチ 1 本では決められないので、例題全体での回数と「写した値を誰とも共有していなかった」回数を出す。
//
// 入口の種類（[`CopyKind`]）ごとに:
//   - `calls`  … 複製の入口に来た回数（写す必要のない値〔数値・文字列など〕も含む）
//   - `copied` … 実際に作り直す値（インスタンス・リスト・辞書・集合・タプル）が来た回数
//   - `nodes`  … 作り直した値の数（入れ子を含むインスタンスとコンテナの数 ＝ 確保の回数の目安）
//   - `elems`  … 写した要素・フィールドの数
//   - `alone1` / `alone2` / `alone3` … 入れ子まで含めて誰とも共有していなかった回数。いちばん外の参照が
//     1 個まで / 2 個まで / 3 個までのとき（入口が値を受け取った 1 個のほかに、呼ぶ側が写しを 1〜2 個持っている
//     入口がある。コンパイル済みコードではアリーナの欄が 1 個持つ）。`alone1 ≤ alone2 ≤ alone3`
//   - `skipped` … `copied` のうち、入口が共有していないと判定して**写さずに済ませた**回数（実際に写したのは
//     `copied - skipped`）
//
// **既定ではコンパイルされない**。`cargo build --release --features copy_stats` ＋ `AR_COPY_STATS=1` で有効
// （`scripts/copy_stats.ps1` が専用のビルドを作って例題を回す）。feature を切る理由は `tw_stats.rs` の `enabled()` と同じ。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::interpreter::{Interpreter, Value};

/// 複製の入口の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyKind {
    /// `let self` で受ける `mut` の受け手（`bind_self_value`・コンパイル済みコードの `ar_deep_copy`）。
    LetSelf,
    /// `let` 仮引数で受ける `mut` の値（`copy_value`）。
    LetParam,
    /// 格納（`copy_for_store`: フィールド・`append`・添字・辞書・集合・クロージャの取り込み）。
    Store,
    /// 合成した `__init__` の直接格納（`store_auto_init`。この前に `LetParam` の複製も通る）。
    AutoInitStore,
    /// 宣言の束縛（`bind_copy_held`: `let x = e` / `mut x = e` / `freeze x`）。共有していない値は実際には写さない（12-1）。
    Bind,
    /// 利用者が書いた複製（`x.copy()`・`d.copy()`）。
    Explicit,
    /// `LetSelf` のうち、呼ばれた関数が読むだけだったもの（`crate::readonly_fns`・copy_alloc_speed_plan.md 4-1）。
    /// ⚠ VM の速い束縛の道（`try_fast_bind`）だけを数える。`LetSelf` と重ねて数える（内訳）。
    LetSelfReadonly,
    /// `LetParam` のうち、呼ばれた関数が読むだけだったもの（同上）。
    LetParamReadonly,
}

const KINDS: [(CopyKind, &str); 8] = [
    (CopyKind::LetSelf, "let_self"),
    (CopyKind::LetParam, "let_param"),
    (CopyKind::Store, "store"),
    (CopyKind::AutoInitStore, "auto_init_store"),
    (CopyKind::Bind, "bind"),
    (CopyKind::Explicit, "explicit"),
    (CopyKind::LetSelfReadonly, "let_self_readonly"),
    (CopyKind::LetParamReadonly, "let_param_readonly"),
];

/// 種類ごとの数（`calls` / `copied` / `nodes` / `elems` / `alone1` / `alone2` / `alone3` / `skipped`）。
/// 非同期タスクのワーカースレッドからも数えるので、スレッドごとではなくプロセスに 1 つ。
const N_FIELDS: usize = 8;
#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ZERO_ROW: [AtomicU64; N_FIELDS] = [ZERO; N_FIELDS];
static COUNTS: [[AtomicU64; N_FIELDS]; KINDS.len()] = [ZERO_ROW; KINDS.len()];

/// 計測が有効か。⚠ `cfg!` の判定を先に置く（feature 無しのビルドでは定数 `false` になり、呼ぶ側ごと消える）。
#[inline(always)]
pub(crate) fn enabled() -> bool {
    if !cfg!(feature = "copy_stats") {
        return false;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AR_COPY_STATS").is_ok_and(|v| !v.is_empty()))
}

/// 複製の入口を 1 件数える。`val` は写す前の値、`skipped` は入口が「共有していない」と判定して写さないか。
#[inline(always)]
pub(crate) fn record(kind: CopyKind, val: &Value, skipped: bool) {
    if !enabled() {
        return;
    }
    record_slow(kind, val, skipped);
}

#[inline(never)]
fn record_slow(kind: CopyKind, val: &Value, skipped: bool) {
    let row = &COUNTS[KINDS.iter().position(|(k, _)| *k == kind).expect("every CopyKind is listed in KINDS")];
    row[0].fetch_add(1, Ordering::Relaxed);
    let (mut nodes, mut elems) = (0u64, 0u64);
    measure(val, &mut nodes, &mut elems);
    if nodes == 0 {
        return;
    }
    row[1].fetch_add(1, Ordering::Relaxed);
    row[2].fetch_add(nodes, Ordering::Relaxed);
    row[3].fetch_add(elems, Ordering::Relaxed);
    for (i, outer) in [1usize, 2, 3].into_iter().enumerate() {
        if Interpreter::is_unshared(val, outer) {
            row[4 + i].fetch_add(1, Ordering::Relaxed);
        }
    }
    if skipped {
        row[7].fetch_add(1, Ordering::Relaxed);
    }
}

/// `deep_copy_value` が作り直す値の数（`nodes`）と、写す要素・フィールドの数（`elems`）。
/// ⚠ 借りられない値（書き込み中の `RefCell`）は数えずに飛ばす（診断が実行を止めない）。
fn measure(val: &Value, nodes: &mut u64, elems: &mut u64) {
    match val {
        Value::Instance(rc) => {
            *nodes += 1;
            if let Ok(inst) = rc.try_borrow() {
                for (v, _) in inst.boxed_fields.iter().flatten() {
                    *elems += 1;
                    measure(v, nodes, elems);
                }
            }
        }
        Value::List(rc) | Value::Set(rc) => {
            *nodes += 1;
            if let Ok(xs) = rc.try_borrow() {
                *elems += xs.len() as u64;
                xs.iter().for_each(|v| measure(v, nodes, elems));
            }
        }
        Value::Dict(rc) => {
            *nodes += 1;
            if let Ok(d) = rc.try_borrow() {
                let items = d.all_items();
                *elems += items.len() as u64;
                items.iter().for_each(|v| measure(v, nodes, elems));
            }
        }
        Value::Tuple(t) => {
            *nodes += 1;
            *elems += t.all_values().len() as u64;
            t.all_values().iter().for_each(|v| measure(v, nodes, elems));
        }
        Value::FrozenSet(items) => {
            *nodes += 1;
            *elems += items.len() as u64;
            items.iter().for_each(|v| measure(v, nodes, elems));
        }
        // 作り直さない値（数値・文字列・関数・クラス・CPython の物など）
        _ => {}
    }
}

/// 実行の終わりに内訳を stderr へ出す（1 種類 1 行・`scripts/copy_stats.ps1` が読む）。
pub(crate) fn dump() {
    for (i, (_, name)) in KINDS.iter().enumerate() {
        let v: Vec<u64> = COUNTS[i].iter().map(|c| c.load(Ordering::Relaxed)).collect();
        eprintln!(
            "CopyStats[{name}] calls={} copied={} nodes={} elems={} alone1={} alone2={} alone3={} skipped={}",
            v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]
        );
    }
}
