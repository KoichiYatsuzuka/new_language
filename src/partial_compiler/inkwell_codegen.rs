// inkwell_codegen.rs — LLVM IR builder via inkwell for native JIT compilation.
//
// Replaces the text-IR / clang pipeline in llvm_codegen.rs.
// This module is compiled only when the `llvm` Cargo feature is enabled.
//
// Pipeline:
//   AST → inkwell IR builder (in-process) → ExecutionEngine (JIT)
//       → raw fn pointers  (used immediately by the interpreter)
//       → LLVM bitcode     (stored in .arc v2 for cross-session reuse)
//
// On `import` with a v2 .arc:
//   bitcode bytes → inkwell Module::parse_bitcode → ExecutionEngine → fn ptrs
//
// ArCallbacks ABI and handle semantics are identical to the old codegen.

#![cfg(feature = "llvm")]

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

// ── Public types ──────────────────────────────────────────────────────────────

/// Compiled function metadata returned alongside the JIT handle.
#[derive(Debug, Clone)]
pub struct FnExport {
    pub name:     String,
    pub n_params: usize,
    /// Raw address of `fname_tl` in the JIT-compiled code.
    /// Cast to `unsafe extern "C" fn(*const i64, i32) -> i64` at call time.
    pub fn_ptr:   usize,
}

/// Owns the inkwell `Context` and `ExecutionEngine` so function pointers remain
/// valid for as long as this handle is alive.
///
/// # Safety
/// `ExecutionEngine<'static>` is produced by transmuting the `'ctx` lifetime away.
/// Safety is upheld because:
///   1. `_engine` is declared before `_context` → engine drops first, then context.
///   2. Both wrap heap-allocated LLVM C objects; moving the Rust wrappers does not
///      move the underlying objects.
pub struct JitHandle {
    _engine:  Box<ExecutionEngine<'static>>,
    _context: Context,
}

impl JitHandle {
    fn new(context: Context, engine: ExecutionEngine<'_>) -> Self {
        // SAFETY: see struct-level doc comment.
        let engine_static: ExecutionEngine<'static> =
            unsafe { std::mem::transmute(engine) };
        Self { _engine: Box::new(engine_static), _context: context }
    }
}

// SAFETY: LLVM C objects are safe to move between threads.
unsafe impl Send for JitHandle {}
unsafe impl Sync for JitHandle {}

// ── Internal type tracking ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ty { Int, Float, Bool, Handle }

struct FnSig {
    ret: Ty,
    param_mutabilities: Vec<bool>,
}

/// フラット配列の末端プリミティブフィールド（SWD クラスは再帰展開済み）。
#[derive(Clone)]
struct FlatLeaf {
    /// ループ変数からのドット区切りパス（例: `"v"`, `"start.x"`）。
    path: String,
    /// 要素先頭からのバイトオフセット。
    byte_offset: usize,
    /// 型（Int または Float）。
    ty: Ty,
}

/// `let fixed_list[ClassName]` パラメータの平坦レイアウト情報。
#[derive(Clone)]
struct FlatListInfo {
    leaves: Vec<FlatLeaf>,
    stride: usize,
}

/// ネストした属性アクセス式をドット区切りパスに変換する。変換できなければ `None`。
fn preread_path(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(n) => Some(n.clone()),
        Expr::Attr { object, attr, .. } => {
            let base = preread_path(object)?;
            Some(format!("{base}.{attr}"))
        }
        _ => None,
    }
}

/// クラス `class_name` の全 SWD 末端フィールドを収集する（再帰的）。
fn collect_flat_leaves(
    all_class_fields: &HashMap<String, Vec<(String, String)>>,
    class_name: &str,
    path_prefix: &str,
    byte_base: usize,
) -> Vec<FlatLeaf> {
    let raw = match all_class_fields.get(class_name) {
        Some(f) if !f.is_empty() => f.clone(),
        _ => return vec![],
    };
    let mut sorted = raw;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut leaves = Vec::new();
    let mut byte_offset = byte_base;
    for (fname, ftype) in sorted {
        let full_path = if path_prefix.is_empty() { fname.clone() } else { format!("{path_prefix}.{fname}") };
        match ftype.as_str() {
            "int" => { leaves.push(FlatLeaf { path: full_path, byte_offset, ty: Ty::Int }); byte_offset += 8; }
            "float" => { leaves.push(FlatLeaf { path: full_path, byte_offset, ty: Ty::Float }); byte_offset += 8; }
            nested => {
                let sub = collect_flat_leaves(all_class_fields, nested, &full_path, byte_offset);
                if sub.is_empty() { return vec![]; }
                byte_offset += sub.len() * 8;
                leaves.extend(sub);
            }
        }
    }
    leaves
}

