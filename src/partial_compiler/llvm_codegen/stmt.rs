// llvm_codegen/stmt.rs — 文の LLVM IR 生成: gen_stmts / gen_stmt。

#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use crate::ast::{BinOp, CallArg, Expr, MatchPattern, Param, Stmt, UnaryOp};
use super::*;

impl<'a> GenCtx<'a> {
    pub fn gen_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts { self.gen_stmt(s); }
    }

    pub(super) fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(name, _, expr) | Stmt::Const(name, _, expr) => {
                let (v, vt) = self.gen_expr(expr);
                let st = store_ty(vt);
                let ptr = self.alloca_var(name, st);
                let coerced = match st {
                    Ty::Int   => self.to_i64(&v, vt),
                    Ty::Float => self.to_f64(&v, vt),
                    _         => self.to_handle(&v, vt),
                };
                self.store_val(st, &coerced, &ptr.clone());
            }
            Stmt::Mut(name, _, expr) => {
                let (v, vt) = self.gen_expr(expr);
                let st  = store_ty(vt);
                let ptr = self.alloca_var(name, st);
                let coerced = match st {
                    Ty::Int   => self.to_i64(&v, vt),
                    Ty::Float => self.to_f64(&v, vt),
                    _         => self.to_handle(&v, vt),
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
            Stmt::AttrAssign { target, value } => {
                if let Expr::Attr { object, attr, .. } = target {
                    let (obj, ot) = self.gen_expr(object);
                    let (val, vt) = self.gen_expr(value);
                    let oh  = self.to_handle(&obj, ot);
                    let vh  = self.to_handle(&val, vt);
                    let ptr = self.str_const(attr.as_bytes());
                    let len = attr.len() as i32;
                    self.call_cb(CB_SET_ATTR, &[format!("i64 {oh}"), ptr, format!("i32 {len}"), format!("i64 {vh}")]);
                }
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
                if let Expr::Attr { object, attr, .. } = target {
                    let (obj, ot) = self.gen_expr(object);
                    let oh   = self.to_handle(&obj, ot);
                    let ptr  = self.str_const(attr.as_bytes());
                    let len  = attr.len() as i32;
                    let old  = self.call_cb(CB_GET_ATTR, &[format!("i64 {oh}"), ptr.clone(), format!("i32 {len}")]);
                    let (rv, rt) = self.gen_expr(value);
                    let (res, rest) = self.specialize_binop(op, &old, Ty::Handle, &rv, rt);
                    let rh = self.to_handle(&res, rest);
                    self.call_cb(CB_SET_ATTR, &[format!("i64 {oh}"), ptr, format!("i32 {len}"), format!("i64 {rh}")]);
                }
            }
            Stmt::Return(Some(expr)) => {
                let (v, vt) = self.gen_expr(expr);
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
                } else {
                    let h = self.to_handle(&v, vt);
                    self.ret_handle(&h);
                }
            }
            Stmt::Return(None) => {
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
            Stmt::Expr(e) => { self.gen_expr(e); }
            Stmt::Freeze(..) => {}
            Stmt::Block(body) => self.gen_stmts(body),

            Stmt::BlockReturn(expr, _) => {
                if let Some(ctx) = self.block_stack.last().cloned() {
                    let (v, vt) = self.gen_expr(expr);
                    let h = self.to_handle(&v, vt);
                    self.ec(&format!("store i64 {h}, ptr {}", ctx.result_al));
                    self.br(&ctx.exit_label);
                }
            }

            Stmt::LoopYield(expr) => {
                if let Some(ctx) = self.block_stack.last().cloned() {
                    if let Some(la) = ctx.list_al.clone() {
                        let list_cur = self.fresh_reg();
                        self.ec(&format!("{list_cur} = load i64, ptr {la}"));
                        let (v, vt) = self.gen_expr(expr);
                        let h = self.to_handle(&v, vt);
                        let updated = self.call_cb(CB_LIST_APPEND, &[format!("i64 {list_cur}"), format!("i64 {h}")]);
                        self.ec(&format!("store i64 {updated}, ptr {la}"));
                    }
                }
            }

            Stmt::Yield(expr) => {
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

            Stmt::If { branches, else_body } => {
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

            Stmt::While { cond, body } => {
                let cond_blk = self.fresh_blk();
                let body_blk = self.fresh_blk();
                let exit_blk = self.fresh_blk();
                self.br(&cond_blk);
                self.start_block(&cond_blk);
                let (cv, ct) = self.gen_expr(cond);
                let cc = self.to_cond(&cv, ct);
                self.br_cond(&cc, &body_blk, &exit_blk);
                self.start_block(&body_blk);
                self.loop_stack.push((cond_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.br(&cond_blk);
                self.start_block(&exit_blk);
            }

            Stmt::For { targets, iter, body } => {
                let target   = &targets[0];
                let exit_blk = self.fresh_blk();
                let loop_blk = self.fresh_blk();

                // ── Flat list iteration path ──────────────────────────────────
                // Activated when the iterator is a `let fixed_list[ClassName]` param.
                let flat_info: Option<FlatListInfo> = if let Expr::Ident(n) = iter {
                    self.flat_list_params.get(n.as_str()).cloned()
                } else {
                    None
                };

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
                    let tgt_al = format!("%_al_{target}");
                    self.alloca_buf.push_str(&format!("  {tgt_al} = alloca i64, align 8\n"));
                    self.ec(&format!("store i64 0, ptr {tgt_al}"));
                    self.locals.insert(target.clone(), (tgt_al.clone(), Ty::Handle));

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

                    self.loop_stack.push((loop_blk.clone(), exit_blk.clone()));
                    self.gen_stmts(body);
                    self.loop_stack.pop();

                    // Increment index and branch back.
                    let idx_next = self.fresh_reg();
                    self.ec(&format!("{idx_next} = add i64 {idx_r}, 1"));
                    self.ec(&format!("store i64 {idx_next}, ptr {idx_al}"));
                    self.br(&loop_blk);
                    self.start_block(&exit_blk);

                } else {
                // ── Original CB_ITER_FROM / CB_ITER_NEXT path ────────────────
                let (iv, it) = self.gen_expr(iter);
                let ih       = self.to_handle(&iv, it);
                let iter_h   = self.call_cb(CB_ITER_FROM, &[format!("i64 {ih}")]);
                let iter_al  = format!("%_iter{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {iter_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {iter_h}, ptr {iter_al}"));

                let tgt_al = format!("%_al_{target}");
                self.alloca_buf.push_str(&format!("  {tgt_al} = alloca i64, align 8\n"));
                self.locals.insert(target.clone(), (tgt_al.clone(), Ty::Handle));

                self.br(&loop_blk);
                self.start_block(&loop_blk);
                let iter_reload = self.fresh_reg();
                self.ec(&format!("{iter_reload} = load i64, ptr {iter_al}"));
                let next = self.call_cb(CB_ITER_NEXT, &[format!("i64 {iter_reload}")]);
                self.ec(&format!("store i64 {next}, ptr {tgt_al}"));
                let is_done = self.fresh_reg();
                self.ec(&format!("{is_done} = icmp eq i64 {next}, -1"));
                let body_blk2 = self.fresh_blk();
                self.br_cond(&is_done, &exit_blk, &body_blk2);
                self.start_block(&body_blk2);
                self.loop_stack.push((loop_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.br(&loop_blk);
                self.start_block(&exit_blk);
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
                        MatchPattern::Case(Expr::Ident(w)) if w == "_" => {
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

    // ── Emit a complete function into fn_defs ─────────────────────────────────

}
