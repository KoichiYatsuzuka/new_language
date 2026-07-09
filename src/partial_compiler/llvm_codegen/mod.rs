// llvm_codegen.rs — LLVM IR text generator for Arrow native compilation.
//
// Replaces codegen.rs. Emits LLVM IR (.ll) instead of Rust source (.rs).
// The .ll file is compiled to a shared library by module_compiler using clang.
//
// The handle-based ABI (all values as i64 tags) and ArCallbacks struct layout
// are preserved exactly from the old codegen, so exec.rs/native_api.rs are
// unchanged.
//
// Eligibility rules are identical to the old codegen.

use std::collections::{HashMap, HashSet};
use crate::ast::{BinOp, CallArg, Expr, MatchPattern, Param, Stmt};

// ── Public types ──────────────────────────────────────────────────────────────

/// ネイティブコンパイルされた関数の公開情報。`.arc` ファイルへ埋め込まれる。
#[derive(Debug, Clone)]
pub struct FnExport {
    /// エクスポートされた関数名（LLVM IR および DLL のシンボル名と一致する）。
    pub name:       String,
    /// 関数が受け取るパラメータ数（`self` を含む）。
    pub n_params:   usize,
    /// クラスメソッドの場合は所有クラス名、通常関数は `None`。
    pub class_name: Option<String>,
}

// ── Internal type system ──────────────────────────────────────────────────────

/// コードジェネレータ内部で使用する型区分。
/// LLVM IR の型と handle ABI の間のマッピングに使用する。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ty {
    /// `i64` ネイティブ整数として扱う。
    Int,
    /// `double` ネイティブ浮動小数点として扱う。
    Float,
    /// `i64` ハンドル（`TL_TRUE`/`TL_FALSE`）として扱う。ストア時は Handle に昇格。
    Bool,
    /// `i64` ハンドル（アリーナ参照）として扱う。
    Handle,
}

/// コードジェネレータ内部の関数シグネチャ情報。
struct FnSig {
    /// 戻り値の型区分。
    ret: Ty,
    /// パラメータごとの可変フラグリスト。`true` は `mut` パラメータ。
    param_mutabilities: Vec<bool>,
}

/// フラット配列の末端プリミティブフィールド情報。ネストしたクラスは再帰的に展開される。
#[derive(Clone)]
struct FlatLeaf {
    /// ループ変数からのドット区切りパス。例: `"v"`, `"start.x"`。
    path: String,
    /// 要素先頭からのバイトオフセット。
    byte_offset: usize,
    /// 型（Int または Float のみ）。
    ty: Ty,
}

/// `let fixed_list[ClassName]` パラメータの平坦レイアウト情報。
/// クラスの数値フィールドを連続メモリに展開した flat-array 形式でネイティブコードに渡す。
#[derive(Clone)]
struct FlatListInfo {
    /// 要素クラス名。
    class_name: String,
    /// 全末端プリミティブフィールドのリスト（深さ優先アルファベット順）。
    leaves: Vec<FlatLeaf>,
    /// 要素あたりのバイト数 = `leaves.len() * 8`。
    stride: usize,
}

/// クラス `class_name` の全 SWD 末端フィールドを収集する（再帰的）。
/// テンプレートを持つクラスや非 SWD フィールドが含まれる場合は空 Vec を返す。
fn collect_flat_leaves(
    all_class_fields: &std::collections::HashMap<String, Vec<(String, String)>>,
    class_name: &str,
    path_prefix: &str,
    byte_base: usize,
) -> Vec<FlatLeaf> {
    // フィールドは宣言順（C ABI 準拠 — .claude/skills/c-abi-interop/SKILL.md P0c）
    let raw_fields = match all_class_fields.get(class_name) {
        Some(f) if !f.is_empty() => f.clone(),
        _ => return vec![],
    };

    let mut leaves = Vec::new();
    let mut byte_offset = byte_base;

    for (fname, ftype) in raw_fields {
        let full_path = if path_prefix.is_empty() {
            fname.clone()
        } else {
            format!("{path_prefix}.{fname}")
        };
        match ftype.as_str() {
            "int" => {
                leaves.push(FlatLeaf { path: full_path, byte_offset, ty: Ty::Int });
                byte_offset += 8;
            }
            "float" => {
                leaves.push(FlatLeaf { path: full_path, byte_offset, ty: Ty::Float });
                byte_offset += 8;
            }
            nested => {
                let sub = collect_flat_leaves(all_class_fields, nested, &full_path, byte_offset);
                if sub.is_empty() { return vec![]; } // not SWD
                byte_offset += sub.len() * 8;
                leaves.extend(sub);
            }
        }
    }
    leaves
}

