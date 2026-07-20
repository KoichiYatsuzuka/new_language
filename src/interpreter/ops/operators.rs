// ops/operators.rs — 単項・二項演算子の適用と真偽値/文字列評価: apply_unary(_dyn) / apply_binop(_dyn) / eval_truthy / display_str。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::{BinOp, UnaryOp},
    crate::interpreter::str_methods::percent_format,
    crate::interpreter::{Interpreter, Value},
};

impl Interpreter {
    /// 単項演算子を適用した結果の値を返す。
    ///
    /// - `op`: 適用する単項演算子（`Neg`=`-`, `Not`=`not`, `BitNot`=`~`）
    /// - `val`: オペランドの値
    ///
    /// 戻り値: `Ok(Value)` — 演算結果。`Err(message)` — 型エラー（例: `~str`）
    pub(crate) fn apply_unary(&self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        match op {
            UnaryOp::Neg => match val {
                Value::Int(n) => Ok(Value::Int(-n)),
                Value::Float(f) => Ok(Value::Float(-f)),
                Value::Complex(re, im) => Ok(Value::Complex(-re, -im)),
                _ => Err(format!(
                    "TypeError: bad operand type for unary `-`: {}",
                    self.type_name(&val)
                )),
            },
            UnaryOp::Not => Ok(Value::Bool(!self.is_truthy(&val))),
            UnaryOp::BitNot => match val {
                Value::Int(n) => Ok(Value::Int(!n)),
                Value::UInt(n) => Ok(Value::UInt(!n)),
                _ => Err(format!(
                    "TypeError: bad operand type for unary `~`: {}",
                    self.type_name(&val)
                )),
            },
        }
    }

    /// `Value::Instance` に対してダンダーメソッドを経由して単項演算子を適用する。
    /// `__neg__`（`-`）/ `__invert__`（`~`）が定義されていれば呼び出し、なければ `apply_unary` へフォールバック。
    /// `not` 演算子は `__bool__` を持つインスタンスに対応するため `eval_truthy` 経由で処理する。
    pub(crate) fn apply_unary_dyn(&mut self, op: &UnaryOp, val: Value) -> Result<Value, String> {
        if let Value::Instance(ref inst_rc) = val {
            let method_name = match op {
                UnaryOp::Neg => Some("__neg__"),
                UnaryOp::BitNot => Some("__invert__"),
                UnaryOp::Not => None,
            };
            if let Some(m) = method_name {
                if inst_rc.borrow().class.methods.contains_key(m) {
                    return self.eval_method_call_evaled(val, m, vec![]);
                }
            }
            if let UnaryOp::Not = op {
                let b = self.eval_truthy(&val)?;
                return Ok(Value::Bool(!b));
            }
        }
        self.apply_unary(op, val)
    }

    /// `Value::Instance` に対してダンダーメソッドを経由して二項演算子を適用する。
    /// 対応するダンダーメソッドが定義されていれば呼び出し、なければ `apply_binop` へフォールバック。
    pub(crate) fn apply_binop_dyn(&mut self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
        if let Value::Instance(ref inst_rc) = lv {
            let method_name = match op {
                BinOp::Add => Some("__add__"),
                BinOp::Sub => Some("__sub__"),
                BinOp::Mul => Some("__mul__"),
                BinOp::Div => Some("__truediv__"),
                BinOp::FloorDiv => Some("__floordiv__"),
                BinOp::Mod => Some("__mod__"),
                BinOp::Pow => Some("__pow__"),
                BinOp::BitAnd => Some("__and__"),
                BinOp::BitOr => Some("__or__"),
                BinOp::BitXor => Some("__xor__"),
                BinOp::LShift => Some("__lshift__"),
                BinOp::RShift => Some("__rshift__"),
                BinOp::Eq => Some("__eq__"),
                BinOp::NotEq => Some("__ne__"),
                BinOp::Lt => Some("__lt__"),
                BinOp::Gt => Some("__gt__"),
                BinOp::LtEq => Some("__le__"),
                BinOp::GtEq => Some("__ge__"),
                _ => None,
            };
            if let Some(m) = method_name {
                if inst_rc.borrow().class.methods.contains_key(m) {
                    return self.eval_method_call_evaled(lv, m, vec![(None, rv, true)]);
                }
            }
        }
        self.apply_binop(op, lv, rv)
    }

