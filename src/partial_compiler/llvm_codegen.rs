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
use crate::ast::{BinOp, CallArg, Expr, MatchPattern, Param, Stmt, UnaryOp};

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
    let raw_fields = match all_class_fields.get(class_name) {
        Some(f) if !f.is_empty() => f.clone(),
        _ => return vec![],
    };
    let mut sorted = raw_fields;
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut leaves = Vec::new();
    let mut byte_offset = byte_base;

    for (fname, ftype) in sorted {
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
fn ann_ty(s: Option<&str>) -> Ty {
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
}

impl<'a> GenCtx<'a> {
    fn new(
        module_fns:        &'a HashSet<String>,
        fn_sigs:           &'a HashMap<String, FnSig>,
        class_fields:      &'a HashMap<String, HashMap<String, Ty>>,
        class_fields_ord:  &'a HashMap<String, Vec<(String, Ty)>>,
        all_class_fields:  &'a HashMap<String, Vec<(String, String)>>,
        fast_fns:          &'a HashSet<String>,
    ) -> Self {
        Self {
            str_globals: String::new(),
            fn_defs:     String::new(),
            alloca_buf:  String::new(),
            code_buf:    String::new(),
            reg: 0, blk: 0, terminated: false,
            locals:          HashMap::new(),
            loop_stack:      Vec::new(),
            module_fns,
            fn_sigs,
            str_consts:      HashMap::new(),
            str_ctr:         0,
            block_stack:     Vec::new(),
            class_fields,
            class_fields_ord,
            all_class_fields,
            current_class:   None,
            param_classes:   HashMap::new(),
            current_fn_ret:  Ty::Handle,
            fast_fns,
            preread_fields:        HashMap::new(),
            flat_list_params:      HashMap::new(),
            fn_param_trampolines:  HashMap::new(),
        }
    }

    /// Look up the native type of a field access on a known class instance.
    /// Returns Some(Ty) only if the field is typed int/float in the class definition.
    fn field_ty(&self, object: &Expr, attr: &str) -> Option<Ty> {
        let class_name = match object {
            Expr::Ident(n) if n == "self" => self.current_class.as_deref(),
            Expr::Ident(n) => self.param_classes.get(n.as_str()).map(|s| s.as_str()),
            _ => None,
        }?;
        self.class_fields.get(class_name)?.get(attr).copied()
    }

    fn fresh_reg(&mut self) -> String { let r = self.reg; self.reg += 1; format!("%_r{r}") }
    fn fresh_blk(&mut self) -> String { let b = self.blk; self.blk += 1; format!("_bb{b}") }

    /// Emit a line into the alloca buffer (entry block).
    fn ea(&mut self, line: &str) { self.alloca_buf.push_str("  "); self.alloca_buf.push_str(line); self.alloca_buf.push('\n'); }

    /// Emit a line into the code buffer.
    fn ec(&mut self, line: &str) {
        if !self.terminated {
            self.code_buf.push_str("  ");
            self.code_buf.push_str(line);
            self.code_buf.push('\n');
        }
    }

    /// Start a new basic block label in the code buffer.
    fn start_block(&mut self, label: &str) {
        self.code_buf.push('\n');
        self.code_buf.push_str(label);
        self.code_buf.push_str(":\n");
        self.terminated = false;
    }

    /// Emit an unconditional branch if the current block is not yet terminated.
    fn br(&mut self, target: &str) {
        if !self.terminated {
            self.ec(&format!("br label %{target}"));
            self.terminated = true;
        }
    }

    /// Emit a conditional branch.
    fn br_cond(&mut self, cond: &str, then_lbl: &str, else_lbl: &str) {
        if !self.terminated {
            self.ec(&format!("br i1 {cond}, label %{then_lbl}, label %{else_lbl}"));
            self.terminated = true;
        }
    }

    /// Emit a ret instruction.
    fn ret_handle(&mut self, val: &str) {
        if !self.terminated {
            self.ec(&format!("ret i64 {val}"));
            self.terminated = true;
        }
    }

    // ── Alloca helpers ────────────────────────────────────────────────────────

    fn alloca_var(&mut self, name: &str, ty: Ty) -> String {
        let reg = format!("%_al_{name}");
        let t = llvm_ty(ty);
        self.ea(&format!("{reg} = alloca {t}, align 8"));
        self.locals.insert(name.to_string(), (reg.clone(), ty));
        reg
    }

    fn store_val(&mut self, ty: Ty, val: &str, ptr: &str) {
        let t = llvm_ty(ty);
        self.ec(&format!("store {t} {val}, ptr {ptr}"));
    }

    fn load_var(&mut self, name: &str) -> (String, Ty) {
        let (ptr, ty) = self.locals.get(name).cloned()
            .unwrap_or_else(|| ("%_UNDEF".to_string(), Ty::Handle));
        let t = llvm_ty(ty);
        let r = self.fresh_reg();
        self.ec(&format!("{r} = load {t}, ptr {ptr}"));
        (r, ty)
    }

    // ── String constant pool ──────────────────────────────────────────────────

    fn str_const(&mut self, bytes: &[u8]) -> String {
        if let Some(name) = self.str_consts.get(bytes) {
            return format!("ptr @{name}");
        }
        let name = format!("_s{}", self.str_ctr);
        self.str_ctr += 1;
        self.str_consts.insert(bytes.to_vec(), name.clone());
        let esc = escape_for_llvm(bytes);
        let len = bytes.len() + 1;
        self.str_globals.push_str(&format!(
            "@{name} = private unnamed_addr constant [{len} x i8] c\"{esc}\\00\", align 1\n"
        ));
        format!("ptr @{name}")
    }

    // ── Callback dispatch ─────────────────────────────────────────────────────

    /// Load @CB, GEP to field, load fn ptr, call it. Returns result register (or "void").
    fn call_cb(&mut self, field: usize, args: &[String]) -> String {
        let (ret_ty, param_tys) = cb_sig(field);
        let cb  = self.fresh_reg();
        let fp  = self.fresh_reg();
        let fn_ = self.fresh_reg();
        self.ec(&format!("{cb} = load ptr, ptr @CB"));
        self.ec(&format!("{fp} = getelementptr inbounds %ArCallbacks, ptr {cb}, i32 0, i32 {field}"));
        self.ec(&format!("{fn_} = load ptr, ptr {fp}"));
        let args_str = args.join(", ");
        let fn_ty = if param_tys.is_empty() {
            format!("{ret_ty} ()")
        } else {
            format!("{ret_ty} ({param_tys})")
        };
        if ret_ty == "void" {
            self.ec(&format!("call {fn_ty} {fn_}({args_str})"));
            "void".to_string()
        } else {
            let r = self.fresh_reg();
            self.ec(&format!("{r} = call {fn_ty} {fn_}({args_str})"));
            r
        }
    }

    // ── Type coercions ────────────────────────────────────────────────────────

    /// Coerce an (expr_reg, Ty) to an i64 handle.
    fn to_handle(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Handle => val.to_string(),
            Ty::Int    => self.call_cb(CB_MAKE_INT,   &[format!("i64 {val}")]),
            Ty::Float  => self.call_cb(CB_MAKE_FLOAT, &[format!("double {val}")]),
            Ty::Bool   => {
                let r = self.fresh_reg();
                self.ec(&format!("{r} = select i1 {val}, i64 1, i64 2"));
                r
            }
        }
    }

    /// Coerce an (expr_reg, Ty) to a raw i64.
    fn to_i64(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Int    => val.to_string(),
            Ty::Float  => { let r = self.fresh_reg(); self.ec(&format!("{r} = fptosi double {val} to i64")); r }
            Ty::Bool   => { let r = self.fresh_reg(); self.ec(&format!("{r} = zext i1 {val} to i64")); r }
            Ty::Handle => self.call_cb(CB_TO_INT, &[format!("i64 {val}")]),
        }
    }

    /// Coerce to a double.
    fn to_f64(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Float  => val.to_string(),
            Ty::Int    => { let r = self.fresh_reg(); self.ec(&format!("{r} = sitofp i64 {val} to double")); r }
            Ty::Bool   => { let r = self.fresh_reg(); self.ec(&format!("{r} = uitofp i1 {val} to double")); r }
            Ty::Handle => self.call_cb(CB_TO_FLOAT, &[format!("i64 {val}")]),
        }
    }

    /// Coerce to i1 for use as a branch condition.
    fn to_cond(&mut self, val: &str, ty: Ty) -> String {
        match ty {
            Ty::Bool   => val.to_string(),
            Ty::Int    => { let r = self.fresh_reg(); self.ec(&format!("{r} = icmp ne i64 {val}, 0")); r }
            Ty::Float  => { let r = self.fresh_reg(); self.ec(&format!("{r} = fcmp une double {val}, 0.0")); r }
            Ty::Handle => {
                let tr = self.call_cb(CB_IS_TRUTHY, &[format!("i64 {val}")]);
                let r  = self.fresh_reg();
                self.ec(&format!("{r} = icmp ne i32 {tr}, 0"));
                r
            }
        }
    }

    // ── Expression generation ─────────────────────────────────────────────────

    pub fn gen_expr(&mut self, expr: &Expr) -> (String, Ty) {
        match expr {
            Expr::Int(n)    => (format!("{n}"), Ty::Int),
            Expr::Float(f)  => (fmt_float(*f), Ty::Float),
            Expr::Bool(b)   => (if *b { "1" } else { "2" }.to_string(), Ty::Handle), // TL_TRUE / TL_FALSE
            Expr::None      => ("0".to_string(), Ty::Handle),
            Expr::Undefined => ("0".to_string(), Ty::Handle),
            Expr::Str(s) => {
                let bytes = s.as_bytes();
                let ptr   = self.str_const(bytes);
                let len   = bytes.len() as i32;
                let r = self.call_cb(CB_MAKE_STR, &[ptr, format!("i32 {len}")]);
                (r, Ty::Handle)
            }

            Expr::Ident(name) => {
                if self.locals.contains_key(name.as_str()) {
                    self.load_var(name)
                } else if self.module_fns.contains(name.as_str()) {
                    // Intra-module fn reference: fetch as global
                    let bytes  = name.as_bytes();
                    let ptr    = self.str_const(bytes);
                    let len    = bytes.len() as i32;
                    let r      = self.call_cb(CB_GET_GLOBAL, &[ptr, format!("i32 {len}")]);
                    (r, Ty::Handle)
                } else {
                    let bytes = name.as_bytes();
                    let ptr   = self.str_const(bytes);
                    let len   = bytes.len() as i32;
                    let r     = self.call_cb(CB_GET_GLOBAL, &[ptr, format!("i32 {len}")]);
                    (r, Ty::Handle)
                }
            }

            Expr::BinOp { op, left, right, .. } => self.gen_binop(op, left, right),

            Expr::UnaryOp { op, operand } => {
                let op_code = match op { UnaryOp::Neg => 0i32, UnaryOp::Not => 1, UnaryOp::BitNot => 2 };
                let (v, vt) = self.gen_expr(operand);
                let h = self.to_handle(&v, vt);
                let r = self.call_cb(CB_UNOP, &[format!("i32 {op_code}"), format!("i64 {h}")]);
                (r, Ty::Handle)
            }

            Expr::Call { func, args, .. } => {
                // Cell built-ins: __make_cell / __get_cell / __set_cell
                if let Expr::Ident(n) = func.as_ref() {
                    if n == "__make_cell" || n == "__get_cell" || n == "__set_cell" {
                        let arg_vals: Vec<(String, Ty)> = args.iter()
                            .map(|a| self.gen_expr(a.expr()))
                            .collect();
                        let handles: Vec<String> = arg_vals.iter()
                            .map(|(v, vt)| { let h = self.to_handle(v, *vt); format!("i64 {h}") })
                            .collect();
                        return match n.as_str() {
                            "__make_cell" => {
                                let init = handles.first().cloned().unwrap_or("i64 0".to_string());
                                let r = self.call_cb(CB_MAKE_CELL, &[init]);
                                (r, Ty::Handle)
                            }
                            "__get_cell" => {
                                let cell = handles.first().cloned().unwrap_or("i64 0".to_string());
                                let r = self.call_cb(CB_GET_CELL, &[cell]);
                                (r, Ty::Handle)
                            }
                            _ => { // __set_cell
                                if handles.len() >= 2 {
                                    self.call_cb(CB_SET_CELL, &[handles[0].clone(), handles[1].clone()]);
                                }
                                let r = handles.first().cloned().unwrap_or("0".to_string());
                                (r.trim_start_matches("i64 ").to_string(), Ty::Handle)
                            }
                        };
                    }
                }
                // ── Typed intra-module direct function calls ──────────────────
                if let Expr::Ident(name) = func.as_ref() {
                    if self.module_fns.contains(name.as_str())
                        && !self.locals.contains_key(name.as_str())
                    {
                        let ret_ty = self.fn_sigs.get(name.as_str())
                            .map(|s| s.ret).unwrap_or(Ty::Handle);

                        if ret_ty == Ty::Float {
                            // Fast path: _impl returns double — no arena save/compact/boxing.
                            let mutabilities = self.fn_sigs.get(name.as_str())
                                .map(|s| s.param_mutabilities.clone());
                            let arg_exprs: Vec<(String, Ty)> = args.iter()
                                .map(|a| self.gen_expr(a.expr())).collect();
                            let call_args: Vec<String> = arg_exprs.iter().enumerate()
                                .map(|(i, (v, vt))| {
                                    let h = self.to_handle(v, *vt);
                                    let is_mut = mutabilities.as_ref()
                                        .and_then(|m| m.get(i)).copied().unwrap_or(true);
                                    if is_mut { format!("i64 {h}") }
                                    else {
                                        let dc = self.call_cb(CB_DEEP_COPY, &[format!("i64 {h}")]);
                                        format!("i64 {dc}")
                                    }
                                })
                                .collect();
                            let param_str = call_args.join(", ");
                            let r = self.fresh_reg();
                            self.ec(&format!("{r} = call double @{name}_impl({param_str})"));
                            return (r, Ty::Float);
                        }

                        // Non-float typed return: use handle ABI + CB_TO_INT unwrap.
                        let h = self.gen_call(func, args);
                        return match ret_ty {
                            Ty::Int => {
                                let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]);
                                (r, Ty::Int)
                            }
                            _ => (h, Ty::Handle),
                        };
                    }
                }
                // ── Typed intra-module method calls on known class instances ──
                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                    let class_name = match object.as_ref() {
                        Expr::Ident(n) if n == "self" => self.current_class.clone(),
                        Expr::Ident(n) => self.param_classes.get(n.as_str()).cloned(),
                        _ => None,
                    };
                    if let Some(cls) = &class_name {
                        let sym = method_symbol(cls, attr);
                        if self.module_fns.contains(&sym) && !self.locals.contains_key(&sym) {
                            let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);

                            if ret_ty == Ty::Float {
                                // Preferred: _fast variant — passes pre-read field values as
                                // scalars; zero callbacks and LLVM-inlineable pure arithmetic.
                                if self.fast_fns.contains(&sym) {
                                    if let Some(fast_args) = self.build_fast_call_args(object, args) {
                                        let r = self.fresh_reg();
                                        self.ec(&format!("{r} = call double @{sym}_fast({fast_args})"));
                                        return (r, Ty::Float);
                                    }
                                }
                                // Fallback: _impl returns double, no arena overhead.
                                let arg_exprs: Vec<(String, Ty)> = args.iter()
                                    .map(|a| self.gen_expr(a.expr())).collect();
                                let (ov, ot) = self.gen_expr(object);
                                let oh = self.to_handle(&ov, ot);
                                let explicit: Vec<String> = arg_exprs.iter()
                                    .map(|(v, vt)| format!("i64 {}", self.to_handle(v, *vt)))
                                    .collect();
                                let all_params = std::iter::once(format!("i64 {oh}"))
                                    .chain(explicit).collect::<Vec<_>>().join(", ");
                                let r = self.fresh_reg();
                                self.ec(&format!("{r} = call double @{sym}_impl({all_params})"));
                                return (r, Ty::Float);
                            }

                            // Non-float typed return: handle ABI + CB_TO_INT.
                            let h = self.gen_call(func, args);
                            return match ret_ty {
                                Ty::Int => { let r = self.call_cb(CB_TO_INT, &[format!("i64 {h}")]); (r, Ty::Int) }
                                _       => (h, Ty::Handle),
                            };
                        }
                    }
                }
                (self.gen_call(func, args), Ty::Handle)
            }

            Expr::Attr { object, attr, .. } => {
                // Fast path: check preread_fields using the full dotted path.
                // Covers class params ("self.x", "p.x"), flat list loop vars ("item.v"),
                // and nested flat fields ("item.start.x").
                if let Some(path) = preread_path(expr) {
                    if let Some((al_ptr, ty)) = self.preread_fields.get(&path).cloned() {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = load {}, ptr {al_ptr}", llvm_ty(ty)));
                        return (r, ty);
                    }
                }
                // Also check single-level preread for "self.attr" and param class attrs.
                let preread_key = match object.as_ref() {
                    Expr::Ident(n) if n == "self" && self.current_class.is_some() => {
                        Some(format!("self.{attr}"))
                    }
                    Expr::Ident(n) if self.param_classes.contains_key(n.as_str()) => {
                        Some(format!("{n}.{attr}"))
                    }
                    _ => None,
                };
                if let Some(key) = preread_key {
                    if let Some((al_ptr, ty)) = self.preread_fields.get(&key).cloned() {
                        let r = self.fresh_reg();
                        self.ec(&format!("{r} = load {}, ptr {al_ptr}", llvm_ty(ty)));
                        return (r, ty);
                    }
                }

                // Callback path: typed single-callback read (CB_GET_FLOAT_FIELD /
                // CB_GET_INT_FIELD) for known typed fields, plain CB_GET_ATTR otherwise.
                let known_ty = self.field_ty(object, attr);
                let (obj, ot) = self.gen_expr(object);
                let h   = self.to_handle(&obj, ot);
                let ptr = self.str_const(attr.as_bytes());
                let len = attr.len() as i32;
                match known_ty {
                    Some(Ty::Float) => {
                        let r = self.call_cb(CB_GET_FLOAT_FIELD, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (r, Ty::Float)
                    }
                    Some(Ty::Int) => {
                        let r = self.call_cb(CB_GET_INT_FIELD, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (r, Ty::Int)
                    }
                    _ => {
                        let raw = self.call_cb(CB_GET_ATTR, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                        (raw, Ty::Handle)
                    }
                }
            }

            Expr::TraitAccess { object, trait_name, attr } => {
                let key  = format!("{trait_name}::{attr}");
                let (obj, ot) = self.gen_expr(object);
                let h    = self.to_handle(&obj, ot);
                let ptr  = self.str_const(key.as_bytes());
                let len  = key.len() as i32;
                let r    = self.call_cb(CB_GET_ATTR, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                (r, Ty::Handle)
            }

            Expr::Subscript { object, index } => {
                let (obj, ot) = self.gen_expr(object);
                let (idx, it) = self.gen_expr(index);
                let h1 = self.to_handle(&obj, ot);
                let h2 = self.to_handle(&idx, it);
                let r  = self.call_cb(CB_SUBSCRIPT, &[format!("i64 {h1}"), format!("i64 {h2}")]);
                (r, Ty::Handle)
            }

            Expr::IsType { expr, negated, type_name, .. } => {
                let (v, vt) = self.gen_expr(expr);
                let h   = self.to_handle(&v, vt);
                let ptr = self.str_const(type_name.as_bytes());
                let len = type_name.len() as i32;
                let r   = self.call_cb(CB_IS_TYPE, &[format!("i64 {h}"), ptr, format!("i32 {len}")]);
                if *negated {
                    let cmp = self.fresh_reg();
                    let nr  = self.fresh_reg();
                    self.ec(&format!("{cmp} = icmp eq i64 {r}, 1")); // 1 = TL_TRUE
                    self.ec(&format!("{nr}  = select i1 {cmp}, i64 2, i64 1")); // swap TRUE↔FALSE
                    (nr, Ty::Handle)
                } else {
                    (r, Ty::Handle)
                }
            }

            Expr::List(items) => {
                if items.is_empty() {
                    let r = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n    = items.len();
                let arr  = format!("%_la{}", self.reg); self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(item);
                    let h  = self.to_handle(&v, vt);
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                let r = self.call_cb(CB_MAKE_LIST, &[format!("ptr {arr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::Tuple(items) => {
                if items.is_empty() {
                    let r = self.call_cb(CB_MAKE_TUPLE, &["ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n   = items.len();
                let arr = format!("%_ta{}", self.reg); self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, item) in items.iter().enumerate() {
                    let (v, vt) = self.gen_expr(item);
                    let h  = self.to_handle(&v, vt);
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                let r = self.call_cb(CB_MAKE_TUPLE, &[format!("ptr {arr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    let r = self.call_cb(CB_MAKE_DICT, &["ptr null".to_string(), "ptr null".to_string(), "i32 0".to_string()]);
                    return (r, Ty::Handle);
                }
                let n    = pairs.len();
                let karr = format!("%_dka{}", self.reg); self.reg += 1;
                let varr = format!("%_dva{}", self.reg); self.reg += 1;
                self.ea(&format!("{karr} = alloca [{n} x i64], align 8"));
                self.ea(&format!("{varr} = alloca [{n} x i64], align 8"));
                for (i, (k, v)) in pairs.iter().enumerate() {
                    let (kv, kt) = self.gen_expr(k);
                    let (vv, vt) = self.gen_expr(v);
                    let kh = self.to_handle(&kv, kt);
                    let vh = self.to_handle(&vv, vt);
                    let kp = self.fresh_reg();
                    let vp = self.fresh_reg();
                    self.ec(&format!("{kp} = getelementptr inbounds [{n} x i64], ptr {karr}, i32 0, i32 {i}"));
                    self.ec(&format!("{vp} = getelementptr inbounds [{n} x i64], ptr {varr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {kh}, ptr {kp}"));
                    self.ec(&format!("store i64 {vh}, ptr {vp}"));
                }
                let r = self.call_cb(CB_MAKE_DICT, &[format!("ptr {karr}"), format!("ptr {varr}"), format!("i32 {n}")]);
                (r, Ty::Handle)
            }

            Expr::TemplateInstantiate { base, .. } => self.gen_expr(base),

            // ── Control-flow expressions ──────────────────────────────────────

            Expr::Block { stmts, .. } => {
                let result_al = format!("%_blk_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let exit_lbl = self.fresh_blk();
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: exit_lbl.clone(), list_al: None });
                self.gen_stmts(stmts);
                self.block_stack.pop();
                self.br(&exit_lbl);
                self.start_block(&exit_lbl);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            Expr::IfExpr { branches, else_body, .. } => {
                let result_al = format!("%_if_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let merge = self.fresh_blk();
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None });
                for (cond, body) in branches {
                    let then_blk = self.fresh_blk();
                    let next_blk = self.fresh_blk();
                    let (cv, ct) = self.gen_expr(cond);
                    let cc = self.to_cond(&cv, ct);
                    self.br_cond(&cc, &then_blk, &next_blk);
                    self.start_block(&then_blk);
                    self.gen_stmts(body);
                    self.br(&merge);
                    self.start_block(&next_blk);
                }
                if let Some(else_stmts) = else_body {
                    self.gen_stmts(else_stmts);
                }
                self.br(&merge);
                self.block_stack.pop();
                self.start_block(&merge);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            Expr::ForExpr { target, iter, body, .. } => {
                // Accumulator list (for loop_yield) or result slot (for block_return)
                let result_al = format!("%_for_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let has_yield = body_has_loop_yield(body);
                let list_al = if has_yield {
                    let la = format!("%_for_list{}", self.blk);
                    self.blk += 1;
                    self.alloca_buf.push_str(&format!("  {la} = alloca i64, align 8\n"));
                    let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    self.ec(&format!("store i64 {empty}, ptr {la}"));
                    Some(la)
                } else { None };

                let exit_blk = self.fresh_blk();
                let loop_blk = self.fresh_blk();
                let (iv, it) = self.gen_expr(iter);
                let ih = self.to_handle(&iv, it);
                let iter_h = self.call_cb(CB_ITER_FROM, &[format!("i64 {ih}")]);
                let iter_al = format!("%_iter{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {iter_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {iter_h}, ptr {iter_al}"));
                let tgt_al = format!("%_al_{target}");
                self.alloca_buf.push_str(&format!("  {tgt_al} = alloca i64, align 8\n"));
                self.locals.insert(target.clone(), (tgt_al.clone(), Ty::Handle));
                self.br(&loop_blk);
                self.start_block(&loop_blk);
                let ir = self.fresh_reg();
                self.ec(&format!("{ir} = load i64, ptr {iter_al}"));
                let next = self.call_cb(CB_ITER_NEXT, &[format!("i64 {ir}")]);
                self.ec(&format!("store i64 {next}, ptr {tgt_al}"));
                let done = self.fresh_reg();
                self.ec(&format!("{done} = icmp eq i64 {next}, -1"));
                let body_blk = self.fresh_blk();
                self.br_cond(&done, &exit_blk, &body_blk);
                self.start_block(&body_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone()
                });
                self.loop_stack.push((loop_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.block_stack.pop();
                self.br(&loop_blk);
                self.start_block(&exit_blk);
                // Return list if loop_yield was used, else result slot
                let r = self.fresh_reg();
                if let Some(la) = &list_al {
                    self.ec(&format!("{r} = load i64, ptr {la}"));
                } else {
                    self.ec(&format!("{r} = load i64, ptr {result_al}"));
                }
                (r, Ty::Handle)
            }

            Expr::WhileExpr { cond, body, .. } => {
                let result_al = format!("%_whl_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let has_yield = body_has_loop_yield(body);
                let list_al = if has_yield {
                    let la = format!("%_whl_list{}", self.blk);
                    self.blk += 1;
                    self.alloca_buf.push_str(&format!("  {la} = alloca i64, align 8\n"));
                    let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
                    self.ec(&format!("store i64 {empty}, ptr {la}"));
                    Some(la)
                } else { None };
                let cond_blk = self.fresh_blk();
                let body_blk = self.fresh_blk();
                let exit_blk = self.fresh_blk();
                self.br(&cond_blk);
                self.start_block(&cond_blk);
                let (cv, ct) = self.gen_expr(cond);
                let cc = self.to_cond(&cv, ct);
                self.br_cond(&cc, &body_blk, &exit_blk);
                self.start_block(&body_blk);
                self.block_stack.push(BlockCtx {
                    result_al: result_al.clone(), exit_label: exit_blk.clone(), list_al: list_al.clone()
                });
                self.loop_stack.push((cond_blk.clone(), exit_blk.clone()));
                self.gen_stmts(body);
                self.loop_stack.pop();
                self.block_stack.pop();
                self.br(&cond_blk);
                self.start_block(&exit_blk);
                let r = self.fresh_reg();
                if let Some(la) = &list_al {
                    self.ec(&format!("{r} = load i64, ptr {la}"));
                } else {
                    self.ec(&format!("{r} = load i64, ptr {result_al}"));
                }
                (r, Ty::Handle)
            }

            Expr::MatchExpr { subject, arms, .. } => {
                let result_al = format!("%_mtch_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {result_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 0, ptr {result_al}"));
                let merge = self.fresh_blk();
                let (sv, st) = self.gen_expr(subject);
                let subj_h = self.to_handle(&sv, st);
                let subj_al = format!("%_msubj{}", self.reg);
                self.reg += 1;
                self.alloca_buf.push_str(&format!("  {subj_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {subj_h}, ptr {subj_al}"));
                self.block_stack.push(BlockCtx { result_al: result_al.clone(), exit_label: merge.clone(), list_al: None });
                for (i, arm) in arms.iter().enumerate() {
                    let is_last = i == arms.len() - 1;
                    let body_blk = self.fresh_blk();
                    let next_blk = if is_last { merge.clone() } else { self.fresh_blk() };
                    let subj_r = self.fresh_reg();
                    self.ec(&format!("{subj_r} = load i64, ptr {subj_al}"));
                    match &arm.pattern {
                        MatchPattern::Case(Expr::Ident(w)) if w == "_" => { self.br(&body_blk); }
                        MatchPattern::Case(pat) => {
                            let (pv, pt) = self.gen_expr(pat);
                            let ph = self.to_handle(&pv, pt);
                            let eq = self.call_cb(CB_BINOP, &[format!("i32 7"), format!("i64 {subj_r}"), format!("i64 {ph}")]);
                            let cnd = self.to_cond(&eq, Ty::Handle);
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                        MatchPattern::IsType(tn) => {
                            let ptr = self.str_const(tn.as_bytes());
                            let len = tn.len() as i32;
                            let r = self.call_cb(CB_IS_TYPE, &[format!("i64 {subj_r}"), ptr, format!("i32 {len}")]);
                            let cnd = self.fresh_reg();
                            self.ec(&format!("{cnd} = icmp eq i64 {r}, 1"));
                            self.br_cond(&cnd, &body_blk, &next_blk);
                        }
                    }
                    self.start_block(&body_blk);
                    self.gen_stmts(&arm.body);
                    self.br(&merge);
                    if !is_last { self.start_block(&next_blk); }
                }
                self.block_stack.pop();
                self.start_block(&merge);
                let r = self.fresh_reg();
                self.ec(&format!("{r} = load i64, ptr {result_al}"));
                (r, Ty::Handle)
            }

            _ => ("0".to_string(), Ty::Handle), // unsupported expr → None handle
        }
    }

    fn gen_binop(&mut self, op: &BinOp, left: &Expr, right: &Expr) -> (String, Ty) {
        // Short-circuit and/or
        match op {
            BinOp::And => {
                let (l, lt) = self.gen_expr(left);
                let lh   = self.to_handle(&l, lt);
                let lc   = self.to_cond(&lh, Ty::Handle);
                let rblk = self.fresh_blk();
                let mblk = self.fresh_blk();
                let res_al = format!("%_and_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {res_al} = alloca i64, align 8\n"));
                // store lh into res before branch
                self.ec(&format!("store i64 {lh}, ptr {res_al}"));
                self.br_cond(&lc, &rblk, &mblk);
                self.start_block(&rblk);
                let (r2, r2t) = self.gen_expr(right);
                let rh = self.to_handle(&r2, r2t);
                self.ec(&format!("store i64 {rh}, ptr {res_al}"));
                self.br(&mblk);
                self.start_block(&mblk);
                let result = self.fresh_reg();
                self.ec(&format!("{result} = load i64, ptr {res_al}"));
                return (result, Ty::Handle);
            }
            BinOp::Or => {
                let (l, lt) = self.gen_expr(left);
                let lh   = self.to_handle(&l, lt);
                let lc   = self.to_cond(&lh, Ty::Handle);
                let rblk = self.fresh_blk();
                let mblk = self.fresh_blk();
                let res_al = format!("%_or_res{}", self.blk);
                self.blk += 1;
                self.alloca_buf.push_str(&format!("  {res_al} = alloca i64, align 8\n"));
                self.ec(&format!("store i64 {lh}, ptr {res_al}"));
                self.br_cond(&lc, &mblk, &rblk); // if truthy, skip right
                self.start_block(&rblk);
                let (r2, r2t) = self.gen_expr(right);
                let rh = self.to_handle(&r2, r2t);
                self.ec(&format!("store i64 {rh}, ptr {res_al}"));
                self.br(&mblk);
                self.start_block(&mblk);
                let result = self.fresh_reg();
                self.ec(&format!("{result} = load i64, ptr {res_al}"));
                return (result, Ty::Handle);
            }
            _ => {}
        }

        let (l, lt) = self.gen_expr(left);
        let (r, rt) = self.gen_expr(right);
        self.specialize_binop(op, &l, lt, &r, rt)
    }

    fn specialize_binop(&mut self, op: &BinOp, l: &str, lt: Ty, r: &str, rt: Ty) -> (String, Ty) {
        // If either side is a handle, fall back to cb_binop.
        if lt == Ty::Handle || rt == Ty::Handle {
            let op_code = binop_code(op);
            let lh = self.to_handle(l, lt);
            let rh = self.to_handle(r, rt);
            let res = self.call_cb(CB_BINOP, &[format!("i32 {op_code}"), format!("i64 {lh}"), format!("i64 {rh}")]);
            return (res, Ty::Handle);
        }

        // Promote mixed Int/Float → Float
        let (l_s, r_s, nt): (String, String, Ty) = match (lt, rt) {
            (Ty::Int, Ty::Float) => {
                let lf = self.to_f64(l, lt); (lf, r.to_string(), Ty::Float)
            }
            (Ty::Float, Ty::Int) => {
                let rf = self.to_f64(r, rt); (l.to_string(), rf, Ty::Float)
            }
            _ => (l.to_string(), r.to_string(), lt),
        };
        let (l, r) = (l_s.as_str(), r_s.as_str());

        match (op, nt) {
            (BinOp::Add, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = add i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Sub, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = sub i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Mul, Ty::Int)  => { let res = self.fresh_reg(); self.ec(&format!("{res} = mul i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::Div, Ty::Int)  => {
                let lf = self.to_f64(l, Ty::Int);
                let rf = self.to_f64(r, Ty::Int);
                let res = self.fresh_reg();
                self.ec(&format!("{res} = fdiv double {lf}, {rf}"));
                (res, Ty::Float)
            }
            (BinOp::FloorDiv, Ty::Int) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call i64 @_tl_idiv(i64 {l}, i64 {r})"));
                (res, Ty::Int)
            }
            (BinOp::Mod, Ty::Int) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call i64 @_tl_imod(i64 {l}, i64 {r})"));
                (res, Ty::Int)
            }
            (BinOp::BitAnd, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = and i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::BitOr,  Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = or i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::BitXor, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = xor i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::LShift, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = shl i64 {l}, {r}")); (res, Ty::Int) }
            (BinOp::RShift, Ty::Int) => { let res = self.fresh_reg(); self.ec(&format!("{res} = ashr i64 {l}, {r}")); (res, Ty::Int) }

            (BinOp::Add, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fadd double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Sub, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fsub double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Mul, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fmul double {l}, {r}")); (res, Ty::Float) }
            (BinOp::Div, Ty::Float) => { let res = self.fresh_reg(); self.ec(&format!("{res} = fdiv double {l}, {r}")); (res, Ty::Float) }
            (BinOp::FloorDiv, Ty::Float) => {
                let d   = self.fresh_reg();
                let res = self.fresh_reg();
                self.ec(&format!("{d} = fdiv double {l}, {r}"));
                self.ec(&format!("{res} = call double @llvm.floor.f64(double {d})"));
                (res, Ty::Float)
            }
            (BinOp::Mod, Ty::Float) => {
                // Python float mod: a - floor(a/b)*b
                let d   = self.fresh_reg();
                let fl  = self.fresh_reg();
                let mul = self.fresh_reg();
                let res = self.fresh_reg();
                self.ec(&format!("{d}   = fdiv double {l}, {r}"));
                self.ec(&format!("{fl}  = call double @llvm.floor.f64(double {d})"));
                self.ec(&format!("{mul} = fmul double {fl}, {r}"));
                self.ec(&format!("{res} = fsub double {l}, {mul}"));
                (res, Ty::Float)
            }
            (BinOp::Pow, Ty::Float) => {
                let res = self.fresh_reg();
                self.ec(&format!("{res} = call double @llvm.pow.f64(double {l}, double {r})"));
                (res, Ty::Float)
            }

            // Comparisons (work for both Int and Float)
            (BinOp::Eq,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp eq  i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::NotEq, Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp ne  i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Lt,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp slt i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::LtEq,  Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sle i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Gt,    Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sgt i64 {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::GtEq,  Ty::Int)   => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = icmp sge i64 {l}, {r}")); (r2, Ty::Bool) }

            (BinOp::Eq,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp oeq double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::NotEq, Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp one double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Lt,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp olt double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::LtEq,  Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp ole double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::Gt,    Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp ogt double {l}, {r}")); (r2, Ty::Bool) }
            (BinOp::GtEq,  Ty::Float) => { let r2 = self.fresh_reg(); self.ec(&format!("{r2} = fcmp oge double {l}, {r}")); (r2, Ty::Bool) }

            _ => {
                // Fall back to cb_binop for anything not directly lowered
                let op_code = binop_code(op);
                let lh  = self.to_handle(l, nt);
                let rh  = self.to_handle(r, nt);
                let res = self.call_cb(CB_BINOP, &[format!("i32 {op_code}"), format!("i64 {lh}"), format!("i64 {rh}")]);
                (res, Ty::Handle)
            }
        }
    }

    /// Build the argument string for a `@{sym}_fast(...)` call.
    /// Returns None if any required pre-read value is missing (fallback to _impl).
    ///
    /// The _fast ABI for a method is:
    ///   for each param in declaration order:
    ///     if it's a class instance with pre-read fields → expand all typed fields as scalars
    ///     else → pass as i64 handle
    /// The receiver (`self` / first arg of Expr::Attr) maps to param 0.
    fn build_fast_call_args(&mut self, object: &Expr, args: &[CallArg]) -> Option<String> {
        // Determine receiver param name
        let recv_name: &str = match object {
            Expr::Ident(n) if n == "self" => "self",
            Expr::Ident(n) => n.as_str(),
            _ => return None,
        };

        let mut parts: Vec<String> = Vec::new();

        // Expand receiver's fields
        let recv_class = if recv_name == "self" {
            self.current_class.as_deref()?.to_string()
        } else {
            self.param_classes.get(recv_name)?.clone()
        };
        let recv_fields = self.class_fields_ord.get(&recv_class)?.clone();
        for (field_name, field_ty) in &recv_fields {
            let key = format!("{recv_name}.{field_name}");
            let (al, ty) = self.preread_fields.get(&key)?.clone();
            let r = self.fresh_reg();
            self.ec(&format!("{r} = load {}, ptr {al}", llvm_ty(ty)));
            parts.push(format!("{} {r}", llvm_ty(*field_ty)));
        }

        // Expand each explicit arg if it's a class instance with pre-read fields;
        // otherwise pass as i64 handle.
        for call_arg in args {
            let expr = call_arg.expr();
            let arg_class = match expr {
                Expr::Ident(n) if n == "self" => self.current_class.as_deref().map(|s| s.to_string()),
                Expr::Ident(n) => self.param_classes.get(n.as_str()).cloned(),
                _ => None,
            };
            if let Some(ac) = arg_class {
                let arg_name = match expr { Expr::Ident(n) => n.as_str(), _ => return None };
                let afields = self.class_fields_ord.get(&ac)?.clone();
                for (field_name, field_ty) in &afields {
                    let key = format!("{arg_name}.{field_name}");
                    let (al, ty) = self.preread_fields.get(&key)?.clone();
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = load {}, ptr {al}", llvm_ty(ty)));
                    parts.push(format!("{} {r}", llvm_ty(*field_ty)));
                }
            } else {
                // Non-class arg: pass as i64 handle
                let (v, vt) = self.gen_expr(expr);
                let h = self.to_handle(&v, vt);
                parts.push(format!("i64 {h}"));
            }
        }

        Some(parts.join(", "))
    }

    fn gen_call(&mut self, func: &Expr, args: &[CallArg]) -> String {
        let arg_exprs: Vec<(String, Ty)> = args.iter()
            .map(|a| self.gen_expr(a.expr()))
            .collect();

        // Method call via attribute access: obj.method(args)
        if let Expr::Attr { object, attr, .. } | Expr::TraitAccess { object, attr, .. } = func {
            let key = if let Expr::TraitAccess { trait_name, .. } = func {
                format!("{trait_name}::{attr}")
            } else {
                attr.clone()
            };
            let (ov, ot) = self.gen_expr(object);
            let oh = self.to_handle(&ov, ot);

            // ── Direct intra-module method dispatch ───────────────────────────
            // If the method was compiled in this module, call its _impl directly —
            // no CB_CALL_METHOD overhead, no NATIVE_METHODS table lookup.
            let class_name = match object.as_ref() {
                Expr::Ident(n) if n == "self" => self.current_class.clone(),
                Expr::Ident(n) => self.param_classes.get(n.as_str()).cloned(),
                _ => None,
            };
            if let Some(cls) = &class_name {
                let sym = crate::partial_compiler::llvm_codegen::method_symbol(cls, &key);
                if self.module_fns.contains(&sym) && !self.locals.contains_key(&sym) {
                    let ret_ty = self.fn_sigs.get(&sym).map(|s| s.ret).unwrap_or(Ty::Handle);
                    // Collect explicit args as handles; self_h is prepended.
                    let explicit: Vec<String> = arg_exprs.iter()
                        .map(|(v, vt)| format!("i64 {}", self.to_handle(v, *vt)))
                        .collect();
                    let save = self.call_cb(CB_ARENA_SAVE, &[]);
                    let all_params = std::iter::once(format!("i64 {oh}"))
                        .chain(explicit).collect::<Vec<_>>().join(", ");
                    let raw = self.fresh_reg();
                    self.ec(&format!("{raw} = call i64 @{sym}_impl({all_params})"));
                    let result = self.call_cb(CB_ARENA_COMPACT,
                        &[format!("i64 {raw}"), format!("i64 {save}")]);
                    // Always return i64 handle; gen_expr unwraps to native type.
                    let _ = ret_ty;
                    return result;
                }
            }

            // ── Fall back to CB_CALL_METHOD (external / non-compiled method) ──
            let method_ptr = self.str_const(key.as_bytes());
            let method_len = key.len() as i32;
            let handles: Vec<String> = arg_exprs.iter()
                .map(|(v, vt)| self.to_handle(v, *vt))
                .collect();
            return if handles.is_empty() {
                self.call_cb(CB_CALL_METHOD, &[
                    format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                    "ptr null".to_string(), "i32 0".to_string()
                ])
            } else {
                let n = handles.len();
                // Use entry-block alloca to avoid stack growth inside loops.
                let arr = format!("%_margs{}", self.reg);
                self.reg += 1;
                self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                for (i, h) in handles.iter().enumerate() {
                    let ep = self.fresh_reg();
                    self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                    self.ec(&format!("store i64 {h}, ptr {ep}"));
                }
                self.call_cb(CB_CALL_METHOD, &[
                    format!("i64 {oh}"), method_ptr, format!("i32 {method_len}"),
                    format!("ptr {arr}"), format!("i32 {n}")
                ])
            };
        }

        // Intra-module direct call
        if let Expr::Ident(name) = func {
            if self.module_fns.contains(name.as_str()) && !self.locals.contains_key(name.as_str()) {
                let mutabilities = self.fn_sigs.get(name.as_str())
                    .map(|s| s.param_mutabilities.clone());
                let call_args: Vec<String> = arg_exprs.iter().enumerate()
                    .map(|(i, (v, vt))| {
                        let h = self.to_handle(v, *vt);
                        let is_mut = mutabilities.as_ref()
                            .and_then(|m| m.get(i)).copied().unwrap_or(true);
                        if is_mut {
                            format!("i64 {h}")
                        } else {
                            let dc = self.call_cb(CB_DEEP_COPY, &[format!("i64 {h}")]);
                            format!("i64 {dc}")
                        }
                    })
                    .collect();

                let save = self.call_cb(CB_ARENA_SAVE, &[]);
                let param_str = call_args.join(", ");
                let raw = self.fresh_reg();
                self.ec(&format!("{raw} = call i64 @{name}_impl({param_str})"));
                let result = self.call_cb(CB_ARENA_COMPACT, &[format!("i64 {raw}"), format!("i64 {save}")]);

                return result; // always return a handle; gen_expr unwraps for typed callees
            }
        }

        // Fast path: function-typed param with a cached trampoline pointer.
        // Loads one local ptr instead of the three-instruction ArCallbacks GEP chain,
        // letting LLVM hoist the load out of loops and keep it in a register.
        if let Expr::Ident(name) = func {
            if let Some(tp_al) = self.fn_param_trampolines.get(name.as_str()).cloned() {
                let (fn_h_val, fn_h_ty) = self.gen_expr(func);
                let fn_h = self.to_handle(&fn_h_val, fn_h_ty);
                let handles: Vec<String> = arg_exprs.iter()
                    .map(|(v, vt)| self.to_handle(v, *vt))
                    .collect();
                let tp = self.fresh_reg();
                self.ec(&format!("{tp} = load ptr, ptr {tp_al}"));
                return if handles.is_empty() {
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 (i64, ptr, i32) {tp}(i64 {fn_h}, ptr null, i32 0)"));
                    r
                } else {
                    let n   = handles.len();
                    let arr = format!("%_targs{}", self.reg); self.reg += 1;
                    self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
                    for (i, h) in handles.iter().enumerate() {
                        let ep = self.fresh_reg();
                        self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                        self.ec(&format!("store i64 {h}, ptr {ep}"));
                    }
                    let r = self.fresh_reg();
                    self.ec(&format!("{r} = call i64 (i64, ptr, i32) {tp}(i64 {fn_h}, ptr {arr}, i32 {n})"));
                    r
                };
            }
        }

        // Generic call through cb_call_fn
        let (fn_h_val, fn_h_ty) = self.gen_expr(func);
        let fn_h = self.to_handle(&fn_h_val, fn_h_ty);
        let handles: Vec<String> = arg_exprs.iter()
            .map(|(v, vt)| self.to_handle(v, *vt))
            .collect();
        if handles.is_empty() {
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), "ptr null".to_string(), "i32 0".to_string()])
        } else {
            let n   = handles.len();
            let arr = format!("%_cargs{}", self.reg); self.reg += 1;
            self.ea(&format!("{arr} = alloca [{n} x i64], align 8"));
            for (i, h) in handles.iter().enumerate() {
                let ep = self.fresh_reg();
                self.ec(&format!("{ep} = getelementptr inbounds [{n} x i64], ptr {arr}, i32 0, i32 {i}"));
                self.ec(&format!("store i64 {h}, ptr {ep}"));
            }
            self.call_cb(CB_CALL_FN, &[format!("i64 {fn_h}"), format!("ptr {arr}"), format!("i32 {n}")])
        }
    }

    // ── Statement generation ──────────────────────────────────────────────────

    pub fn gen_stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts { self.gen_stmt(s); }
    }

    fn gen_stmt(&mut self, stmt: &Stmt) {
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
                if self.current_fn_ret == Ty::Float {
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
                if self.current_fn_ret == Ty::Float {
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
                self.call_cb(CB_RAISE, &[format!("i64 0"), format!("i64 0")]);
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
                            let eq  = self.call_cb(CB_BINOP, &[format!("i32 7"), format!("i64 {subj_r}"), format!("i64 {ph}")]);
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

    fn emit_fn(&mut self, name: &str, params: &[Param], ret_ann: Option<&str>, body: &[Stmt]) {
        let ret_ty = ann_ty(ret_ann);

        // Reset per-function state
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.fn_param_trampolines.clear();
        self.current_fn_ret = ret_ty;

        // Populate param_classes for type-specialised field reads.
        self.param_classes.clear();
        self.flat_list_params.clear();
        for p in params {
            if let Some(ann) = &p.type_ann {
                if self.class_fields.contains_key(ann.as_str()) {
                    self.param_classes.insert(p.name.clone(), ann.clone());
                }
            }
        }

        // Populate flat_list_params for `let fixed_list[ClassName]` parameters.
        // Uses recursive collect_flat_leaves to support nested SWD classes.
        self.flat_list_params.clear();
        for p in params {
            if p.mutable { continue; } // only `let` params
            let Some(ann) = &p.type_ann else { continue };
            if !ann.starts_with("fixed_list[") || !ann.ends_with(']') { continue; }
            let class_name = &ann[11..ann.len() - 1]; // strip "fixed_list[" and "]"
            let leaves = collect_flat_leaves(self.all_class_fields, class_name, "", 0);
            if leaves.is_empty() { continue; }
            let stride = leaves.len() * 8;
            self.flat_list_params.insert(p.name.clone(), FlatListInfo {
                class_name: class_name.to_string(),
                leaves,
                stride,
            });
        }

        let param_sigs: Vec<String> = params.iter().enumerate()
            .map(|(i, _)| format!("i64 %_h{i}"))
            .collect();

        // Unwrap typed (int/float) params; store class-instance params as handles.
        for (i, p) in params.iter().enumerate() {
            let pt  = ann_ty(p.type_ann.as_deref());
            let st  = store_ty(pt);
            let ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int => {
                    let r = self.call_cb(CB_TO_INT, &[format!("i64 %_h{i}")]);
                    self.store_val(Ty::Int, &r, &ptr.clone());
                }
                Ty::Float => {
                    let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 %_h{i}")]);
                    self.store_val(Ty::Float, &r, &ptr.clone());
                }
                _ => {
                    self.store_val(Ty::Handle, &format!("%_h{i}"), &ptr.clone());
                }
            }
        }

        // ── Approach-1 pre-reads: for class-instance params that are never written
        // in the body, read all typed fields once at function entry via a single
        // callback per field.  gen_expr then converts Expr::Attr accesses on these
        // params into plain loads (zero callbacks in the hot path).
        let mut has_preread = false;
        for (i, p) in params.iter().enumerate() {
            // self param in methods has type_ann = None; use current_class instead.
            let class_name = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => ann.clone(),
                None if p.name == "self" => match self.current_class.as_deref() {
                    Some(cls) if self.class_fields_ord.contains_key(cls) => cls.to_string(),
                    _ => continue,
                },
                _ => continue,
            };
            // Only pre-read if the param is never written in this function body.
            if body_writes_param(body, &p.name) { continue; }
            let fields = match self.class_fields_ord.get(&class_name) {
                Some(f) => f.clone(),
                None => continue,
            };
            for (field_name, field_ty) in &fields {
                let ptr = self.str_const(field_name.as_bytes());
                let len = field_name.len() as i32;
                let r = match field_ty {
                    Ty::Float => self.call_cb(CB_GET_FLOAT_FIELD, &[
                        format!("i64 %_h{i}"), ptr, format!("i32 {len}"),
                    ]),
                    Ty::Int => self.call_cb(CB_GET_INT_FIELD, &[
                        format!("i64 %_h{i}"), ptr, format!("i32 {len}"),
                    ]),
                    _ => continue,
                };
                let al = format!("%_prf_{}_{}", p.name, field_name);
                let tstr = llvm_ty(*field_ty);
                self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                self.ec(&format!("store {tstr} {r}, ptr {al}"));
                self.preread_fields.insert(format!("{}.{field_name}", p.name), (al, *field_ty));
                has_preread = true;
            }
        }

        // Cache trampoline ptrs for `function[...]->R` typed parameters.
        // Called once at function entry so the hot path avoids the ArCallbacks GEP chain.
        for (i, p) in params.iter().enumerate() {
            let Some(ann) = &p.type_ann else { continue };
            if !ann.starts_with("function[") && !ann.starts_with("function{") { continue; }
            let tp_al = format!("%_tp_{}", p.name);
            self.alloca_buf.push_str(&format!("  {tp_al} = alloca ptr, align 8\n"));
            let tp = self.call_cb(CB_FN_TRAMPOLINE, &[format!("i64 %_h{i}")]);
            self.ec(&format!("store ptr {tp}, ptr {tp_al}"));
            self.fn_param_trampolines.insert(p.name.clone(), tp_al);
        }

        self.gen_stmts(body);

        // Fallback return (unreachable after optimiser)
        if ret_ty == Ty::Float {
            if !self.terminated { self.ec("ret double 0.0"); self.terminated = true; }
        } else {
            self.ret_handle("0");
        }

        let attr = export_attr();
        let vis  = if attr.is_empty() { "" } else { attr };
        let sig  = param_sigs.join(", ");

        // Float-returning functions use double ABI in _impl; others use i64 handle ABI.
        let impl_ret     = if ret_ty == Ty::Float { "double" } else { "i64" };
        let impl_ret_str = impl_ret;
        self.fn_defs.push_str(&format!(
            "\ndefine internal {impl_ret} @{name}_impl({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        // Public wrapper: {name}_tl(ptr %args, i32 %n) -> i64 (handle for interpreter)
        let n = params.len();
        let mut wrapper = format!("\ndefine {vis}i64 @{name}_tl(ptr %args, i32 %_n) {{\n");
        let mut load_args: Vec<String> = Vec::new();
        for i in 0..n {
            wrapper.push_str(&format!("  %_a{i} = getelementptr inbounds i64, ptr %args, i32 {i}\n"));
            wrapper.push_str(&format!("  %_v{i} = load i64, ptr %_a{i}\n"));
            load_args.push(format!("i64 %_v{i}"));
        }
        let call_args_str = load_args.join(", ");
        if ret_ty == Ty::Float {
            wrapper.push_str(&format!("  %_raw = call double @{name}_impl({call_args_str})\n"));
            wrapper.push_str("  %_cb_tl = load ptr, ptr @CB\n");
            wrapper.push_str(&format!(
                "  %_mf_p = getelementptr inbounds %ArCallbacks, ptr %_cb_tl, i32 0, i32 {CB_MAKE_FLOAT}\n"
            ));
            wrapper.push_str("  %_mf = load ptr, ptr %_mf_p\n");
            wrapper.push_str("  %_res = call i64 (double) %_mf(double %_raw)\n");
            wrapper.push_str("  ret i64 %_res\n}\n");
        } else {
            wrapper.push_str(&format!("  %_res = call {impl_ret_str} @{name}_impl({call_args_str})\n"));
            wrapper.push_str("  ret i64 %_res\n}\n");
        }
        self.fn_defs.push_str(&wrapper);

        // Emit _fast variant if there were pre-reads (class params with typed fields).
        if has_preread {
            self.emit_fn_fast(name, params, ret_ty, body);
        }

        self.current_fn_ret = Ty::Handle;
        self.preread_fields.clear();
    }

    /// Emit `@{name}_fast(...)` — like _impl but receives class param fields as
    /// raw scalars instead of arena handles.  No callbacks inside the body; the
    /// function is pure arithmetic and LLVM can inline and hoist it freely.
    fn emit_fn_fast(&mut self, name: &str, params: &[Param], ret_ty: Ty, body: &[Stmt]) {
        // Build fast signature: for each param, if it's a class instance with
        // pre-readable fields → expand to scalars; otherwise keep as i64 handle.
        let mut fast_sig: Vec<String> = Vec::new();
        // Mapping: (param_name, fast_llvm_reg_name, Ty) for each scalar field
        let mut fast_field_setup: Vec<(String, String, Ty)> = Vec::new();
        // Handle params (non-class or un-pre-readable) with their LLVM names
        let mut handle_idx = 0usize;

        for p in params {
            // self param in methods has type_ann = None; resolve via current_class.
            let class_name: Option<String> = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => Some(ann.clone()),
                None if p.name == "self" => self.current_class.as_deref()
                    .filter(|cls| self.class_fields_ord.contains_key(*cls))
                    .map(|cls| cls.to_string()),
                _ => None,
            };
            if let Some(cls) = class_name {
                if !body_writes_param(body, &p.name) {
                    if let Some(fields) = self.class_fields_ord.get(&cls).cloned() {
                        for (field_name, field_ty) in &fields {
                            let reg = format!("%_fp_{}__{field_name}", p.name);
                            let tstr = llvm_ty(*field_ty);
                            fast_sig.push(format!("{tstr} {reg}"));
                            fast_field_setup.push((
                                format!("{}.{field_name}", p.name),
                                reg,
                                *field_ty,
                            ));
                        }
                        continue; // skip handle param
                    }
                }
            }
            // Non-class or written param: pass as i64 handle
            let reg = format!("%_fph{handle_idx}");
            handle_idx += 1;
            fast_sig.push(format!("i64 {reg}"));
        }

        if fast_sig.is_empty() { return; }

        // Reset per-function codegen state for the fast variant
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0;
        self.blk = 0;
        self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();
        self.preread_fields.clear();
        self.current_fn_ret = ret_ty;

        // Rebuild param_classes
        self.param_classes.clear();
        for p in params {
            if let Some(ann) = &p.type_ann {
                if self.class_fields.contains_key(ann.as_str()) {
                    self.param_classes.insert(p.name.clone(), ann.clone());
                }
            }
        }

        // Populate preread_fields from the fast scalar params (no callbacks).
        let mut handle_idx2 = 0usize;
        for p in params {
            let class_name: Option<String> = match &p.type_ann {
                Some(ann) if self.class_fields_ord.contains_key(ann.as_str()) => Some(ann.clone()),
                None if p.name == "self" => self.current_class.as_deref()
                    .filter(|cls| self.class_fields_ord.contains_key(*cls))
                    .map(|cls| cls.to_string()),
                _ => None,
            };
            if let Some(cls) = class_name {
                if !body_writes_param(body, &p.name) {
                    if let Some(fields) = self.class_fields_ord.get(&cls).cloned() {
                        for (field_name, field_ty) in &fields {
                            let reg = format!("%_fp_{}__{field_name}", p.name);
                            let al  = format!("%_fpral_{}__{field_name}", p.name);
                            let tstr = llvm_ty(*field_ty);
                            self.alloca_buf.push_str(&format!("  {al} = alloca {tstr}, align 8\n"));
                            self.ec(&format!("store {tstr} {reg}, ptr {al}"));
                            self.preread_fields.insert(format!("{}.{field_name}", p.name), (al, *field_ty));
                        }
                        // Store the handle param as the local variable so existing code
                        // that reads it for CB_CALL_METHOD etc. still works (even though
                        // for a fully-typed method the handle is never needed in the body).
                        // We don't have the handle in the fast variant, so skip alloca_var.
                        continue;
                    }
                }
            }
            // Non-class param: unwrap from handle as in _impl
            let reg = format!("%_fph{handle_idx2}");
            handle_idx2 += 1;
            let pt  = ann_ty(p.type_ann.as_deref());
            let st  = store_ty(pt);
            let al_ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int   => { let r = self.call_cb(CB_TO_INT,   &[format!("i64 {reg}")]); self.store_val(Ty::Int,   &r, &al_ptr.clone()); }
                Ty::Float => { let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 {reg}")]); self.store_val(Ty::Float, &r, &al_ptr.clone()); }
                _         => { self.store_val(Ty::Handle, &reg, &al_ptr.clone()); }
            }
        }
        let _ = fast_field_setup; // already handled above

        self.gen_stmts(body);

        if ret_ty == Ty::Float {
            if !self.terminated { self.ec("ret double 0.0"); self.terminated = true; }
        } else {
            self.ret_handle("0");
        }

        let impl_ret = if ret_ty == Ty::Float { "double" } else { "i64" };
        let sig = fast_sig.join(", ");
        self.fn_defs.push_str(&format!(
            "\ndefine internal {impl_ret} @{name}_fast({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        self.current_fn_ret = Ty::Handle;
        self.preread_fields.clear();
    }

    fn emit_gen_fn(&mut self, name: &str, params: &[Param], body: &[Stmt]) {
        // Reset per-function state
        self.alloca_buf.clear();
        self.code_buf.clear();
        self.reg = 0; self.blk = 0; self.terminated = false;
        self.locals.clear();
        self.loop_stack.clear();
        self.block_stack.clear();

        let param_sigs: Vec<String> = params.iter().enumerate()
            .map(|(i, _)| format!("i64 %_h{i}"))
            .collect();

        for (i, p) in params.iter().enumerate() {
            let pt = ann_ty(p.type_ann.as_deref());
            let st = store_ty(pt);
            let ptr = self.alloca_var(&p.name, st);
            match pt {
                Ty::Int   => { let r = self.call_cb(CB_TO_INT,   &[format!("i64 %_h{i}")]); self.store_val(Ty::Int,   &r, &ptr.clone()); }
                Ty::Float => { let r = self.call_cb(CB_TO_FLOAT, &[format!("i64 %_h{i}")]); self.store_val(Ty::Float, &r, &ptr.clone()); }
                _         => { self.store_val(Ty::Handle, &format!("%_h{i}"), &ptr.clone()); }
            }
        }

        // Pre-allocate the accumulator list
        let list_al = "%_gen_list".to_string();
        self.ea(&format!("{list_al} = alloca i64, align 8"));
        let empty = self.call_cb(CB_MAKE_LIST, &["ptr null".to_string(), "i32 0".to_string()]);
        self.ec(&format!("store i64 {empty}, ptr {list_al}"));

        // Push a block context so Stmt::Yield can append to it
        let exit_lbl = self.fresh_blk();
        self.block_stack.push(BlockCtx {
            result_al: list_al.clone(),
            exit_label: exit_lbl.clone(),
            list_al: Some(list_al.clone()),
        });

        self.gen_stmts(body);
        self.block_stack.pop();

        // Return the accumulated list
        let list_final = self.fresh_reg();
        self.ec(&format!("{list_final} = load i64, ptr {list_al}"));
        self.ret_handle(&list_final);

        let attr = export_attr();
        let vis = if attr.is_empty() { "" } else { attr };
        let sig = param_sigs.join(", ");
        self.fn_defs.push_str(&format!(
            "\ndefine internal i64 @{name}_impl({sig}) {{\nentry:\n{}{}}}\n",
            self.alloca_buf, self.code_buf
        ));

        // Public wrapper: fname_tl(ptr %args, i32 %n) -> i64
        let n = params.len();
        let mut wrapper = format!("\ndefine {vis}i64 @{name}_tl(ptr %args, i32 %_n) {{\n");
        let mut load_args: Vec<String> = Vec::new();
        for i in 0..n {
            wrapper.push_str(&format!("  %_a{i} = getelementptr inbounds i64, ptr %args, i32 {i}\n"));
            wrapper.push_str(&format!("  %_v{i} = load i64, ptr %_a{i}\n"));
            load_args.push(format!("i64 %_v{i}"));
        }
        let call_args = load_args.join(", ");
        wrapper.push_str(&format!("  %_res = call i64 @{name}_impl({call_args})\n"));
        wrapper.push_str("  ret i64 %_res\n}\n");
        self.fn_defs.push_str(&wrapper);
    }

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
                eligible.push(EligibleFn {
                    symbol: name.clone(), orig_name: name, class_name: None,
                    params, return_type: return_type.as_deref(), body, is_gen: false,
                });
            }
            Stmt::GenDef { name, template_params, params, body, .. } => {
                if !template_params.is_empty() || !body_eligible_gen(body) { continue; }
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
                            eligible.push(EligibleFn {
                                symbol: method_symbol(class_name, mname),
                                orig_name: mname,
                                class_name: Some(class_name.clone()),
                                params, return_type: return_type.as_deref(), body, is_gen: false,
                            });
                        }
                        Stmt::GenDef { name: mname, template_params: mtp, params, body, .. } => {
                            if !mtp.is_empty() || !body_eligible_gen(body) { continue; }
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