/// ネストした属性アクセス式をドット区切りパス文字列に変換する。
/// 例: `item.start.x` → `"item.start.x"`。変換できない場合は `None`。
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

/// 型アノテーション文字列を内部型区分 `Ty` に変換する。
/// `"int"` → `Ty::Int`、`"float"` → `Ty::Float`、その他は `Ty::Handle`。
/// C ABI 型（int32 等）は基底型（int/float）として扱う（Arrow 内部は幅変換しない）。
fn ann_ty(s: Option<&str>) -> Ty {
    let s = s.map(|a| crate::ast::c_abi_base_type(a).unwrap_or(a));
    match s {
        Some("int")   => Ty::Int,
        Some("float") => Ty::Float,
        _             => Ty::Handle,
    }
}

/// `Bool` 型を `Handle` に昇格させる（ストア時に使用）。
/// Bool 値はネイティブ i64 ではなく Handle (TL_TRUE/TL_FALSE) として格納する必要があるため。
fn store_ty(t: Ty) -> Ty { if t == Ty::Bool { Ty::Handle } else { t } }

/// 型区分に対応する LLVM IR 型文字列を返す。
/// `Float` → `"double"`、それ以外 → `"i64"`。
fn llvm_ty(ty: Ty) -> &'static str {
    if ty == Ty::Float { "double" } else { "i64" }
}

// ── ArCallbacks field indices (must match native_api.rs) ─────────────────────

const CB_MAKE_INT:      usize = 0;
const CB_MAKE_FLOAT:    usize = 1;
const CB_MAKE_STR:      usize = 3;
const CB_MAKE_LIST:     usize = 4;
const CB_MAKE_TUPLE:    usize = 5;
const CB_MAKE_DICT:     usize = 6;
const CB_IS_TRUTHY:     usize = 8;
const CB_BINOP:         usize = 9;
const CB_UNOP:          usize = 10;
const CB_CALL_FN:       usize = 11;
const CB_GET_ATTR:      usize = 12;
const CB_SET_ATTR:      usize = 13;
const CB_SUBSCRIPT:     usize = 14;
const CB_GET_GLOBAL:    usize = 15;
const CB_ITER_FROM:     usize = 16;
const CB_ITER_NEXT:     usize = 17;
const CB_IS_TYPE:       usize = 18;
const CB_ARENA_SAVE:    usize = 19;
const CB_ARENA_COMPACT: usize = 20;
const CB_TO_INT:        usize = 22;
const CB_TO_FLOAT:      usize = 23;
const CB_DEEP_COPY:     usize = 24;
const CB_LIST_APPEND:   usize = 27;
const CB_RAISE:         usize = 28;
const CB_MAKE_CELL:       usize = 29;
const CB_GET_CELL:        usize = 30;
const CB_SET_CELL:        usize = 31;
const CB_CALL_METHOD:     usize = 32;
const CB_GET_FLOAT_FIELD: usize = 33;
const CB_GET_INT_FIELD:   usize = 34;
const CB_FLAT_DATA_PTR:   usize = 35;
const CB_FLAT_LEN:        usize = 36;
const CB_FN_TRAMPOLINE:   usize = 37;

/// Symbol name for a class method export: `{ClassName}__{method_name}`.
pub fn method_symbol(class_name: &str, method_name: &str) -> String {
    format!("{class_name}__{method_name}")
}

