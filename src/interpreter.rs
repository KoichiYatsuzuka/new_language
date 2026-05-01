use std::collections::HashMap;

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    List(Vec<Value>),
}

/// Returned by exec() to signal normal completion or control flow.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecResult {
    Normal,
    Break,
    Continue,
    Return(Value),
    BlockReturn(Value),
}

struct Var {
    value: Value,
    mutable: bool,
}

pub struct Interpreter {
    // Innermost scope is last; lookup searches from back to front.
    scopes: Vec<HashMap<String, Var>>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self { scopes: vec![HashMap::new()] }
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

    fn get_var(&self, name: &str) -> Option<&Var> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    fn declare_var(&mut self, name: String, var: Var) {
        self.scopes.last_mut().unwrap().insert(name, var);
    }

    // Walks the scope chain and updates the first binding found.
    fn assign_var(&mut self, name: &str, value: Value) -> Result<(), String> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.get_mut(name) {
                if !v.mutable {
                    return Err(format!(
                        "TypeError: cannot assign to immutable variable '{name}'"
                    ));
                }
                v.value = value;
                return Ok(());
            }
        }
        Err(format!("NameError: '{name}' is not defined"))
    }

    // --- Statement execution ---

    pub fn exec(&mut self, stmt: &Stmt) -> Result<ExecResult, String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Let(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Const(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: false });
                Ok(ExecResult::Normal)
            }
            Stmt::Mut(name, expr) => {
                let value = self.eval(expr)?;
                self.declare_var(name.clone(), Var { value, mutable: true });
                Ok(ExecResult::Normal)
            }
            Stmt::Assign(name, expr) => {
                let value = self.eval(expr)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::AttrAssign { .. } => {
                // TODO: attribute assignment (needs object system)
                Ok(ExecResult::Normal)
            }
            Stmt::CompoundAssign(name, op, expr) => {
                let rhs = self.eval(expr)?;
                let lhs = match self.get_var(name) {
                    Some(v) if !v.mutable => {
                        return Err(format!(
                            "TypeError: cannot assign to immutable variable '{name}'"
                        ));
                    }
                    Some(v) => v.value.clone(),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                };
                let value = self.apply_binop(op, lhs, rhs)?;
                self.assign_var(name, value)?;
                Ok(ExecResult::Normal)
            }
            Stmt::Pass => Ok(ExecResult::Normal),
            Stmt::Break => Ok(ExecResult::Break),
            Stmt::Continue => Ok(ExecResult::Continue),
            Stmt::Return(expr) => {
                let val = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::None,
                };
                Ok(ExecResult::Return(val))
            }
            Stmt::BlockReturn(expr) => {
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::BlockYield(expr) => {
                let val = self.eval(expr)?;
                Ok(ExecResult::BlockReturn(val))
            }
            Stmt::If { branches, else_body } => {
                for (cond, body) in branches {
                    let val = self.eval(cond)?;
                    if self.is_truthy(&val) {
                        return self.exec_scoped_block(body);
                    }
                }
                if let Some(body) = else_body {
                    return self.exec_scoped_block(body);
                }
                Ok(ExecResult::Normal)
            }
            Stmt::While { cond, body } => {
                loop {
                    let val = self.eval(cond)?;
                    if !self.is_truthy(&val) {
                        break;
                    }
                    match self.exec_scoped_block(body)? {
                        ExecResult::Break => break,
                        ExecResult::Continue | ExecResult::Normal => {}
                        r => return Ok(r),
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::For { target, iter, body } => {
                let iter_val = self.eval(iter)?;
                let items = match iter_val {
                    Value::List(items) => items,
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    _ => return Err("TypeError: object is not iterable".to_string()),
                };
                for item in items {
                    // Each iteration gets its own scope containing the loop variable.
                    self.push_scope();
                    self.declare_var(target.clone(), Var { value: item, mutable: true });
                    let result = self.exec_block(body);
                    self.pop_scope();
                    match result? {
                        ExecResult::Break => break,
                        ExecResult::Continue | ExecResult::Normal => {}
                        r => return Ok(r),
                    }
                }
                Ok(ExecResult::Normal)
            }
            Stmt::Block(body) => {
                match self.exec_scoped_block(body)? {
                    ExecResult::BlockReturn(_) | ExecResult::Normal => Ok(ExecResult::Normal),
                    r => Ok(r),
                }
            }
            Stmt::FnDef { .. } => {
                // TODO: function objects
                Ok(ExecResult::Normal)
            }
            Stmt::ClassDef { .. } => {
                // TODO: class objects
                Ok(ExecResult::Normal)
            }
        }
    }

    // Executes a list of statements without creating a new scope.
    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        for stmt in stmts {
            match self.exec(stmt)? {
                ExecResult::Normal => {}
                signal => return Ok(signal),
            }
        }
        Ok(ExecResult::Normal)
    }

    // Executes a list of statements in a fresh scope.
    fn exec_scoped_block(&mut self, stmts: &[Stmt]) -> Result<ExecResult, String> {
        self.push_scope();
        let result = self.exec_block(stmts);
        self.pop_scope(); // always runs, even on Err
        result
    }

    // --- Expression evaluation ---

    pub fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::None => Ok(Value::None),
            Expr::Ident(name) => self
                .get_var(name)
                .map(|v| v.value.clone())
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::Attr { .. } => {
                // TODO: attribute access (needs object system)
                Err("AttributeError: attribute access not yet implemented".to_string())
            }
            Expr::List(items) => {
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval(item)?);
                }
                Ok(Value::List(vals))
            }
            Expr::UnaryOp { op, operand } => {
                let val = self.eval(operand)?;
                self.apply_unary(op, val)
            }
            Expr::BinOp { op, left, right } => {
                match op {
                    BinOp::And => {
                        let lv = self.eval(left)?;
                        return if !self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    BinOp::Or => {
                        let lv = self.eval(left)?;
                        return if self.is_truthy(&lv) { Ok(lv) } else { self.eval(right) };
                    }
                    _ => {}
                }
                let lv = self.eval(left)?;
                let rv = self.eval(right)?;
                self.apply_binop(op, lv, rv)
            }
            Expr::Call { func, args } => {
                let Expr::Ident(name) = func.as_ref() else {
                    return Err("TypeError: object is not callable".to_string());
                };
                match name.as_str() {
                    "print" => {
                        let parts: Result<Vec<_>, _> =
                            args.iter().map(|a| self.eval(a).map(|v| self.display(&v))).collect();
                        println!("{}", parts?.join(" "));
                        Ok(Value::None)
                    }
                    "range" => {
                        let evaled: Result<Vec<_>, _> = args.iter().map(|a| self.eval(a)).collect();
                        let evaled = evaled?;
                        match evaled.as_slice() {
                            [Value::Int(stop)] => {
                                Ok(Value::List((0..*stop).map(Value::Int).collect()))
                            }
                            [Value::Int(start), Value::Int(stop)] => {
                                Ok(Value::List((*start..*stop).map(Value::Int).collect()))
                            }
                            [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                                let mut items = Vec::new();
                                let mut i = *start;
                                if *step > 0 {
                                    while i < *stop { items.push(Value::Int(i)); i += step; }
                                } else if *step < 0 {
                                    while i > *stop { items.push(Value::Int(i)); i += step; }
                                }
                                Ok(Value::List(items))
                            }
                            _ => Err("TypeError: range() takes 1–3 integer arguments".to_string()),
                        }
                    }
                    "len" => {
                        if args.len() != 1 {
                            return Err("TypeError: len() takes exactly one argument".to_string());
                        }
                        let val = self.eval(&args[0])?;
                        match val {
                            Value::List(items) => Ok(Value::Int(items.len() as i64)),
                            Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                            _ => Err(format!("TypeError: object of type '{:?}' has no len()", val)),
                        }
                    }
                    _ => Err(format!("NameError: '{name}' is not defined")),
                }
            }
        }
    }

    fn display(&self, val: &Value) -> String {
        match val {
            Value::Int(n) => n.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Str(s) => s.clone(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::None => "None".to_string(),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }

    fn display_repr(&self, val: &Value) -> String {
        match val {
            Value::Str(s) => format!("'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            _ => self.display(val),
        }
    }

    fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::List(items) => !items.is_empty(),
        }
    }

    fn apply_unary(&self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(format!("TypeError: bad operand type for unary `-`: {val:?}")),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                _ => Err(format!("TypeError: bad operand type for unary `~`: {val:?}")),
            },
        }
    }

    fn apply_binop(&self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
        match (op, &lv, &rv) {
            // Arithmetic
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a + *b)),
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),
            (BinOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + *b)),
            (BinOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a + *b as f64)),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            (BinOp::Sub, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a - *b)),
            (BinOp::Sub, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a - *b)),
            (BinOp::Sub, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - *b)),
            (BinOp::Sub, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a - *b as f64)),
            (BinOp::Mul, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a * *b)),
            (BinOp::Mul, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a * *b)),
            (BinOp::Mul, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * *b)),
            (BinOp::Mul, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a * *b as f64)),
            (BinOp::Div, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: division by zero".to_string());
                }
                Ok(Value::Float(*a as f64 / *b as f64))
            }
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),
            (BinOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / *b)),
            (BinOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a / *b as f64)),
            (BinOp::FloorDiv, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: integer division by zero".to_string());
                }
                Ok(Value::Int(a.div_euclid(*b)))
            }
            (BinOp::Mod, Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: modulo by zero".to_string());
                }
                Ok(Value::Int(a.rem_euclid(*b)))
            }
            (BinOp::Pow, Value::Int(a), Value::Int(b)) => {
                if *b >= 0 {
                    Ok(Value::Int(a.pow(*b as u32)))
                } else {
                    Ok(Value::Float((*a as f64).powi(*b as i32)))
                }
            }
            (BinOp::Pow, Value::Float(a), Value::Float(b)) => Ok(Value::Float(a.powf(*b))),
            (BinOp::Pow, Value::Int(a), Value::Float(b)) => {
                Ok(Value::Float((*a as f64).powf(*b)))
            }
            (BinOp::Pow, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.powi(*b as i32))),
            // Comparison
            (BinOp::Eq, _, _) => Ok(Value::Bool(self.values_eq(&lv, &rv))),
            (BinOp::NotEq, _, _) => Ok(Value::Bool(!self.values_eq(&lv, &rv))),
            (BinOp::Lt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::Lt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
            (BinOp::Lt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
            (BinOp::Gt, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::Gt, Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
            (BinOp::Gt, Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
            (BinOp::LtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::LtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::GtEq, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(*a >= *b)),
            (BinOp::GtEq, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(*a >= *b)),
            // Bitwise
            (BinOp::BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a & *b)),
            (BinOp::BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a | *b)),
            (BinOp::BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a ^ *b)),
            (BinOp::LShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a << *b)),
            (BinOp::RShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a >> *b)),
            _ => Err(format!(
                "TypeError: unsupported operand types for `{op:?}`: {lv:?} and {rv:?}"
            )),
        }
    }

    fn values_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run(src: &str) -> Result<(), String> {
        let tokens = Lexer::new(src, "").tokenize().into_iter().map(|s| s.token).collect();
        let stmts = Parser::new(tokens).parse_program()?;
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            let _ = interp.exec(stmt)?;
        }
        Ok(())
    }

    fn eval(src: &str) -> Value {
        let tokens = Lexer::new(src, "").tokenize().into_iter().map(|s| s.token).collect();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        interp.eval(match &stmts[0] {
            Stmt::Expr(e) => e,
            _ => panic!("not an expr"),
        }).unwrap()
    }

    fn run_get(src: &str, var: &str) -> Value {
        let tokens = Lexer::new(src, "").tokenize().into_iter().map(|s| s.token).collect();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            let _ = interp.exec(stmt).unwrap();
        }
        interp.get_var(var).unwrap().value.clone()
    }

    #[test]
    fn test_arithmetic() {
        assert!(matches!(eval("2 + 3"), Value::Int(5)));
        assert!(matches!(eval("10 - 4"), Value::Int(6)));
        assert!(matches!(eval("3 * 4"), Value::Int(12)));
        assert!(matches!(eval("7 // 2"), Value::Int(3)));
        assert!(matches!(eval("7 % 3"), Value::Int(1)));
        assert!(matches!(eval("2 ** 10"), Value::Int(1024)));
    }

    #[test]
    fn test_float_arithmetic() {
        if let Value::Float(f) = eval("1.0 + 2.0") {
            assert!((f - 3.0).abs() < f64::EPSILON);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_string_concat() {
        if let Value::Str(s) = eval(r#""hello" + " " + "world""#) {
            assert_eq!(s, "hello world");
        } else {
            panic!();
        }
    }

    #[test]
    fn test_comparison() {
        assert!(matches!(eval("1 < 2"), Value::Bool(true)));
        assert!(matches!(eval("2 > 3"), Value::Bool(false)));
        assert!(matches!(eval("4 == 4"), Value::Bool(true)));
        assert!(matches!(eval("4 != 5"), Value::Bool(true)));
    }

    #[test]
    fn test_logical() {
        assert!(matches!(eval("True and False"), Value::Bool(false)));
        assert!(matches!(eval("True or False"), Value::Bool(true)));
        assert!(matches!(eval("not True"), Value::Bool(false)));
    }

    #[test]
    fn test_let_immutable() {
        assert!(run("let x = 1\nx = 2").is_err());
    }

    #[test]
    fn test_mut_mutable() {
        assert!(run("mut x = 1\nx = 2").is_ok());
    }

    #[test]
    fn test_compound_assign() {
        if let Value::Int(n) = run_get("mut x = 10\nx += 5", "x") {
            assert_eq!(n, 15);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_print_runs() {
        assert!(run(r#"print("hello", "world")"#).is_ok());
    }

    #[test]
    fn test_zero_division() {
        assert!(run("1 // 0").is_err());
    }

    // --- if ---

    #[test]
    fn test_if_true_branch() {
        // x is declared in outer scope, assigned inside if → outer x updated
        if let Value::Int(n) = run_get("mut x = 0\nif True:\n    x = 1\n", "x") {
            assert_eq!(n, 1);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_if_false_else_branch() {
        if let Value::Int(n) = run_get("mut x = 0\nif False:\n    x = 1\nelse:\n    x = 2\n", "x") {
            assert_eq!(n, 2);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_if_scope_isolation() {
        // Variable declared INSIDE if is not visible outside
        assert!(run("if True:\n    let x = 1\nprint(x)\n").is_err());
    }

    // --- while ---

    #[test]
    fn test_while_loop() {
        // i is outer; incremented inside while
        if let Value::Int(n) = run_get("mut i = 0\nwhile i < 5:\n    i += 1\n", "i") {
            assert_eq!(n, 5);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_while_break() {
        if let Value::Int(n) = run_get(
            "mut i = 0\nwhile True:\n    i += 1\n    if i == 3:\n        break\n",
            "i",
        ) {
            assert_eq!(n, 3);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_while_scope_isolation() {
        // Variable declared inside while body is not visible outside
        assert!(run("mut cond = True\nwhile cond:\n    let x = 1\n    cond = False\nprint(x)\n").is_err());
    }

    // --- for ---

    #[test]
    fn test_for_range() {
        // s is outer; accumulated inside for
        if let Value::Int(n) = run_get("mut s = 0\nfor i in range(5):\n    s += i\n", "s") {
            assert_eq!(n, 10);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_for_list() {
        if let Value::Int(n) = run_get("mut s = 0\nfor x in [1, 2, 3]:\n    s += x\n", "s") {
            assert_eq!(n, 6);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_for_loop_var_scope_isolation() {
        // Loop variable is scoped to the loop; not visible outside
        assert!(run("for i in range(3):\n    pass\nprint(i)\n").is_err());
    }

    #[test]
    fn test_for_body_scope_isolation() {
        // Variable declared inside for body is not visible outside
        assert!(run("for i in range(1):\n    let x = 99\nprint(x)\n").is_err());
    }

    // --- block ---

    #[test]
    fn test_block_scope_isolation() {
        // Variable declared inside block is not visible outside
        assert!(run("block:\n    let x = 1\nprint(x)\n").is_err());
    }

    #[test]
    fn test_block_reads_outer() {
        // Block can read outer variables
        assert!(run("let x = 1\nblock:\n    print(x)\n").is_ok());
    }

    #[test]
    fn test_block_modifies_outer() {
        // Assigning to an existing outer mut variable works from inside a block
        if let Value::Int(n) = run_get("mut x = 0\nblock:\n    x = 42\n", "x") {
            assert_eq!(n, 42);
        } else {
            panic!();
        }
    }

    // --- builtins ---

    #[test]
    fn test_range_builtin() {
        if let Value::List(items) = eval("range(3)") {
            assert_eq!(items.len(), 3);
        } else {
            panic!();
        }
    }

    #[test]
    fn test_len_builtin() {
        assert!(matches!(eval("len([1, 2, 3])"), Value::Int(3)));
    }
}
