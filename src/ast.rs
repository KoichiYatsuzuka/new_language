#![allow(dead_code)]

use crate::token::Span;

/// A template type parameter with its trait constraints.
/// Syntax: `T: Trait1 and Trait2`
#[derive(Debug, Clone)]
pub struct TemplateParam {
    /// Name of the type variable (e.g. `T`, `T1`).
    pub name: String,
    /// Trait names the concrete type must implement (`and`-combined).
    pub constraints: Vec<String>,
}

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
    /// Template instantiation: `expr[T1, T2]` — type arguments applied to a template value.
    /// Must appear as the `func` of a `Call` expression; not valid as a standalone value.
    TemplateInstantiate { base: Box<Expr>, type_args: Vec<String> },
    /// Subscript access: `expr[index]` — index lookup on a dict (or future subscriptable types).
    Subscript { object: Box<Expr>, index: Box<Expr> },
    /// Dict literal: `{key: value, ...}` — evaluates to a `dict[Any, Any]` value.
    Dict(Vec<(Expr, Expr)>),
    /// Tuple literal: `(val, val, ...)` — evaluates to a `tuple[T1, T2, ...]` value.
    Tuple(Vec<Expr>),
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
    /// `yield expr` inside a generator function body.
    Yield(Expr),
    /// `freeze x` — demotes a `mut` variable to `let` (immutable) at runtime.
    /// If the value has a `__freeze__` method, it is called before the demotion.
    Freeze(String, Span),
    FnDef {
        name: String,
        /// Template type parameters (empty for non-template functions).
        template_params: Vec<TemplateParam>,
        params: Vec<Param>,
        return_type: Option<String>,
        body: Vec<Stmt>,
        is_virtual: bool,
    },
    /// Generator function definition (`gen name[T: Trait](params) -> YieldType:`).
    /// `yield_type` is T in `Generator[T]` — the type produced by each `yield`.
    /// The actual call-site return type is `Generator[yield_type]`.
    GenDef {
        name: String,
        template_params: Vec<TemplateParam>,
        params: Vec<Param>,
        /// Type of each yielded value (the `T` in `Generator[T]`).
        yield_type: Option<String>,
        body: Vec<Stmt>,
    },
    ClassDef {
        name: String,
        /// Template type parameters (empty for non-template classes).
        template_params: Vec<TemplateParam>,
        bases: Vec<String>,
        body: Vec<Stmt>,
    },
    TraitDef {
        name: String,
        template_params: Vec<TemplateParam>,
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
    /// `new_type NewName: OriginalType`
    /// Creates a structurally identical subclass of OriginalType with a new name.
    /// The binding is always const; reassignment is a parse error.
    NewTypeDef {
        name: String,
        /// The original type name (class, primitive type, or new_type).
        original: String,
    },
    /// `try: ... except Type as name: ... finally: ...`
    Try {
        body: Vec<Stmt>,
        handlers: Vec<ExceptHandler>,
        finally_body: Option<Vec<Stmt>>,
    },
    /// `raise expr` or bare `raise` (re-raise current exception).
    Raise {
        exc: Option<Expr>,
        span: Span,
    },
}

/// A single `except` clause inside a `try` statement.
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    /// Exception type to match, e.g. `ValueError`. `None` = bare `except:` (catch-all).
    pub exc_type: Option<String>,
    /// Name to bind the caught exception to (`as e`). `None` if omitted.
    pub name: Option<String>,
    pub body: Vec<Stmt>,
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
