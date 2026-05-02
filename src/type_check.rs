#![allow(dead_code)]

use std::collections::HashMap;

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};
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
    Unknown, // cannot be determined statically
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
    // Future: TypeMismatch, UndefinedVariable, ReturnTypeMismatch, …
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
    pub errors: Vec<StaticTypeError>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self { scopes: vec![HashMap::new()], errors: Vec::new() }
    }

    /// Run static type checking over a full program; returns collected errors.
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new();
        tc.check_stmts(stmts);
        tc.errors
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
            Stmt::FnDef { name, params, body } => {
                self.declare(name.clone(), InferredType::Unknown, false);
                self.push_scope();
                for param in params {
                    self.declare(param.name.clone(), InferredType::Unknown, param.mutable);
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
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.infer(e);
                }
            }
            Stmt::BlockReturn(expr) | Stmt::BlockYield(expr) => {
                self.infer(expr);
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
            Expr::Call { func, args } => {
                self.infer(func);
                for arg in args {
                    self.infer(arg);
                }
                InferredType::Unknown
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

    // Unknown on either side → deferred to runtime, no error
    #[test]
    fn unknown_param_comparison_ok() {
        // fn params are inferred as Unknown; comparing with any type is allowed.
        assert!(ok("fn f(x):\n    x < 1\n"));
        assert!(ok("fn f(x):\n    x < \"hello\"\n"));
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
}