    /// `__bool__` を持つ `Value::Instance` に対してそのメソッドを呼び出し真偽値を返す。
    /// 定義されていなければ `is_truthy` へフォールバック。
    pub(crate) fn eval_truthy(&mut self, val: &Value) -> Result<bool, String> {
        if let Value::Instance(inst_rc) = val {
            if inst_rc.borrow().class.methods.contains_key("__bool__") {
                let result = self.eval_method_call_evaled(val.clone(), "__bool__", vec![])?;
                return match result {
                    Value::Bool(b) => Ok(b),
                    other => Err(format!(
                        "TypeError: __bool__ should return bool, not '{}'",
                        self.type_name(&other)
                    )),
                };
            }
        }
        Ok(self.is_truthy(val))
    }

    /// `__str__` を持つ `Value::Instance` に対してそのメソッドを呼び出し文字列表現を返す。
    /// 定義されていなければ `display` へフォールバック。
    pub(crate) fn display_str(&mut self, val: &Value) -> Result<String, String> {
        if let Value::Instance(inst_rc) = val {
            if inst_rc.borrow().class.methods.contains_key("__str__") {
                let result = self.eval_method_call_evaled(val.clone(), "__str__", vec![])?;
                return match result {
                    Value::Str(s) => Ok(s),
                    other => Ok(self.display(&other)),
                };
            }
        }
        Ok(self.display(val))
    }

