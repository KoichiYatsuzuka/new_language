// vm/compiler/block_expr.rs — ブロック式（`block:` / `if` / `match` / `for` / `while`）。
//
// 各 `compile_*_expr` は「脱出検査 → `stmt_base` 退避 → `_inner` → 復元」の同じ骨格を持つ。
// ⚠ `block_return` は `result_slot` へ、`loop_yield` は `yield_slot` のリストへ。
// `if`/`match` 式は yield に対して**透過**（`yield_slot=None`）＝外側の for/while/block へ届く。


use crate::ast::{
    BinOp, Expr, MatchArm, MatchPattern, Stmt,
};

use crate::vm::op::Op;
use super::*;


impl Compiler {
    /// `block: <stmts>` 式。block_return 値、なければ loop_yield 蓄積リスト、どちらもなければ None。
    ///
    /// `entry_depth` は**この式が始まるオペランド深さ**（#34）。本体の文はこの深さで走るので、
    /// 本体内の `break`/`continue` はこの数だけ `Pop` してから外側ループへ跳ぶ。
    ///
    /// `owns_yields` は「このブロックが `loop_yield` の蓄積先を持つか」（#35）。
    /// **`block:` 式は持ち（true）、`block:` 文は持たない（false）**。ツリーウォークの
    /// `exec_block_stmt`（#33 で削除）は `BLOCK_YIELDS` を push しなかったので、文の中の `loop_yield` は
    /// **外側の for/while 式へ届く**（届く先が無ければ実行時エラー）。ここを true にすると
    /// 蓄積が文に吸い込まれて捨てられる（`for … ->list[int]: block: loop_yield i` が `None` になった）。
    pub(super) fn compile_block_expr(
        &mut self,
        stmts: &[Stmt],
        entry_depth: Option<u16>,
        ann: Option<u32>,
        owns_yields: bool,
    ) -> Option<()> {
        if block_body_bails(stmts) {
            bail("block-expr-escape", None);
            return None;
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_block_expr_inner(stmts, ann, owns_yields);
        self.stmt_base = saved_base;
        r
    }

    pub(super) fn compile_block_expr_inner(
        &mut self,
        stmts: &[Stmt],
        ann: Option<u32>,
        owns_yields: bool,
    ) -> Option<()> {
        let yield_slot = if owns_yields {
            let s = self.alloc_temp()?;
            self.emit(Op::BuildEmptyList);
            self.emit(Op::StoreLocal(s));
            Some(s)
        } else {
            None
        };
        let result_slot = self.alloc_temp()?;
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in stmts {
            self.compile_stmt(s)?;
        }
        let ctx = self.block_ctxs.pop().unwrap();
        // 正常フォールスルー: 値 = 蓄積リスト or None（蓄積先を持たなければ常に None）。
        match yield_slot {
            Some(s) => {
                self.emit(Op::LoadLocal(s));
                self.emit(Op::ListOrNone);
            }
            None => {
                self.emit(Op::Nil);
            }
        }
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // block_return 出口: 値 = result_slot。
        let br_end = self.here();
        for j in ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        if yield_slot.is_some() {
            self.free_temp(); // yield_slot
        }
        Some(())
    }

    /// `if cond -> T: ... [elif][else]` 式。マッチした分岐の block_return 値、なければ None。
    /// yield に対しては透過（yield_slot=None）。
    pub(super) fn compile_if_expr(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
        entry_depth: Option<u16>,
        ann: Option<u32>,
    ) -> Option<()> {
        for (_, b) in branches {
            if block_body_bails(b) {
                bail("if-expr-escape", None);
                return None;
            }
        }
        if let Some(eb) = else_body {
            if block_body_bails(eb) {
                bail("ifexpr-else-escape", None);
                return None;
            }
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_if_expr_inner(branches, else_body, ann);
        self.stmt_base = saved_base;
        r
    }

    pub(super) fn compile_if_expr_inner(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
        ann: Option<u32>,
    ) -> Option<()> {
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot)); // 既定 None
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        let mut branch_ends: Vec<usize> = Vec::new();
        for (cond, body) in branches {
            self.compile_expr(cond)?;
            let jf = self.emit(Op::JumpIfFalse(0));
            for s in body {
                self.compile_stmt(s)?;
            }
            branch_ends.push(self.emit(Op::Jump(0)));
            let next = self.here();
            self.patch_jump(jf, next);
        }
        if let Some(eb) = else_body {
            for s in eb {
                self.compile_stmt(s)?;
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in branch_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot)); // block_return 値 or None
        self.free_temp();
        Some(())
    }

    /// `match subj -> T: arms` 式。マッチしたアームの block_return 値、なければ None。
    pub(super) fn compile_match_expr(
        &mut self,
        subject: &Expr,
        arms: &[MatchArm],
        entry_depth: Option<u16>,
        ann: Option<u32>,
    ) -> Option<()> {
        for arm in arms {
            if block_body_bails(&arm.body) {
                bail("matchexpr-escape", None);
                return None;
            }
        }
        let saved_base = std::mem::replace(&mut self.stmt_base, entry_depth);
        let r = self.compile_match_expr_inner(subject, arms, ann);
        self.stmt_base = saved_base;
        r
    }

