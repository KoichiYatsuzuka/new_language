// ops.rs — 演算・比較・真偽値・表示 (is_truthy / type_name / display / display_repr / apply_unary / apply_binop / values_eq)

use std::rc::Rc;

use crate::ast::{BinOp, UnaryOp};

use super::{Interpreter, Value};

impl Interpreter {
    pub(super) fn is_truthy(&self, val: &Value) -> bool {
        match val {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::None => false,
            Value::List(items) => !items.is_empty(),
            Value::Function(_) | Value::OverloadedFn(_) | Value::Class(_) | Value::Instance(_) | Value::Type(_)
            | Value::Trait(_)
            | Value::TemplateFn(_) | Value::TemplateClass(_)
            | Value::GeneratorFn(_) | Value::TemplateGenFn(_) | Value::Generator(_) => true,
        }
    }

    pub(super) fn type_name(&self, val: &Value) -> &'static str {
        match val {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Bool(_) => "bool",
            Value::None => "NoneType",
            Value::List(_) => "list",
            Value::Function(_) | Value::OverloadedFn(_) => "function",
            Value::Class(_) | Value::Type(_) => "type",
            Value::Trait(_) => "trait",
            Value::Instance(_) => "object",
            Value::TemplateFn(_) | Value::TemplateClass(_) => "template",
            Value::GeneratorFn(_) | Value::TemplateGenFn(_) => "gen_function",
            Value::Generator(_) => "generator",
        }
    }

    pub(super) fn display(&self, val: &Value) -> String {
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
            Value::Function(_) => "<function>".to_string(),
            Value::OverloadedFn(fns) => format!("<function ({} overloads)>", fns.len()),
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => format!("<{} object>", i.borrow().class.name),
            Value::Type(name) => format!("<type '{name}'>"),
            Value::Trait(name) => format!("<trait '{name}'>"),
            Value::TemplateFn(t) => format!("<template fn ({} type params)>", t.template_params.len()),
            Value::TemplateClass(t) => format!("<template class '{}'>", t.name),
            Value::GeneratorFn(_) => "<generator function>".to_string(),
            Value::TemplateGenFn(t) => format!("<template gen ({} type params)>", t.template_params.len()),
            Value::Generator(s) => {
                let s = s.borrow();
                format!("<generator {}/{}>", s.index, s.values.len())
            }
        }
    }

    pub(super) fn display_repr(&self, val: &Value) -> String {
        match val {
            Value::Str(s) => format!("'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| self.display_repr(v)).collect();
                format!("[{}]", parts.join(", "))
            }
            _ => self.display(val),
        }
    }

    pub(super) fn apply_unary(&self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(format!("TypeError: bad operand type for unary `-`: {}", self.type_name(&val))),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                _ => Err(format!("TypeError: bad operand type for unary `~`: {}", self.type_name(&val))),
            },
        }
    }

    pub(super) fn apply_binop(&self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
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
                if *b == 0 { return Err("ZeroDivisionError: division by zero".to_string()); }
                Ok(Value::Float(*a as f64 / *b as f64))
            }
            (BinOp::Div, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),
            (BinOp::Div, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / *b)),
            (BinOp::Div, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a / *b as f64)),
            (BinOp::FloorDiv, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: integer division by zero".to_string()); }
                Ok(Value::Int(a.div_euclid(*b)))
            }
            (BinOp::Mod, Value::Int(a), Value::Int(b)) => {
                if *b == 0 { return Err("ZeroDivisionError: modulo by zero".to_string()); }
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
            (BinOp::Pow, Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).powf(*b))),
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
                "TypeError: unsupported operand types for `{op:?}`: {} and {}",
                self.type_name(&lv), self.type_name(&rv)
            )),
        }
    }

    pub(super) fn values_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}
