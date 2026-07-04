// inkwell_codegen/stmt.rs — 文の inkwell IR 生成: gen_stmts / gen_stmt。
#![cfg(feature = "llvm")]

#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::execution_engine::ExecutionEngine;
use inkwell::module::Module;
use inkwell::targets::{InitializationConfig, Target};
use inkwell::types::{BasicType, FloatType, IntType, PointerType, StructType, VoidType};
use inkwell::values::{BasicValue, BasicValueEnum, FloatValue, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::ast::{BinOp, CallArg, Expr, FieldKind, MatchPattern, Param, Stmt, UnaryOp};
#[allow(unused_imports)]
use super::*;

impl<'ctx> GenCtx<'ctx> {
    /// Returns true if the current basic block was terminated (return/break/continue).
    pub(super) fn gen_stmts(&mut self, module: &Module<'ctx>, stmts: &[Stmt]) -> bool {
        for s in stmts {
            if self.gen_stmt(module, s) { return true; }
        }
        false
    }

    /// Returns true if this statement terminates the block (ret, br).
    pub(super) fn gen_stmt(&mut self, module: &Module<'ctx>, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Let(name, _, expr) | Stmt::Const(name, _, expr) => {
                let (v, vt) = self.gen_expr(module, expr);
                let st  = store_ty(vt);
                let ptr = self.build_entry_alloca(name, st);
                self.store_coerced(v, vt, st, ptr);
                self.locals.insert(name.clone(), (ptr, st));
                false
            }
            Stmt::Mut(name, _, expr) => {
                let (v, vt) = self.gen_expr(module, expr);
                let st  = store_ty(vt);
                let ptr = self.build_entry_alloca(name, st);
                self.store_coerced(v, vt, st, ptr);
                self.locals.insert(name.clone(), (ptr, st));
                false
            }
            Stmt::Assign { name, value, .. } => {
                let (v, vt) = self.gen_expr(module, value);
                if let Some((ptr, st)) = self.locals.get(name).cloned() {
                    self.store_coerced(v, vt, st, ptr);
                }
                false
            }
            Stmt::CompoundAssign { name, op, value, .. } => {
                if let Some((ptr, lt)) = self.locals.get(name).cloned() {
                    let lv = self.load_alloca(ptr, lt);
                    let (rv, rt) = self.gen_expr(module, value);
                    let (res, rest) = self.specialize_binop(module, op, lv, lt, rv, rt);
                    self.store_coerced(res, rest, lt, ptr);
                }
                false
            }
            Stmt::AttrAssign { target, value } => {
                if let Expr::Attr { object, attr, .. } = target {
                    let (obj, ot) = self.gen_expr(module, object);
                    let (val, vt) = self.gen_expr(module, value);
                    let oh  = self.to_handle(obj, ot);
                    let vh  = self.to_handle(val, vt);
                    let gv  = self.make_str_const(module, attr.as_bytes());
                    let len = self.i32.const_int(attr.len() as u64, false);
                    self.call_cb_void(CB_SET_ATTR,
                        &[self.i64.into(), self.ptr.into(), self.i32.into(), self.i64.into()],
                        &[oh.into(), gv.into(), len.into(), vh.into()]);
                }
                false
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
                if let Expr::Attr { object, attr, .. } = target {
                    let (obj, ot) = self.gen_expr(module, object);
                    let oh  = self.to_handle(obj, ot);
                    let gv  = self.make_str_const(module, attr.as_bytes());
                    let len = self.i32.const_int(attr.len() as u64, false);
                    let old = self.call_cb_i64(CB_GET_ATTR,
                        &[self.i64.into(), self.ptr.into(), self.i32.into()],
                        &[oh.into(), gv.into(), len.into()], "old");
                    let (rv, rt) = self.gen_expr(module, value);
                    let (res, rest) = self.specialize_binop(
                        module, op, old.into(), Ty::Handle, rv, rt);
                    let rh = self.to_handle(res, rest);
                    self.call_cb_void(CB_SET_ATTR,
                        &[self.i64.into(), self.ptr.into(), self.i32.into(), self.i64.into()],
                        &[oh.into(), gv.into(), len.into(), rh.into()]);
                }
                false
            }
            Stmt::Return(Some(e)) => {
                let (v, vt) = self.gen_expr(module, e);
                let h = self.to_handle(v, vt);
                self.bld.build_return(Some(&h)).unwrap();
                true
            }
            Stmt::Return(None) => {
                self.bld.build_return(Some(&self.i64.const_zero())).unwrap();
                true
            }
            Stmt::Pass | Stmt::Freeze(..) => false,
            Stmt::Expr(e) => { self.gen_expr(module, e); false }
            Stmt::Block(body) => self.gen_stmts(module, body),
            Stmt::Break => {
                if let Some((_, exit)) = self.loop_stack.last().cloned() {
                    self.bld.build_unconditional_branch(exit).unwrap();
                    true
                } else { false }
            }
            Stmt::Continue => {
                if let Some((header, _)) = self.loop_stack.last().cloned() {
                    self.bld.build_unconditional_branch(header).unwrap();
                    true
                } else { false }
            }

            Stmt::If { branches, else_body } => {
                let merge = self.ctx.append_basic_block(self.fn_val, "if_merge");
                for (cond_expr, body) in branches {
                    let (cv, ct) = self.gen_expr(module, cond_expr);
                    let cond     = self.to_cond(cv, ct);
                    let then_bb  = self.ctx.append_basic_block(self.fn_val, "then");
                    let next_bb  = self.ctx.append_basic_block(self.fn_val, "elif_or_else");
                    self.bld.build_conditional_branch(cond, then_bb, next_bb).unwrap();
                    self.bld.position_at_end(then_bb);
                    let term = self.gen_stmts(module, body);
                    if !term { self.bld.build_unconditional_branch(merge).unwrap(); }
                    self.bld.position_at_end(next_bb);
                }
                if let Some(else_stmts) = else_body {
                    let term = self.gen_stmts(module, else_stmts);
                    if !term { self.bld.build_unconditional_branch(merge).unwrap(); }
                } else {
                    self.bld.build_unconditional_branch(merge).unwrap();
                }
                self.bld.position_at_end(merge);
                false
            }

            Stmt::While { cond, body } => {
                let cond_bb = self.ctx.append_basic_block(self.fn_val, "while_cond");
                let body_bb = self.ctx.append_basic_block(self.fn_val, "while_body");
                let exit_bb = self.ctx.append_basic_block(self.fn_val, "while_exit");
                self.bld.build_unconditional_branch(cond_bb).unwrap();
                self.bld.position_at_end(cond_bb);
                let (cv, ct) = self.gen_expr(module, cond);
                let c = self.to_cond(cv, ct);
                self.bld.build_conditional_branch(c, body_bb, exit_bb).unwrap();
                self.bld.position_at_end(body_bb);
                self.loop_stack.push((cond_bb, exit_bb));
                let term = self.gen_stmts(module, body);
                self.loop_stack.pop();
                if !term { self.bld.build_unconditional_branch(cond_bb).unwrap(); }
                self.bld.position_at_end(exit_bb);
                false
            }

            Stmt::For { targets, iter, body } => {
                let target  = &targets[0];
                let loop_bb = self.ctx.append_basic_block(self.fn_val, "for_loop");
                let body_bb = self.ctx.append_basic_block(self.fn_val, "for_body");
                let exit_bb = self.ctx.append_basic_block(self.fn_val, "for_exit");

                // ── Flat list fast path ───────────────────────────────────────
                let flat_info: Option<FlatListInfo> = if let Expr::Ident(n) = iter {
                    self.flat_list_params.get(n.as_str()).cloned()
                } else { None };

                if let Some(fi) = flat_info {
                    let (iv, it) = self.gen_expr(module, iter);
                    let ih = self.to_handle(iv, it);

                    // Get flat data pointer (returned as i64 = raw *u8) and element count.
                    let flat_ptr_i64 = self.call_cb_i64(CB_FLAT_DATA_PTR,
                        &[self.i64.into()], &[ih.into()], "fptr_i");
                    let flat_len = self.call_cb_i64(CB_FLAT_LEN,
                        &[self.i64.into()], &[ih.into()], "flen");

                    // Store in allocas so they survive across basic-block boundaries.
                    let fptr_al = self.build_entry_alloca("__fptr", Ty::Handle);
                    let flen_al = self.build_entry_alloca("__flen", Ty::Handle);
                    self.bld.build_store(flat_ptr_i64, fptr_al).unwrap();
                    self.bld.build_store(flat_len, flen_al).unwrap();

                    // Loop index alloca.
                    let idx_al = self.build_entry_alloca("__fidx", Ty::Int);
                    self.bld.build_store(self.i64.const_zero(), idx_al).unwrap();

                    // Allocate per-leaf pre-read allocas and register in preread_fields.
                    for leaf in &fi.leaves {
                        let al = self.build_entry_alloca(
                            &format!("_prf_{}_{}", target, leaf.path.replace('.', "_")),
                            leaf.ty,
                        );
                        self.preread_fields.insert(
                            format!("{target}.{}", leaf.path), (al, leaf.ty)
                        );
                    }

                    // Dummy handle alloca for the loop variable (non-field use → TL_NONE).
                    let tgt_al = self.build_entry_alloca(target, Ty::Handle);
                    self.bld.build_store(self.i64.const_zero(), tgt_al).unwrap();
                    self.locals.insert(target.clone(), (tgt_al, Ty::Handle));

                    self.bld.build_unconditional_branch(loop_bb).unwrap();
                    self.bld.position_at_end(loop_bb);

                    // Loop condition: idx < flat_len
                    let idx_r = self.bld.build_load(self.i64, idx_al, "idx").unwrap().into_int_value();
                    let len_r = self.bld.build_load(self.i64, flen_al, "len").unwrap().into_int_value();
                    let done  = self.bld.build_int_compare(IntPredicate::SGE, idx_r, len_r, "done").unwrap();
                    self.bld.build_conditional_branch(done, exit_bb, body_bb).unwrap();

                    self.bld.position_at_end(body_bb);

                    // Compute element base pointer: flat_ptr + idx * stride
                    let fptr_i64 = self.bld.build_load(self.i64, fptr_al, "fptrv").unwrap().into_int_value();
                    let data_ptr = self.bld.build_int_to_ptr(fptr_i64, self.ptr, "dptr").unwrap();
                    let stride   = self.i64.const_int(fi.stride as u64, false);
                    let byte_off = self.bld.build_int_mul(idx_r, stride, "boff").unwrap();
                    let i8_ty    = self.ctx.i8_type();
                    let elem_ptr = self.bld.build_gep(i8_ty, data_ptr, &[byte_off], "eptr").unwrap();

                    // Load each leaf field into its pre-read alloca.
                    for leaf in &fi.leaves {
                        let key = format!("{target}.{}", leaf.path);
                        let (alloca, _) = *self.preread_fields.get(&key).unwrap();
                        let field_off = self.i64.const_int(leaf.byte_offset as u64, false);
                        let field_ptr = if leaf.byte_offset == 0 {
                            elem_ptr
                        } else {
                            self.bld.build_gep(i8_ty, elem_ptr, &[field_off], "fptr2").unwrap()
                        };
                        let val = match leaf.ty {
                            Ty::Int   => self.bld.build_load(self.i64, field_ptr, "fv").unwrap(),
                            Ty::Float => self.bld.build_load(self.f64, field_ptr, "fv").unwrap(),
                            _ => unreachable!(),
                        };
                        self.bld.build_store(val, alloca).unwrap();
                    }

                    self.loop_stack.push((loop_bb, exit_bb));
                    let term = self.gen_stmts(module, body);
                    self.loop_stack.pop();

                    // Increment index and branch back.
                    if !term {
                        let idx_next = self.bld.build_int_add(
                            idx_r, self.i64.const_int(1, false), "idx_next").unwrap();
                        self.bld.build_store(idx_next, idx_al).unwrap();
                        self.bld.build_unconditional_branch(loop_bb).unwrap();
                    }
                    self.bld.position_at_end(exit_bb);

                    // Clean up preread_fields for this loop variable's leaves.
                    for leaf in &fi.leaves {
                        self.preread_fields.remove(&format!("{target}.{}", leaf.path));
                    }

                } else {
                // ── Original CB_ITER_FROM / CB_ITER_NEXT path ────────────────
                let (iv, it) = self.gen_expr(module, iter);
                let ih       = self.to_handle(iv, it);
                let iter_h   = self.call_cb_i64(CB_ITER_FROM, &[self.i64.into()],
                                                &[ih.into()], "iter");
                let iter_al  = self.build_entry_alloca("__iter", Ty::Handle);
                self.bld.build_store(iter_h, iter_al).unwrap();
                let tgt_al   = self.build_entry_alloca(target, Ty::Handle);
                self.locals.insert(target.clone(), (tgt_al, Ty::Handle));
                self.bld.build_unconditional_branch(loop_bb).unwrap();

                self.bld.position_at_end(loop_bb);
                let iter_reload = self.bld.build_load(self.i64, iter_al, "iter_r")
                    .unwrap().into_int_value();
                let next = self.call_cb_i64(CB_ITER_NEXT, &[self.i64.into()],
                                            &[iter_reload.into()], "next");
                self.bld.build_store(next, tgt_al).unwrap();
                let stop = self.i64.const_int(u64::MAX, false); // TL_STOP_ITER = -1 = 0xFFFF..
                let done = self.bld.build_int_compare(IntPredicate::EQ, next, stop, "done").unwrap();
                self.bld.build_conditional_branch(done, exit_bb, body_bb).unwrap();

                self.bld.position_at_end(body_bb);
                self.loop_stack.push((loop_bb, exit_bb));
                let term = self.gen_stmts(module, body);
                self.loop_stack.pop();
                if !term { self.bld.build_unconditional_branch(loop_bb).unwrap(); }
                self.bld.position_at_end(exit_bb);
                } // end else (original path)

                false
            }

            Stmt::Match { subject, arms, .. } => {
                let (sv, st) = self.gen_expr(module, subject);
                let subj_h   = self.to_handle(sv, st);
                let merge    = self.ctx.append_basic_block(self.fn_val, "match_merge");

                for arm in arms {
                    let body_bb = self.ctx.append_basic_block(self.fn_val, "arm_body");
                    let next_bb = self.ctx.append_basic_block(self.fn_val, "arm_next");

                    match &arm.pattern {
                        MatchPattern::Case(Expr::Ident(w)) if w == "_" => {
                            self.bld.build_unconditional_branch(body_bb).unwrap();
                        }
                        MatchPattern::Case(pat) => {
                            let (pv, pt) = self.gen_expr(module, pat);
                            let ph  = self.to_handle(pv, pt);
                            let oc  = self.i32.const_int(7, false); // OP_EQ
                            let eq  = self.call_cb_i64(CB_BINOP,
                                &[self.i32.into(), self.i64.into(), self.i64.into()],
                                &[oc.into(), subj_h.into(), ph.into()], "eq");
                            let cnd = self.to_cond(eq.into(), Ty::Handle);
                            self.bld.build_conditional_branch(cnd, body_bb, next_bb).unwrap();
                        }
                        MatchPattern::IsType(type_name) => {
                            let gv  = self.make_str_const(module, type_name.as_bytes());
                            let len = self.i32.const_int(type_name.len() as u64, false);
                            let r   = self.call_cb_i64(CB_IS_TYPE,
                                &[self.i64.into(), self.ptr.into(), self.i32.into()],
                                &[subj_h.into(), gv.into(), len.into()], "is_t");
                            let tl_true = self.i64.const_int(1, false);
                            let cnd = self.bld.build_int_compare(
                                IntPredicate::EQ, r, tl_true, "is_cnd").unwrap();
                            self.bld.build_conditional_branch(cnd, body_bb, next_bb).unwrap();
                        }
                    }

                    self.bld.position_at_end(body_bb);
                    let term = self.gen_stmts(module, &arm.body);
                    if !term { self.bld.build_unconditional_branch(merge).unwrap(); }
                    self.bld.position_at_end(next_bb);
                }
                self.bld.build_unconditional_branch(merge).unwrap();
                self.bld.position_at_end(merge);
                false
            }

            _ => false,
        }
    }

    // ── Per-function emission ─────────────────────────────────────────────────

}
