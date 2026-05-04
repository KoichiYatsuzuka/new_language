#![allow(dead_code)]

use crate::token::Span;

/// A single argument in a function call: positional or keyword (`name=value`).
#[derive(Debug, Clone)]
pub enum CallArg {
    Positional(Expr),
    Keyword { name: String, value: Expr },
}

impl CallArg {
    pub fn expr(&self) -> &Expr {
        match self {
            Self::Positional(e) | Self::Keyword { value: e, .. } => e,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub mutable: bool,
    pub type_ann: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, FloorDiv, Mod, Pow,
    Eq, NotEq, Lt, Gt, LtEq, GtEq,
    And, Or,
    BitAnd, BitOr, BitXor, LShift, RShift,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    Ident(String),
    List(Vec<Expr>),
    Attr { object: Box<Expr>, attr: String }, // obj.attr
    TraitAccess { object: Box<Expr>, trait_name: String, attr: String }, // obj::Trait.attr
    BinOp { op: BinOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    Call { func: Box<Expr>, args: Vec<CallArg> },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Let(String, Expr),                    // let x = expr   (immutable)
    Const(String, Expr),                  // const X = expr  (immutable)
    Mut(String, Expr),                    // mut x = expr    (mutable)
    Assign { name: String, value: Expr, span: Span },                       // x = expr
    AttrAssign { target: Expr, value: Expr },                                // obj.attr = expr
    AttrCompoundAssign { target: Expr, op: BinOp, value: Expr },            // obj.attr += expr
    CompoundAssign { name: String, op: BinOp, value: Expr, span: Span },    // x += expr
    If {
        branches: Vec<(Expr, Vec<Stmt>)>, // (condition, body) — if + elif arms
        else_body: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    For {
        target: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),        // block: …  (anonymous scope)
    Return(Option<Expr>),
    Break,
    Continue,
    Pass,
    BlockReturn(Expr),       // block_return expr
    BlockYield(Expr),        // block_yield expr
    FnDef {
        name: String,
        params: Vec<Param>,
        return_type: Option<String>,
        body: Vec<Stmt>,
        is_virtual: bool,
    },
    ClassDef {
        name: String,
        bases: Vec<String>,
        body: Vec<Stmt>,
    },
    TraitDef {
        name: String,
        body: Vec<Stmt>,
    },
    /// Typed field declaration inside a class body.
    /// Syntax: `[mut|let|const] name: Type [= default]`
    /// `const` fields are class variables and must have a default value.
    Field {
        name: String,
        kind: FieldKind,
        type_ann: String,
        default: Option<Expr>,
    },
}

/// Mutability/ownership kind for a class field declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// Mutable instance variable (`mut name: Type [= default]`)
    Mut,
    /// Immutable instance variable (`let name: Type [= default]`)
    Let,
    /// Class variable — immutable, shared across all instances (`const name: Type = default`)
    Const,
}
