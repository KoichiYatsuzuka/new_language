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
// 新しくコード索引を持つ op を足したら `remap_targets` にも足すこと（`code_target_mut` が唯一の窓口）。
//
// ⚠ `Chunk::spans` は**プール**（op が索引を持つ）であって per-op の行テーブルではないので、
// 命令の削除でずれない。将来 per-op の行テーブル（#1）を入れるときは、ここで一緒に
// 詰め直す必要がある。

use super::op::Op;

/// 連鎖を追う回数の上限（`JUMP` が自分自身を指す等の異常系で無限ループしないための保険）。
const MAX_CHAIN: usize = 64;

/// 除去と再ターゲットを繰り返す回数の上限。除去で新しい「次命令への JUMP」が生まれることが
/// あるため反復するが、実際は 1〜2 回で収束する。
const MAX_PASSES: usize = 8;

/// op が持つ**コード索引**への可変参照を返す（持たなければ `None`）。
///
/// コード索引を持つ op を一箇所に集約するための関数。飛び先の書き換えは必ずここを通す。
fn code_target_mut(op: &mut Op) -> Option<&mut u32> {
    match op {
        Op::Jump(t)
        | Op::JumpIfFalse(t)
        | Op::JumpIfFalseOrPop(t)
        | Op::JumpIfTrueOrPop(t)
        | Op::SetupTry(t)
        | Op::ForIter(_, _, t) => Some(t),
        _ => None,
    }
}

/// `code` を覗き穴最適化する（意味論不変）。
pub fn optimize(code: &mut Vec<Op>) {
    for _ in 0..MAX_PASSES {
        let collapsed = collapse_jump_chains(code);
        let removed = remove_jumps_to_next(code);
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

/// `i: JUMP i+1`（次命令へ跳ぶだけ）を除去し、全てのコード索引を詰め直す。
fn remove_jumps_to_next(code: &mut Vec<Op>) -> bool {
    let n = code.len();
    let remove: Vec<bool> = code
        .iter()
        .enumerate()
        .map(|(i, op)| matches!(op, Op::Jump(t) if *t as usize == i + 1))
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
        optimize(&mut code);
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
        optimize(&mut code);
        // 2 は「次命令(3)への JUMP」ではなく 0 へ向くので、連鎖畳み込みが先に効く。
        assert!(matches!(code[1], Op::JumpIfFalse(0)));
        assert!(matches!(code[2], Op::Jump(0)));
    }

    /// **コード索引を持つのは飛び先だけではない**: `ForIter` の exit と `SetupTry` の
    /// ハンドラ入口も再マップされること（漏らすとループ脱出・例外ハンドラが壊れる）。
    #[test]
    fn remaps_for_iter_exit_and_setup_try_handler() {
        let mut code = vec![
            Op::SetupTry(5),       // 0 → ハンドラは索引 5
            Op::ForIter(0, 1, 5),  // 1 → 脱出先は索引 5
            Op::Jump(3),           // 2 ← 除去対象（次命令へ）
            Op::Bin(BinOp::Add),   // 3
            Op::Jump(5),           // 4 ← 除去対象（次命令へ）
            Op::Return,            // 5 ← 除去後は索引 3
        ];
        optimize(&mut code);
        assert_eq!(code.len(), 4);
        assert!(matches!(code[0], Op::SetupTry(3)), "{code:?}");
        assert!(matches!(code[1], Op::ForIter(0, 1, 3)), "{code:?}");
        assert!(matches!(code[3], Op::Return), "{code:?}");
    }

    /// 自分自身へ跳ぶ `JUMP`（異常系）で無限ループしないこと。
    #[test]
    fn self_jump_terminates() {
        let mut code = vec![Op::Jump(0), Op::Return];
        optimize(&mut code);
        assert!(matches!(code[0], Op::Jump(0)));
    }
}
