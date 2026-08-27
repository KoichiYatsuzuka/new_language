// vm/compiler/control.rs — `try`/`except`/`finally` と `match` 文、および脱出時の巻き戻し。
//
// ⚠ **脱出制御はオペランドスタックとハンドラスタックの両方を戻す**（#34）。`try` から跳ぶと
// `PopTry` を通らず**ハンドラが残り、ループを抜けた後の無関係な例外を横取りする**。


use crate::ast::{
    BinOp, ExceptHandler, Expr, MatchArm, MatchPattern, Stmt,
};

use crate::vm::op::Op;
use super::*;


impl Compiler {
    pub(super) fn emit_unwind_to_loop(&mut self) -> Option<()> {
        let loop_try_len = self.loops.last().map_or(0, |l| l.try_len);
        // break/continue の経路では値を積んでいない（pops は finally の後に出す）。
        self.emit_unwind_tries(loop_try_len, true, 0)?;
        let Some(depth) = self.stmt_base else {
            bail("break-unknown-depth", None);
            return None;
        };
        for _ in 0..depth {
            self.emit(Op::Pop);
        }
        Some(())
    }

    /// 脱出が跨ぐ `try` を**内側から**巻き戻す（#34/#37）。
    ///
    /// - `try/except`: `PopTry` だけ（`pop_except` が真のとき）。⚠ `return` は `run` から
    ///   即復帰してハンドラごと捨てられるので不要 ＝ 既存 Chunk を変えないため偽を渡す。
    /// - `try/finally`: `PopTry` ＋ **`finally` 本体をこの経路にも複製**する。
    ///   ネストしていれば内側の finally から順に走る（ツリーウォーク・Python と同じ順）。
    ///
    /// `keep` は「跨がない外側の try の数」。break/continue は最内ループ入口、
    /// block_return は最内ブロック式入口、return は 0（全部）を渡す。
    pub(super) fn emit_unwind_tries(&mut self, keep: usize, pop_except: bool, extra: u16) -> Option<()> {
        for i in (keep..self.try_stack.len()).rev() {
            let Some(fin) = self.try_stack[i].clone() else {
                if pop_except {
                    self.emit(Op::PopTry);
                }
                continue;
            };
            self.emit(Op::PopTry);
            // ⚠ 複製中は「巻き戻し済みの try」を見せない（同じ finally を二重に出さない）。
            // 外側の try は残るので、複製の中の `break` は**外側の finally を走らせてから**跳ぶ。
            let saved = self.try_stack.split_off(i);
            let ok = self.compile_finally_copy(&fin, extra);
            self.try_stack.extend(saved);
            ok?;
        }
        Some(())
    }

    /// `finally` 本体を**この経路のスタックの上に**複製する（#37/#40）。
    ///
    /// `extra` は「この複製が載っているオペランドの数」。例外経路は `[exc]`、`return` 経路は
    /// 戻り値が 1 つ下に積まれている。⚠ **`stmt_base` をその分だけ持ち上げる**ことで、
    /// 複製の中の `break`/`continue`（#40）が跳ぶときに**その値まで捨ててくれる**
    /// （＝ Python と同じ「保留中の動作を破棄する」意味論になる）。
    ///
    /// 複製は経路ごとに増えるので、入れ子が深いとコードが指数的に膨らむ。
    /// **`MAX_FINALLY_NEST` で頭打ちにして、それ以上は bail** する（ツリーウォークへ落とす）。
    pub(super) fn compile_finally_copy(&mut self, fin: &[Stmt], extra: u16) -> Option<()> {
        if self.in_finally >= MAX_FINALLY_NEST {
            bail("finally-nest-limit", None);
            return None;
        }
        let saved_base = self.stmt_base;
        if extra > 0 {
            self.stmt_base = self.stmt_base.map(|d| d + extra);
        }
        self.in_finally += 1;
        let mut ok = true;
        for s in fin {
            if self.compile_stmt(s).is_none() {
                ok = false;
                break;
            }
        }
        self.in_finally -= 1;
        self.stmt_base = saved_base;
        if ok {
            Some(())
        } else {
            None
        }
    }