    /// 二項演算子を適用した結果の値を返す。
    ///
    /// サポートする演算カテゴリ:
    /// - 算術: `+`, `-`, `*`, `/`, `//`, `%`, `**`（int/float 混在時は昇格）
    /// - 文字列連結: `+`（str + str）
    /// - 比較: `==`, `!=`, `<`, `>`, `<=`, `>=`
    /// - ビット演算: `&`, `|`, `^`, `<<`, `>>`（int のみ）
    ///
    /// - `op`: 適用する二項演算子
    /// - `lv`: 左オペランドの値（評価済み）
    /// - `rv`: 右オペランドの値（評価済み）
    ///
    /// 戻り値: `Ok(Value)` — 演算結果。`Err(message)` — 型エラーまたはゼロ除算エラー
    pub(crate) fn apply_binop(&self, op: &BinOp, lv: Value, rv: Value) -> Result<Value, String> {
        // いずれかのオペランドが PyObject の場合は Python に委譲する
        if let Value::PyObject(h) = &lv {
            return crate::interpreter::py_interop::py_binop(h, op, &rv);
        }
        if let Value::PyObject(h) = &rv {
            return crate::interpreter::py_interop::py_rbinop(h, op, &lv);
        }
        match (op, &lv, &rv) {
            // セット演算（算術演算より先にチェック）
            (BinOp::BitOr, Value::Set(a), Value::Set(b)) => {
                let mut result = a.borrow().clone();
                for v in b.borrow().iter() {
                    if !result.iter().any(|x| self.values_eq(x, v)) {
                        result.push(v.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::BitAnd, Value::Set(a), Value::Set(b)) => {
                let b_ref = b.borrow();
                let result: Vec<Value> = a
                    .borrow()
                    .iter()
                    .filter(|v| b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::Sub, Value::Set(a), Value::Set(b)) => {
                let b_ref = b.borrow();
                let result: Vec<Value> = a
                    .borrow()
                    .iter()
                    .filter(|v| !b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            (BinOp::BitXor, Value::Set(a), Value::Set(b)) => {
                let a_ref = a.borrow();
                let b_ref = b.borrow();
                let mut result: Vec<Value> = a_ref
                    .iter()
                    .filter(|v| !b_ref.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                for v in b_ref.iter() {
                    if !a_ref.iter().any(|x| self.values_eq(x, v)) {
                        result.push(v.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            // 包含検査 `in` / `not in`
            (BinOp::In, item, Value::List(lst)) => Ok(Value::Bool(
                lst.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, item, Value::FrozenList { state, layout }) => {
                let st = state.borrow();
                Ok(Value::Bool(
                    (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).any(|v| self.values_eq(&v, item)),
                ))
            }
            (BinOp::In, item, Value::Set(s)) => Ok(Value::Bool(
                s.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, Value::Str(sub), Value::Str(s)) => {
                Ok(Value::Bool(s.contains(sub.as_str())))
            }
            (BinOp::In, item, Value::Dict(d)) => Ok(Value::Bool(d.borrow().get(item).is_some())),
            (BinOp::In, item, Value::Tuple(t)) => Ok(Value::Bool(
                t.all_values().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, item, Value::List(lst)) => Ok(Value::Bool(
                !lst.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, item, Value::FrozenList { state, layout }) => {
                let st = state.borrow();
                Ok(Value::Bool(
                    !(0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).any(|v| self.values_eq(&v, item)),
                ))
            }
            (BinOp::NotIn, item, Value::Set(s)) => Ok(Value::Bool(
                !s.borrow().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::NotIn, Value::Str(sub), Value::Str(s)) => {
                Ok(Value::Bool(!s.contains(sub.as_str())))
            }
            (BinOp::NotIn, item, Value::Dict(d)) => Ok(Value::Bool(d.borrow().get(item).is_none())),
            (BinOp::NotIn, item, Value::Tuple(t)) => Ok(Value::Bool(
                !t.all_values().iter().any(|v| self.values_eq(v, item)),
            )),
            (BinOp::In, _, rv) => Err(format!(
                "TypeError: argument of type '{}' is not iterable",
                self.type_name(rv)
            )),
            (BinOp::NotIn, _, rv) => Err(format!(
                "TypeError: argument of type '{}' is not iterable",
                self.type_name(rv)
            )),
            // 算術演算
            (BinOp::Add, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a + *b)),
            (BinOp::Add, Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),
            (BinOp::Add, Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + *b)),
            (BinOp::Add, Value::Float(a), Value::Int(b)) => Ok(Value::Float(*a + *b as f64)),
            (BinOp::Add, Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
            // str * int / int * str → repeat
            (BinOp::Mul, Value::Str(s), Value::Int(n)) => {
                Ok(Value::Str(s.repeat((*n).max(0) as usize)))
            }
            (BinOp::Mul, Value::Int(n), Value::Str(s)) => {
                Ok(Value::Str(s.repeat((*n).max(0) as usize)))
            }
            // str % args → printf-style format
            (BinOp::Mod, Value::Str(fmt), rv) => {
                let display_fn = |v: &Value| self.display(v);
                let args: Vec<Value> = match rv {
                    Value::Tuple(t) => t.all_values().to_vec(),
                    other => vec![other.clone()],
                };
                let result = percent_format(fmt, &args, &display_fn)?;
                Ok(Value::Str(result))
            }
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
            (BinOp::Pow, Value::Int(a), Value::Float(b)) => Ok(Value::Float((*a as f64).powf(*b))),
            (BinOp::Pow, Value::Float(a), Value::Int(b)) => Ok(Value::Float(a.powi(*b as i32))),
            // 比較演算
            (BinOp::Eq, _, _) => Ok(Value::Bool(self.values_eq(&lv, &rv))),
            (BinOp::RefEq, _, _) => Ok(Value::Bool(self.values_ref_eq(&lv, &rv))),
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
            // uint 算術・比較
            (BinOp::Add, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_add(*b))),
            (BinOp::Sub, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_sub(*b))),
            (BinOp::Mul, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_mul(*b))),
            (BinOp::Div, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: division by zero".to_string());
                }
                Ok(Value::UInt(*a / *b))
            }
            (BinOp::FloorDiv, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: integer division by zero".to_string());
                }
                Ok(Value::UInt(*a / *b))
            }
            (BinOp::Mod, Value::UInt(a), Value::UInt(b)) => {
                if *b == 0 {
                    return Err("ZeroDivisionError: modulo by zero".to_string());
                }
                Ok(Value::UInt(*a % *b))
            }
            (BinOp::Lt, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a < *b)),
            (BinOp::LtEq, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a <= *b)),
            (BinOp::Gt, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a > *b)),
            (BinOp::GtEq, Value::UInt(a), Value::UInt(b)) => Ok(Value::Bool(*a >= *b)),
            (BinOp::BitAnd, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a & *b)),
            (BinOp::BitOr, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a | *b)),
            (BinOp::BitXor, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a ^ *b)),
            (BinOp::LShift, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a << *b)),
            (BinOp::RShift, Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(*a >> *b)),
            // ビット演算（int のみ対応）
            (BinOp::BitAnd, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a & *b)),
            (BinOp::BitOr, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a | *b)),
            (BinOp::BitXor, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a ^ *b)),
            (BinOp::LShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a << *b)),
            (BinOp::RShift, Value::Int(a), Value::Int(b)) => Ok(Value::Int(*a >> *b)),
            // 複素数算術（complex との加減乗除）
            (BinOp::Add, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 + r2, i1 + i2))
            }
            (BinOp::Sub, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 - r2, i1 - i2))
            }
            (BinOp::Mul, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                Ok(Value::Complex(r1 * r2 - i1 * i2, r1 * i2 + i1 * r2))
            }
            (BinOp::Div, Value::Complex(r1, i1), Value::Complex(r2, i2)) => {
                let denom = r2 * r2 + i2 * i2;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(
                    (r1 * r2 + i1 * i2) / denom,
                    (i1 * r2 - r1 * i2) / denom,
                ))
            }
            // complex と scalar の混合
            (BinOp::Add, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re + s, *im))
            }
            (BinOp::Add, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s + re, *im))
            }
            (BinOp::Add, Value::Complex(re, im), Value::Int(n)) => {
                Ok(Value::Complex(re + *n as f64, *im))
            }
            (BinOp::Add, Value::Int(n), Value::Complex(re, im)) => {
                Ok(Value::Complex(*n as f64 + re, *im))
            }
            (BinOp::Sub, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re - s, *im))
            }
            (BinOp::Sub, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s - re, -im))
            }
            (BinOp::Sub, Value::Complex(re, im), Value::Int(n)) => {
                Ok(Value::Complex(re - *n as f64, *im))
            }
            (BinOp::Sub, Value::Int(n), Value::Complex(re, im)) => {
                Ok(Value::Complex(*n as f64 - re, -im))
            }
            (BinOp::Mul, Value::Complex(re, im), Value::Float(s)) => {
                Ok(Value::Complex(re * s, im * s))
            }
            (BinOp::Mul, Value::Float(s), Value::Complex(re, im)) => {
                Ok(Value::Complex(s * re, s * im))
            }
            (BinOp::Mul, Value::Complex(re, im), Value::Int(n)) => {
                let ns = *n as f64;
                Ok(Value::Complex(re * ns, im * ns))
            }
            (BinOp::Mul, Value::Int(n), Value::Complex(re, im)) => {
                let ns = *n as f64;
                Ok(Value::Complex(ns * re, ns * im))
            }
            (BinOp::Div, Value::Complex(re, im), Value::Float(s)) => {
                if *s == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(re / s, im / s))
            }
            (BinOp::Div, Value::Float(s), Value::Complex(re, im)) => {
                let denom = re * re + im * im;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(s * re / denom, -s * im / denom))
            }
            (BinOp::Div, Value::Complex(re, im), Value::Int(n)) => {
                let ns = *n as f64;
                if ns == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(re / ns, im / ns))
            }
            (BinOp::Div, Value::Int(n), Value::Complex(re, im)) => {
                let ns = *n as f64;
                let denom = re * re + im * im;
                if denom == 0.0 {
                    return Err("ZeroDivisionError: complex division by zero".to_string());
                }
                Ok(Value::Complex(ns * re / denom, -ns * im / denom))
            }
            _ => Err(format!(
                "TypeError: unsupported operand types for `{op:?}`: {} and {}",
                self.type_name(&lv),
                self.type_name(&rv)
            )),
        }
    }

}
