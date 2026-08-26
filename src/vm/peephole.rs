// vm/peephole.rs — 生成済み命令列の覗き穴最適化（Phase V-F, タスク #2a）。
//
// `compile_fn` がコード生成を終えた直後に 1 回だけ走る後処理。**意味論は不変**で、
// 実行されないディスパッチだけを削る。コンパイラ本体（compiler.rs）は「素直に出す」責務のまま
// にしておき、構造的に出てしまう無駄をここでまとめて回収する分業にしている。
//
// 対象は 2 つ:
//
// 1. **ジャンプ連鎖の畳み込み** — `JUMP a` の飛び先が `JUMP b` なら直接 `b` を狙う。
// 2. **次命令への JUMP の除去** — `i: JUMP i+1` は何もしないので消す。
//
// どちらも `if` に `else` が無いときに構造的に出る（compiler.rs の `Stmt::If` は分岐ごとに
// 無条件で `Jump(end)` を置き、最後の分岐ではその `end` が直後の命令になる）:
//
// ```text
//    3  JUMP_IF_FALSE 7
//    4  CONST 1
//    5  STORE_LOCAL 1
//    6  JUMP 7          ← 次命令への JUMP（除去対象）
//    7  LOAD_LOCAL 1
// ```
//
// ⚠ **命令を消すとインデックスがずれるので、コード索引を持つ op を全て再マップする必要がある**。
// 再マップ対象は「飛び先を持つ op」だけではない: `ForIter` の exit_ip と `SetupTry` の
// handler_ip も**コード索引**なので、漏らすと例外ハンドラやループ脱出が壊れる。
// 新しくコード索引を持つ op を足したら **`code_target_mut` に登録すること**（唯一の窓口）。
// ⚠ 忘れてもテストも例題も通ってしまう（#27-d: `StaticInit` の飛び先を忘れた実例）。
//
// ⚠ `Chunk::spans` は**プール**（op が索引を持つ）であって per-op の行テーブルではないので、
// 命令の削除でずれない。一方 `Chunk::stmt_spans`（文境界の行テーブル・#1）は code と 1:1 なので、
// `optimize` が**同じ写像で一緒に詰め直す**。さらに **文の先頭 op は除去対象から外す**
// （理由は `remove_jumps_to_next` の中のコメント）。

use super::op::Op;

/// 連鎖を追う回数の上限（`JUMP` が自分自身を指す等の異常系で無限ループしないための保険）。
const MAX_CHAIN: usize = 64;

/// 除去と再ターゲットを繰り返す回数の上限。除去で新しい「次命令への JUMP」が生まれることが
/// あるため反復するが、実際は 1〜2 回で収束する。
const MAX_PASSES: usize = 8;