/// Returns (return_type_str, param_types_str) for a callback field.
fn cb_sig(field: usize) -> (&'static str, &'static str) {
    match field {
        CB_MAKE_INT      => ("i64",    "i64"),
        CB_MAKE_FLOAT    => ("i64",    "double"),
        CB_MAKE_STR      => ("i64",    "ptr, i32"),
        CB_MAKE_LIST     => ("i64",    "ptr, i32"),
        CB_MAKE_TUPLE    => ("i64",    "ptr, i32"),
        CB_MAKE_DICT     => ("i64",    "ptr, ptr, i32"),
        CB_IS_TRUTHY     => ("i32",    "i64"),
        CB_BINOP         => ("i64",    "i32, i64, i64"),
        CB_UNOP          => ("i64",    "i32, i64"),
        CB_CALL_FN       => ("i64",    "i64, ptr, i32"),
        CB_GET_ATTR      => ("i64",    "i64, ptr, i32"),
        CB_SET_ATTR      => ("void",   "i64, ptr, i32, i64"),
        CB_SUBSCRIPT     => ("i64",    "i64, i64"),
        CB_GET_GLOBAL    => ("i64",    "ptr, i32"),
        CB_ITER_FROM     => ("i64",    "i64"),
        CB_ITER_NEXT     => ("i64",    "i64"),
        CB_IS_TYPE       => ("i64",    "i64, ptr, i32"),
        CB_ARENA_SAVE    => ("i64",    ""),
        CB_ARENA_COMPACT => ("i64",    "i64, i64"),
        CB_TO_INT        => ("i64",    "i64"),
        CB_TO_FLOAT      => ("double", "i64"),
        CB_DEEP_COPY     => ("i64",    "i64"),
        CB_LIST_APPEND   => ("i64",    "i64, i64"),
        CB_RAISE         => ("i64",    "i64, i64"),
        CB_MAKE_CELL     => ("i64",    "i64"),
        CB_GET_CELL      => ("i64",    "i64"),
        CB_SET_CELL      => ("void",   "i64, i64"),
        CB_CALL_METHOD      => ("i64",    "i64, ptr, i32, ptr, i32"),
        CB_GET_FLOAT_FIELD  => ("double", "i64, ptr, i32"),
        CB_GET_INT_FIELD    => ("i64",    "i64, ptr, i32"),
        CB_FLAT_DATA_PTR    => ("i64",    "i64"),
        CB_FLAT_LEN         => ("i64",    "i64"),
        CB_FN_TRAMPOLINE    => ("ptr",    "i64"),
        _                   => ("i64",    "i64"),
    }
}

// ── Module-level header (constant) ────────────────────────────────────────────

fn module_header() -> &'static str {
    r#"; Auto-generated by Arrow compiler — do not edit.

; ArCallbacks: 38 function-pointer fields (opaque ptr, LLVM 15+)
%ArCallbacks = type { ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr, ptr }

@CB = internal global ptr null

define void @ar_init(ptr %cb) {
  store ptr %cb, ptr @CB
  ret void
}

; Python-compatible floor division for i64
define internal i64 @_tl_idiv(i64 %a, i64 %b) {
  %d    = sdiv i64 %a, %b
  %r    = srem i64 %a, %b
  %rnz  = icmp ne i64 %r, 0
  %rneg = icmp slt i64 %r, 0
  %bneg = icmp slt i64 %b, 0
  %diff = xor i1 %rneg, %bneg
  %need = and i1 %rnz, %diff
  %dadj = sub i64 %d, 1
  %res  = select i1 %need, i64 %dadj, i64 %d
  ret i64 %res
}

; Python-compatible modulo for i64
define internal i64 @_tl_imod(i64 %a, i64 %b) {
  %r    = srem i64 %a, %b
  %rnz  = icmp ne i64 %r, 0
  %rneg = icmp slt i64 %r, 0
  %bneg = icmp slt i64 %b, 0
  %diff = xor i1 %rneg, %bneg
  %need = and i1 %rnz, %diff
  %radj = add i64 %r, %b
  %res  = select i1 %need, i64 %radj, i64 %r
  ret i64 %res
}

declare double @llvm.pow.f64(double, double)
declare double @llvm.floor.f64(double)

"#
}

/// dllexport attribute on Windows, empty on other platforms.
fn export_attr() -> &'static str {
    if cfg!(target_os = "windows") { "dllexport " } else { "" }
}

// ── Block context stack (for block_return / loop_yield) ───────────────────────

#[derive(Clone)]
struct BlockCtx {
    /// alloca holding the block's result value (i64).
    result_al: String,
    /// label to branch to when block_return is executed.
    exit_label: String,
    /// alloca holding the accumulated list for loop_yield (only for for/while expressions).
    list_al: Option<String>,
}

// ── Code generation context ───────────────────────────────────────────────────