fn ann_ty(s: Option<&str>) -> Ty {
    match s {
        Some("int")   => Ty::Int,
        Some("float") => Ty::Float,
        _             => Ty::Handle,
    }
}

fn store_ty(t: Ty) -> Ty { if t == Ty::Bool { Ty::Handle } else { t } }

// ── ArCallbacks field indices ─────────────────────────────────────────────────

const CB_MAKE_INT:      u32 = 0;
const CB_MAKE_FLOAT:    u32 = 1;
const CB_IS_TRUTHY:     u32 = 8;
const CB_BINOP:         u32 = 9;
const CB_UNOP:          u32 = 10;
const CB_CALL_FN:       u32 = 11;
const CB_GET_ATTR:      u32 = 12;
const CB_SET_ATTR:      u32 = 13;
const CB_SUBSCRIPT:     u32 = 14;
const CB_GET_GLOBAL:    u32 = 15;
const CB_ITER_FROM:     u32 = 16;
const CB_ITER_NEXT:     u32 = 17;
const CB_IS_TYPE:       u32 = 18;
const CB_ARENA_SAVE:    u32 = 19;
const CB_ARENA_COMPACT: u32 = 20;
const CB_TO_INT:        u32 = 22;
const CB_TO_FLOAT:      u32 = 23;
const CB_DEEP_COPY:     u32 = 24;
const CB_MAKE_STR:      u32 = 3;
const CB_MAKE_LIST:     u32 = 4;
const CB_MAKE_TUPLE:    u32 = 5;
const CB_MAKE_DICT:     u32 = 6;
const CB_FLAT_DATA_PTR:  u32 = 35;
const CB_FLAT_LEN:       u32 = 36;
const CB_FN_TRAMPOLINE:  u32 = 37;

// ── Code generation context ───────────────────────────────────────────────────