/// op が持つ**コード索引**への可変参照を返す（持たなければ `None`）。
///
/// コード索引を持つ op を一箇所に集約するための関数。飛び先の書き換えは必ずここを通す。
///
/// ⚠⚠ **`match` は exhaustive**（#87）。`_ => None` だった頃は
/// **op を足しても何も強制されず**、`ForIter` の exit・`SetupTry` の handler・
/// `StaticInit` の after を登録し忘れても **テストも例題も通ってしまった**（#27-d が実例）。
/// ⇒ 今は op を足すとここが止まり、「**この op は飛び先を持つのか**」を必ず決めさせられる。
/// ⚠ **`_` を書き足してこの仕掛けを無効化しないこと。** 持たないなら列挙側へ足す。
///
/// ⚠ **1 op につき飛び先は 1 つ**という前提の API（`Option<&mut u32>`）。
/// 2 つ持つ op を足すときは**まずこの戻り値の型から直す**こと
/// （再マップする [`collapse_jump_chains`] と [`remove_jumps_to_next`] も同時に見直しが要る）。
///
/// ⚠ 分類は **op の doc を機械抽出して突き合わせた**（#86 の教訓 — 名前で分類すると取りこぼす）。
/// 生成器は `doc が飛び先に言及しているのに分類されていない op` があれば落ちる。
fn code_target_mut(op: &mut Op) -> Option<&mut u32> {
    match op {
        Op::Jump(t)
        | Op::JumpIfFalse(t)
        | Op::JumpIfFalseOrPop(t)
        | Op::JumpIfTrueOrPop(t)
        | Op::SetupTry(t)
        => Some(t),
        Op::StaticInit(_, t) => Some(t),
        Op::ForIter(_, _, t) => Some(t),
        // ── コード索引を持たない op ──
        // ⚠⚠ **`_` を書かない**。op を足すとここが止まり、
        //    「この op は飛び先を持つのか」を必ず決めさせられる（#87）。
        //    #27-d で `StaticInit` の飛び先を登録し忘れた実例があり、
        //    **テストも例題も通ってしまった**（たまたま除去対象が無かっただけ）。
        // ⚠ `Fail`: doc に「**飛び先索引を持たない**ので `code_target_mut` は不要」と明記（u32 は名前プール索引）。
        Op::Const(_)
        | Op::Nil
        | Op::LoadLocal(_)
        | Op::LoadGlobal(_, _)
        | Op::StoreGlobal(_, _)
        | Op::StoreName(_)
        | Op::StoreLocal(_)
        | Op::StoreLocalDeepCopy(_)
        | Op::StoreLocalCopyFreeze(_)
        | Op::StoreLocalFreezeInstance(_)
        | Op::StoreLocalFromIdent(_, _)
        | Op::CallBuiltinKw(_)
        | Op::CallMethodKw(_)
        | Op::StaticStore(_)
        | Op::LoadStatic(_)
        | Op::StoreStatic(_)
        | Op::LoadCell(_)
        | Op::StoreCell(_)
        | Op::StoreCellDeepCopy(_)
        | Op::Pop
        | Op::Bin(_)
        | Op::BinLocalLocal(_, _, _)
        | Op::BinLocalConst(_, _, _)
        | Op::IntBinLL(_, _, _)
        | Op::IntBinLC(_, _, _)
        | Op::IntBinGG(_, _, _)
        | Op::IntBinGC(_, _, _)
        | Op::FloatBinLL(_, _, _)
        | Op::FloatBinLC(_, _, _)
        | Op::IntBinSS(_)
        | Op::FloatBinSS(_)
        | Op::Un(_)
        | Op::GetAttr(_, _)
        | Op::SetAttr(_)
        | Op::Swap
        | Op::CallMethodLocal(_, _, _, _, _)
        | Op::GetAttrLocal(_, _, _)
        | Op::IsType(_)
        | Op::MustBe(_, _)
        | Op::Cast(_)
        | Op::CallBuiltin(_, _)
        | Op::GetIter
        | Op::Call(_, _, _, _, _)
        | Op::CallMethod(_, _, _, _)
        | Op::Return
        | Op::ReturnNil
        | Op::PopTry
        | Op::Raise(_)
        | Op::Reraise
        | Op::Dup
        | Op::ExcMatch(_)
        | Op::CheckBlockReturn(_, _)
        | Op::CheckLoopYield(_, _)
        | Op::Fail(_)
        | Op::BuildEmptyList
        | Op::ListAppendLocal(_)
        | Op::ListOrNone
        | Op::LoadName(_)
        | Op::DeclareName(_)
        | Op::MakeFn(_)
        | Op::EnumDef(_)
        | Op::UnpackTuple(_, _)
        | Op::LetTuple(_)
        | Op::FreezeVar(_, _)
        | Op::EventSubscribe(_, _)
        | Op::EventUnsubscribe
        | Op::CallKw(_)
        | Op::CallTemplate(_, _, _)
        | Op::BuildSlice
        | Op::DeclareGlobal(_, _)
        | Op::LoadSelfClass
        | Op::GetTraitAttr(_, _)
        | Op::SetTraitAttr(_, _)
        | Op::BreakPoint(_)
        | Op::Subscript
        | Op::SetIndex
        | Op::BuildList(_)
        | Op::BuildTuple(_)
        | Op::BuildSet(_)
        | Op::BuildDict(_)
        | Op::Yield
        | Op::AsyncSubmit(_)
        => None,
    }
}

/// `code` を覗き穴最適化する（意味論不変）。
///
/// `stmt_spans` は `code` と 1:1 の**文境界行テーブル**（#1）。命令を消すとずれるので
/// **同じ写像で一緒に詰め直す**。呼び出し側が行テーブルを持たない場合は空スライスでよい。
pub fn optimize(code: &mut Vec<Op>, stmt_spans: &mut Vec<u32>) {
    debug_assert!(
        stmt_spans.is_empty() || stmt_spans.len() == code.len(),
        "stmt_spans は code と 1:1 か空のいずれかであること"
    );
    for _ in 0..MAX_PASSES {
        let collapsed = collapse_jump_chains(code);
        let removed = remove_jumps_to_next(code, stmt_spans);
        if !collapsed && !removed {
            break;
        }
    }
}