struct GenCtx<'a> {
    // Module-level accumulators
    str_globals: String,     // @_sN string constant definitions
    fn_defs:     String,     // all generated function bodies

    // Per-function buffers
    alloca_buf: String,      // alloca instructions (entry block only)
    code_buf:   String,      // rest of the function body
    reg:        usize,
    blk:        usize,
    terminated: bool,        // current basic block already has a terminator

    // Variable table: name → (alloca_reg, storage_Ty)
    locals: HashMap<String, (String, Ty)>,

    // Loop stack: (header_label, exit_label)
    loop_stack: Vec<(String, String)>,
    // Block context stack for block_return / loop_yield
    block_stack: Vec<BlockCtx>,

    // Module function metadata
    module_fns: &'a HashSet<String>,
    fn_sigs:    &'a HashMap<String, FnSig>,

    // String constant deduplication
    str_consts: HashMap<Vec<u8>, String>,
    str_ctr:    usize,

    // Class field type information for type-specialised field reads.
    // class_name → field_name → Ty (Int | Float only; Handle fields are excluded)
    class_fields: &'a HashMap<String, HashMap<String, Ty>>,
    // Ordered typed field list per class (declaration order, same slice used for _fast ABI).
    class_fields_ord: &'a HashMap<String, Vec<(String, Ty)>>,
    // All instance fields for every non-template class: class_name → [(field_name, type_ann)].
    // Used by collect_flat_leaves for recursive SWD class detection.
    all_class_fields: &'a HashMap<String, Vec<(String, String)>>,
    // Owning class of the method currently being compiled (None for top-level fns).
    current_class: Option<String>,
    // param_name → class_name for parameters whose types are known class instances.
    param_classes: HashMap<String, String>,
    // Return type of the function currently being compiled.
    // Float-returning _impl functions use "double" ABI (no boxing/unboxing).
    current_fn_ret: Ty,
    // Functions that have a _fast variant (pure class params → scalar ABI).
    fast_fns: &'a HashSet<String>,
    // Pre-read field locals: dotted path (e.g. "item.v", "item.start.x") → (alloca_ptr, Ty).
    // Populated at flat-list loop body entry; gen_expr uses this for zero-callback field reads.
    preread_fields: HashMap<String, (String, Ty)>,
    // `let fixed_list[ClassName]` params that can use the flat GEP iteration path.
    flat_list_params: HashMap<String, FlatListInfo>,
    // `function[...]->R` typed params: param_name → alloca ptr holding the trampoline fn ptr.
    // Populated at function entry; used by gen_call to avoid the ArCallbacks GEP chain.
    fn_param_trampolines: HashMap<String, String>,

    // ── Typed ABI (`{name}_typed`) emission mode ──────────────────────────────
    // typed_mode: emitting a typed entry (raw-value ABI, ErrSlot error path, no CB).
    // typed_failed: set when the body needed any ArCallbacks call (→ discard variant).
    typed_mode: bool,
    typed_failed: bool,
    // Symbols currently assumed to have a _typed variant (fixpoint set).
    typed_ok: HashSet<String>,
    // symbol → (param Tys, ret Ty) for typed candidates (Int/Float only).
    typed_sigs: HashMap<String, (Vec<Ty>, Ty)>,
}


// ── Eligibility (identical to old codegen) ────────────────────────────────────

fn body_eligible(stmts: &[Stmt]) -> bool { stmts.iter().all(stmt_eligible) }

fn body_has_loop_yield(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_has_loop_yield)
}

fn stmt_has_loop_yield(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::LoopYield(_) => true,
        Stmt::Block(ss) => body_has_loop_yield(ss),
        Stmt::If { branches, else_body } =>
            branches.iter().any(|(_, b)| body_has_loop_yield(b))
            || else_body.as_ref().map_or(false, |b| body_has_loop_yield(b)),
        // Do NOT descend into nested For/While — loop_yield there belongs to that inner loop
        _ => false,
    }
}

