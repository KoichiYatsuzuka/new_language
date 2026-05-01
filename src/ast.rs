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
    BinOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    Call { func: Box<Expr>, args: Vec<Expr> },
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Let(String, Expr),                   // let x = expr   (immutable)
    Const(String, Expr),                 // const X = expr  (immutable)
    Mut(String, Expr),                   // mut x = expr    (mutable)
    Assign(String, Expr),                // x = expr
    CompoundAssign(String, BinOp, Expr), // x += expr
}