/// `JUMP a` の飛び先が無条件 `JUMP b` なら `b` へ直接向ける（推移的に追う）。
///
/// 条件付きジャンプ・`ForIter` の exit・`SetupTry` のハンドラ入口についても、
/// 飛び先が無条件 `JUMP` なら同様に短絡してよい（`Jump` はスタックに触れないため、
/// 途中を飛ばしても観測される状態は同じ）。
fn collapse_jump_chains(code: &mut [Op]) -> bool {
    let n = code.len();
    // 各索引の「最終的な飛び先」を先に求めてから書き戻す（借用の都合と、
    // 途中結果を読んで結論が変わるのを避けるため）。
    let final_target: Vec<u32> = (0..n)
        .map(|i| {
            let mut t = i;
            for _ in 0..MAX_CHAIN {
                match code.get(t) {
                    Some(Op::Jump(next)) if (*next as usize) != t => t = *next as usize,
                    _ => break,
                }
            }
            t as u32
        })
        .collect();

    let mut changed = false;
    for op in code.iter_mut() {
        if let Some(t) = code_target_mut(op) {
            let cur = *t as usize;
            if let Some(&dest) = final_target.get(cur) {
                if dest != *t {
                    *t = dest;
                    changed = true;
                }
            }
        }
    }
    changed
}

