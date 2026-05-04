#![allow(dead_code)]

use std::collections::HashMap;

use crate::ast::{BinOp, CallArg, Expr, Stmt, UnaryOp};
use crate::token::Span;

// ---------------------------------------------------------------------------
// Inferred type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum InferredType {
    Int,
    Float,
    Str,
    Bool,
    None,
    List,
    /// A value whose runtime type is `type` — it holds a type itself (e.g. `int`, a class).
    TypeVal,
    Unknown, // cannot be determined statically
}

impl InferredType {
    /// Convert a type annotation string (e.g. "int", "list") to InferredType.
    fn from_ann(ann: &str) -> Option<Self> {
        match ann {
            "int" => Some(Self::Int),
            "float" => Some(Self::Float),
            "str" => Some(Self::Str),
            "bool" => Some(Self::Bool),
            "None" => Some(Self::None),
            "list" => Some(Self::List),
            "type" => Some(Self::TypeVal),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Function signature (param types + return type)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FnSig {
    /// (parameter name, declared type); type is None when no annotation.
    params: Vec<(String, Option<InferredType>)>,
    return_type: Option<InferredType>,
}

impl std::fmt::Display for InferredType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Float => write!(f, "float"),
            Self::Str => write!(f, "str"),
            Self::Bool => write!(f, "bool"),
            Self::None => write!(f, "None"),
            Self::List => write!(f, "list"),
            Self::TypeVal => write!(f, "type"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

// ---------------------------------------------------------------------------
// Error kind — add new variants here when extending checks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Ordering comparison between incompatible types (e.g. str < int)
    IncompatibleComparison {
        lhs: InferredType,
        rhs: InferredType,
        op: &'static str,
    },
    /// Assignment (plain or compound) to an immutable variable
    AssignToImmutable { name: String },
    /// Function called with wrong number of arguments
    CallArgCountMismatch { func_name: String, expected: usize, got: usize },
    /// Function called with an argument whose type conflicts with the declared parameter type
    CallArgTypeMismatch {
        func_name: String,
        param_index: usize,
        expected: InferredType,
        got: InferredType,
    },
    /// A function parameter is missing a type annotation
    MissingParamTypeAnn { func_name: String, param_name: String },
    /// A function definition is missing a return type annotation
    MissingReturnTypeAnn { func_name: String },
    /// A keyword argument name does not match any parameter of the function
    UnknownKeywordArg { func_name: String, arg_name: String },
    /// Overloaded function: no overload accepts the given number of arguments
    NoMatchingOverload { func_name: String, got: usize, available: Vec<usize> },
}

// ---------------------------------------------------------------------------
// StaticTypeError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StaticTypeError {
    pub kind: TypeErrorKind,
    /// Source location of the offending token (None only when span is unavailable).
    pub span: Option<Span>,
}

impl StaticTypeError {
    fn incompatible_cmp(lhs: InferredType, rhs: InferredType, op: &'static str, span: Span) -> Self {
        Self { kind: TypeErrorKind::IncompatibleComparison { lhs, rhs, op }, span: Some(span) }
    }

    fn assign_immutable(name: &str, span: Span) -> Self {
        Self { kind: TypeErrorKind::AssignToImmutable { name: name.to_string() }, span: Some(span) }
    }
}

impl std::fmt::Display for StaticTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Prefix with location when available.
        match &self.span {
            Some(span) => write!(f, "{span}: ")?,
            None => write!(f, "<unknown>: ")?,
        }
        match &self.kind {
            TypeErrorKind::IncompatibleComparison { lhs, rhs, op } => write!(
                f,
                "StaticTypeError: cannot compare '{lhs}' and '{rhs}' with `{op}`"
            ),
            TypeErrorKind::AssignToImmutable { name } => write!(
                f,
                "StaticTypeError: cannot assign to immutable variable '{name}'"
            ),
            TypeErrorKind::CallArgCountMismatch { func_name, expected, got } => write!(
                f,
                "StaticTypeError: '{func_name}' takes {expected} argument(s) but {got} were given"
            ),
            TypeErrorKind::CallArgTypeMismatch { func_name, param_index, expected, got } => write!(
                f,
                "StaticTypeError: argument {param_index} of '{func_name}' expects '{expected}' but got '{got}'"
            ),
            TypeErrorKind::MissingParamTypeAnn { func_name, param_name } => write!(
                f,
                "StaticTypeError: parameter '{param_name}' of function '{func_name}' is missing a type annotation"
            ),
            TypeErrorKind::MissingReturnTypeAnn { func_name } => write!(
                f,
                "StaticTypeError: function '{func_name}' is missing a return type annotation"
            ),
            TypeErrorKind::UnknownKeywordArg { func_name, arg_name } => write!(
                f,
                "StaticTypeError: '{func_name}' has no parameter named '{arg_name}'"
            ),
            TypeErrorKind::NoMatchingOverload { func_name, got, available } => {
                let avail = available.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
                write!(
                    f,
                    "StaticTypeError: no overload of '{func_name}' takes {got} argument(s) (overloads take: {avail})"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type environment
// ---------------------------------------------------------------------------

struct VarInfo {
    ty: InferredType,
    mutable: bool,
}

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

pub struct TypeChecker {
    // Innermost scope is last; lookup searches back-to-front.
    scopes: Vec<HashMap<String, VarInfo>>,
    /// All overloads per function name, collected in a pre-pass for forward-call checking.
    fn_sigs: HashMap<String, Vec<FnSig>>,
    pub errors: Vec<StaticTypeError>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut global: HashMap<String, VarInfo> = HashMap::new();
        // Pre-define built-in type values so that `int`, `str`, `float`, `bool`
        // are recognised as `InferredType::TypeVal` in expression context.
        for name in ["int", "str", "float", "bool"] {
            global.insert(name.to_string(), VarInfo { ty: InferredType::TypeVal, mutable: false });
        }
        Self { scopes: vec![global], fn_sigs: HashMap::new(), errors: Vec::new() }
    }

    /// Run static type checking over a full program; returns collected errors.
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new();
        tc.collect_fn_sigs(stmts);
        tc.check_stmts(stmts);
        tc.errors
    }

    /// Pre-pass: register all FnDef signatures so calls can be validated regardless of order.
    fn collect_fn_sigs(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::FnDef { name, params, return_type, body, .. } => {
                    let sig = FnSig {
                        params: params.iter()
                            .map(|p| (p.name.clone(), p.type_ann.as_deref().and_then(InferredType::from_ann)))
                            .collect(),
                        return_type: return_type.as_deref().and_then(InferredType::from_ann),
                    };
                    self.fn_sigs.entry(name.clone()).or_default().push(sig);
                    self.collect_fn_sigs(body);
                }
                Stmt::ClassDef { body, .. } => self.collect_fn_sigs(body),
                Stmt::TraitDef { body, .. } => self.collect_fn_sigs(body),
                _ => {}
            }
        }
    }

    // --- Scope helpers ---

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn declare(&mut self, name: String, ty: InferredType, mutable: bool) {
        self.scopes.last_mut().unwrap().insert(name, VarInfo { ty, mutable });
    }

    fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn emit(&mut self, err: StaticTypeError) {
        self.errors.push(err);
    }

    // --- Statement checking ---

    fn check_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(name, expr) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Const(name, expr) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Mut(name, expr) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, true);
            }
            Stmt::Assign { name, value, span } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.emit(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::CompoundAssign { name, op: _, value, span } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.emit(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::AttrAssign { target, value } => {
                self.infer(target);
                self.infer(value);
            }
            Stmt::AttrCompoundAssign { target, op: _, value } => {
                self.infer(target);
                self.infer(value);
            }
            Stmt::Expr(expr) => {
                self.infer(expr);
            }
            Stmt::If { branches, else_body } => {
                for (cond, body) in branches {
                    self.infer(cond);
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
                if let Some(body) = else_body {
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
            }
            Stmt::While { cond, body } => {
                self.infer(cond);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::For { target, iter, body } => {
                self.infer(iter);
                self.push_scope();
                // Loop variable type is unknown until we track collection element types.
                self.declare(target.clone(), InferredType::Unknown, true);
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::Block(body) => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::FnDef { name, params, return_type, body, .. } => {
                // Check for missing type annotations on parameters (skip `self`).
                for param in params.iter() {
                    if param.name == "self" { continue; }
                    if param.type_ann.is_none() {
                        self.emit(StaticTypeError {
                            kind: TypeErrorKind::MissingParamTypeAnn {
                                func_name: name.clone(),
                                param_name: param.name.clone(),
                            },
                            span: None,
                        });
                    }
                }
                // Check for missing return type annotation.
                if return_type.is_none() {
                    self.emit(StaticTypeError {
                        kind: TypeErrorKind::MissingReturnTypeAnn { func_name: name.clone() },
                        span: None,
                    });
                }
                self.declare(name.clone(), InferredType::Unknown, false);
                self.push_scope();
                for param in params {
                    let ty = param.type_ann.as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unknown);
                    self.declare(param.name.clone(), ty, param.mutable);
                }
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::ClassDef { name, bases: _, body } => {
                self.declare(name.clone(), InferredType::Unknown, false);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::TraitDef { name, body } => {
                self.declare(name.clone(), InferredType::Unknown, false);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.infer(e);
                }
            }
            Stmt::BlockReturn(expr) | Stmt::BlockYield(expr) => {
                self.infer(expr);
            }
            Stmt::Field { name, kind, type_ann, default } => {
                let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unknown);
                if let Some(expr) = default {
                    self.infer(expr);
                }
                let mutable = matches!(kind, crate::ast::FieldKind::Mut);
                self.declare(name.clone(), ty, mutable);
            }
            Stmt::Pass | Stmt::Break | Stmt::Continue => {}
        }
    }

    // --- Expression type inference ---

    fn infer(&mut self, expr: &Expr) -> InferredType {
        match expr {
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
            Expr::List(_) => InferredType::List,
            Expr::Attr { object, .. } => {
                self.infer(object);
                InferredType::Unknown
            }
            Expr::TraitAccess { object, .. } => {
                self.infer(object);
                InferredType::Unknown
            }
            Expr::Call { func, args } => {
                // Capture function name before mutably borrowing self for infer calls.
                let func_name = if let Expr::Ident(name) = func.as_ref() {
                    Some(name.clone())
                } else {
                    None
                };
                self.infer(func);

                // Collect (keyword_name, inferred_type) for each argument.
                let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
                for arg in args.iter() {
                    match arg {
                        CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                        CallArg::Keyword { name, value } => {
                            arg_data.push((Some(name.clone()), self.infer(value)))
                        }
                    }
                }

                if let Some(ref fname) = func_name {
                    if let Some(sigs) = self.fn_sigs.get(fname).cloned() {
                        let call_count = arg_data.len();

                        // Signatures whose parameter count matches the call.
                        let count_matching: Vec<FnSig> = sigs.iter()
                            .filter(|s| s.params.len() == call_count)
                            .cloned()
                            .collect();

                        if count_matching.is_empty() {
                            // No overload accepts this many arguments.
                            if sigs.len() == 1 {
                                self.emit(StaticTypeError {
                                    kind: TypeErrorKind::CallArgCountMismatch {
                                        func_name: fname.clone(),
                                        expected: sigs[0].params.len(),
                                        got: call_count,
                                    },
                                    span: None,
                                });
                            } else {
                                let available = sigs.iter().map(|s| s.params.len()).collect();
                                self.emit(StaticTypeError {
                                    kind: TypeErrorKind::NoMatchingOverload {
                                        func_name: fname.clone(),
                                        got: call_count,
                                        available,
                                    },
                                    span: None,
                                });
                            }
                        } else if count_matching.len() == 1 {
                            // Exactly one overload matches the count → check arg types.
                            let sig = &count_matching[0];
                            let mut positional_idx = 0usize;
                            for (key, arg_ty) in &arg_data {
                                match key {
                                    Some(kwarg_name) => {
                                        match sig.params.iter().position(|(n, _)| n == kwarg_name) {
                                            None => self.emit(StaticTypeError {
                                                kind: TypeErrorKind::UnknownKeywordArg {
                                                    func_name: fname.clone(),
                                                    arg_name: kwarg_name.clone(),
                                                },
                                                span: None,
                                            }),
                                            Some(param_pos) => {
                                                if let Some(expected) = &sig.params[param_pos].1 {
                                                    if *arg_ty != InferredType::Unknown && arg_ty != expected {
                                                        self.emit(StaticTypeError {
                                                            kind: TypeErrorKind::CallArgTypeMismatch {
                                                                func_name: fname.clone(),
                                                                param_index: param_pos,
                                                                expected: expected.clone(),
                                                                got: arg_ty.clone(),
                                                            },
                                                            span: None,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    None => {
                                        if let Some((_, param_ty)) = sig.params.get(positional_idx) {
                                            if let Some(expected) = param_ty {
                                                if *arg_ty != InferredType::Unknown && arg_ty != expected {
                                                    self.emit(StaticTypeError {
                                                        kind: TypeErrorKind::CallArgTypeMismatch {
                                                            func_name: fname.clone(),
                                                            param_index: positional_idx,
                                                            expected: expected.clone(),
                                                            got: arg_ty.clone(),
                                                        },
                                                        span: None,
                                                    });
                                                }
                                            }
                                        }
                                        positional_idx += 1;
                                    }
                                }
                            }
                        }
                        // Multiple count-matching overloads: runtime dispatch decides, skip type check.
                    }
                }

                // Return type: use the unique count-matching overload's return type if there is one.
                func_name
                    .as_deref()
                    .and_then(|n| self.fn_sigs.get(n))
                    .and_then(|sigs| {
                        let matching: Vec<_> = sigs.iter()
                            .filter(|s| s.params.len() == arg_data.len())
                            .collect();
                        if matching.len() == 1 { matching[0].return_type.clone() } else { None }
                    })
                    .unwrap_or(InferredType::Unknown)
            }
            Expr::Ident(name) => {
                self.lookup(name).map(|v| v.ty.clone()).unwrap_or(InferredType::Unknown)
            }
            Expr::UnaryOp { op, operand } => {
                let ty = self.infer(operand);
                match op {
                    UnaryOp::Not => InferredType::Bool,
                    UnaryOp::Neg => match ty {
                        InferredType::Int => InferredType::Int,
                        InferredType::Float => InferredType::Float,
                        _ => InferredType::Unknown,
                    },
                    UnaryOp::BitNot => InferredType::Int,
                }
            }
            Expr::BinOp { op, left, right, span } => {
                let lt = self.infer(left);
                let rt = self.infer(right);
                self.check_binop(op, &lt, &rt, span.clone());
                Self::infer_binop_result(op, &lt, &rt)
            }
        }
    }

    // --- Binary operator checks ---

    fn check_binop(&mut self, op: &BinOp, lt: &InferredType, rt: &InferredType, span: Span) {
        match op {
            BinOp::Lt => self.check_ordered_cmp(lt, rt, "<", span),
            BinOp::Gt => self.check_ordered_cmp(lt, rt, ">", span),
            BinOp::LtEq => self.check_ordered_cmp(lt, rt, "<=", span),
            BinOp::GtEq => self.check_ordered_cmp(lt, rt, ">=", span),
            // Extend: BinOp::Add => check_add_types(lt, rt, span), etc.
            _ => {}
        }
    }

    fn check_ordered_cmp(&mut self, lt: &InferredType, rt: &InferredType, op: &'static str, span: Span) {
        if !Self::ordered_comparable(lt, rt) {
            self.emit(StaticTypeError::incompatible_cmp(lt.clone(), rt.clone(), op, span));
        }
    }

    /// Returns true when `lt op rt` is valid for ordering operators.
    /// Unknown on either side is treated as "may be compatible" (deferred to runtime).
    fn ordered_comparable(lt: &InferredType, rt: &InferredType) -> bool {
        use InferredType::*;
        matches!(
            (lt, rt),
            (Unknown, _)
                | (_, Unknown)
                | (Int, Int)
                | (Float, Float)
                | (Int, Float)
                | (Float, Int)
                | (Str, Str)
        )
    }

    fn infer_binop_result(op: &BinOp, lt: &InferredType, rt: &InferredType) -> InferredType {
        use InferredType::*;
        match op {
            BinOp::Eq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::And
            | BinOp::Or => Bool,
            BinOp::Add => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Str, Str) => Str,
                _ => Unknown,
            },
            BinOp::Sub | BinOp::Mul | BinOp::Pow => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unknown,
            },
            BinOp::Div => Float,
            BinOp::FloorDiv | BinOp::Mod => match (lt, rt) {
                (Int, Int) => Int,
                _ => Unknown,
            },
            BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::LShift
            | BinOp::RShift => Int,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check(src: &str) -> Vec<StaticTypeError> {
        let tokens = Lexer::new(src, "").tokenize();
        let stmts = Parser::new(tokens).parse_program().expect("parse error");
        TypeChecker::check(&stmts)
    }

    fn ok(src: &str) -> bool {
        check(src).is_empty()
    }
    fn err(src: &str) -> bool {
        !check(src).is_empty()
    }

    // --- Immutable assignment ---

    #[test]
    fn let_immutable_assign() {
        assert!(err("let x = 1\nx = 2"));
    }

    #[test]
    fn const_immutable_assign() {
        assert!(err("const X = 1\nX = 2"));
    }

    #[test]
    fn mut_assign_ok() {
        assert!(ok("mut x = 1\nx = 2"));
    }

    #[test]
    fn let_compound_assign_immutable() {
        assert!(err("let x = 1\nx += 1"));
    }

    #[test]
    fn mut_compound_assign_ok() {
        assert!(ok("mut x = 1\nx += 1"));
    }

    #[test]
    fn immutable_assign_inside_if() {
        assert!(err("let x = 1\nif True:\n    x = 2\n"));
    }

    #[test]
    fn mut_assign_inside_if_ok() {
        assert!(ok("mut x = 1\nif True:\n    x = 2\n"));
    }

    // --- Ordering comparison ---

    #[test]
    fn int_int_lt_ok() {
        assert!(ok("1 < 2"));
    }

    #[test]
    fn float_float_lt_ok() {
        assert!(ok("1.0 < 2.0"));
    }

    #[test]
    fn int_float_lt_ok() {
        assert!(ok("1 < 2.0"));
    }

    #[test]
    fn str_str_lt_ok() {
        assert!(ok(r#""a" < "b""#));
    }

    #[test]
    fn str_int_lt_err() {
        assert!(err(r#""hello" < 42"#));
    }

    #[test]
    fn int_str_gt_err() {
        assert!(err(r#"42 > "hello""#));
    }

    #[test]
    fn bool_int_lt_err() {
        assert!(err("True < 1"));
    }

    #[test]
    fn str_float_le_err() {
        assert!(err(r#""x" <= 1.5"#));
    }

    // == / != between different types is NOT an error
    #[test]
    fn eq_different_types_ok() {
        assert!(ok(r#"1 == "hello""#));
    }

    #[test]
    fn neq_different_types_ok() {
        assert!(ok(r#"True != "x""#));
    }

    // Unknown on either side → deferred to runtime, no IncompatibleComparison error
    #[test]
    fn unknown_param_comparison_ok() {
        // Unannotated params produce MissingParamTypeAnn / MissingReturnTypeAnn errors,
        // but NOT IncompatibleComparison errors — comparison is deferred to runtime.
        let errors = check("fn f(x):\n    x < 1\n");
        assert!(!errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::IncompatibleComparison { .. })));
        let errors = check("fn f(x):\n    x < \"hello\"\n");
        assert!(!errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::IncompatibleComparison { .. })));
    }

    #[test]
    fn int_str_lt_is_error() {
        // Once x is known to be Int, comparing with Str must be flagged.
        assert!(err("mut x = 1\nx < \"hello\""));
    }

    // Multiple errors collected
    #[test]
    fn collects_multiple_errors() {
        let errors = check("let a = 1\na = 2\nlet b = 1\nb = 3\n");
        assert_eq!(errors.len(), 2);
    }

    // Error message format
    #[test]
    fn error_display_assign() {
        let errors = check("let x = 1\nx = 2");
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("immutable"));
        assert!(errors[0].to_string().contains("'x'"));
    }

    #[test]
    fn error_display_comparison() {
        let errors = check(r#""a" < 1"#);
        assert!(errors[0].to_string().contains("StaticTypeError"));
        assert!(errors[0].to_string().contains("str"));
        assert!(errors[0].to_string().contains("int"));
    }

    // --- Function call argument checking ---

    #[test]
    fn call_correct_types_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2)\n"));
    }

    #[test]
    fn call_arg_type_mismatch_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1, \"hello\")\n"));
    }

    #[test]
    fn call_arg_count_too_few_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1)\n"));
    }

    #[test]
    fn call_arg_count_too_many_err() {
        assert!(err("fn add(a: int, b: int) -> int:\n    pass\nadd(1, 2, 3)\n"));
    }

    #[test]
    fn call_no_annotation_no_type_mismatch() {
        // Unannotated params still do NOT produce CallArgTypeMismatch — type check is deferred.
        // (MissingParamTypeAnn / MissingReturnTypeAnn errors ARE produced.)
        let errors = check("fn f(x, y):\n    pass\nf(1, \"hello\")\n");
        assert!(!errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn call_unknown_arg_skipped_ok() {
        // If the argument type is Unknown (e.g. a variable without annotation),
        // the check is deferred to runtime.
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\nmut x = 1\nadd(x, x)\n"));
    }

    #[test]
    fn call_forward_definition_checked() {
        // Call appears before fn definition — pre-pass must still catch the error.
        assert!(err("add(1, \"oops\")\nfn add(a: int, b: int) -> int:\n    pass\n"));
    }

    #[test]
    fn call_return_type_inferred() {
        // Return type from annotation flows into inferred type of the call expression.
        // Assigning it to a variable and using in an ordering comparison should be fine.
        assert!(ok("fn get_int() -> int:\n    pass\nlet v = get_int()\nv < 10\n"));
    }

    #[test]
    fn error_display_call_count() {
        // Return type annotated to avoid MissingReturnTypeAnn noise.
        let errors = check("fn f(a: int, b: int) -> None:\n    pass\nf(1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'f'"));
        assert!(msg.contains("2"));
        assert!(msg.contains("1"));
    }

    #[test]
    fn error_display_call_type() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(\"hello\")\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'f'"));
        assert!(msg.contains("int"));
        assert!(msg.contains("str"));
    }

    // --- Missing type annotation ---

    #[test]
    fn fn_fully_annotated_ok() {
        assert!(ok("fn add(a: int, b: int) -> int:\n    pass\n"));
    }

    #[test]
    fn fn_missing_param_ann_err() {
        assert!(err("fn f(x) -> int:\n    pass\n"));
    }

    #[test]
    fn fn_missing_return_ann_err() {
        assert!(err("fn f(x: int):\n    pass\n"));
    }

    #[test]
    fn fn_missing_both_ann_err() {
        let errors = check("fn f(x):\n    pass\n");
        // Expects MissingParamTypeAnn for x AND MissingReturnTypeAnn for f.
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn fn_multiple_missing_params_err() {
        let errors = check("fn f(a, b, c) -> int:\n    pass\n");
        // One MissingParamTypeAnn per unannotated parameter.
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn fn_no_params_missing_return_err() {
        assert!(err("fn greet():\n    pass\n"));
    }

    #[test]
    fn error_display_missing_param_ann() {
        let errors = check("fn f(x) -> int:\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'x'"));
        assert!(msg.contains("'f'"));
    }

    #[test]
    fn error_display_missing_return_ann() {
        let errors = check("fn f(x: int):\n    pass\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'f'"));
    }

    // --- Keyword arguments ---

    #[test]
    fn kwarg_correct_ok() {
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(a=1, b=\"hi\")\n"));
    }

    #[test]
    fn kwarg_reversed_order_ok() {
        // Keyword args may appear in any order.
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(b=\"hi\", a=1)\n"));
    }

    #[test]
    fn kwarg_unknown_name_err() {
        assert!(err("fn f(a: int, b: int) -> None:\n    pass\nf(a=1, z=2)\n"));
    }

    #[test]
    fn kwarg_type_mismatch_err() {
        assert!(err("fn f(a: int) -> None:\n    pass\nf(a=\"hello\")\n"));
    }

    #[test]
    fn kwarg_mixed_positional_keyword_ok() {
        assert!(ok("fn f(a: int, b: str) -> None:\n    pass\nf(1, b=\"hi\")\n"));
    }

    #[test]
    fn error_display_unknown_kwarg() {
        let errors = check("fn f(a: int) -> None:\n    pass\nf(z=1)\n");
        let msg = errors[0].to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'f'"));
        assert!(msg.contains("'z'"));
    }

    // --- Overloading ---

    #[test]
    fn overload_by_count_ok() {
        assert!(ok(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1)\n",
            "f(1, 2)\n",
        )));
    }

    #[test]
    fn overload_by_type_ok() {
        // Different type per overload — both calls are valid
        assert!(ok(concat!(
            "fn show(x: int) -> None:\n    pass\n",
            "fn show(x: str) -> None:\n    pass\n",
            "show(1)\n",
            "show(\"hi\")\n",
        )));
    }

    #[test]
    fn overload_wrong_count_err() {
        // Neither overload takes 3 args → NoMatchingOverload
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        assert!(errors.iter().any(|e| matches!(
            &e.kind, TypeErrorKind::NoMatchingOverload { got: 3, .. }
        )));
    }

    #[test]
    fn overload_single_def_count_err_uses_count_mismatch() {
        // With one definition, count error should be CallArgCountMismatch (not NoMatchingOverload)
        let errors = check("fn f(a: int) -> None:\n    pass\nf(1, 2)\n");
        assert!(errors.iter().any(|e| matches!(
            &e.kind, TypeErrorKind::CallArgCountMismatch { .. }
        )));
    }

    #[test]
    fn overload_multiple_count_match_skips_type_check() {
        // Two overloads both take 1 arg — type errors are NOT emitted (runtime decides)
        let errors = check(concat!(
            "fn f(x: int) -> None:\n    pass\n",
            "fn f(x: str) -> None:\n    pass\n",
            "f(True)\n",  // True is bool, doesn't match int or str, but we skip type checking
        ));
        assert!(!errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }

    #[test]
    fn overload_display_no_matching() {
        let errors = check(concat!(
            "fn f(a: int) -> None:\n    pass\n",
            "fn f(a: int, b: int) -> None:\n    pass\n",
            "f(1, 2, 3)\n",
        ));
        let msg = errors.iter()
            .find(|e| matches!(&e.kind, TypeErrorKind::NoMatchingOverload { .. }))
            .unwrap()
            .to_string();
        assert!(msg.contains("StaticTypeError"));
        assert!(msg.contains("'f'"));
        assert!(msg.contains('3'));
    }

    // --- trait ---

    #[test]
    fn trait_with_virtual_method_no_type_errors() {
        // A well-formed trait with a virtual method should produce no type errors.
        assert!(ok(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
        )));
    }

    #[test]
    fn trait_with_non_virtual_method_no_type_errors() {
        assert!(ok(concat!(
            "trait Logger:\n",
            "    fn log(self, msg: str) -> None:\n",
            "        pass\n",
        )));
    }

    #[test]
    fn trait_with_fields_no_type_errors() {
        assert!(ok(concat!(
            "trait HasValue:\n",
            "    mut value: int\n",
            "    const MAX: int = 100\n",
        )));
    }

    #[test]
    fn trait_class_inheriting_no_type_errors() {
        assert!(ok(concat!(
            "trait Shape:\n",
            "    fn area(self) -> float:\n",
            "        ...\n",
            "class Square(Shape):\n",
            "    mut side: float\n",
            "    fn area(self) -> float:\n",
            "        pass\n",
        )));
    }

    #[test]
    fn trait_class_call_type_mismatch_detected() {
        // Type checker still catches arg-type mismatches on classes that inherit traits.
        let errors = check(concat!(
            "trait T:\n",
            "    fn f(self) -> None:\n",
            "        ...\n",
            "class C(T):\n",
            "    mut x: int\n",
            "    fn f(self) -> None:\n",
            "        pass\n",
            "fn use_x(v: int) -> None:\n",
            "    pass\n",
            "use_x(\"wrong\")\n",
        ));
        assert!(errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::CallArgTypeMismatch { .. })));
    }
}
