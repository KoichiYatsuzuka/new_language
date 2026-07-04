// inkwell_codegen/function.rs — 関数単位の IR 出力と補助: emit_fn、entry alloca、load/store、文字列定数、floor/pow、関数宣言取得。
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
    pub(super) fn emit_fn(&mut self, module: &Module<'ctx>, name: &str, params: &[Param], body: &[Stmt]) {
        // Reset per-function state
        self.locals.clear();
        self.loop_stack.clear();
        self.counter = 0;
        self.flat_list_params.clear();
        self.preread_fields.clear();

        // Populate flat_list_params for `let fixed_list[T]` params where T is SWD.
        for p in params {
            if p.mutable { continue; }
            let Some(ann) = &p.type_ann else { continue };
            if !ann.starts_with("fixed_list[") || !ann.ends_with(']') { continue; }
            let class_name = &ann[11..ann.len() - 1];
            let leaves = collect_flat_leaves(&self.all_class_fields, class_name, "", 0);
            if leaves.is_empty() { continue; }
            let stride = leaves.len() * 8;
            self.flat_list_params.insert(p.name.clone(), FlatListInfo { leaves, stride });
        }

        // Build _impl(i64, i64, ...) -> i64
        let param_types: Vec<_> = (0..params.len()).map(|_| self.i64.into()).collect();
        let fn_ty   = self.i64.fn_type(&param_types, false);
        let fn_val  = module.add_function(&format!("{name}_impl"), fn_ty,
                                          Some(inkwell::module::Linkage::Internal));
        self.fn_val = fn_val;
        let entry   = self.ctx.append_basic_block(fn_val, "entry");
        self.bld.position_at_end(entry);

        // Unwrap typed parameters
        for (i, p) in params.iter().enumerate() {
            let raw = fn_val.get_nth_param(i as u32).unwrap().into_int_value();
            raw.set_name(&format!("h_{}", p.name));
            let pt  = ann_ty(p.type_ann.as_deref());
            let st  = store_ty(pt);
            let ptr = self.build_entry_alloca(&p.name, st);
            match pt {
                Ty::Int => {
                    let v = self.call_cb_i64(CB_TO_INT, &[self.i64.into()], &[raw.into()], "pi");
                    self.bld.build_store(v, ptr).unwrap();
                }
                Ty::Float => {
                    let v = self.call_cb_f64(CB_TO_FLOAT, &[self.i64.into()], &[raw.into()], "pf");
                    self.bld.build_store(v, ptr).unwrap();
                }
                _ => { self.bld.build_store(raw, ptr).unwrap(); }
            }
            self.locals.insert(p.name.clone(), (ptr, st));
        }

        let terminated = self.gen_stmts(module, body);
        if !terminated {
            self.bld.build_return(Some(&self.i64.const_zero())).unwrap();
        }

        // Build exported wrapper: fname_tl(ptr args, i32 n) -> i64
        let tl_ty  = self.i64.fn_type(&[self.ptr.into(), self.i32.into()], false);
        let vis    = if cfg!(target_os = "windows") {
            Some(inkwell::module::Linkage::External)
        } else {
            None
        };
        let tl_fn  = module.add_function(&format!("{name}_tl"), tl_ty, vis);
        let tl_bb  = self.ctx.append_basic_block(tl_fn, "entry");
        self.bld.position_at_end(tl_bb);
        let args_ptr = tl_fn.get_first_param().unwrap().into_pointer_value();
        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        for i in 0..params.len() {
            let ep = self.bld.build_gep(
                self.i64, args_ptr,
                &[self.i32.const_int(i as u64, false)],
                "ap").unwrap();
            let a = self.bld.build_load(self.i64, ep, "a").unwrap().into_int_value();
            call_args.push(a.into());
        }
        let res = self.bld.build_call(fn_val, &call_args, "res").unwrap()
            .try_as_basic_value().left().unwrap();
        self.bld.build_return(Some(&res)).unwrap();
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Allocate an alloca in the function's entry block (required for mem2reg).
    pub(super) fn build_entry_alloca(&self, name: &str, ty: Ty) -> PointerValue<'ctx> {
        let entry  = self.fn_val.get_first_basic_block().unwrap();
        let saved  = self.bld.get_insert_block().unwrap();
        // Insert at the very start of entry
        match entry.get_first_instruction() {
            Some(first) => self.bld.position_before(&first),
            None        => self.bld.position_at_end(entry),
        }
        let ptr = match ty {
            Ty::Float => self.bld.build_alloca(self.f64, name).unwrap(),
            _         => self.bld.build_alloca(self.i64, name).unwrap(),
        };
        self.bld.position_at_end(saved);
        ptr
    }

    pub(super) fn load_alloca(&self, ptr: PointerValue<'ctx>, ty: Ty) -> BasicValueEnum<'ctx> {
        match ty {
            Ty::Float => self.bld.build_load(self.f64, ptr, "lf").unwrap(),
            _         => self.bld.build_load(self.i64, ptr, "li").unwrap(),
        }
    }

    pub(super) fn store_coerced(&self, val: BasicValueEnum<'ctx>, vt: Ty, st: Ty, ptr: PointerValue<'ctx>) {
        let coerced: BasicValueEnum<'ctx> = match st {
            Ty::Int   => self.to_i64(val, vt).into(),
            Ty::Float => self.to_f64(val, vt).into(),
            _         => self.to_handle(val, vt).into(),
        };
        self.bld.build_store(coerced, ptr).unwrap();
    }

    /// Create a global string constant and return a pointer to it.
    pub(super) fn make_str_const(&self, module: &Module<'ctx>, bytes: &[u8]) -> PointerValue<'ctx> {
        // Reuse existing global if already created for these bytes
        let name = format!("__s{}", simple_hash(bytes));
        if let Some(existing) = module.get_global(&name) {
            return existing.as_pointer_value();
        }
        let mut buf = bytes.to_vec();
        buf.push(0); // null terminator
        let arr_ty  = self.ctx.i8_type().array_type(buf.len() as u32);
        let init    = self.ctx.i8_type().const_array(
            &buf.iter().map(|&b| self.ctx.i8_type().const_int(b as u64, false)).collect::<Vec<_>>()
        );
        let gv = module.add_global(arr_ty, None, &name);
        gv.set_initializer(&init);
        gv.set_constant(true);
        gv.set_linkage(inkwell::module::Linkage::Private);
        gv.as_pointer_value()
    }

    pub(super) fn build_floor(&self, module: &Module<'ctx>, v: FloatValue<'ctx>) -> FloatValue<'ctx> {
        let f = self.get_or_declare(module, "llvm.floor.f64",
            self.f64.fn_type(&[self.f64.into()], false));
        self.bld.build_call(f, &[v.into()], "floor")
            .unwrap().try_as_basic_value().left().unwrap().into_float_value()
    }

    pub(super) fn build_pow(&self, module: &Module<'ctx>, base: FloatValue<'ctx>, exp: FloatValue<'ctx>) -> FloatValue<'ctx> {
        let f = self.get_or_declare(module, "llvm.pow.f64",
            self.f64.fn_type(&[self.f64.into(), self.f64.into()], false));
        self.bld.build_call(f, &[base.into(), exp.into()], "pow")
            .unwrap().try_as_basic_value().left().unwrap().into_float_value()
    }

    pub(super) fn get_or_declare(&self, module: &Module<'ctx>, name: &str,
                      ty: inkwell::types::FunctionType<'ctx>) -> FunctionValue<'ctx> {
        module.get_function(name).unwrap_or_else(|| module.add_function(name, ty, None))
    }
}