struct GenCtx<'ctx> {
    ctx:    &'ctx Context,
    bld:    Builder<'ctx>,

    // Cached primitive types
    i1:   IntType<'ctx>,
    i32:  IntType<'ctx>,
    i64:  IntType<'ctx>,
    f64:  FloatType<'ctx>,
    ptr:  PointerType<'ctx>,
    void: VoidType<'ctx>,
    /// %ArCallbacks struct (38 ptr fields)
    cb_ty: StructType<'ctx>,
    /// @CB global: ptr to ArCallbacks
    cb_global: PointerValue<'ctx>,

    // Module-level function metadata
    module_fns: HashSet<String>,
    fn_sigs:    HashMap<String, FnSig>,

    // All instance fields for every non-template class (for flat SWD leaf collection)
    all_class_fields: HashMap<String, Vec<(String, String)>>,

    // Per-function state
    fn_val:     FunctionValue<'ctx>,
    locals:     HashMap<String, (PointerValue<'ctx>, Ty)>,
    loop_stack: Vec<(inkwell::basic_block::BasicBlock<'ctx>,
                     inkwell::basic_block::BasicBlock<'ctx>)>,
    counter:    usize,

    // Flat list iteration support
    // "param_name" → FlatListInfo for `let fixed_list[T]` params where T is SWD
    flat_list_params: HashMap<String, FlatListInfo>,
    // Dotted path (e.g. "item.v", "item.start.x") → (alloca holding value, Ty)
    // Populated at flat loop body entry; used by gen_expr for zero-callback field reads
    preread_fields: HashMap<String, (PointerValue<'ctx>, Ty)>,
}

impl<'ctx> GenCtx<'ctx> {
    fn new(
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

    fn fresh(&mut self, prefix: &str) -> String {
        let n = self.counter; self.counter += 1; format!("{prefix}{n}")
    }

    // ── ArCallbacks access ────────────────────────────────────────────────────

    /// Load @CB pointer
    fn load_cb(&self) -> PointerValue<'ctx> {
        self.bld.build_load(self.ptr, self.cb_global, "cb")
            .unwrap().into_pointer_value()
    }

    /// Load a function pointer from @CB[field_idx]
    fn load_cb_fn(&self, cb: PointerValue<'ctx>, field: u32) -> PointerValue<'ctx> {
        let fp = self.bld
            .build_struct_gep(self.cb_ty, cb, field, "fp").unwrap();
        self.bld.build_load(self.ptr, fp, "fn_ptr")
            .unwrap().into_pointer_value()
    }

    /// Call a callback that returns i64. `arg_types` is the LLVM fn param list.
    fn call_cb_i64(
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
    fn call_cb_f64(
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
    fn call_cb_void(
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
    fn call_cb_i32(
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

    fn to_handle(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
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

    fn to_i64(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
        match ty {
            Ty::Int    => val.into_int_value(),
            Ty::Handle => self.call_cb_i64(CB_TO_INT, &[self.i64.into()], &[val.into()], "i"),
            Ty::Float  => self.bld
                .build_float_to_signed_int(val.into_float_value(), self.i64, "i").unwrap(),
            Ty::Bool   => self.bld
                .build_int_z_extend(val.into_int_value(), self.i64, "i").unwrap(),
        }
    }

    fn to_f64(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> FloatValue<'ctx> {
        match ty {
            Ty::Float  => val.into_float_value(),
            Ty::Int    => self.bld
                .build_signed_int_to_float(val.into_int_value(), self.f64, "f").unwrap(),
            Ty::Handle => self.call_cb_f64(CB_TO_FLOAT, &[self.i64.into()], &[val.into()], "f"),
            Ty::Bool   => self.bld
                .build_unsigned_int_to_float(val.into_int_value(), self.f64, "f").unwrap(),
        }
    }

    fn to_cond(&self, val: BasicValueEnum<'ctx>, ty: Ty) -> IntValue<'ctx> {
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

    fn gen_expr(&mut self, module: &Module<'ctx>, expr: &Expr) -> (BasicValueEnum<'ctx>, Ty) {
        match expr {
            Expr::Int(n) => (self.i64.const_int(*n as u64, true).into(), Ty::Int),
            Expr::Float(f) => (self.f64.const_float(*f).into(), Ty::Float),
            Expr::Bool(b) => {
                let h = if *b { 1u64 } else { 2u64 };
                (self.i64.const_int(h, false).into(), Ty::Handle)
            }
            Expr::None => (self.i64.const_zero().into(), Ty::Handle),

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

    fn gen_binop(
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

    fn specialize_binop(
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

    fn gen_call(&mut self, module: &Module<'ctx>, func: &Expr, args: &[CallArg]) -> IntValue<'ctx> {
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

    /// Returns true if the current basic block was terminated (return/break/continue).
    fn gen_stmts(&mut self, module: &Module<'ctx>, stmts: &[Stmt]) -> bool {
        for s in stmts {
            if self.gen_stmt(module, s) { return true; }
        }
        false
    }

    /// Returns true if this statement terminates the block (ret, br).
    fn gen_stmt(&mut self, module: &Module<'ctx>, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Let(name, expr) | Stmt::Const(name, expr) => {
                let (v, vt) = self.gen_expr(module, expr);
                let st  = store_ty(vt);
                let ptr = self.build_entry_alloca(name, st);
                self.store_coerced(v, vt, st, ptr);
                self.locals.insert(name.clone(), (ptr, st));
                false
            }
            Stmt::Mut(name, expr) => {
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

    fn emit_fn(&mut self, module: &Module<'ctx>, name: &str, params: &[Param], body: &[Stmt]) {
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
    fn build_entry_alloca(&self, name: &str, ty: Ty) -> PointerValue<'ctx> {
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

    fn load_alloca(&self, ptr: PointerValue<'ctx>, ty: Ty) -> BasicValueEnum<'ctx> {
        match ty {
            Ty::Float => self.bld.build_load(self.f64, ptr, "lf").unwrap(),
            _         => self.bld.build_load(self.i64, ptr, "li").unwrap(),
        }
    }

    fn store_coerced(&self, val: BasicValueEnum<'ctx>, vt: Ty, st: Ty, ptr: PointerValue<'ctx>) {
        let coerced: BasicValueEnum<'ctx> = match st {
            Ty::Int   => self.to_i64(val, vt).into(),
            Ty::Float => self.to_f64(val, vt).into(),
            _         => self.to_handle(val, vt).into(),
        };
        self.bld.build_store(coerced, ptr).unwrap();
    }

    /// Create a global string constant and return a pointer to it.
    fn make_str_const(&self, module: &Module<'ctx>, bytes: &[u8]) -> PointerValue<'ctx> {
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

    fn build_floor(&self, module: &Module<'ctx>, v: FloatValue<'ctx>) -> FloatValue<'ctx> {
        let f = self.get_or_declare(module, "llvm.floor.f64",
            self.f64.fn_type(&[self.f64.into()], false));
        self.bld.build_call(f, &[v.into()], "floor")
            .unwrap().try_as_basic_value().left().unwrap().into_float_value()
    }

    fn build_pow(&self, module: &Module<'ctx>, base: FloatValue<'ctx>, exp: FloatValue<'ctx>) -> FloatValue<'ctx> {
        let f = self.get_or_declare(module, "llvm.pow.f64",
            self.f64.fn_type(&[self.f64.into(), self.f64.into()], false));
        self.bld.build_call(f, &[base.into(), exp.into()], "pow")
            .unwrap().try_as_basic_value().left().unwrap().into_float_value()
    }

    fn get_or_declare(&self, module: &Module<'ctx>, name: &str,
                      ty: inkwell::types::FunctionType<'ctx>) -> FunctionValue<'ctx> {
        module.get_function(name).unwrap_or_else(|| module.add_function(name, ty, None))
    }
}

// ── Eligibility (identical to llvm_codegen.rs) ────────────────────────────────

fn body_eligible(stmts: &[Stmt]) -> bool { stmts.iter().all(stmt_eligible) }

fn stmt_eligible(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let(_, e) | Stmt::Mut(_, e) | Stmt::Const(_, e) => expr_eligible(e),
        Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => expr_eligible(value),
        Stmt::AttrAssign { target, value } => expr_eligible(target) && expr_eligible(value),
        Stmt::AttrCompoundAssign { target, value, .. } => expr_eligible(target) && expr_eligible(value),
        Stmt::Return(Some(e)) => expr_eligible(e),
        Stmt::Return(None) | Stmt::Pass | Stmt::Break | Stmt::Continue => true,
        Stmt::Expr(e) => expr_eligible(e),
        Stmt::Freeze(..) => true,
        Stmt::Block(ss) => body_eligible(ss),
        Stmt::If { branches, else_body } =>
            branches.iter().all(|(c, b)| expr_eligible(c) && body_eligible(b))
            && else_body.as_ref().map_or(true, |b| body_eligible(b)),
        Stmt::While { cond, body } => expr_eligible(cond) && body_eligible(body),
        Stmt::For { targets, iter, body } =>
            targets.len() == 1 && expr_eligible(iter) && body_eligible(body),
        Stmt::Match { subject, arms, .. } =>
            expr_eligible(subject) && arms.iter().all(|a| {
                (match &a.pattern {
                    MatchPattern::Case(e) => expr_eligible(e),
                    MatchPattern::IsType(_) => true,
                }) && body_eligible(&a.body)
            }),
        _ => false,
    }
}

fn expr_eligible(expr: &Expr) -> bool {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::ImaginaryLit(_)
        | Expr::Str(_) | Expr::Bool(_) | Expr::None => true,
        Expr::Ident(_) => true,
        Expr::BinOp { left, right, .. } => expr_eligible(left) && expr_eligible(right),
        Expr::UnaryOp { operand, .. } => expr_eligible(operand),
        Expr::List(items) | Expr::Tuple(items) => items.iter().all(expr_eligible),
        Expr::Dict(pairs) => pairs.iter().all(|(k, v)| expr_eligible(k) && expr_eligible(v)),
        Expr::Call { func, args, .. } =>
            expr_eligible(func) && args.iter().all(|a| matches!(a, CallArg::Positional(e) if expr_eligible(e))),
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => expr_eligible(object),
        Expr::Subscript { object, index } => expr_eligible(object) && expr_eligible(index),
        Expr::IsType { expr, .. } => expr_eligible(expr),
        Expr::TemplateInstantiate { base, .. } => expr_eligible(base),
        _ => false,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn binop_code(op: &BinOp) -> i32 {
    match op {
        BinOp::Add => 0, BinOp::Sub => 1, BinOp::Mul => 2, BinOp::Div => 3,
        BinOp::FloorDiv => 4, BinOp::Mod => 5, BinOp::Pow => 6,
        BinOp::Eq => 7, BinOp::NotEq => 8, BinOp::Lt => 9, BinOp::LtEq => 10,
        BinOp::Gt => 11, BinOp::GtEq => 12, BinOp::BitAnd => 13,
        BinOp::BitOr => 14, BinOp::BitXor => 15, BinOp::LShift => 16,
        BinOp::RShift => 17, BinOp::In => 18, BinOp::NotIn => 19,
        BinOp::And | BinOp::Or => unreachable!(),
    }
}

fn simple_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}

// ── Public entry points ───────────────────────────────────────────────────────

/// Compile eligible functions from `stmts` via LLVM JIT.
/// Returns the JIT handle (keeps code alive) and a list of exports with fn ptrs.
pub fn compile_jit(stmts: &[crate::ast::Stmt])
    -> Result<(JitHandle, Vec<FnExport>), String>
{
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("LLVM target init: {e}"))?;

    struct EligibleFn<'a> {
        name: &'a str,
        params: &'a [Param],
        return_type: Option<&'a str>,
        body: &'a [crate::ast::Stmt],
    }

    let eligible: Vec<EligibleFn> = stmts.iter().filter_map(|s| {
        if let Stmt::FnDef { name, template_params, params, body, is_abstract, return_type, .. } = s {
            if !template_params.is_empty() || *is_abstract || !body_eligible(body) {
                return None;
            }
            Some(EligibleFn { name, params, return_type: return_type.as_deref(), body })
        } else { None }
    }).collect();

    if eligible.is_empty() {
        return Err("no JIT-eligible functions".to_string());
    }

    let module_fns: HashSet<String> = eligible.iter().map(|f| f.name.to_string()).collect();
    let fn_sigs: HashMap<String, FnSig> = eligible.iter().map(|f| (
        f.name.to_string(),
        FnSig {
            ret: ann_ty(f.return_type),
            param_mutabilities: f.params.iter().map(|p| p.mutable).collect(),
        }
    )).collect();

    // Collect all non-template class instance fields for flat SWD leaf detection.
    let all_class_fields: HashMap<String, Vec<(String, String)>> = stmts.iter()
        .filter_map(|s| {
            if let Stmt::ClassDef { name, body, template_params, .. } = s {
                if !template_params.is_empty() { return None; }
                let fields: Vec<(String, String)> = body.iter()
                    .filter_map(|st| {
                        if let Stmt::Field { name: fname, type_ann, kind, .. } = st {
                            if matches!(kind, FieldKind::Mut | FieldKind::Let) {
                                Some((fname.clone(), type_ann.clone()))
                            } else { None }
                        } else { None }
                    })
                    .collect();
                Some((name.clone(), fields))
            } else { None }
        })
        .collect();

    let context = Context::create();
    let module  = context.create_module("ar_native");

    // Emit ar_init: store CB pointer into @CB global
    {
        let ptr_t   = context.ptr_type(AddressSpace::default());
        let void_t  = context.void_type();
        let init_ty = void_t.fn_type(&[ptr_t.into()], false);
        let ar_init = module.add_function("ar_init", init_ty, None);
        let entry   = context.append_basic_block(ar_init, "entry");
        let bld     = context.create_builder();
        bld.position_at_end(entry);
        let cb_arg   = ar_init.get_first_param().unwrap().into_pointer_value();
        let cb_global = module.get_global("CB");
        if let Some(g) = cb_global {
            bld.build_store(cb_arg, g.as_pointer_value()).unwrap();
        }
        bld.build_return(None).unwrap();
    }

    let mut gen = GenCtx::new(&context, &module, module_fns, fn_sigs, all_class_fields);

    let fn_names: Vec<&str> = eligible.iter().map(|f| f.name).collect();
    eprintln!("NativeLib: compiling {} function(s): {}", fn_names.len(), fn_names.join(", "));

    for f in &eligible {
        gen.emit_fn(&module, f.name, f.params, f.body);
    }

    // Run optimisation passes
    let pm = inkwell::passes::PassManager::create(());
    pm.add_promote_memory_to_register_pass();
    pm.add_instruction_combining_pass();
    pm.add_reassociate_pass();
    pm.add_gvn_pass();
    pm.add_cfg_simplification_pass();
    pm.run_on(&module);

    // JIT compile
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT init failed: {}", e.to_string()))?;

    // Collect function pointers
    let exports: Vec<FnExport> = eligible.iter().map(|f| {
        let symbol = format!("{}_tl", f.name);
        let fn_ptr = unsafe { engine.get_function_address(&symbol) } as usize;
        FnExport { name: f.name.to_string(), n_params: f.params.len(), fn_ptr }
    }).collect();

    eprintln!("NativeLib: JIT compiled {} function(s)", exports.len());

    // Produce bitcode for .arc embedding
    // (bitcode is also obtainable via get_bitcode() if needed)

    let handle = JitHandle::new(context, engine);
    Ok((handle, exports))
}

/// Generate LLVM bitcode bytes for the eligible functions.
/// The bitcode can be stored in a .arc v2 file and later re-JIT'd via `jit_from_bitcode`.
pub fn get_bitcode(stmts: &[crate::ast::Stmt]) -> Result<(Vec<u8>, Vec<crate::partial_compiler::llvm_codegen::FnExport>), String> {
    use crate::partial_compiler::llvm_codegen::FnExport as LegacyExport;

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("LLVM target init: {e}"))?;

    struct EligibleFn<'a> {
        name: &'a str,
        params: &'a [Param],
        return_type: Option<&'a str>,
        body: &'a [crate::ast::Stmt],
    }

    let eligible: Vec<EligibleFn> = stmts.iter().filter_map(|s| {
        if let Stmt::FnDef { name, template_params, params, body, is_abstract, return_type, .. } = s {
            if !template_params.is_empty() || *is_abstract || !body_eligible(body) {
                return None;
            }
            Some(EligibleFn { name, params, return_type: return_type.as_deref(), body })
        } else { None }
    }).collect();

    if eligible.is_empty() {
        return Err("no JIT-eligible functions".to_string());
    }

    let module_fns: HashSet<String> = eligible.iter().map(|f| f.name.to_string()).collect();
    let fn_sigs: HashMap<String, FnSig> = eligible.iter().map(|f| (
        f.name.to_string(),
        FnSig {
            ret: ann_ty(f.return_type),
            param_mutabilities: f.params.iter().map(|p| p.mutable).collect(),
        }
    )).collect();

    let all_class_fields: HashMap<String, Vec<(String, String)>> = stmts.iter()
        .filter_map(|s| {
            if let Stmt::ClassDef { name, body, template_params, .. } = s {
                if !template_params.is_empty() { return None; }
                let fields: Vec<(String, String)> = body.iter()
                    .filter_map(|st| {
                        if let Stmt::Field { name: fname, type_ann, kind, .. } = st {
                            if matches!(kind, FieldKind::Mut | FieldKind::Let) {
                                Some((fname.clone(), type_ann.clone()))
                            } else { None }
                        } else { None }
                    })
                    .collect();
                Some((name.clone(), fields))
            } else { None }
        })
        .collect();

    let context = Context::create();
    let module  = context.create_module("ar_native");
    let mut gen = GenCtx::new(&context, &module, module_fns, fn_sigs, all_class_fields);
    for f in &eligible {
        gen.emit_fn(&module, f.name, f.params, f.body);
    }

    let pm = inkwell::passes::PassManager::create(());
    pm.add_promote_memory_to_register_pass();
    pm.add_instruction_combining_pass();
    pm.add_cfg_simplification_pass();
    pm.run_on(&module);

    let bitcode = module.write_bitcode_to_memory_buffer();
    let bytes   = bitcode.as_slice().to_vec();

    let exports: Vec<LegacyExport> = eligible.iter()
        .map(|f| LegacyExport { name: f.name.to_string(), n_params: f.params.len() })
        .collect();

    Ok((bytes, exports))
}

/// Re-JIT LLVM bitcode bytes previously produced by `get_bitcode`.
/// Returns `(JitHandle, fn_ptrs)` where fn_ptrs maps fn_name → address.
pub fn jit_from_bitcode(
    bitcode:  &[u8],
    exports:  &[crate::partial_compiler::llvm_codegen::FnExport],
) -> Result<(JitHandle, Vec<(String, usize)>), String> {
    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("LLVM target init: {e}"))?;

    let context = Context::create();
    let buf     = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range(bitcode, "bc");
    let module  = Module::parse_bitcode_from_buffer(&buf, &context)
        .map_err(|e| format!("bitcode parse: {}", e.to_string()))?;

    let engine  = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT init: {}", e.to_string()))?;

    let fn_ptrs: Vec<(String, usize)> = exports.iter().map(|e| {
        let symbol = format!("{}_tl", e.name);
        let ptr    = unsafe { engine.get_function_address(&symbol) } as usize;
        (e.name.clone(), ptr)
    }).collect();

    let handle = JitHandle::new(context, engine);
    Ok((handle, fn_ptrs))
}