    /// `try/except` / `try/finally` / `try/except/finally` をコンパイルする。
    ///
    /// 3 点セットは **`try/except` を `try/finally` で包む**形に落とす（#27-c）。
    /// これは Python と同じ等価変形で、finally とハンドラの相互作用を別実装しなくて済む:
    /// - 本体が正常終了 → 内側 `PopTry` → 外側 `PopTry` → finally
    /// - ハンドラがマッチ → ハンドラ本体 → 外側 `PopTry` → finally
    /// - どのハンドラにもマッチしない → 内側の `Reraise` が**外側の landing pad** へ落ちる
    ///   （例外時に `run` がハンドラを pop 済みなので、内側で捕まり直すことはない）→ finally → 再送出
    /// - ハンドラ本体が例外を出した → 同上（外側 landing pad）→ finally → 再送出
    pub(super) fn compile_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Option<()> {
        match finally_body {
            None => self.compile_try_except(body, handlers),
            Some(fin) => self.compile_try_finally(body, handlers, fin),
        }
    }

    /// `try: <body> except ...:` をハンドラスタック（SetupTry/PopTry）＋ landing pad にコンパイルする。
    pub(super) fn compile_try_except(&mut self, body: &[Stmt], handlers: &[ExceptHandler]) -> Option<()> {
        // try を飛び越える制御フロー（break/continue/block_return/loop_yield）があると
        // SetupTry ハンドラが残るため bail。return は run から即復帰しハンドラは破棄されるので OK。
        // #37: `break`/`continue`/`block_return` は `emit_unwind_tries` が `PopTry` を
        // 出して正しく抜けるので、ここで弾く必要はなくなった（旧 `has_escape` の門は削除済み）。

        let setup = self.emit(Op::SetupTry(0)); // handler_ip は後でパッチ
        // 本体の間だけハンドラが 1 つ多い（#34）。ここから外側ループへ跳ぶ `break` は
        // `PopTry` を通らないので、跳ぶ側が同じ数だけ戻す必要がある（`finally` は無いので `None`）。
        self.try_stack.push(None);
        let r = (|| {
            for s in body {
                self.compile_stmt(s)?;
            }
            Some(())
        })();
        self.try_stack.pop();
        r?;
        self.emit(Op::PopTry);
        let mut end_jumps = vec![self.emit(Op::Jump(0))]; // 正常終了 → END
        // landing pad: 例外時にここへ来る（スタック = [exc]）。
        let land = self.here();
        self.chunk.code[setup] = Op::SetupTry(land);
        for h in handlers {
            match &h.exc_type {
                // bare `except:` — 無条件マッチ。
                None => {
                    self.bind_or_pop_exc(h)?;
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // bare except 以降のハンドラは到達不能（パーサが末尾を保証）。
                }
                // `except E [as e]:` — 型マッチ。
                Some(type_name) => {
                    self.emit(Op::Dup); // [exc, exc]
                    let ni = self.add_name(type_name);
                    self.emit(Op::ExcMatch(ni)); // [exc, bool]
                    let jf = self.emit(Op::JumpIfFalse(0)); // bool を pop・不一致は next へ
                    self.bind_or_pop_exc(h)?; // 一致: [exc] を束縛 or 破棄
                    for s in &h.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        // どのハンドラにもマッチしなかった: [exc] を捨てて再送出。
        self.emit(Op::Pop);
        self.emit(Op::Reraise);
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Some(())
    }

    /// `try: <body> [except ...:] finally: <fin>`。正常経路・例外経路の両方で finally を走らせる。
    ///
    /// `handlers` が空でなければ、**内側に `try/except` をそのまま埋め込む**（`compile_try` の doc）。
    pub(super) fn compile_try_finally(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        fin: &[Stmt],
    ) -> Option<()> {
        // #37/#40: 本体・ハンドラ・`finally` 本体のいずれの脱出も
        // `emit_unwind_tries` / `compile_finally_copy` が扱う（旧 `has_escape` の門は削除済み）。
        let setup = self.emit(Op::SetupTry(0));
        // 本体から跳ぶ脱出は `finally` を走らせてから跳ぶ（#37）。そのために本体を
        // **この finally 付きで**登録する。⚠ **式の中の `break` を見る静的判定は無い**ので、
        // ブロック式の中の `break` はこの登録だけが捕まえる。
        self.try_stack.push(Some(fin.to_vec()));
        let r = (|| {
            if handlers.is_empty() {
                for s in body {
                    self.compile_stmt(s)?;
                }
            } else {
                self.compile_try_except(body, handlers)?;
            }
            Some(())
        })();
        self.try_stack.pop();
        r?;
        self.emit(Op::PopTry);
        // 正常経路の finally（オペランドは積まれていない）。
        self.compile_finally_copy(fin, 0)?;
        let normal_jump = self.emit(Op::Jump(0)); // END
        // 例外 landing pad: スタック = [exc]。finally はスタック中立なので [exc] は底に残る。
        let land = self.here();
        self.chunk.code[setup] = Op::SetupTry(land);
        // 例外経路の finally。⚠ `[exc]` が 1 つ積まれた上で走る（#40）。
        // ここから `break`/`return` で跳ぶと `Pop`/`Reraise` を飛ばす＝**例外は破棄される**
        // （ツリーウォーク・Python と同じ）。
        self.compile_finally_copy(fin, 1)?;
        self.emit(Op::Pop); // [exc] を捨てる
        self.emit(Op::Reraise); // 再伝播（current_exception は設定済み）
        let end = self.here();
        self.patch_jump(normal_jump, end);
        Some(())
    }

    /// except ハンドラ landing の [exc] を、別名があれば slot へ束縛、なければ捨てる。
    pub(super) fn bind_or_pop_exc(&mut self, h: &ExceptHandler) -> Option<()> {
        if let Some(alias) = &h.name {
            let slot = *self.slots.get(alias)?;
            self.emit(Op::StoreLocal(slot)); // exc を束縛（消費）
        } else {
            self.emit(Op::Pop); // exc を破棄
        }
        Some(())
    }

    /// `match` 文を temp slot + ジャンプ列にコンパイルする（`exec_match_stmt`＝#33 で削除、と同一意味論）。
    /// サブジェクトを一度だけ評価して temp に格納し、各アームを順に照合する。
    pub(super) fn compile_match(&mut self, subject: &Expr, arms: &[MatchArm]) -> Option<()> {
        // サブジェクトを一度評価して temp に退避（各アームの照合で使い回す）。
        let temp = self.alloc_temp()?;
        self.compile_expr(subject)?;
        self.emit(Op::StoreLocal(temp));

        let mut end_jumps: Vec<usize> = Vec::new();
        for arm in arms {
            match &arm.pattern {
                // ワイルドカード `case _:` は無条件マッチ。
                MatchPattern::Case(Expr::Ident { name: n, .. }) if n == "_" => {
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    // 以降のアームは到達不能だが害はない（emit を続けても正しさは保たれる）。
                }
                MatchPattern::Case(pattern_expr) => {
                    self.emit(Op::LoadLocal(temp));
                    self.compile_expr(pattern_expr)?;
                    self.emit(Op::Bin(BinOp::Eq)); // subject == pattern（apply_binop_dyn 委譲）
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
                MatchPattern::IsType(type_name) => {
                    self.emit(Op::LoadLocal(temp));
                    let ni = self.add_name(type_name);
                    self.emit(Op::IsType(ni));
                    let jf = self.emit(Op::JumpIfFalse(0));
                    for s in &arm.body {
                        self.compile_stmt(s)?;
                    }
                    end_jumps.push(self.emit(Op::Jump(0)));
                    let next = self.here();
                    self.patch_jump(jf, next);
                }
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        self.free_temp();
        Some(())
    }
}
