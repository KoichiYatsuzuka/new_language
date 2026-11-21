// llvm_codegen/stmt.rs — 文の LLVM IR 生成: gen_stmts / gen_stmt。

use crate::ast::{Expr, MatchPattern, Stmt};
use super::*;

impl<'a> GenCtx<'a> {
    pub fn gen_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts { self.gen_stmt(s); }
    }

    pub(super) fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            // 宣言の複製は VM と同じ（`let` / `mut` は `bind_decl`・`const` はそのまま・1-6）。
            // ⚠ 以前は 3 つとも単なる格納で、`mut q = o.inner` が `o.inner` と共有されていた。
            Stmt::Let(name, _, expr, _) | Stmt::Mut(name, _, expr, _) | Stmt::Const(name, _, expr, _) => {
                let (v, vt) = self.gen_expr(expr);
                let st = store_ty(vt);
                let ptr = self.alloca_var(name, st);
                let coerced = match st {
                    Ty::Int   => self.to_i64(&v, vt),
                    Ty::Float => self.to_f64(&v, vt),
                    _         => {
                        let h = self.to_handle(&v, vt);
                        match stmt {
                            Stmt::Let(..) => self.bind_decl(&h, vt, false, expr),
                            Stmt::Mut(..) => self.bind_decl(&h, vt, true, expr),
                            Stmt::Const(..) => h,
                            _ => unreachable!("outer arm matches only let / mut / const"),
                        }
                    }
                };
                self.store_val(st, &coerced, &ptr.clone());
            }
            Stmt::Assign { name, value, .. } => {
                let (v, vt) = self.gen_expr(value);
                if let Some((ptr, st)) = self.locals.get(name).cloned() {
                    let coerced = match st {
                        Ty::Int   => self.to_i64(&v, vt),
                        Ty::Float => self.to_f64(&v, vt),
                        _         => self.to_handle(&v, vt),
                    };
                    self.store_val(st, &coerced, &ptr);
                }
            }
            Stmt::CompoundAssign { name, op, value, .. } => {
                let (ptr, lt) = self.locals.get(name).cloned()
                    .unwrap_or_else(|| ("%_UNDEF".to_string(), Ty::Handle));
                let lv_raw = self.fresh_reg();
                let t_str  = llvm_ty(lt);
                self.ec(&format!("{lv_raw} = load {t_str}, ptr {ptr}"));
                let (rv, rt) = self.gen_expr(value);
                let (res, rest) = self.specialize_binop(op, &lv_raw, lt, &rv, rt);
                let coerced = match lt {
                    Ty::Int   => self.to_i64(&res, rest),
                    Ty::Float => self.to_f64(&res, rest),
                    _         => self.to_handle(&res, rest),
                };
                self.store_val(lt, &coerced, &ptr);
            }
            // 代入は VM の `SetAttr` / `SetTraitAttr` と同じ入口（`attr_assign_evaled` / `trait_assign_evaled`・1-4）。
            // 代入先の形は `stmt_eligible` が `obj.attr` / `obj::T.attr` に絞っている。
            Stmt::AttrAssign { target, value, span: _ } => {
                let (object, trait_name, attr) = attr_target(target);
                // 番地で呼ぶ変種の本体の `self.f = v`（7-14）: 検査なしで書く（呼ぶ側が不変でないことを確かめてある）
                if trait_name.is_none() && self.blk_self.is_some() && ident_name(object) == Some("self") {
                    let (val, vt) = self.gen_expr(value);
                    self.emit_blk_self_write(attr, &val, vt);
                    return;
                }
                // `a.b.c = v`（途中に属性が 1 段以上）で値の評価が何も起こさなければ、途中を積まない 1 回の呼び出し（12-3）
                // `a.b.x = v` で `a.b` が raw レイアウトのクラス・値の評価が何も起こさないなら、`a.b` を積まずに番地へ書く（8-7）
                if trait_name.is_none() && self.emit_nested_field_assign(object, attr, value) {
                    return;
                }
                // 値が raw レイアウトのクラスの数値のフィールドを読むなら、読み書きする番地を先に確かめる（7-17 ②）
                if trait_name.is_none() && self.emit_checked_nested_assign(object, attr, value) {
                    return;
                }
                if trait_name.is_none() && self.emit_attr_path_assign(target, value) {
                    return;
                }
                let (obj, ot) = self.gen_expr(object);
                let (val, vt) = self.gen_expr(value);
                let oh  = self.to_handle(&obj, ot);
                match trait_name {
                    // `int` / `float` の値は箱に入れずに書く（4-2）
                    None => self.emit_field_assign(object, &oh, attr, &val, vt),
                    Some(_) => {
                        let vh = self.to_handle(&val, vt);
                        self.emit_attr_store(&oh, trait_name, attr, &vh);
                    }
                }
            }
            Stmt::AttrCompoundAssign { target, op, value, span: _ } => {
                let (object, trait_name, attr) = attr_target(target);
                // 番地で呼ぶ変種の本体の `self.f op= v`（7-14）: 右辺 → 読み → 演算 → 書き（下と同じ順）
                if trait_name.is_none() && self.blk_self.is_some() && ident_name(object) == Some("self") {
                    let (rv, rt) = self.gen_expr(value);
                    let (old, old_t) = self.emit_blk_self_read(attr);
                    let (res, rest) = self.specialize_binop(op, &old, old_t, &rv, rt);
                    self.emit_blk_self_write(attr, &res, rest);
                    return;
                }
                // ⚠⚠ ツリーウォーク（`attr_assign`）・VM と同じ順で評価する: 右辺 → 受け手（読み）→ 二項演算 →
                //   受け手（書き・もう 1 度評価）。以前は受け手を先に読んで右辺を後に評価しており、右辺が同じ
                //   フィールドを書き換えると（`self.n += self.bump()`）その書き換えが消えた（§0.3 ⑪・4-2 で発見・
                //   非コンパイル 11・コンパイル済み 1）。受け手が名前（`self`・ローカル）なら 2 度目の評価は省く。
                let (rv, rt) = self.gen_expr(value);
                let (obj, ot) = self.gen_expr(object);
                let oh = self.to_handle(&obj, ot);
                // 型の分かっている `int` / `float` のフィールドは、型付きで読み → ネイティブの演算 → 型付きで書く（4-2）
                let known = match (trait_name, target) {
                    (None, Expr::Attr { node_id, .. }) => self.field_ty_resolved(object, attr, *node_id),
                    _ => None,
                };
                // raw レイアウトでないクラスの数値のフィールドは、値の番地を 1 回のコールバックで取って書く（7-16）
                if let (None, Some(kt @ (Ty::Int | Ty::Float))) = (trait_name, known) {
                    if self.fused_compound_ok(object, attr, op, kt, rt) {
                        self.emit_fused_num_compound(object, &oh, attr, op, kt, &rv, rt);
                        return;
                    }
                }
                let (old, old_t) = match (trait_name, known) {
                    (None, Some(t @ (Ty::Int | Ty::Float))) => self.emit_field_read(object, &oh, attr, Some(t)),
                    _ => (self.emit_attr_load(&oh, trait_name, attr), Ty::Handle),
                };
                let (res, rest) = self.specialize_binop(op, &old, old_t, &rv, rt);
                let oh = if ident_name(object).is_some() {
                    oh
                } else {
                    let (obj2, ot2) = self.gen_expr(object);
                    self.to_handle(&obj2, ot2)
                };
                match trait_name {
                    None => self.emit_field_assign(object, &oh, attr, &res, rest),
                    Some(_) => {
                        let rh = self.to_handle(&res, rest);
                        self.emit_attr_store(&oh, trait_name, attr, &rh);
                    }
                }
            }
            Stmt::Return(Some(expr), _) => {
                let (v, vt) = self.gen_expr(expr);
                // ループの中の `return`: 囲む `for` のイテレータを全部捨てる（11-1）。返す値はアリーナにあるので影響しない
                self.drop_iters_from(0);
                if self.typed_mode {
                    // typed ABI: 生値を %_ret スロットへ格納して status 0 を返す。
                    self.emit_typed_return(&v, vt);
                } else if self.current_fn_ret == Ty::Float {
                    // Float-returning _impl: return raw double, no boxing.
                    let f = self.to_f64(&v, vt);
                    if !self.terminated {
                        self.ec(&format!("ret double {f}"));
                        self.terminated = true;
                    }
                } else if self.current_fn_ret == Ty::Int {
                    // `int` を返す _impl は生の i64 を返す（4-1・箱に入れるのは `_tl` の境界）
                    let n = self.to_i64(&v, vt);
                    self.ret_handle(&n);
                } else {
                    let h = self.to_handle(&v, vt);
                    self.ret_handle(&h);
                }
            }
            Stmt::Return(None, _) => {
                self.drop_iters_from(0);
                if self.typed_mode {
                    if !self.terminated {
                        let t = llvm_ty(self.current_fn_ret);
                        let zero = if self.current_fn_ret == Ty::Float { "0.0" } else { "0" };
                        self.ec(&format!("store {t} {zero}, ptr %_ret"));
                        self.ec("ret i32 0");
                        self.terminated = true;
                    }
                } else if self.current_fn_ret == Ty::Float {
                    if !self.terminated { self.ec("ret double 0.0"); self.terminated = true; }
                } else {
                    self.ret_handle("0");
                }
            }
            Stmt::Pass => {}
            Stmt::Break => {
                if let Some((_, exit)) = self.loop_stack.last().cloned() {
                    self.br(&exit);
                }
            }
            Stmt::Continue => {
                if let Some((header, _)) = self.loop_stack.last().cloned() {
                    self.br(&header);
                }
            }
            // 式の文は値を使わない: いちばん外の呼び出しの遅い道は、結果を残さずに巻き戻せる（7-17・`discarded_func`）。
            // 呼び出しの `func` の式（AST のノード）で覚えるので、引数の中の呼び出しは当たらない
            Stmt::Expr(e) => {
                self.discarded_func = match e {
                    Expr::Call { func, .. } => Some(&**func as *const Expr),
                    _ => None,
                };
                self.gen_expr(e);
                self.discarded_func = None;
            }
            Stmt::Freeze(..) => {}
            Stmt::Block(body) => self.gen_stmts(body),

            Stmt::BlockReturn(expr, _) => {
                if let Some(ctx) = self.block_stack.last().cloned() {
                    let (v, vt) = self.gen_expr(expr);
                    let h = self.to_handle(&v, vt);
                    self.ec(&format!("store i64 {h}, ptr {}", ctx.result_al));
                    // 式の中の `for` から飛び出すなら、そのイテレータを捨てる（式の出口は捨てない・11-1）
                    self.drop_iters_from(ctx.iter_depth);
                    self.br(&ctx.exit_label);
                }
            }

            Stmt::LoopYield(expr, _) => {
                if let Some(ctx) = self.block_stack.last().cloned() {
                    if let Some(la) = ctx.list_al.clone() {
                        let list_cur = self.fresh_reg();
                        self.ec(&format!("{list_cur} = load i64, ptr {la}"));
                        let (v, vt) = self.gen_expr(expr);
                        let h = self.to_handle(&v, vt);
                        let updated = self.call_cb(CB_LIST_APPEND, &[format!("i64 {list_cur}"), format!("i64 {h}")]);
                        self.ec(&format!("store i64 {updated}, ptr {la}"));
                        self.note_handle_store(&la);
                    }
                }
            }

            Stmt::Yield(expr, _) => {
                // Inside a GenDef body compiled as eager accumulator:
                // the outermost block_stack entry is the generator's list alloca.
                if let Some(ctx) = self.block_stack.first().cloned() {
                    if let Some(la) = ctx.list_al.clone() {
                        let list_cur = self.fresh_reg();
                        self.ec(&format!("{list_cur} = load i64, ptr {la}"));
                        let (v, vt) = self.gen_expr(expr);
                        let h = self.to_handle(&v, vt);
                        let updated = self.call_cb(CB_LIST_APPEND, &[format!("i64 {list_cur}"), format!("i64 {h}")]);
                        self.ec(&format!("store i64 {updated}, ptr {la}"));
                        self.note_handle_store(&la);
                    }
                }
            }

            Stmt::Raise { exc: Some(exc_expr), .. } => {
                if self.typed_mode {
                    // typed ABI: ErrSlot に静的文字列を書き込み status 1 を返す。
                    // 対応パターンは `raise Name("literal")` / `raise Name()` のみ。
                    self.emit_typed_raise(exc_expr);
                    return;
                }
                // CB_RAISE(type_handle, msg_handle) → returns TL_EXCEPTION
                let (type_h, msg_h) = match exc_expr {
                    Expr::Call { func, args, .. } => {
                        // func → type name, args[0] → message (or None)
                        let (fv, ft) = self.gen_expr(func);
                        let th = self.to_handle(&fv, ft);
                        let mh = if let Some(a) = args.first() {
                            let (av, at) = self.gen_expr(a.expr());
                            self.to_handle(&av, at)
                        } else {
                            "0".to_string() // TL_NONE
                        };
                        (th, mh)
                    }
                    other => {
                        let (v, vt) = self.gen_expr(other);
                        let h = self.to_handle(&v, vt);
                        (h, "0".to_string())
                    }
                };
                self.call_cb(CB_RAISE, &[format!("i64 {type_h}"), format!("i64 {msg_h}")]);
                self.ret_handle("-2"); // TL_EXCEPTION sentinel
            }
            Stmt::Raise { exc: None, .. } => {
                // bare re-raise: signal exception with empty type
                let empty = self.str_const(b"");
                self.call_cb(CB_RAISE, &["i64 0".to_string(), "i64 0".to_string()]);
                let _ = empty;
                self.ret_handle("-2");
            }

            Stmt::If { branches, else_body, span: _ } => {
                let merge = self.fresh_blk();
                for (i, (cond, body)) in branches.iter().enumerate() {
                    let then_blk = self.fresh_blk();
                    let next_blk = if i + 1 < branches.len() || else_body.is_some() {
                        self.fresh_blk()
                    } else {
                        merge.clone()
                    };
                    let (cv, ct) = self.gen_expr(cond);
                    let cc = self.to_cond(&cv, ct);
                    self.br_cond(&cc, &then_blk, &next_blk);
                    self.start_block(&then_blk);
                    self.gen_stmts(body);
                    self.br(&merge);
                    if next_blk != merge {
                        self.start_block(&next_blk);
                    }
                }
                if let Some(else_stmts) = else_body {
                    self.gen_stmts(else_stmts);
                    self.br(&merge);
                }
                self.start_block(&merge);
            }

            Stmt::While { cond, body, span: _ } => {
                let cond_blk = self.fresh_blk();
                let body_blk = self.fresh_blk();
                let exit_blk = self.fresh_blk();
                let pre_pos = self.loop_pre_pos();
                self.br(&cond_blk);
                self.start_block(&cond_blk);
                let (cv, ct) = self.gen_expr(cond);
                let cc = self.to_cond(&cv, ct);
                self.br_cond(&cc, &body_blk, &exit_blk);
                self.start_block(&body_blk);
                self.gen_loop_body(body, &cond_blk, &exit_blk, pre_pos, |_| {});
                self.start_block(&exit_blk);
            }

            Stmt::For { targets, iter, body, mut_targets: _, span: _ } => {
                let target   = &targets[0];
                let exit_blk = self.fresh_blk();
                let loop_blk = self.fresh_blk();

                // ── Flat list iteration path ──────────────────────────────────
                // Activated when the iterator is a `let fixed_list[ClassName]` param.
                let flat_info: Option<FlatListInfo> = ident_name(iter)
                    .and_then(|n| self.flat_list_params.get(n).cloned());

                if let Some(ref fi) = flat_info {
                    // Get flat data ptr and length (2 callbacks, paid once).
                    let (iv, it) = self.gen_expr(iter);
                    let ih = self.to_handle(&iv, it);
                    let flat_ptr_i64 = self.call_cb(CB_FLAT_DATA_PTR, &[format!("i64 {ih}")]);
                    let flat_ptr_al  = format!("%_fptr{}", self.reg); self.reg += 1;
                    self.alloca_buf.push_str(&format!("  {flat_ptr_al} = alloca i64, align 8\n"));
                    self.ec(&format!("store i64 {flat_ptr_i64}, ptr {flat_ptr_al}"));
                    let flat_len_i64 = self.call_cb(CB_FLAT_LEN, &[format!("i64 {ih}")]);
                    let flat_len_al  = format!("%_flen{}", self.reg); self.reg += 1;
                    self.alloca_buf.push_str(&format!("  {flat_len_al} = alloca i64, align 8\n"));
                    self.ec(&format!("store i64 {flat_len_i64}, ptr {flat_len_al}"));

                    // Loop index alloca.
                    let idx_al = format!("%_fidx{}", self.reg); self.reg += 1;
                    self.alloca_buf.push_str(&format!("  {idx_al} = alloca i64, align 8\n"));
                    self.ec(&format!("store i64 0, ptr {idx_al}"));

                    // Allocate pre-read allocas for each leaf; register in preread_fields.
                    // Keys are "{target}.{leaf.path}" (e.g. "item.v", "item.start.x").
                    for leaf in &fi.leaves {
                        let al   = format!("%_prf_{}_{}", target, leaf.path.replace('.', "_"));
                        let tstr = llvm_ty(leaf.ty);
                        self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                        self.preread_fields.insert(format!("{target}.{}", leaf.path), (al, leaf.ty));
                    }

                    // Dummy alloca for target handle (non-field use returns TL_NONE/0).
                    let tgt_al = self.alloca_var(target, Ty::Handle);
                    self.ec(&format!("store i64 0, ptr {tgt_al}"));

                    let pre_pos = self.loop_pre_pos();
                    self.br(&loop_blk);
                    self.start_block(&loop_blk);
                    let idx_r   = self.fresh_reg();
                    let len_r   = self.fresh_reg();
                    let is_done = self.fresh_reg();
                    self.ec(&format!("{idx_r}   = load i64, ptr {idx_al}"));
                    self.ec(&format!("{len_r}   = load i64, ptr {flat_len_al}"));
                    self.ec(&format!("{is_done} = icmp sge i64 {idx_r}, {len_r}"));
                    let body_blk = self.fresh_blk();
                    self.br_cond(&is_done, &exit_blk, &body_blk);
                    self.start_block(&body_blk);

                    // Compute element base pointer: flat_ptr + idx * stride.
                    let fp_cur  = self.fresh_reg();
                    let fp_ptr  = self.fresh_reg();
                    let eb      = self.fresh_reg();
                    let ep      = self.fresh_reg();
                    let stride  = fi.stride as i64;
                    self.ec(&format!("{fp_cur} = load i64, ptr {flat_ptr_al}"));
                    self.ec(&format!("{fp_ptr} = inttoptr i64 {fp_cur} to ptr"));
                    self.ec(&format!("{eb}     = mul i64 {idx_r}, {stride}"));
                    self.ec(&format!("{ep}     = getelementptr i8, ptr {fp_ptr}, i64 {eb}"));

                    // Load each leaf field into its pre-read alloca using byte-level GEP.
                    for leaf in &fi.leaves {
                        let byte_off = leaf.byte_offset as i64;
                        let prf_al  = format!("%_prf_{}_{}", target, leaf.path.replace('.', "_"));
                        let tstr    = llvm_ty(leaf.ty);
                        let fval    = self.fresh_reg();
                        if byte_off == 0 {
                            self.ec(&format!("{fval} = load {tstr}, ptr {ep}"));
                        } else {
                            let fptr = self.fresh_reg();
                            self.ec(&format!("{fptr} = getelementptr i8, ptr {ep}, i64 {byte_off}"));
                            self.ec(&format!("{fval} = load {tstr}, ptr {fptr}"));
                        }
                        self.ec(&format!("store {tstr} {fval}, ptr {prf_al}"));
                    }

                    // 添字は戻り口で進める（`continue` も戻り口へ行く）。⚠ 以前は `continue` が見出しへ直接飛び、
                    //   添字を進めずに同じ要素を繰り返していた（終わらないループ・9-1 で発見）。
                    let (idx_r2, idx_al2) = (idx_r.clone(), idx_al.clone());
                    self.gen_loop_body(body, &loop_blk, &exit_blk, pre_pos, move |s| {
                        let idx_next = s.fresh_reg();
                        s.ec(&format!("{idx_next} = add i64 {idx_r2}, 1"));
                        s.ec(&format!("store i64 {idx_next}, ptr {idx_al2}"));
                    });
                    self.start_block(&exit_blk);

                } else {
                // 5-2: `for i in range(..)`（組み込みの range・引数が全部 int・刻みが無いか 0 でないリテラル）は
                // カウンタのループにする（`CB_ITER_FROM` / `CB_ITER_NEXT` もループ変数の箱詰めもしない）。
                // 載らなければ、評価済みの引数で range を作って今までの道へ（引数は 1 回だけ評価する）。
                let ih = match range_call_args(iter).filter(|_| self.plain_builtin(range_callee(iter), "range")) {
                    Some(rargs) => {
                        let evaled: Vec<(String, Ty)> = rargs.iter().map(|a| self.gen_expr(a)).collect();
                        if let Some(step) = range_counter_step(&evaled, rargs) {
                            self.emit_range_loop(target, &evaled, step, body, &loop_blk, &exit_blk);
                            return;
                        }
                        let handles: Vec<String> = evaled.iter().map(|(v, vt)| self.to_handle(v, *vt)).collect();
                        self.emit_builtin_call("range", &handles)
                    }
                    None => {
                        let (iv, it) = self.gen_expr(iter);
                        self.to_handle(&iv, it)
                    }
                };
                // ── Original CB_ITER_FROM / CB_ITER_NEXT path ────────────────
                let iter_h   = self.call_cb(CB_ITER_FROM, &[format!("i64 {ih}")]);
                let iter_al  = format!("%_iter{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {iter_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {iter_h}, ptr {iter_al}"));
                self.iter_scopes.push(iter_al.clone());

                // 名前の場所は `alloca_var` が決める（同じ名前の 2 つ目の `for`・1-10）
                let tgt_al = self.alloca_var(target, Ty::Handle);

                let fuse = self.new_loop_fuse_slot();
                let pre_pos = self.loop_pre_pos();
                self.br(&loop_blk);
                self.start_block(&loop_blk);
                let next = self.emit_iter_next_at(&iter_al, &fuse);
                self.ec(&format!("store i64 {next}, ptr {tgt_al}"));
                self.note_handle_store(&tgt_al);
                let is_done = self.fresh_reg();
                self.ec(&format!("{is_done} = icmp eq i64 {next}, -1"));
                let body_blk2 = self.fresh_blk();
                self.br_cond(&is_done, &exit_blk, &body_blk2);
                self.start_block(&body_blk2);
                self.loop_fuse_slot = Some(fuse);
                self.gen_loop_body(body, &loop_blk, &exit_blk, pre_pos, |_| {});
                self.start_block(&exit_blk);
                self.drop_iter_scope(&iter_al);
                } // end else (original path)
            }

            Stmt::Match { subject, arms, .. } => {
                let (sv, st) = self.gen_expr(subject);
                let subj_h   = self.to_handle(&sv, st);
                let merge    = self.fresh_blk();

                // store subject in alloca for reuse
                let subj_al = format!("%_subj{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {subj_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {subj_h}, ptr {subj_al}"));

                for (i, arm) in arms.iter().enumerate() {
                    let is_last  = i == arms.len() - 1;
                    let body_blk = self.fresh_blk();
                    let next_blk = if is_last { merge.clone() } else { self.fresh_blk() };

                    let subj_r = self.fresh_reg();
                    self.ec(&format!("{subj_r} = load i64, ptr {subj_al}"));

                    match &arm.pattern {
                        MatchPattern::Case(e) if ident_name(e) == Some("_") => {
                            self.br(&body_blk);
                        }
                        MatchPattern::Case(pat_expr) => {
                            let (pv, pt) = self.gen_expr(pat_expr);
                            let ph  = self.to_handle(&pv, pt);
                            let eq  = self.call_cb(CB_BINOP, &["i32 7".to_string(), format!("i64 {subj_r}"), format!("i64 {ph}")]);
                            let cnd = self.to_cond(&eq, Ty::Handle);
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                        MatchPattern::IsType(type_name) => {
                            let ptr = self.str_const(type_name.as_bytes());
                            let len = type_name.len() as i32;
                            let r   = self.call_cb(CB_IS_TYPE, &[format!("i64 {subj_r}"), ptr, format!("i32 {len}")]);
                            let cnd = self.fresh_reg();
                            self.ec(&format!("{cnd} = icmp eq i64 {r}, 1"));
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                    }

                    self.start_block(&body_blk);
                    self.gen_stmts(&arm.body);
                    self.br(&merge);

                    if !is_last {
                        self.start_block(&next_blk);
                    }
                }
                self.start_block(&merge);
            }

            _ => {} // ineligible statements are filtered before we get here
        }
    }

    /// `for target in range(start, stop[, step])` のカウンタのループ（5-2）。`step` はコンパイル時に分かる 0 でない値。
    /// 刻みは本体の前に進めるので、`continue`（見出しへ飛ぶ）でも次の値になる。ループ変数は `int` の場所。
    fn emit_range_loop(
        &mut self,
        target: &str,
        evaled: &[(String, Ty)],
        step: i64,
        body: &[Stmt],
        header: &str,
        exit: &str,
    ) {
        let (start, stop) = match evaled {
            [stop] => ("0".to_string(), stop.0.clone()),
            [start, stop, ..] => (start.0.clone(), stop.0.clone()),
            [] => unreachable!("range_call_args gives 1 to 3 arguments"),
        };
        let cnt = format!("%_rng{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{cnt} = alloca i64, align 8"));
        self.ec(&format!("store i64 {start}, ptr {cnt}"));
        let tgt = self.alloca_var(target, Ty::Int);
        let pre_pos = self.loop_pre_pos();
        self.br(header);
        self.start_block(header);
        let c = self.fresh_reg();
        self.ec(&format!("{c} = load i64, ptr {cnt}"));
        let cond = self.fresh_reg();
        let pred = if step > 0 { "slt" } else { "sgt" };
        self.ec(&format!("{cond} = icmp {pred} i64 {c}, {stop}"));
        let body_blk = self.fresh_blk();
        self.br_cond(&cond, &body_blk, exit);
        self.start_block(&body_blk);
        self.ec(&format!("store i64 {c}, ptr {tgt}"));
        let next = self.fresh_reg();
        self.ec(&format!("{next} = add i64 {c}, {step}"));
        self.ec(&format!("store i64 {next}, ptr {cnt}"));
        self.gen_loop_body(body, header, exit, pre_pos, |_| {});
        self.start_block(exit);
    }

    /// `x.f op= v`（型 `kt` の数値のフィールド・右辺は評価済みで型 `rt`）を、値の番地を返すコールバック
    /// （`CB_NUM_FIELD_PTR`）の 1 回で行えるか（copy_alloc_speed_plan.md 7-16）。
    /// - 受け手が名前（`self`・ローカル）: VM は名前でない受け手を書く前にもう 1 度評価する（`f().n += 1` は `f` を 2 回呼ぶ）
    /// - 受け手のクラスが raw レイアウトのクラスでない: raw レイアウトなら番地から直接読み書きする道（8-3・8-4）の方が速い
    /// - 演算の結果の型がフィールドの型と同じ形で、演算が何も呼ばない: `int` は `+` `-` `*`（`int` の右辺）、`float` は
    ///   `+` `-` `*` `/`（`int` / `float` の右辺）。`int` のフィールドへ `float` を足すと型の誤りになるので今までの道
    pub(super) fn fused_compound_ok(&self, object: &Expr, attr: &str, op: &crate::ast::BinOp, kt: Ty, rt: Ty) -> bool {
        use crate::ast::BinOp;
        if self.typed_mode || self.blk_mode || self.terminated || ident_name(object).is_none() {
            return false;
        }
        if self.receiver_class(object).is_some_and(|r| self.direct_read_plan(&r.class, attr, kt).is_some()) {
            return false;
        }
        match kt {
            Ty::Int => rt == Ty::Int && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul),
            Ty::Float => matches!(rt, Ty::Int | Ty::Float) && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div),
            _ => false,
        }
    }

    /// [`Self::fused_compound_ok`] の形のコード: 番地を取り（`CB_NUM_FIELD_PTR`）、空でなければ読んで演算して書く
    /// （その間に何も呼ばない）。空なら今までの道（同じ IC のセルで型付きで読む → 演算 → `emit_field_assign`）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_fused_num_compound(
        &mut self,
        object: &Expr,
        oh: &str,
        attr: &str,
        op: &crate::ast::BinOp,
        kt: Ty,
        rv: &str,
        rt: Ty,
    ) {
        let cell = self.ic_cell();
        let name = self.str_const(attr.as_bytes());
        let len = attr.len();
        let kind = i32::from(kt == Ty::Float);
        let p = self.call_cb(CB_NUM_FIELD_PTR, &[
            format!("i64 {oh}"), name.clone(), format!("i32 {len}"), cell.clone(), format!("i32 {kind}"),
        ]);
        let (b_fast, b_slow, b_done) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        let empty = self.fresh_reg();
        self.ec(&format!("{empty} = icmp eq ptr {p}, null"));
        self.br_cond(&empty, &b_slow, &b_fast);

        self.start_block(&b_fast);
        let t = llvm_ty(kt);
        let old = self.fresh_reg();
        self.ec(&format!("{old} = load {t}, ptr {p}, align 8"));
        let (res, rest) = self.specialize_binop(op, &old, kt, rv, rt);
        debug_assert_eq!(rest, kt, "fused_compound_ok only lets through operations that keep the field's type");
        self.ec(&format!("store {t} {res}, ptr {p}, align 8"));
        self.br(&b_done);

        // 外れ: 読み出しは同じ IC のセルで（外れの道がセルを埋め、次から番地が取れる）
        self.start_block(&b_slow);
        let cb = if kt == Ty::Float { CB_GET_FLOAT_FIELD_IC } else { CB_GET_INT_FIELD_IC };
        let old2 = self.call_cb(cb, &[format!("i64 {oh}"), name, format!("i32 {len}"), cell]);
        let (res2, rest2) = self.specialize_binop(op, &old2, kt, rv, rt);
        self.emit_field_assign(object, oh, attr, &res2, rest2);
        self.br(&b_done);
        self.start_block(&b_done);
    }

    /// `for` の見出しが読む保存点の置き場を作り、ループに入るたびに -1（巻き戻さない）にする（7-15）。ループの本体が値を
    /// 積み、周の終わりに残す値が無ければ、`gen_loop_body` がループに入る所で保存点を書く。⚠ `loop_pre_pos` より前に呼ぶ
    /// （保存点の書き込みは -1 の後に差し込まれる）。
    pub(super) fn new_loop_fuse_slot(&mut self) -> String {
        let slot = format!("%_itsave{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{slot} = alloca i64, align 8"));
        self.ec(&format!("store i64 -1, ptr {slot}"));
        slot
    }

    /// `for` の見出しの取り出し（7-15）: 置き場の保存点（-1 なら巻き戻さない）まで巻き戻してから次の要素（`CB_ITER_NEXT_AT`）。
    pub(super) fn emit_iter_next_at(&mut self, iter_al: &str, slot: &str) -> String {
        let it = self.fresh_reg();
        self.ec(&format!("{it} = load i64, ptr {iter_al}"));
        let save = self.fresh_reg();
        self.ec(&format!("{save} = load i64, ptr {slot}"));
        self.call_cb(CB_ITER_NEXT_AT, &[format!("i64 {it}"), format!("i64 {save}")])
    }

    /// ループに入る直前（見出しへの `br` を出す前）の `code_buf` の位置。ループの前が到達しないなら `None`（9-1）。
    pub(super) fn loop_pre_pos(&self) -> Option<usize> {
        (!self.terminated).then_some(self.code_buf.len())
    }

    /// ループの本体を生成し、周の終わりの戻り口を作る（native_object_access_plan.md 9-1・D-10）。本体の終わりと
    /// `continue` は戻り口へ行き、戻り口で `latch_extra`（添字を進めるなど）の後、`header` へ戻る。
    /// 本体がアリーナへ値を積みうる（`push_sites` が増えた）ときだけ、戻り口で、ループに入る前の保存点（`pre_pos` に
    /// 後から差し込む・積まないループのコードは変わらない）まで `CB_COMPACT_MANY` で巻き戻す。残すのは本体でハンドルを
    /// 書き込んだローカル変数（`handle_store_log`）で、詰め直した新しいハンドルを書き戻す。ほかのハンドル（式の途中の値・
    /// 呼び出しの結果）は周の中でしか使わず、オブジェクト・大域・コンテナへの格納は `Value` の複製なので、捨てても読まれない。
    /// ループの前に作った値は保存点より下なので触らない。`break` は戻り口を通らない（最後の周の値はループの後も使う）。
    /// ⚠ 以前は周ごとに巻き戻さず、ループの中で作った値がいちばん外の呼び出しが終わるまで残った（`Vec3(..)` を 100 万回
    /// 作るループが最大 419 MB・1 回 1,083 ns。ファイル・外部言語のポインタも閉じるのが遅れた・§0.5）。
    /// `a.b.c = v` を `CB_ASSIGN_PATH_IC` の 1 回で書く（12-3・D-13）。載らなければ何も出さずに `false`。
    ///
    /// 載る形: 代入先が `Expr::Attr` の 2 段以上の連なり（`Trait::attr` を含まない）で、値の評価が何も起こさないもの
    /// （[`Self::effect_free_value`]）。根は今までどおり先に評価し、値を評価してから途中の段を読む。VM は途中の段を
    /// 値より先に読むが、値の評価が何も起こさないので順の違いは見分けられない（途中の読み出しはどちらの順でも同じ値）。
    fn emit_attr_path_assign(&mut self, target: &Expr, value: &Expr) -> bool {
        let mut names: Vec<&str> = Vec::new();
        let mut cur = target;
        while let Expr::Attr { object, attr, .. } = cur {
            names.push(attr.as_str());
            cur = object.as_ref();
        }
        if names.len() < 2 || !self.effect_free_value(value) {
            return false;
        }
        names.reverse();
        let (rv, rt) = self.gen_expr(cur);
        let root = self.to_handle(&rv, rt);
        let (val, vt) = self.gen_expr(value);
        let (kind, bits) = match vt {
            Ty::Int => (1, val),
            Ty::Float => {
                let b = self.fresh_reg();
                self.ec(&format!("{b} = bitcast double {val} to i64"));
                (2, b)
            }
            _ => (0, self.to_handle(&val, vt)),
        };
        let n = names.len();
        let arr = format!("%_path{}", self.reg);
        self.reg += 1;
        self.ea(&format!("{arr} = alloca [{} x i64], align 8", 3 * n));
        for (k, name) in names.iter().enumerate() {
            let sp = self.str_const(name.as_bytes());
            let cell = self.ic_cell();
            let entries = [
                sp.trim_start_matches("ptr ").to_string(),
                name.len().to_string(),
                cell.trim_start_matches("ptr ").to_string(),
            ];
            for (j, e) in entries.iter().enumerate() {
                let slot = self.fresh_reg();
                self.ec(&format!("{slot} = getelementptr inbounds [{} x i64], ptr {arr}, i32 0, i32 {}", 3 * n, 3 * k + j));
                if j == 1 {
                    self.ec(&format!("store i64 {e}, ptr {slot}"));
                } else {
                    let p = self.fresh_reg();
                    self.ec(&format!("{p} = ptrtoint ptr {e} to i64"));
                    self.ec(&format!("store i64 {p}, ptr {slot}"));
                }
            }
        }
        self.call_cb(CB_ASSIGN_PATH_IC, &[
            format!("i64 {root}"), format!("ptr {arr}"), format!("i32 {n}"), format!("i32 {kind}"), format!("i64 {bits}"),
        ]);
        true
    }

    /// `a.b.x = v`（受け手 `object` が属性 `a.b`）を、`a.b` をアリーナに積まずに番地へ書く（copy_alloc_speed_plan.md 8-7・
    /// D-13 ⑥）。出したら `true`。当てはまらなければ何も出さずに `false`。
    ///
    /// 当てはまるのは、`a.b` のクラスがこのモジュールの raw レイアウトのクラスで `x` が `mut` の数値のフィールド、かつ
    /// **値の評価が何も起こさない**とき（[`Self::effect_free_value`]）。評価の順は `a` → 値 → `a.b` の読み出し → 書き込みで、
    /// `emit_attr_path_assign`（12-3）と同じ（VM は `a.b` を値より先に読むが、値の評価が何も起こさないので見分けられない）。
    /// ⚠ 値の評価が何かを起こしうる式（呼び出し・属性の読み出しを含む）は対象外: `a.b` を値より先に読んでアリーナに
    ///   積む今までの順でないと、値の評価が `a.b` を差し替えたときに書く先が変わる（そちらは `emit_field_assign` が
    ///   ハンドルから番地を取って書く・8-4）。
    fn emit_nested_field_assign(&mut self, object: &Expr, attr: &str, value: &Expr) -> bool {
        let Expr::Attr { object: parent, attr: pattr, .. } = object else { return false };
        if !self.effect_free_value(value) {
            return false;
        }
        let Some(class) = self.receiver_class(object).map(|r| r.class) else { return false };
        if !self.direct_write_field(&class, attr) {
            return false;
        }
        let (pv, pt) = self.gen_expr(parent);
        let ph = self.to_handle(&pv, pt);
        let (val, vt) = self.gen_expr(value);
        let src = BlockSrc::Field { parent_h: &ph, attr: pattr };
        if !self.emit_direct_write(&class, src, attr, &val, vt) {
            // 値の型がフィールドに入らない（`float` を整数のフィールドへ など）: 今までの道（読んでから IC 付きで書く）
            let (oh, _) = self.emit_attr_read(&ph, pattr, None);
            self.emit_attr_assign_typed(&oh, attr, &val, vt);
        }
        true
    }

    /// `a.b.x = <数の式>` を、読み書きする番地を先に確かめてから検査なしで行う（copy_alloc_speed_plan.md 7-17 ②）。
    /// 出したら `true`。当てはまらなければ何も出さずに `false`。
    ///
    /// 当てはまるのは、`a` がローカル変数で、`a.b` がこのモジュールの raw レイアウトのクラス・`x` がそこへ値の型のまま書ける
    /// フィールド、値が数のリテラル・数のローカル変数・**確かめられる読み出し**（`L.f` / `L.c.f`: `L` はローカル変数、
    /// 受け手は raw レイアウトのクラスで `f` は数値のフィールド）の `+` / `-` / `*` だけのとき（[`Self::checked_num_ty`]）。
    ///
    /// 出すコード:
    /// 1. 書き込み先と読み出し先の番地を取る（`CB_ATTR_BLOCK` / `CB_INST_BLOCK`・覚えた番地は使い回す）
    /// 2. 確かめる: 番地が空でない（クラスが表のとおり）・読むフィールドの初期化の印・書き込み先が不変でない
    /// 3. 速い道: 検査なしで読んで演算し、書いて初期化の印を立てる
    /// 4. 遅い道: 今までの道（`a.b` を読んでアリーナに積み → 値 → 書き込み）。文は値を残さないので道の中で巻き戻す（①）
    ///
    /// ⚠ 評価の順: 1 で番地を取るコールバックは利用者のコードを走らせず何も書き換えないので、外れて今までの道（VM と
    ///   同じ `a.b` → 値の順）を通っても、先に番地を取ったことは見分けられない。速い道は何も起こさない読み出しと演算だけ。
    /// ⚠ 以前は `emit_nested_field_assign`（8-7）だけで、値の評価が何も起こさない式に限るため、属性を読む値
    ///   （`Body.bounce` の `self.vel.x = 0.0 - self.vel.x`）は `a.b` を積んだ。積む箇所があると関数が「値を積む関数」になり、
    ///   呼ぶ側が毎回保存点と巻き戻しで囲む。
    fn emit_checked_nested_assign(&mut self, object: &Expr, attr: &str, value: &Expr) -> bool {
        use crate::interpreter::value::block;
        if self.typed_mode || self.blk_mode || self.terminated {
            return false;
        }
        let Expr::Attr { object: parent, attr: pattr, .. } = object else { return false };
        let Some(pname) = ident_name(parent).filter(|_| self.is_local_ident(parent)) else { return false };
        let Some(tclass) = self.receiver_class(object).map(|r| r.class) else { return false };

        // ── 1) 当てはまるかを、コードを出す前に全部確かめる ──
        let mut reads: Vec<CheckedRead<'_>> = Vec::new();
        let Some(vt) = self.checked_num_ty(value, &mut reads) else { return false };
        // 読み出しが無い値は 8-7 の形（そちらで出せなかったものは、ここでも出せない）
        if reads.is_empty() {
            return false;
        }
        let Some((tidx, tdesc, (tity, tconv))) = self.direct_write_plan(&tclass, attr, vt) else { return false };
        // 番地ごと（ローカル変数, 属性）に確かめる印。書き込み先が先頭（同じ番地を読むなら印をまとめる）
        let mut blks: Vec<CheckedBlock<'_>> = vec![CheckedBlock {
            recv: parent,
            name: pname.to_string(),
            attr: Some(pattr.clone()),
            class: tclass.clone(),
            mask: block::IMMUTABLE,
        }];
        for r in &reads {
            let bit = block::init_bit(r.idx);
            match blks.iter_mut().find(|b| b.name == r.name && b.attr == r.attr) {
                Some(b) if b.class == r.class => b.mask |= bit,
                // 同じ受け手を別のクラスとして読む（注釈が食い違う）: 今までの道
                Some(_) => return false,
                None => blks.push(CheckedBlock {
                    recv: r.recv,
                    name: r.name.clone(),
                    attr: r.attr.clone(),
                    class: r.class.clone(),
                    mask: bit,
                }),
            }
        }

        // ── 2) 番地を取る: (親か変数のハンドル, 番地, 入れ子なら IC のセル) ──
        let mut got: Vec<(String, String, Option<String>)> = Vec::with_capacity(blks.len());
        for b in &blks {
            let (v, t) = self.gen_expr(b.recv);
            let h = self.to_handle(&v, t);
            let src = match &b.attr {
                Some(a) => BlockSrc::Field { parent_h: &h, attr: a },
                None => BlockSrc::Handle(&h),
            };
            // クラスは raw レイアウト（`direct_read_plan` / `direct_write_plan` で確かめた）なので表があり、必ず番地のコードを出す
            let (p, ic) = self.emit_block_src(&b.class, src).expect("a raw-layout class has a direct class table");
            got.push((h, p, ic));
        }

        // ── 3) 確かめる（外れたら遅い道へ） ──
        let (b_fast, b_slow, b_join) = (self.fresh_blk(), self.fresh_blk(), self.fresh_blk());
        for (b, (_, p, _)) in blks.iter().zip(&got) {
            let nonnull = self.fresh_reg();
            self.ec(&format!("{nonnull} = icmp ne ptr {p}, null"));
            let b_chk = self.fresh_blk();
            self.br_cond(&nonnull, &b_chk, &b_slow);
            self.start_block(&b_chk);
            let head = self.fresh_reg();
            self.ec(&format!("{head} = load i64, ptr {p}, align 8"));
            let masked = self.fresh_reg();
            // LLVM の整数の定数は符号つきの 10 進で書く。不変の印は立っていないこと・初期化の印は立っていること
            self.ec(&format!("{masked} = and i64 {head}, {}", b.mask as i64));
            let ok = self.fresh_reg();
            self.ec(&format!("{ok} = icmp eq i64 {masked}, {}", (b.mask & !block::IMMUTABLE) as i64));
            let b_next = self.fresh_blk();
            self.br_cond(&ok, &b_next, &b_slow);
            self.start_block(&b_next);
        }
        self.br(&b_fast);

        // ── 4) 速い道: 検査なしで読んで演算し、書く ──
        self.start_block(&b_fast);
        let ptrs: Vec<String> = got.iter().map(|(_, p, _)| p.clone()).collect();
        let (v, t) = self.emit_checked_num(value, &blks, &ptrs);
        debug_assert_eq!(t, vt, "checked_num_ty and emit_checked_num agree on the type");
        let stored = match tconv {
            None => v,
            Some(op) => {
                let c = self.fresh_reg();
                self.ec(&format!("{c} = {op} {v} to {tity}"));
                c
            }
        };
        let tp = &ptrs[0];
        let fp = self.fresh_reg();
        self.ec(&format!("{fp} = getelementptr inbounds i8, ptr {tp}, i64 {}", block::field_offset(tdesc.byte_offset)));
        self.ec(&format!("store {tity} {stored}, ptr {fp}, align {}", tdesc.width.byte_width()));
        let head = self.fresh_reg();
        self.ec(&format!("{head} = load i64, ptr {tp}, align 8"));
        let head2 = self.fresh_reg();
        self.ec(&format!("{head2} = or i64 {head}, {}", block::init_bit(tidx)));
        self.ec(&format!("store i64 {head2}, ptr {tp}, align 8"));
        self.br(&b_join);

        // ── 5) 遅い道: 今までの道 ──
        self.start_block(&b_slow);
        let region = self.begin_fallback_region();
        // 次から速い道を通れるように、空だった入れ子の番地の読み出しの IC のセルを埋める（`CB_ATTR_BLOCK` はセルが空の
        // うちは空を返す。7-14 の遅い道と同じ）。番地が取れてほかの検査で外れたときは何も足さない
        for (b, (h, p, ic)) in blks.iter().zip(got.clone()) {
            let Some(a) = &b.attr else { continue };
            let empty = self.fresh_reg();
            self.ec(&format!("{empty} = icmp eq ptr {p}, null"));
            let (b_fill, b_cont) = (self.fresh_blk(), self.fresh_blk());
            self.br_cond(&empty, &b_fill, &b_cont);
            self.start_block(&b_fill);
            self.block_src_handle(BlockSrc::Field { parent_h: &h, attr: a }, ic);
            self.br(&b_cont);
            self.start_block(&b_cont);
        }
        let (obj, ot) = self.gen_expr(object);
        let (val, vt2) = self.gen_expr(value);
        let oh = self.to_handle(&obj, ot);
        self.emit_field_assign(object, &oh, attr, &val, vt2);
        self.end_fallback_region(region);
        self.br(&b_join);

        self.start_block(&b_join);
        true
    }

    /// [`Self::emit_checked_nested_assign`] の値の型（`int` / `float`）。載らない式は `None`。確かめられる読み出し
    /// （`L.f` / `L.c.f`）を `reads` に足す。関数の入口で先読みしたフィールド（`preread_fields`）は読み出しに数えない
    /// （`gen_expr` が置き場から読む）。
    fn checked_num_ty<'e>(&self, e: &'e Expr, reads: &mut Vec<CheckedRead<'e>>) -> Option<Ty> {
        match e {
            Expr::Int(_) => Some(Ty::Int),
            Expr::Float(_) => Some(Ty::Float),
            Expr::Ident { name, .. } => {
                self.locals.get(name).map(|(_, t)| *t).filter(|t| matches!(t, Ty::Int | Ty::Float))
            }
            Expr::Attr { object: o, attr: f, .. } => {
                if let Some((_, t)) = preread_path(e).and_then(|p| self.preread_fields.get(&p)) {
                    return matches!(t, Ty::Int | Ty::Float).then_some(*t);
                }
                let (l, a) = match o.as_ref() {
                    Expr::Attr { object: l, attr: a, .. } => (l.as_ref(), Some(a.clone())),
                    l => (l, None),
                };
                let name = ident_name(l).filter(|_| self.is_local_ident(l))?;
                let class = self.receiver_class(o)?.class;
                let ty = self.class_fields.get(&class)?.get(f.as_str()).copied()?;
                if !matches!(ty, Ty::Int | Ty::Float) {
                    return None;
                }
                let (idx, _, _) = self.direct_read_plan(&class, f, ty)?;
                reads.push(CheckedRead { recv: l, name: name.to_string(), attr: a, class, idx });
                Some(ty)
            }
            Expr::BinOp { op: crate::ast::BinOp::Add | crate::ast::BinOp::Sub | crate::ast::BinOp::Mul, left, right, .. } => {
                let lt = self.checked_num_ty(left, reads)?;
                let rt = self.checked_num_ty(right, reads)?;
                Some(if lt == Ty::Int && rt == Ty::Int { Ty::Int } else { Ty::Float })
            }
            _ => None,
        }
    }

    /// [`Self::checked_num_ty`] が通した式を、確かめ済みの番地 `ptrs`（`blks` と同じ並び）から検査なしで評価する。
    fn emit_checked_num(&mut self, e: &Expr, blks: &[CheckedBlock<'_>], ptrs: &[String]) -> (String, Ty) {
        match e {
            Expr::BinOp { op, left, right, .. } => {
                let (l, lt) = self.emit_checked_num(left, blks, ptrs);
                let (r, rt) = self.emit_checked_num(right, blks, ptrs);
                self.specialize_binop(op, &l, lt, &r, rt)
            }
            Expr::Attr { object: o, attr: f, .. }
                if preread_path(e).is_none_or(|p| !self.preread_fields.contains_key(&p)) =>
            {
                let (l, a) = match o.as_ref() {
                    Expr::Attr { object: l, attr: a, .. } => (l.as_ref(), Some(a.as_str())),
                    l => (l, None),
                };
                let name = ident_name(l).expect("checked by checked_num_ty");
                let k = blks
                    .iter()
                    .position(|b| b.name == name && b.attr.as_deref() == a)
                    .expect("every checked read has a block");
                let class = blks[k].class.clone();
                let ty = self.class_fields.get(&class).and_then(|m| m.get(f.as_str())).copied().expect("checked");
                self.emit_trusted_read(&class, &ptrs[k], f, ty).expect("checked by checked_num_ty")
            }
            // リテラル・数のローカル変数・先読みしたフィールド: 何も起こさない
            _ => self.gen_expr(e),
        }
    }

    /// 値の評価が何も起こさないか（利用者のコード・外部言語のコードを走らせず、何も書き換えない）。12-3 で途中の段の
    /// 読み出しを値の後へ回してよいかの判定。リテラル・名前（読むだけ）と、`int` / `float` と分かっている項の演算だけ。
    /// ⚠ 属性の読み出しは入れない（C# のプロパティ・Python の属性は読むだけでコードを走らせる）。演算も項がインスタンスなら
    ///   `__add__` などを呼ぶので、数と分かっている項だけ。
    pub(super) fn effect_free_value(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::None | Expr::Str(_) => true,
            Expr::Ident { .. } => true,
            Expr::UnaryOp { op: crate::ast::UnaryOp::Neg, operand, .. } => self.numeric_leaf(operand),
            Expr::BinOp { op, left, right, .. } => {
                matches!(op, crate::ast::BinOp::Add | crate::ast::BinOp::Sub | crate::ast::BinOp::Mul)
                    && self.numeric_leaf(left)
                    && self.numeric_leaf(right)
            }
            _ => false,
        }
    }

    /// `int` / `float` と分かっている項（数のリテラル・`int` / `float` のローカル変数・その演算）。
    pub(super) fn numeric_leaf(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) => true,
            Expr::Ident { name, .. } => self.locals.get(name).is_some_and(|(_, t)| matches!(t, Ty::Int | Ty::Float)),
            Expr::UnaryOp { op: crate::ast::UnaryOp::Neg, operand, .. } => self.numeric_leaf(operand),
            Expr::BinOp { op, left, right, .. } => {
                matches!(op, crate::ast::BinOp::Add | crate::ast::BinOp::Sub | crate::ast::BinOp::Mul)
                    && self.numeric_leaf(left)
                    && self.numeric_leaf(right)
            }
            _ => false,
        }
    }

    /// `for` のループの出口（終わり・`break` の飛び先）: このループのイテレータを捨てる（`CB_ITER_DROP`・11-1・D-12）。
    /// VM はイテレータを関数のローカルの欄に置くので、ループを抜けた後は使われずに関数の終わりで消える。
    pub(super) fn drop_iter_scope(&mut self, iter_al: &str) {
        debug_assert_eq!(self.iter_scopes.last().map(String::as_str), Some(iter_al));
        self.iter_scopes.pop();
        self.emit_iter_drop(iter_al);
    }

    /// ループを飛び出す `return` / `block_return`: `iter_scopes[depth..]` のイテレータを捨てる。表はその位置まで縮むので、
    /// いちばん外のもの（`depth` 番目）を渡せば内側のものも捨てる。⚠ 生成の続きのために `iter_scopes` は変えない。
    pub(super) fn drop_iters_from(&mut self, depth: usize) {
        if let Some(al) = self.iter_scopes.get(depth).cloned() {
            self.emit_iter_drop(&al);
        }
    }

    fn emit_iter_drop(&mut self, iter_al: &str) {
        // typed 変種はコールバックを使えない（イテレータも作らない）。到達しない位置にも出さない
        if self.terminated || self.typed_mode {
            return;
        }
        let h = self.fresh_reg();
        self.ec(&format!("{h} = load i64, ptr {iter_al}"));
        self.call_cb(CB_ITER_DROP, &[format!("i64 {h}")]);
    }

    pub(super) fn gen_loop_body(
        &mut self,
        body: &[Stmt],
        header: &str,
        exit: &str,
        pre_pos: Option<usize>,
        latch_extra: impl FnOnce(&mut Self),
    ) {
        let latch = self.fresh_blk();
        let pushes_before = self.push_sites;
        let log_before = self.handle_store_log.len();
        // 見出しが巻き戻しを受け持てるループ（7-15）。本体の入れ子のループに渡さないよう、本体を出す前に取り出す
        let fuse = self.loop_fuse_slot.take();
        self.loop_stack.push((latch.clone(), exit.to_string()));
        self.gen_stmts(body);
        self.loop_stack.pop();
        self.br(&latch);
        self.start_block(&latch);
        latch_extra(self);
        // typed 変種はコールバックを使えない（積む箇所も無いので巻き戻しは要らない）
        if let (true, Some(pos), false) = (self.push_sites > pushes_before, pre_pos, self.typed_mode) {
            let mut live: Vec<String> = Vec::new();
            for p in &self.handle_store_log[log_before..] {
                if !live.contains(p) {
                    live.push(p.clone());
                }
            }
            // 保存点はループに入る直前（見出しへの `br` の前）に差し込む
            let tail = self.code_buf.split_off(pos);
            let save = self.call_cb(CB_ARENA_SAVE, &[]);
            // 残す値が無く、見出しが `CB_ITER_NEXT_AT` で読むなら、巻き戻しは次の周の頭の取り出しに任せる（7-15）。
            // 保存点を見出しの置き場へ書き（ループに入るたび・置き場の -1 の後）、戻り口では何も呼ばない。`break` は
            // 見出しを通らないので、今までどおり最後の周の値を残す。
            if let (true, Some(slot)) = (live.is_empty(), &fuse) {
                self.ec(&format!("store i64 {save}, ptr {slot}"));
                self.code_buf.push_str(&tail);
                self.br(header);
                return;
            }
            self.code_buf.push_str(&tail);
            let n = live.len();
            let (ins, outs) = if n == 0 {
                ("null".to_string(), "null".to_string())
            } else {
                let ins = format!("%_lin{}", self.reg);
                let outs = format!("%_lout{}", self.reg);
                self.reg += 1;
                self.ea(&format!("{ins} = alloca [{n} x i64], align 8"));
                self.ea(&format!("{outs} = alloca [{n} x i64], align 8"));
                for (i, p) in live.iter().enumerate() {
                    let v = self.fresh_reg();
                    let slot = self.fresh_reg();
                    self.ec(&format!("{v} = load i64, ptr {p}"));
                    self.ec(&format!("{slot} = getelementptr inbounds [{n} x i64], ptr {ins}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {v}, ptr {slot}"));
                }
                (ins, outs)
            };
            self.call_cb(CB_COMPACT_MANY, &[
                format!("ptr {ins}"), format!("i32 {n}"), format!("i64 {save}"), format!("ptr {outs}"),
            ]);
            for (i, p) in live.iter().enumerate() {
                let slot = self.fresh_reg();
                let v = self.fresh_reg();
                self.ec(&format!("{slot} = getelementptr inbounds [{n} x i64], ptr {outs}, i32 0, i32 {i}"));
                self.ec(&format!("{v} = load i64, ptr {slot}"));
                self.ec(&format!("store i64 {v}, ptr {p}"));
            }
        }
        self.br(header);
    }

    /// 属性の値を読む（複合代入の左辺）。`trait_name` があれば `obj::Trait.attr`（VM の `GetTraitAttr`）。
    fn emit_attr_load(&mut self, oh: &str, trait_name: Option<&str>, attr: &str) -> String {
        match trait_name {
            None => self.emit_attr_read(oh, attr, None).0,
            Some(t) => {
                let ptr = self.str_const(attr.as_bytes());
                let len = attr.len() as i32;
                let tptr = self.str_const(t.as_bytes());
                let tlen = t.len() as i32;
                self.call_cb(CB_GET_TRAIT_ATTR, &[format!("i64 {oh}"), tptr, format!("i32 {tlen}"), ptr, format!("i32 {len}")])
            }
        }
    }

    /// 属性へ書く（VM の `SetAttr` / `SetTraitAttr` と同じ入口・1-4）。
    fn emit_attr_store(&mut self, oh: &str, trait_name: Option<&str>, attr: &str, vh: &str) {
        match trait_name {
            None => self.emit_attr_assign(oh, attr, vh),
            Some(t) => {
                let ptr = self.str_const(attr.as_bytes());
                let len = attr.len() as i32;
                let tptr = self.str_const(t.as_bytes());
                let tlen = t.len() as i32;
                self.call_cb(CB_ASSIGN_TRAIT_ATTR, &[
                    format!("i64 {oh}"), tptr, format!("i32 {tlen}"), ptr, format!("i32 {len}"), format!("i64 {vh}"),
                ]);
            }
        }
    }

    // ── Emit a complete function into fn_defs ─────────────────────────────────

}

