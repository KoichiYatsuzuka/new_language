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
    // フィールドは宣言順（C ABI 準拠 — for_claude/c_abi_interop.md P0c）
    let fields = match all_class_fields.get(class_name) {
        Some(f) if !f.is_empty() => f.clone(),
        _ => return vec![],
    };

    let mut leaves = Vec::new();
    let mut byte_offset = byte_base;
    for (fname, ftype) in fields {
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
    // C ABI 型（int32 等）は基底型（int/float）として扱う
    let s = s.map(|a| crate::ast::c_abi_base_type(a).unwrap_or(a));
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


// ── Eligibility (identical to llvm_codegen.rs) ────────────────────────────────

fn body_eligible(stmts: &[Stmt]) -> bool { stmts.iter().all(stmt_eligible) }

fn stmt_eligible(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Const(_, _, e) => expr_eligible(e),
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
        | Expr::Str(_) | Expr::Bool(_) | Expr::None | Expr::Undefined => true,
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
            if params.iter().any(|p| p.type_ann.as_deref().map_or(false, |a| a.contains("Intersection[")))
                || return_type.as_deref().map_or(false, |a| a.contains("Intersection[")) {
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
            if params.iter().any(|p| p.type_ann.as_deref().map_or(false, |a| a.contains("Intersection[")))
                || return_type.as_deref().map_or(false, |a| a.contains("Intersection[")) {
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

mod context;
mod expr;
mod stmt;
mod function;
