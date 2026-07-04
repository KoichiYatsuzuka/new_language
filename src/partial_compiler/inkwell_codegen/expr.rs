// inkwell_codegen/expr.rs — 式の inkwell IR 生成: gen_expr / gen_binop / specialize_binop / gen_call。
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
    pub(super) fn gen_expr(&mut self, module: &Module<'ctx>, expr: &Expr) -> (BasicValueEnum<'ctx>, Ty) {
        match expr {
            Expr::Int(n) => (self.i64.const_int(*n as u64, true).into(), Ty::Int),
            Expr::Float(f) => (self.f64.const_float(*f).into(), Ty::Float),
            Expr::Bool(b) => {
                let h = if *b { 1u64 } else { 2u64 };
                (self.i64.const_int(h, false).into(), Ty::Handle)
            }
            Expr::None => (self.i64.const_zero().into(), Ty::Handle),
            Expr::Undefined => (self.i64.const_zero().into(), Ty::Handle),

            Expr::Str(s) => {
                let bytes = s.as_bytes();
                let gv    = self.make_str_const(module, bytes);
                let len   = self.i32.const_int(bytes.len() as u64, false);
                let h = self.call_cb_i64(CB_MAKE_STR, &[self.ptr.into(), self.i32.into()],
                                         &[gv.into(), len.into()], "str_h");
                (h.into(), Ty::Handle)
            }

            Expr::Ident(name) => {
                if let Some((alloca, ty)) = self.locals.get(name).cloned() {
                    let v = self.load_alloca(alloca, ty);
                    (v, ty)
                } else {
                    let gv  = self.make_str_const(module, name.as_bytes());
                    let len = self.i32.const_int(name.len() as u64, false);
                    let h   = self.call_cb_i64(CB_GET_GLOBAL,
                                               &[self.ptr.into(), self.i32.into()],
                                               &[gv.into(), len.into()], "global");
                    (h.into(), Ty::Handle)
                }
            }

            Expr::BinOp { op, left, right, .. } => self.gen_binop(module, op, left, right),

            Expr::UnaryOp { op, operand } => {
                let (v, vt) = self.gen_expr(module, operand);
                let h       = self.to_handle(v, vt).into();
                let op_code = self.i32.const_int(match op {
                    UnaryOp::Neg => 0, UnaryOp::Not => 1, UnaryOp::BitNot => 2,
                }, false);
                let r = self.call_cb_i64(CB_UNOP, &[self.i32.into(), self.i64.into()],
                                         &[op_code.into(), h], "unop");
                (r.into(), Ty::Handle)
            }

            Expr::Call { func, args, .. } => {
                // Type-specialized return for typed intra-module calls
                if let Expr::Ident(name) = func.as_ref() {
                    if self.module_fns.contains(name.as_str())
                        && !self.locals.contains_key(name.as_str())
                    {
                        let ret_ty = self.fn_sigs.get(name.as_str())
                            .map(|s| s.ret).unwrap_or(Ty::Handle);
                        let h = self.gen_call(module, func, args);
                        return match ret_ty {
                            Ty::Int => {
                                let r = self.call_cb_i64(CB_TO_INT, &[self.i64.into()],
                                                         &[h.into()], "unwrap");
                                (r.into(), Ty::Int)
                            }
                            Ty::Float => {
                                let r = self.call_cb_f64(CB_TO_FLOAT, &[self.i64.into()],
                                                         &[h.into()], "unwrap");
                                (r.into(), Ty::Float)
                            }
                            _ => (h.into(), Ty::Handle),
                        };
                    }
                }
                (self.gen_call(module, func, args).into(), Ty::Handle)
            }

            Expr::Attr { .. } if preread_path(expr).and_then(|p| self.preread_fields.get(&p)).is_some() => {
                let path = preread_path(expr).unwrap();
                let (alloca, ty) = *self.preread_fields.get(&path).unwrap();
                let v = self.load_alloca(alloca, ty);
                (v, ty)
            }

            Expr::Attr { object, attr, .. } | Expr::TraitAccess { object, attr, .. } => {
                let key = if let Expr::TraitAccess { trait_name, .. } = expr {
                    format!("{trait_name}::{attr}")
                } else {
                    attr.clone()
                };
                let (obj, ot) = self.gen_expr(module, object);
                let oh  = self.to_handle(obj, ot);
                let gv  = self.make_str_const(module, key.as_bytes());
                let len = self.i32.const_int(key.len() as u64, false);
                let r   = self.call_cb_i64(CB_GET_ATTR,
                                           &[self.i64.into(), self.ptr.into(), self.i32.into()],
                                           &[oh.into(), gv.into(), len.into()], "attr");
                (r.into(), Ty::Handle)
            }

            Expr::Subscript { object, index } => {
                let (obj, ot) = self.gen_expr(module, object);
                let (idx, it) = self.gen_expr(module, index);
                let oh = self.to_handle(obj, ot);
                let ih = self.to_handle(idx, it);
                let r  = self.call_cb_i64(CB_SUBSCRIPT,
                                          &[self.i64.into(), self.i64.into()],
                                          &[oh.into(), ih.into()], "sub");
                (r.into(), Ty::Handle)
            }

            Expr::IsType { expr, negated, type_name, .. } => {
                let (v, vt) = self.gen_expr(module, expr);
                let h   = self.to_handle(v, vt);
                let gv  = self.make_str_const(module, type_name.as_bytes());
                let len = self.i32.const_int(type_name.len() as u64, false);
                let r   = self.call_cb_i64(CB_IS_TYPE,
                                           &[self.i64.into(), self.ptr.into(), self.i32.into()],
                                           &[h.into(), gv.into(), len.into()], "is_type");
                if *negated {
                    let tl_true  = self.i64.const_int(1, false);
                    let tl_false = self.i64.const_int(2, false);
                    let is_true  = self.bld.build_int_compare(
                        IntPredicate::EQ, r, tl_true, "is_t").unwrap();
                    let neg_r = self.bld.build_select(is_true, tl_false, tl_true, "neg")
                        .unwrap().into_int_value();
                    (neg_r.into(), Ty::Handle)
                } else {
                    (r.into(), Ty::Handle)
                }
            }

            Expr::List(items) => {
                let n   = items.len();
                let arr = self.bld.build_alloca(
                    self.i64.array_type(n as u32), "arr").unwrap();
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(module, item);
                    let h = self.to_handle(v, vt);
                    let ep = self.bld.build_in_bounds_gep(
                        self.i64.array_type(n as u32),
                        arr,
                        &[self.i64.const_zero(), self.i32.const_int(i as u64, false)],
                        "ep").unwrap();
                    self.bld.build_store(h, ep).unwrap();
                }
                let ptr_h  = if n == 0 { self.ptr.const_null() } else {
                    self.bld.build_pointer_cast(arr, self.ptr, "lp").unwrap()
                };
                let count = self.i32.const_int(n as u64, false);
                let r = self.call_cb_i64(CB_MAKE_LIST,
                                         &[self.ptr.into(), self.i32.into()],
                                         &[ptr_h.into(), count.into()], "list");
                (r.into(), Ty::Handle)
            }

            Expr::Tuple(items) => {
                let n   = items.len();
                let arr = self.bld.build_alloca(
                    self.i64.array_type(n as u32), "arr").unwrap();
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(module, item);
                    let h  = self.to_handle(v, vt);
                    let ep = self.bld.build_in_bounds_gep(
                        self.i64.array_type(n as u32),
                        arr,
                        &[self.i64.const_zero(), self.i32.const_int(i as u64, false)],
                        "ep").unwrap();
                    self.bld.build_store(h, ep).unwrap();
                }
                let ptr_h = if n == 0 { self.ptr.const_null() } else {
                    self.bld.build_pointer_cast(arr, self.ptr, "tp").unwrap()
                };
                let count = self.i32.const_int(n as u64, false);
                let r = self.call_cb_i64(CB_MAKE_TUPLE,
                                         &[self.ptr.into(), self.i32.into()],
                                         &[ptr_h.into(), count.into()], "tuple");
                (r.into(), Ty::Handle)
            }

            Expr::Dict(pairs) => {
                let n    = pairs.len();
                let karr = self.bld.build_alloca(self.i64.array_type(n as u32), "ka").unwrap();
                let varr = self.bld.build_alloca(self.i64.array_type(n as u32), "va").unwrap();
                for (i, (k, v)) in pairs.iter().enumerate() {
                    let (kv, kt) = self.gen_expr(module, k);
                    let (vv, vt) = self.gen_expr(module, v);
                    let kh = self.to_handle(kv, kt);
                    let vh = self.to_handle(vv, vt);
                    let idx = self.i32.const_int(i as u64, false);
                    let kp = self.bld.build_in_bounds_gep(
                        self.i64.array_type(n as u32), karr,
                        &[self.i64.const_zero(), idx], "kp").unwrap();
                    let vp = self.bld.build_in_bounds_gep(
                        self.i64.array_type(n as u32), varr,
                        &[self.i64.const_zero(), idx], "vp").unwrap();
                    self.bld.build_store(kh, kp).unwrap();
                    self.bld.build_store(vh, vp).unwrap();
                }
                let (kptr, vptr) = if n == 0 {
                    (self.ptr.const_null(), self.ptr.const_null())
                } else {
                    (self.bld.build_pointer_cast(karr, self.ptr, "kp2").unwrap(),
                     self.bld.build_pointer_cast(varr, self.ptr, "vp2").unwrap())
                };
                let count = self.i32.const_int(n as u64, false);
                let r = self.call_cb_i64(
                    CB_MAKE_DICT,
                    &[self.ptr.into(), self.ptr.into(), self.i32.into()],
                    &[kptr.into(), vptr.into(), count.into()], "dict");
                (r.into(), Ty::Handle)
            }

            Expr::TemplateInstantiate { base, .. } => self.gen_expr(module, base),
            _ => (self.i64.const_zero().into(), Ty::Handle),
        }
    }

    pub(super) fn gen_binop(
        &mut self, module: &Module<'ctx>,
        op: &BinOp, left: &Expr, right: &Expr,
    ) -> (BasicValueEnum<'ctx>, Ty) {
        // Short-circuit
        match op {
            BinOp::And => {
                let lhs_bb  = self.bld.get_insert_block().unwrap();
                let rhs_bb  = self.ctx.append_basic_block(self.fn_val, "and_rhs");
                let merge   = self.ctx.append_basic_block(self.fn_val, "and_merge");
                let res_al  = self.bld.build_alloca(self.i64, "and_res").unwrap();

                let (l, lt) = self.gen_expr(module, left);
                let lh = self.to_handle(l, lt);
                self.bld.build_store(lh, res_al).unwrap();
                let cond = self.to_cond(lh.into(), Ty::Handle);
                self.bld.build_conditional_branch(cond, rhs_bb, merge).unwrap();

                self.bld.position_at_end(rhs_bb);
                let (r2, r2t) = self.gen_expr(module, right);
                let rh = self.to_handle(r2, r2t);
                self.bld.build_store(rh, res_al).unwrap();
                self.bld.build_unconditional_branch(merge).unwrap();

                self.bld.position_at_end(merge);
                let result = self.bld.build_load(self.i64, res_al, "and_v").unwrap();
                let _ = lhs_bb; // suppress unused warning
                return (result, Ty::Handle);
            }
            BinOp::Or => {
                let rhs_bb = self.ctx.append_basic_block(self.fn_val, "or_rhs");
                let merge  = self.ctx.append_basic_block(self.fn_val, "or_merge");
                let res_al = self.bld.build_alloca(self.i64, "or_res").unwrap();

                let (l, lt) = self.gen_expr(module, left);
                let lh = self.to_handle(l, lt);
                self.bld.build_store(lh, res_al).unwrap();
                let cond = self.to_cond(lh.into(), Ty::Handle);
                self.bld.build_conditional_branch(cond, merge, rhs_bb).unwrap();

                self.bld.position_at_end(rhs_bb);
                let (r2, r2t) = self.gen_expr(module, right);
                let rh = self.to_handle(r2, r2t);
                self.bld.build_store(rh, res_al).unwrap();
                self.bld.build_unconditional_branch(merge).unwrap();

                self.bld.position_at_end(merge);
                let result = self.bld.build_load(self.i64, res_al, "or_v").unwrap();
                return (result, Ty::Handle);
            }
            _ => {}
        }

        let (l, lt) = self.gen_expr(module, left);
        let (r, rt) = self.gen_expr(module, right);
        self.specialize_binop(module, op, l, lt, r, rt)
    }

    pub(super) fn specialize_binop(
        &mut self, module: &Module<'ctx>,
        op: &BinOp,
        l: BasicValueEnum<'ctx>, lt: Ty,
        r: BasicValueEnum<'ctx>, rt: Ty,
    ) -> (BasicValueEnum<'ctx>, Ty) {
        // Fall back to cb_binop if either side is a handle
        if lt == Ty::Handle || rt == Ty::Handle {
            let lh = self.to_handle(l, lt);
            let rh = self.to_handle(r, rt);
            let oc = self.i32.const_int(binop_code(op) as u64, false);
            let res = self.call_cb_i64(CB_BINOP,
                &[self.i32.into(), self.i64.into(), self.i64.into()],
                &[oc.into(), lh.into(), rh.into()], "binop");
            return (res.into(), Ty::Handle);
        }

        // Promote Int/Float mix → Float
        let (l2, r2, nt) = match (lt, rt) {
            (Ty::Int, Ty::Float) => {
                let lf = self.to_f64(l, lt); (lf.into(), r, Ty::Float)
            }
            (Ty::Float, Ty::Int) => {
                let rf = self.to_f64(r, rt); (l, rf.into(), Ty::Float)
            }
            _ => (l, r, lt),
        };

        match (op, nt) {
            (BinOp::Add, Ty::Int) => {
                let v = self.bld.build_int_add(l2.into_int_value(), r2.into_int_value(), "add").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::Sub, Ty::Int) => {
                let v = self.bld.build_int_sub(l2.into_int_value(), r2.into_int_value(), "sub").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::Mul, Ty::Int) => {
                let v = self.bld.build_int_mul(l2.into_int_value(), r2.into_int_value(), "mul").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::Div, Ty::Int) => {
                let lf = self.to_f64(l2, Ty::Int);
                let rf = self.to_f64(r2, Ty::Int);
                let v  = self.bld.build_float_div(lf, rf, "fdiv").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::FloorDiv, Ty::Int) => {
                // Python floor-div: sdiv with adjustment for negative remainder
                let d = self.bld.build_int_signed_div(
                    l2.into_int_value(), r2.into_int_value(), "d").unwrap();
                let rem = self.bld.build_int_signed_rem(
                    l2.into_int_value(), r2.into_int_value(), "rem").unwrap();
                let zero = self.i64.const_zero();
                let rnz  = self.bld.build_int_compare(IntPredicate::NE, rem, zero, "rnz").unwrap();
                let rneg = self.bld.build_int_compare(IntPredicate::SLT, rem, zero, "rneg").unwrap();
                let bneg = self.bld.build_int_compare(
                    IntPredicate::SLT, r2.into_int_value(), zero, "bneg").unwrap();
                let diff = self.bld.build_xor(rneg, bneg, "diff").unwrap();
                let need = self.bld.build_and(rnz, diff, "need").unwrap();
                let one  = self.i64.const_int(1, false);
                let dadj = self.bld.build_int_sub(d, one, "dadj").unwrap();
                let v    = self.bld.build_select(need, dadj, d, "fdiv_i").unwrap().into_int_value();
                (v.into(), Ty::Int)
            }
            (BinOp::Mod, Ty::Int) => {
                let rem  = self.bld.build_int_signed_rem(
                    l2.into_int_value(), r2.into_int_value(), "rem").unwrap();
                let zero = self.i64.const_zero();
                let rnz  = self.bld.build_int_compare(IntPredicate::NE, rem, zero, "rnz").unwrap();
                let rneg = self.bld.build_int_compare(IntPredicate::SLT, rem, zero, "rneg").unwrap();
                let bneg = self.bld.build_int_compare(
                    IntPredicate::SLT, r2.into_int_value(), zero, "bneg").unwrap();
                let diff = self.bld.build_xor(rneg, bneg, "diff").unwrap();
                let need = self.bld.build_and(rnz, diff, "need").unwrap();
                let radj = self.bld.build_int_add(rem, r2.into_int_value(), "radj").unwrap();
                let v    = self.bld.build_select(need, radj, rem, "mod_i").unwrap().into_int_value();
                (v.into(), Ty::Int)
            }
            (BinOp::BitAnd, Ty::Int) => {
                let v = self.bld.build_and(l2.into_int_value(), r2.into_int_value(), "band").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::BitOr, Ty::Int) => {
                let v = self.bld.build_or(l2.into_int_value(), r2.into_int_value(), "bor").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::BitXor, Ty::Int) => {
                let v = self.bld.build_xor(l2.into_int_value(), r2.into_int_value(), "bxor").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::LShift, Ty::Int) => {
                let v = self.bld.build_left_shift(l2.into_int_value(), r2.into_int_value(), "shl").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::RShift, Ty::Int) => {
                let v = self.bld.build_right_shift(l2.into_int_value(), r2.into_int_value(), true, "shr").unwrap();
                (v.into(), Ty::Int)
            }
            (BinOp::Add, Ty::Float) => {
                let v = self.bld.build_float_add(l2.into_float_value(), r2.into_float_value(), "fadd").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::Sub, Ty::Float) => {
                let v = self.bld.build_float_sub(l2.into_float_value(), r2.into_float_value(), "fsub").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::Mul, Ty::Float) => {
                let v = self.bld.build_float_mul(l2.into_float_value(), r2.into_float_value(), "fmul").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::Div, Ty::Float) => {
                let v = self.bld.build_float_div(l2.into_float_value(), r2.into_float_value(), "fdiv").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::FloorDiv, Ty::Float) => {
                let d  = self.bld.build_float_div(l2.into_float_value(), r2.into_float_value(), "fd").unwrap();
                let fl = self.build_floor(module, d);
                (fl.into(), Ty::Float)
            }
            (BinOp::Mod, Ty::Float) => {
                // Python mod: a - floor(a/b)*b
                let d   = self.bld.build_float_div(l2.into_float_value(), r2.into_float_value(), "fd").unwrap();
                let fl  = self.build_floor(module, d);
                let mul = self.bld.build_float_mul(fl, r2.into_float_value(), "fmul").unwrap();
                let v   = self.bld.build_float_sub(l2.into_float_value(), mul, "fmod").unwrap();
                (v.into(), Ty::Float)
            }
            (BinOp::Pow, Ty::Float) => {
                let v = self.build_pow(module, l2.into_float_value(), r2.into_float_value());
                (v.into(), Ty::Float)
            }
            // Comparisons
            (BinOp::Eq,    Ty::Int) => (self.bld.build_int_compare(IntPredicate::EQ,  l2.into_int_value(), r2.into_int_value(), "eq").unwrap().into(), Ty::Bool),
            (BinOp::NotEq, Ty::Int) => (self.bld.build_int_compare(IntPredicate::NE,  l2.into_int_value(), r2.into_int_value(), "ne").unwrap().into(), Ty::Bool),
            (BinOp::Lt,    Ty::Int) => (self.bld.build_int_compare(IntPredicate::SLT, l2.into_int_value(), r2.into_int_value(), "lt").unwrap().into(), Ty::Bool),
            (BinOp::LtEq,  Ty::Int) => (self.bld.build_int_compare(IntPredicate::SLE, l2.into_int_value(), r2.into_int_value(), "le").unwrap().into(), Ty::Bool),
            (BinOp::Gt,    Ty::Int) => (self.bld.build_int_compare(IntPredicate::SGT, l2.into_int_value(), r2.into_int_value(), "gt").unwrap().into(), Ty::Bool),
            (BinOp::GtEq,  Ty::Int) => (self.bld.build_int_compare(IntPredicate::SGE, l2.into_int_value(), r2.into_int_value(), "ge").unwrap().into(), Ty::Bool),
            (BinOp::Eq,    Ty::Float) => (self.bld.build_float_compare(FloatPredicate::OEQ, l2.into_float_value(), r2.into_float_value(), "feq").unwrap().into(), Ty::Bool),
            (BinOp::NotEq, Ty::Float) => (self.bld.build_float_compare(FloatPredicate::ONE, l2.into_float_value(), r2.into_float_value(), "fne").unwrap().into(), Ty::Bool),
            (BinOp::Lt,    Ty::Float) => (self.bld.build_float_compare(FloatPredicate::OLT, l2.into_float_value(), r2.into_float_value(), "flt").unwrap().into(), Ty::Bool),
            (BinOp::LtEq,  Ty::Float) => (self.bld.build_float_compare(FloatPredicate::OLE, l2.into_float_value(), r2.into_float_value(), "fle").unwrap().into(), Ty::Bool),
            (BinOp::Gt,    Ty::Float) => (self.bld.build_float_compare(FloatPredicate::OGT, l2.into_float_value(), r2.into_float_value(), "fgt").unwrap().into(), Ty::Bool),
            (BinOp::GtEq,  Ty::Float) => (self.bld.build_float_compare(FloatPredicate::OGE, l2.into_float_value(), r2.into_float_value(), "fge").unwrap().into(), Ty::Bool),
            _ => {
                let lh = self.to_handle(l2, nt);
                let rh = self.to_handle(r2, nt);
                let oc = self.i32.const_int(binop_code(op) as u64, false);
                let res = self.call_cb_i64(CB_BINOP,
                    &[self.i32.into(), self.i64.into(), self.i64.into()],
                    &[oc.into(), lh.into(), rh.into()], "binop");
                (res.into(), Ty::Handle)
            }
        }
    }

    pub(super) fn gen_call(&mut self, module: &Module<'ctx>, func: &Expr, args: &[CallArg]) -> IntValue<'ctx> {
        let arg_vals: Vec<(BasicValueEnum<'ctx>, Ty)> = args.iter()
            .map(|a| self.gen_expr(module, a.expr()))
            .collect();

        // Intra-module direct call
        if let Expr::Ident(name) = func {
            if self.module_fns.contains(name.as_str()) && !self.locals.contains_key(name.as_str()) {
                let mutabilities = self.fn_sigs.get(name.as_str())
                    .map(|s| s.param_mutabilities.clone());
                let call_args: Vec<_> = arg_vals.iter().enumerate()
                    .map(|(i, (v, vt))| {
                        let h = self.to_handle(*v, *vt);
                        let is_mut = mutabilities.as_ref()
                            .and_then(|m| m.get(i)).copied().unwrap_or(true);
                        if is_mut { h } else {
                            self.call_cb_i64(CB_DEEP_COPY, &[self.i64.into()], &[h.into()], "dc")
                        }
                    })
                    .map(|h: IntValue<'ctx>| h.into())
                    .collect::<Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>>();

                // arena_save + call_impl + arena_compact
                let save = self.call_cb_i64(CB_ARENA_SAVE, &[], &[], "save");
                let impl_fn = module.get_function(&format!("{name}_impl")).unwrap();
                let raw = self.bld.build_call(impl_fn, &call_args, "raw").unwrap()
                    .try_as_basic_value().left().unwrap().into_int_value();
                let result = self.call_cb_i64(CB_ARENA_COMPACT,
                    &[self.i64.into(), self.i64.into()],
                    &[raw.into(), save.into()], "compact");
                return result;
            }
        }

        // Generic call via cb_call_fn
        let (fv, ft) = self.gen_expr(module, func);
        let fn_h = self.to_handle(fv, ft);
        let handles: Vec<IntValue<'ctx>> = arg_vals.iter()
            .map(|(v, vt)| self.to_handle(*v, *vt))
            .collect();
        if handles.is_empty() {
            self.call_cb_i64(CB_CALL_FN,
                &[self.i64.into(), self.ptr.into(), self.i32.into()],
                &[fn_h.into(), self.ptr.const_null().into(), self.i32.const_zero().into()],
                "call")
        } else {
            let n   = handles.len();
            let arr = self.bld.build_alloca(self.i64.array_type(n as u32), "ca").unwrap();
            for (i, h) in handles.iter().enumerate() {
                let ep = self.bld.build_in_bounds_gep(
                    self.i64.array_type(n as u32), arr,
                    &[self.i64.const_zero(), self.i32.const_int(i as u64, false)],
                    "ep").unwrap();
                self.bld.build_store(*h, ep).unwrap();
            }
            let p   = self.bld.build_pointer_cast(arr, self.ptr, "cp").unwrap();
            let cnt = self.i32.const_int(n as u64, false);
            self.call_cb_i64(CB_CALL_FN,
                &[self.i64.into(), self.ptr.into(), self.i32.into()],
                &[fn_h.into(), p.into(), cnt.into()], "call")
        }
    }

    // ── Statement generation ──────────────────────────────────────────────────

}