/// `i: JUMP i+1`（次命令へ跳ぶだけ）を除去し、全てのコード索引と行テーブルを詰め直す。
fn remove_jumps_to_next(code: &mut Vec<Op>, stmt_spans: &mut Vec<u32>) -> bool {
    let n = code.len();
    let remove: Vec<bool> = code
        .iter()
        .enumerate()
        .map(|(i, op)| {
            // ⚠ **文の先頭 op は消さない**（#1）。`break` のように「1 個の `Jump` だけ」に
            // コンパイルされる文があり、それを消すとデバッガの停止位置が 1 文ぶん失われる。
            // 消した boundary を次の生存 op へ寄せる案もあるが、次の op も boundary だと
            // 「2 文が同じ ip から始まる」を表現できず off/auto の transcript がずれる。
            // 除去できる Jump は全体の 0.31% しかない（#2a 実測）ので、確実さを取る。
            if stmt_spans.get(i).is_some_and(|&s| s != super::chunk::NOT_STMT) {
                return false;
            }
            matches!(op, Op::Jump(t) if *t as usize == i + 1)
        })
        .collect();
    if !remove.iter().any(|&r| r) {
        return false;
    }

    // old→new の索引表。**除去される命令は「次に生き残る命令」の位置を指す**ので、
    // その命令を飛び先にしていたジャンプも自然に正しい位置へ向く。
    // 末尾（＝コード終端）への飛び先があり得るので長さは n+1。
    let mut new_idx = vec![0u32; n + 1];
    let mut k = 0u32;
    for i in 0..n {
        new_idx[i] = k;
        if !remove[i] {
            k += 1;
        }
    }
    new_idx[n] = k;

    let mut out = Vec::with_capacity(k as usize);
    for (i, op) in code.drain(..).enumerate() {
        if !remove[i] {
            out.push(op);
        }
    }
    // 行テーブルも同じ写像で詰め直す（#1）。空なら呼び出し側が持っていないだけなので何もしない。
    if !stmt_spans.is_empty() {
        let mut spans_out = Vec::with_capacity(k as usize);
        for (i, s) in stmt_spans.drain(..).enumerate() {
            if !remove[i] {
                spans_out.push(s);
            }
        }
        *stmt_spans = spans_out;
    }
    for op in out.iter_mut() {
        if let Some(t) = code_target_mut(op) {
            // 範囲外は書き換えない（あり得ないが、壊れた索引を静かに別命令へ向けない）。
            if let Some(&mapped) = new_idx.get(*t as usize) {
                *t = mapped;
            }
        }
    }
    *code = out;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinOp;

    /// `if` に `else` が無いときに出る「次命令への JUMP」が消えること。
    #[test]
    fn removes_jump_to_next() {
        let mut code = vec![
            Op::JumpIfFalse(3), // 0
            Op::Nil,            // 1
            Op::Jump(3),        // 2 ← 次命令へ跳ぶだけ
            Op::Return,         // 3
        ];
        optimize(&mut code, &mut Vec::new());
        assert_eq!(code.len(), 3);
        // 飛び先 3 は詰め直しで 2 になる。
        assert!(matches!(code[0], Op::JumpIfFalse(2)));
        assert!(matches!(code[2], Op::Return));
    }

    /// `JUMP` 連鎖が終点へ短絡されること（ループ内 if で実際に出る形）。
    #[test]
    fn collapses_jump_chain() {
        let mut code = vec![
            Op::Nil,            // 0
            Op::JumpIfFalse(3), // 1
            Op::Jump(3),        // 2 → 3 は JUMP 0 なので 0 へ短絡
            Op::Jump(0),        // 3
            Op::Return,         // 4
        ];
        optimize(&mut code, &mut Vec::new());
        // 2 は「次命令(3)への JUMP」ではなく 0 へ向くので、連鎖畳み込みが先に効く。
        assert!(matches!(code[1], Op::JumpIfFalse(0)));
        assert!(matches!(code[2], Op::Jump(0)));
    }

    /// **コード索引を持つのは飛び先だけではない**: `ForIter` の exit と `SetupTry` の
    /// ハンドラ入口も再マップされること（漏らすとループ脱出・例外ハンドラが壊れる）。
    #[test]
    fn remaps_for_iter_exit_and_setup_try_handler() {
        let mut code = vec![
            Op::SetupTry(6),       // 0 → ハンドラは索引 6
            Op::ForIter(0, 1, 6),  // 1 → 脱出先は索引 6
            // ⚠⚠ `StaticInit` の第 2 引数は**飛び先**（#27-d で登録し忘れた実例）。
            // ここに入れる前は、`code_target_mut` から `StaticInit` を外しても
            // **750 テストが全部通っていた**（#87 で実測）。
            Op::StaticInit(0, 6),  // 2 → 初期化済みなら索引 6 へ
            Op::Jump(4),           // 3 ← 除去対象（次命令へ）
            Op::Bin(BinOp::Add),   // 4
            Op::Jump(6),           // 5 ← 除去対象（次命令へ）
            Op::Return,            // 6 ← 除去後は索引 4
        ];
        optimize(&mut code, &mut Vec::new());
        assert_eq!(code.len(), 5, "{code:?}");
        assert!(matches!(code[0], Op::SetupTry(4)), "{code:?}");
        assert!(matches!(code[1], Op::ForIter(0, 1, 4)), "{code:?}");
        // ⚠ 第 1 引数（span 索引）は**コード索引ではない**ので動いてはいけない。
        assert!(matches!(code[2], Op::StaticInit(0, 4)), "{code:?}");
        assert!(matches!(code[4], Op::Return), "{code:?}");
    }

    /// 自分自身へ跳ぶ `JUMP`（異常系）で無限ループしないこと。
    #[test]
    fn self_jump_terminates() {
        let mut code = vec![Op::Jump(0), Op::Return];
        optimize(&mut code, &mut Vec::new());
        assert!(matches!(code[0], Op::Jump(0)));
    }

    /// 行テーブル（#1）が code と 1:1 のまま詰め直されること。
    #[test]
    fn compacts_stmt_spans_with_code() {
        use super::super::chunk::NOT_STMT;
        let mut code = vec![
            Op::Nil,     // 0  文の先頭（span 7）
            Op::Jump(2), // 1  ← 除去対象（文の先頭ではない）
            Op::Return,  // 2  文の先頭（span 9）
        ];
        let mut spans = vec![7, NOT_STMT, 9];
        optimize(&mut code, &mut spans);
        assert_eq!(code.len(), 2);
        assert_eq!(spans, vec![7, 9], "行テーブルが code と一緒に詰まっていない");
    }

    /// **文の先頭 op は除去しない**こと（`break` のように Jump 1 個だけの文がある）。
    /// 消すとデバッガの停止位置が 1 文ぶん失われる。
    #[test]
    fn keeps_jump_that_starts_a_statement() {
        use super::super::chunk::NOT_STMT;
        let mut code = vec![
            Op::Nil,     // 0
            Op::Jump(2), // 1 ← 次命令への JUMP だが **文の先頭**
            Op::Return,  // 2
        ];
        let mut spans = vec![NOT_STMT, 3, NOT_STMT];
        optimize(&mut code, &mut spans);
        assert_eq!(code.len(), 3, "文の先頭 Jump が消えている: {code:?}");
        assert_eq!(spans, vec![NOT_STMT, 3, NOT_STMT]);
    }
}