fn stmt_eligible(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Const(_, _, e) => expr_eligible(e),
        Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => expr_eligible(value),
        Stmt::AttrAssign { target, value } => expr_eligible(target) && expr_eligible(value),
        Stmt::AttrCompoundAssign { target, value, .. } => expr_eligible(target) && expr_eligible(value),
        Stmt::Return(Some(e)) => expr_eligible(e),
        Stmt::Return(None) | Stmt::Pass | Stmt::Break | Stmt::Continue => true,
        Stmt::Expr(e)    => expr_eligible(e),
        Stmt::Freeze(..) => true,
        Stmt::Block(ss)  => body_eligible(ss),
        Stmt::BlockReturn(e, _) => expr_eligible(e),
        Stmt::LoopYield(e)      => expr_eligible(e),
        Stmt::Yield(e)          => expr_eligible(e),
        // raise ExcType(msg) — only positional constructor calls
        Stmt::Raise { exc: Some(e), .. } => matches!(e, Expr::Call { args, .. }
            if args.iter().all(|a| matches!(a, CallArg::Positional(e) if expr_eligible(e)))),
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
        // Control-flow expressions (block_return / loop_yield inside these is also handled)
        Expr::Block { stmts, .. } => body_eligible(stmts),
        Expr::IfExpr { branches, else_body, .. } =>
            branches.iter().all(|(c, b)| expr_eligible(c) && body_eligible(b))
            && else_body.as_ref().map_or(true, |b| body_eligible(b)),
        Expr::ForExpr { iter, body, .. } => expr_eligible(iter) && body_eligible(body),
        Expr::WhileExpr { cond, body, .. } => expr_eligible(cond) && body_eligible(body),
        Expr::MatchExpr { subject, arms, .. } =>
            expr_eligible(subject) && arms.iter().all(|a| {
                (match &a.pattern {
                    MatchPattern::Case(e) => expr_eligible(e),
                    MatchPattern::IsType(_) => true,
                }) && body_eligible(&a.body)
            }),
        _ => false,
    }
}

// ── Purity analysis for approach-1 pre-reads ─────────────────────────────────

/// Returns true if any statement in `stmts` directly writes a field of `param`
/// (via AttrAssign or AttrCompoundAssign where the object is `Expr::Ident(param)`).
fn body_writes_param(stmts: &[Stmt], param: &str) -> bool {
    stmts.iter().any(|s| stmt_writes_param(s, param))
}

fn stmt_writes_param(stmt: &Stmt, param: &str) -> bool {
    match stmt {
        Stmt::AttrAssign { target, .. } | Stmt::AttrCompoundAssign { target, .. } => {
            if let Expr::Attr { object, .. } = target {
                matches!(object.as_ref(), Expr::Ident(n) if n == param)
            } else { false }
        }
        Stmt::If { branches, else_body } =>
            branches.iter().any(|(_, b)| body_writes_param(b, param))
            || else_body.as_ref().map_or(false, |b| body_writes_param(b, param)),
        Stmt::While { body, .. } | Stmt::For { body, .. } => body_writes_param(body, param),
        Stmt::Block(ss) => body_writes_param(ss, param),
        Stmt::Match { arms, .. } => arms.iter().any(|a| body_writes_param(&a.body, param)),
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
        BinOp::RefEq => 20,
        BinOp::And | BinOp::Or => unreachable!("and/or handled separately"),
    }
}

/// Format an f64 as a valid LLVM IR float constant.
/// Rust's `Display` omits the decimal point for whole-number floats (e.g. `0.0` → `"0"`),
/// which LLVM rejects as an integer constant.  Append `.0` when needed.
fn fmt_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains('.') || s.contains('e') || s.contains('E') { s } else { format!("{s}.0") }
}

fn escape_for_llvm(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"'  => out.push_str("\\22"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\{b:02X}")),
        }
    }
    out
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Generator body is eligible if it uses only yield, not block_return/loop_yield
fn body_eligible_gen(stmts: &[Stmt]) -> bool {
    stmts.iter().all(|s| stmt_eligible_gen(s))
}

fn stmt_eligible_gen(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Yield(e) => expr_eligible(e),
        Stmt::LoopYield(_) | Stmt::BlockReturn(..) => false, // only yield allowed in gen bodies
        other => stmt_eligible(other),
    }
}

fn ann_has_intersection(params: &[crate::ast::Param], return_type: Option<&str>) -> bool {
    params.iter().any(|p| {
        p.type_ann.as_deref().map_or(false, |ann| ann.contains("Intersection["))
    }) || return_type.map_or(false, |ann| ann.contains("Intersection["))
}

