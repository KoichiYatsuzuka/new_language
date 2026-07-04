// inkwell_codegen/context.rs — GenCtx の低レベル IR 構築補助: コンストラクタ、一時名生成、ArCallbacks ロードと各種 CB 呼び出し、型変換(handle/i64/f64/cond)。
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
    pub(super) fn new(
        ctx:              &'ctx Context,
        module:           &Module<'ctx>,
        module_fns:       HashSet<String>,
        fn_sigs:          HashMap<String, FnSig>,
        all_class_fields: HashMap<String, Vec<(String, String)>>,
    ) -> Self {
        let i1   = ctx.bool_type();
        let i32  = ctx.i32_type();
        let i64  = ctx.i64_type();
        let f64  = ctx.f64_type();
        let ptr  = ctx.ptr_type(AddressSpace::default());
        let void = ctx.void_type();

        // %ArCallbacks = { ptr × 38 } (must match ArCallbacks in native_api.rs)
        let fields: Vec<_> = (0..38).map(|_| ptr.as_basic_type_enum()).collect();
        let cb_ty = ctx.struct_type(&fields, false);

        // @CB = internal global ptr null
        let cb_global_val = module.add_global(ptr, None, "CB");
        cb_global_val.set_initializer(&ptr.const_null());
        cb_global_val.set_linkage(inkwell::module::Linkage::Internal);
        let cb_global = cb_global_val.as_pointer_value();

        // dummy fn_val placeholder (replaced in emit_fn)
        let dummy_ty = void.fn_type(&[], false);
        let fn_val   = module.add_function("__dummy", dummy_ty, None);

        let bld = ctx.create_builder();

        GenCtx {
            ctx, bld, i1, i32, i64, f64, ptr, void, cb_ty, cb_global,
            module_fns, fn_sigs, all_class_fields,
            fn_val, locals: HashMap::new(),
            loop_stack: Vec::new(), counter: 0,
            flat_list_params: HashMap::new(),
            preread_fields: HashMap::new(),
        }
    }

    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        let n = self.counter; self.counter += 1; format!("{prefix}{n}")
    }

    // ── ArCallbacks access ────────────────────────────────────────────────────

    /// Load @CB pointer
    pub(super) fn load_cb(&self) -> PointerValue<'ctx> {
        self.bld.build_load(self.ptr, self.cb_global, "cb")
            .unwrap().into_pointer_value()
    }

    /// Load a function pointer from @CB[field_idx]
    pub(super) fn load_cb_fn(&self, cb: PointerValue<'ctx>, field: u32) -> PointerValue<'ctx> {
        let fp = self.bld
            .build_struct_gep(self.cb_ty, cb, field, "fp").unwrap();
        self.bld.build_load(self.ptr, fp, "fn_ptr")
            .unwrap().into_pointer_value()
    }

    /// Call a callback that returns i64. `arg_types` is the LLVM fn param list.
    pub(super) fn call_cb_i64(
        &self,
        field: u32,
        param_types: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
        args:        &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        name:        &str,
    ) -> IntValue<'ctx> {
        let cb     = self.load_cb();
        let fn_ptr = self.load_cb_fn(cb, field);
        let fn_ty  = self.i64.fn_type(param_types, false);
        self.bld.build_indirect_call(fn_ty, fn_ptr, args, name)
            .unwrap().try_as_basic_value().left().unwrap().into_int_value()
    }

    /// Call a callback that returns f64.
    pub(super) fn call_cb_f64(
        &self,
        field: u32,
        param_types: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
        args:        &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        name:        &str,
    ) -> FloatValue<'ctx> {
        let cb     = self.load_cb();
        let fn_ptr = self.load_cb_fn(cb, field);
        let fn_ty  = self.f64.fn_type(param_types, false);
        self.bld.build_indirect_call(fn_ty, fn_ptr, args, name)
            .unwrap().try_as_basic_value().left().unwrap().into_float_value()
    }

    /// Call a void callback (e.g. set_attr).
    pub(super) fn call_cb_void(
        &self,
        field: u32,
        param_types: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
        args:        &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    ) {
        let cb     = self.load_cb();
        let fn_ptr = self.load_cb_fn(cb, field);
        let fn_ty  = self.void.fn_type(param_types, false);
        self.bld.build_indirect_call(fn_ty, fn_ptr, args, "").unwrap();
    }

    /// Call a callback that returns i32 (is_truthy).
    pub(super) fn call_cb_i32(
        &self,
        field: u32,
        param_types: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
        args:        &[inkwell::values::BasicMetadataValueEnum<'ctx>],
        name:        &str,
    ) -> IntValue<'ctx> {
        let cb     = self.load_cb();
        let fn_ptr = self.load_cb_fn(cb, field);
        let fn_ty  = self.i32.fn_type(param_types, false);
        self.bld.build_indirect_call(fn_ty, fn_ptr, args, name)
            .unwrap().try_as_basic_value().left().unwrap().into_int_value()
    }

    // ── Type coercions ────────────────────────────────────────────────────────

    pub(super) fn to_handle(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
        match ty {
            Ty::Handle => val.into_int_value(),
            Ty::Int => self.call_cb_i64(
                CB_MAKE_INT,
                &[self.i64.into()],
                &[val.into()],
                "h",
            ),
            Ty::Float => self.call_cb_i64(
                CB_MAKE_FLOAT,
                &[self.f64.into()],
                &[val.into()],
                "h",
            ),
            Ty::Bool => {
                let b = val.into_int_value();
                // i1 → TL_TRUE(1) or TL_FALSE(2)
                let tl_true  = self.i64.const_int(1, false);
                let tl_false = self.i64.const_int(2, false);
                self.bld.build_select(b, tl_true, tl_false, "bool_h")
                    .unwrap().into_int_value()
            }
        }
    }

    pub(super) fn to_i64(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
        match ty {
            Ty::Int    => val.into_int_value(),
            Ty::Handle => self.call_cb_i64(CB_TO_INT, &[self.i64.into()], &[val.into()], "i"),
            Ty::Float  => self.bld
                .build_float_to_signed_int(val.into_float_value(), self.i64, "i").unwrap(),
            Ty::Bool   => self.bld
                .build_int_z_extend(val.into_int_value(), self.i64, "i").unwrap(),
        }
    }

    pub(super) fn to_f64(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> FloatValue<'ctx> {
        match ty {
            Ty::Float  => val.into_float_value(),
            Ty::Int    => self.bld
                .build_signed_int_to_float(val.into_int_value(), self.f64, "f").unwrap(),
            Ty::Handle => self.call_cb_f64(CB_TO_FLOAT, &[self.i64.into()], &[val.into()], "f"),
            Ty::Bool   => self.bld
                .build_unsigned_int_to_float(val.into_int_value(), self.f64, "f").unwrap(),
        }
    }

    pub(super) fn to_cond(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
        match ty {
            Ty::Bool   => val.into_int_value(),
            Ty::Int    => self.bld
                .build_int_compare(IntPredicate::NE, val.into_int_value(),
                                   self.i64.const_zero(), "cond").unwrap(),
            Ty::Float  => self.bld
                .build_float_compare(FloatPredicate::UNE, val.into_float_value(),
                                     self.f64.const_float(0.0), "cond").unwrap(),
            Ty::Handle => {
                let tr = self.call_cb_i32(CB_IS_TRUTHY, &[self.i64.into()],
                                          &[val.into()], "truthy");
                self.bld.build_int_compare(IntPredicate::NE, tr,
                                           self.i32.const_zero(), "cond").unwrap()
            }
        }
    }

    // ── Expression generation ─────────────────────────────────────────────────

}
