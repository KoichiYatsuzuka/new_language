use std::collections::HashMap;

use crate::ast::{BinOp, Expr, Stmt, UnaryOp};

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
}

struct Var {
    value: Value,
    mutable: bool,
}

pub struct Interpreter {
    env: HashMap<String, Var>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self { env: HashMap::new() }
    }

    pub fn exec(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval(expr)?;
                Ok(())
            }
            Stmt::Let(name, expr) => {
                let value = self.eval(expr)?;
                self.env.insert(name.clone(), Var { value, mutable: false });
                Ok(())
            }
            Stmt::Const(name, expr) => {
                let value = self.eval(expr)?;
                self.env.insert(name.clone(), Var { value, mutable: false });
                Ok(())
            }
            Stmt::Mut(name, expr) => {
                let value = self.eval(expr)?;
                self.env.insert(name.clone(), Var { value, mutable: true });
                Ok(())
            }
            Stmt::Assign(name, expr) => {
                let value = self.eval(expr)?;
                match self.env.get(name) {
                    Some(v) if !v.mutable => {
                        Err(format!("TypeError: cannot assign to immutable variable '{name}'"))
                    }
                    None => Err(format!("NameError: '{name}' is not defined")),
                    _ => {
                        self.env.insert(name.clone(), Var { value, mutable: true });
                        Ok(())
                    }
                }
            }
            Stmt::CompoundAssign(name, op, expr) => {
                let rhs = self.eval(expr)?;
                let lhs = match self.env.get(name) {
                    Some(v) if !v.mutable => {
                        return Err(format!(
                            "TypeError: cannot assign to immutable variable '{name}'"
                        ));
                    }
                    Some(v) => v.value.clone(),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                };
                let value = self.apply_binop(op, lhs, rhs)?;
                self.env.insert(name.clone(), Var { value, mutable: true });
                Ok(())
            }
        }
    }

    pub fn eval(&mut self, expr: &Expr) -> Result<Value, String> {
        match expr {
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Float(f) => Ok(Value::Float(*f)),
            Expr::Str(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::None => Ok(Value::None),
            Expr::Ident(name) => self
                .env
                .get(name)
                .map(|v| v.value.clone())
                .ok_or_else(|| format!("NameError: '{name}' is not defined")),
            Expr::UnaryOp { op, operand } => {
                let val = self.eval(operand)?;
                self.apply_unary(op, val)
            }
            Expr::BinOp { op, left, right } => {
                // Short-circuit evaluation for `and` / `or`
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
        }
    }

    fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
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
        let tokens = Lexer::new(src).tokenize();
        let stmts = Parser::new(tokens).parse_program()?;
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            interp.exec(stmt)?;
        }
        Ok(())
    }

    fn eval(src: &str) -> Value {
        let tokens = Lexer::new(src).tokenize();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        interp.eval(match &stmts[0] {
            Stmt::Expr(e) => e,
            _ => panic!("not an expr"),
        }).unwrap()
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
        let tokens = Lexer::new("mut x = 10\nx += 5").tokenize();
        let stmts = Parser::new(tokens).parse_program().unwrap();
        let mut interp = Interpreter::new();
        for stmt in &stmts {
            interp.exec(stmt).unwrap();
        }
        assert!(matches!(interp.env["x"].value, Value::Int(15)));
    }

    #[test]
    fn test_print_runs() {
        assert!(run(r#"print("hello", "world")"#).is_ok());
    }

    #[test]
    fn test_zero_division() {
        assert!(run("1 // 0").is_err());
    }
}