/// 属性の代入先を `(オブジェクト, trait 名, 属性名)` に分ける。
/// ⚠ ほかの形は `stmt_eligible` が弾いているので来ない（来たら生成を誤るより止める）。
/// `range(..)` の呼び出しなら、その位置引数（1〜3 個）。呼び先が組み込みの range かは呼ぶ側が `plain_builtin` で見る。
fn range_call_args(iter: &Expr) -> Option<Vec<&Expr>> {
    let Expr::Call { func, args, .. } = iter else { return None };
    if ident_name(func) != Some("range") || args.is_empty() || args.len() > 3 {
        return None;
    }
    args.iter()
        .map(|a| match a {
            crate::ast::CallArg::Positional(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// `range_call_args` が `Some` のときの呼び先（`range` の識別子）。
fn range_callee(iter: &Expr) -> &Expr {
    match iter {
        Expr::Call { func, .. } => func,
        other => other,
    }
}

/// カウンタのループにできるなら刻み: 引数が全部 `int` で、刻みが無い（1）か 0 でない int のリテラル（`-1` を含む）。
/// 刻みが変数のときは符号と 0 の検査が実行時に要るので載せない（今までの道で range が確かめる）。
fn range_counter_step(evaled: &[(String, Ty)], rargs: Vec<&Expr>) -> Option<i64> {
    if evaled.iter().any(|(_, t)| *t != Ty::Int) {
        return None;
    }
    if rargs.len() < 3 {
        return Some(1);
    }
    let step = match rargs[2] {
        Expr::Int(n) => *n,
        Expr::UnaryOp { op: crate::ast::UnaryOp::Neg, operand } => match operand.as_ref() {
            Expr::Int(n) => -*n,
            _ => return None,
        },
        _ => return None,
    };
    (step != 0).then_some(step)
}

fn attr_target(target: &Expr) -> (&Expr, Option<&str>, &str) {
    match target {
        Expr::Attr { object, attr, .. } => (object, None, attr),
        Expr::TraitAccess { object, trait_name, attr } => (object, Some(trait_name), attr),
        other => unreachable!("attribute assignment target must be checked by stmt_eligible: {other:?}"),
    }
}

/// [`GenCtx::checked_num_ty`] が見つけた、確かめてから読むフィールド（7-17 ②）: 受け手（ローカル変数 `name`・入れ子なら
/// 属性 `attr`）、受け手のクラス、フィールドの位置（初期化の印の番号）。
struct CheckedRead<'e> {
    recv: &'e Expr,
    name: String,
    attr: Option<String>,
    class: String,
    idx: usize,
}

/// [`GenCtx::emit_checked_nested_assign`] が確かめる番地: 受け手（ローカル変数 `name`・入れ子なら属性 `attr`）と、先頭の語で
/// 見る印（`mask`: 立っていてはいけない不変の印と、立っていなければいけない初期化の印）。
struct CheckedBlock<'e> {
    recv: &'e Expr,
    name: String,
    attr: Option<String>,
    class: String,
    mask: u64,
}