    pub(super) fn compile_match_expr_inner(
        &mut self,
        subject: &Expr,
        arms: &[MatchArm],
        ann: Option<u32>,
    ) -> Option<()> {
        let subj_temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(subj_temp));
        let result_slot = self.alloc_temp()?;
        self.emit(Op::Nil);
        self.emit(Op::StoreLocal(result_slot));
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: None,
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        let mut arm_ends: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                }
                MatchPattern::Case(pat) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    self.compile_expr(pat)?;
                    self.emit(Op::Bin(BinOp::Eq));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(subj_temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    arm_ends.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let ctx = self.block_ctxs.pop().unwrap();
        let end = self.here();
        for j in arm_ends {
            self.patch_jump(j, end);
        }
        for j in ctx.end_jumps {
            self.patch_jump(j, end);
        }
        self.emit(Op::LoadLocal(result_slot));
        self.free_temp(); // result_slot
        self.free_temp(); // subj_temp
        Some(())
    }

    /// `for target in iter -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    pub(super) fn compile_for_expr(
        &mut self,
        target: &str,
        iter: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        if block_body_bails(body) {
            bail("loopexpr-escape", None);
            return None;
        }
        // 自身が最内ループになるので本体の基準深さは 0（#34）。
        // 本体内の `break` はこの式の NORMAL_END（= 蓄積リストを push する位置）へ跳ぶ。
        let saved_base = self.stmt_base.replace(0);
        let r = self.compile_for_expr_inner(target, iter, body, ann);
        self.stmt_base = saved_base;
        r
    }

    pub(super) fn compile_for_expr_inner(
        &mut self,
        target: &str,
        iter: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        let target_slot = *self.slots.get(target)?;
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let iter_temp = self.alloc_temp()?;
        self.compile_expr(iter)?;
        self.emit(Op::GetIter);
        self.emit(Op::StoreLocal(iter_temp));
        let loop_start = self.here();
        let fi = self.emit(Op::ForIter(iter_temp, target_slot, 0)); // exit → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
            try_len: self.try_stack.len(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        // NORMAL_END: 反復終了 or break → 蓄積リスト or None。
        let normal_end = self.here();
        self.code[fi] = Op::ForIter(iter_temp, target_slot, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0)); // → EXPR_END
        // BR_END: block_return → result_slot。
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // iter_temp
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }

    /// `while cond -> T: body` 式。block_return 値、なければ loop_yield 蓄積リスト（空なら None）。
    pub(super) fn compile_while_expr(
        &mut self,
        cond: &Expr,
        body: &[Stmt],
        ann: Option<u32>,
    ) -> Option<()> {
        if block_body_bails(body) {
            bail("loopexpr-escape", None);
            return None;
        }
        // 自身が最内ループになるので本体の基準深さは 0（#34）。
        let saved_base = self.stmt_base.replace(0);
        let r = self.compile_while_expr_inner(cond, body, ann);
        self.stmt_base = saved_base;
        r
    }

    pub(super) fn compile_while_expr_inner(&mut self, cond: &Expr, body: &[Stmt], ann: Option<u32>) -> Option<()> {
        let yield_slot = self.alloc_temp()?;
        self.emit(Op::BuildEmptyList);
        self.emit(Op::StoreLocal(yield_slot));
        let result_slot = self.alloc_temp()?;
        let loop_start = self.here();
        self.compile_expr(cond)?;
        let jf = self.emit(Op::JumpIfFalse(0)); // false → NORMAL_END
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
            try_len: self.try_stack.len(),
        });
        self.block_ctxs.push(BlockCtx {
            result_slot,
            end_jumps: Vec::new(),
            yield_slot: Some(yield_slot),
            return_type: ann,
            try_len: self.try_stack.len(),
            entry_depth: self.stmt_base,
        });
        for s in body {
            self.compile_stmt(s)?;
        }
        self.emit(Op::Jump(loop_start));
        let normal_end = self.here();
        self.patch_jump(jf, normal_end);
        let loop_ctx = self.loops.pop().unwrap();
        for j in loop_ctx.break_jumps {
            self.patch_jump(j, normal_end);
        }
        let block_ctx = self.block_ctxs.pop().unwrap();
        self.emit(Op::LoadLocal(yield_slot));
        self.emit(Op::ListOrNone);
        let after_normal = self.emit(Op::Jump(0));
        let br_end = self.here();
        for j in block_ctx.end_jumps {
            self.patch_jump(j, br_end);
        }
        self.emit(Op::LoadLocal(result_slot));
        let expr_end = self.here();
        self.patch_jump(after_normal, expr_end);
        self.free_temp(); // result_slot
        self.free_temp(); // yield_slot
        Some(())
    }
}