pub fn generate_llvm_module(stmts: &[Stmt]) -> Option<(String, Vec<FnExport>)> {
    struct EligibleFn<'a> {
        /// Symbol prefix used in the DLL (e.g. "dot" or "Vec2D__dot").
        symbol:      String,
        /// Original method/function name (used in FnExport).
        orig_name:   &'a str,
        /// Owning class name for methods; None for top-level functions.
        class_name:  Option<String>,
        params:      &'a [Param],
        return_type: Option<&'a str>,
        body:        &'a [Stmt],
        is_gen:      bool,
    }

    let mut eligible: Vec<EligibleFn> = Vec::new();

    for s in stmts {
        match s {
            Stmt::FnDef { name, template_params, params, body, is_abstract, return_type, .. } => {
                if !template_params.is_empty() || *is_abstract || !body_eligible(body) { continue; }
                if ann_has_intersection(params, return_type.as_deref()) { continue; }
                eligible.push(EligibleFn {
                    symbol: name.clone(), orig_name: name, class_name: None,
                    params, return_type: return_type.as_deref(), body, is_gen: false,
                });
            }
            Stmt::GenDef { name, template_params, params, body, .. } => {
                if !template_params.is_empty() || !body_eligible_gen(body) { continue; }
                if ann_has_intersection(params, None) { continue; }
                eligible.push(EligibleFn {
                    symbol: name.clone(), orig_name: name, class_name: None,
                    params, return_type: None, body, is_gen: true,
                });
            }
            Stmt::ClassDef { name: class_name, body: class_body, template_params, .. } => {
                if !template_params.is_empty() { continue; }
                for method_stmt in class_body {
                    match method_stmt {
                        Stmt::FnDef { name: mname, template_params: mtp, params, body, is_abstract, return_type, .. } => {
                            if !mtp.is_empty() || *is_abstract || !body_eligible(body) { continue; }
                            if ann_has_intersection(params, return_type.as_deref()) { continue; }
                            eligible.push(EligibleFn {
                                symbol: method_symbol(class_name, mname),
                                orig_name: mname,
                                class_name: Some(class_name.clone()),
                                params, return_type: return_type.as_deref(), body, is_gen: false,
                            });
                        }
                        Stmt::GenDef { name: mname, template_params: mtp, params, body, .. } => {
                            if !mtp.is_empty() || !body_eligible_gen(body) { continue; }
                            if ann_has_intersection(params, None) { continue; }
                            eligible.push(EligibleFn {
                                symbol: method_symbol(class_name, mname),
                                orig_name: mname,
                                class_name: Some(class_name.clone()),
                                params, return_type: None, body, is_gen: true,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if eligible.is_empty() { return None; }

    // Collect typed field information for all classes in the module.
    // class_fields_ord: declaration-order Vec (used for _fast ABI signatures)
    // class_fields:     HashMap for O(1) field-type lookup
    // all_class_fields: all instance fields (incl. class-typed) for flat SWD leaf collection
    let class_fields_ord: HashMap<String, Vec<(String, Ty)>> = stmts.iter()
        .filter_map(|s| {
            if let Stmt::ClassDef { name, body, .. } = s {
                let fields: Vec<(String, Ty)> = body.iter()
                    .filter_map(|stmt| {
                        if let Stmt::Field { name: fname, type_ann, .. } = stmt {
                            let ty = ann_ty(Some(type_ann.as_str()));
                            if ty != Ty::Handle { Some((fname.clone(), ty)) } else { None }
                        } else { None }
                    })
                    .collect();
                if fields.is_empty() { None } else { Some((name.clone(), fields)) }
            } else { None }
        })
        .collect();
    let class_fields: HashMap<String, HashMap<String, Ty>> = class_fields_ord.iter()
        .map(|(cls, fields)| (cls.clone(), fields.iter().cloned().collect()))
        .collect();
    // All instance fields (including class-typed) for every non-template class.
    let all_class_fields: HashMap<String, Vec<(String, String)>> = stmts.iter()
        .filter_map(|s| {
            if let Stmt::ClassDef { name, body, template_params, .. } = s {
                if !template_params.is_empty() { return None; }
                let fields: Vec<(String, String)> = body.iter()
                    .filter_map(|stmt| {
                        if let Stmt::Field { name: fname, type_ann, kind, .. } = stmt {
                            if matches!(kind, crate::ast::FieldKind::Mut | crate::ast::FieldKind::Let) {
                                Some((fname.clone(), type_ann.clone()))
                            } else { None }
                        } else { None }
                    })
                    .collect();
                Some((name.clone(), fields))
            } else { None }
        })
        .collect();

    let module_fns: HashSet<String> = eligible.iter().map(|f| f.symbol.clone()).collect();
    let fn_sigs: HashMap<String, FnSig> = eligible.iter().map(|f| {
        (f.symbol.clone(), FnSig {
            ret: if f.is_gen { Ty::Handle } else { ann_ty(f.return_type) },
            param_mutabilities: f.params.iter().map(|p| p.mutable).collect(),
        })
    }).collect();

    // fast_fns: symbols that will get a _fast variant (have at least one pure class param).
    // `self` param has type_ann = None; resolve its class from f.class_name.
    let fast_fns: HashSet<String> = eligible.iter()
        .filter(|f| !f.is_gen && f.params.iter().any(|p| {
            let cls = p.type_ann.as_deref()
                .or_else(|| if p.name == "self" { f.class_name.as_deref() } else { None });
            cls.map(|ann| class_fields_ord.contains_key(ann)).unwrap_or(false)
            && !body_writes_param(f.body, &p.name)
        }))
        .map(|f| f.symbol.clone())
        .collect();

    let mut ctx = GenCtx::new(&module_fns, &fn_sigs, &class_fields, &class_fields_ord, &all_class_fields, &fast_fns);

    for f in &eligible {
        // Set current_class so field reads on 'self' are type-specialised.
        ctx.current_class = f.class_name.clone();
        if f.is_gen {
            ctx.emit_gen_fn(&f.symbol, f.params, f.body);
        } else {
            ctx.emit_fn(&f.symbol, f.params, f.return_type, f.body);
        }
        ctx.current_class = None;
    }

    // ── 統一 typed ABI 変種（{name}_typed）の生成 ──────────────────────────────
    // 候補: トップレベル関数で、全パラメータが `let` かつ int/float 注釈、
    //       戻り値も int/float のもの。
    // 本体がコールバックを要した場合は生成を破棄し、その関数を typed 集合から外して
    // 呼び出し元も含め再コンパイルする（fixpoint）。
    let typed_candidates: Vec<&EligibleFn> = eligible.iter()
        .filter(|f| {
            f.class_name.is_none()
                && !f.is_gen
                && matches!(ann_ty(f.return_type), Ty::Int | Ty::Float)
                && f.params.iter().all(|p| {
                    !p.mutable && matches!(ann_ty(p.type_ann.as_deref()), Ty::Int | Ty::Float)
                })
        })
        .collect();

    ctx.typed_sigs = typed_candidates.iter()
        .map(|f| {
            (
                f.symbol.clone(),
                (
                    f.params.iter().map(|p| ann_ty(p.type_ann.as_deref())).collect::<Vec<Ty>>(),
                    ann_ty(f.return_type),
                ),
            )
        })
        .collect();
    ctx.typed_ok = typed_candidates.iter().map(|f| f.symbol.clone()).collect();

    let mut typed_defs: Vec<String> = Vec::new();
    loop {
        typed_defs.clear();
        let mut evicted: Vec<String> = Vec::new();
        for f in &typed_candidates {
            if !ctx.typed_ok.contains(&f.symbol) {
                continue;
            }
            match ctx.emit_fn_typed(&f.symbol, f.params, f.return_type, f.body) {
                Some(def) => typed_defs.push(def),
                None => evicted.push(f.symbol.clone()),
            }
        }
        if evicted.is_empty() {
            break;
        }
        // 破棄されたシンボルを呼んでいた typed 関数も無効になるため再コンパイル
        for s in evicted {
            ctx.typed_ok.remove(&s);
        }
    }
    let typed_count = typed_defs.len();
    for def in &typed_defs {
        ctx.fn_defs.push_str(def);
    }
    if typed_count > 0 {
        eprintln!("NativeLib: {typed_count} typed entry point(s) (zero-TLS ABI)");
    }

    let header = if cfg!(target_os = "windows") {
        module_header().replace("define void @ar_init(", "define dllexport void @ar_init(")
    } else {
        module_header().to_string()
    };

    let mut module = String::new();
    module.push_str(&header);
    module.push_str(&ctx.str_globals);
    module.push_str(&ctx.fn_defs);

    let exports: Vec<FnExport> = eligible.iter()
        .map(|f| FnExport {
            name:       f.orig_name.to_string(),
            n_params:   f.params.len(),
            class_name: f.class_name.clone(),
        })
        .collect();

    Some((module, exports))
}

mod context;
mod expr;
mod stmt;
mod function;
