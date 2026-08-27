// classes/set_methods.rs — 集合(set)メソッドのディスパッチ: eval_set_method。
//
// #63 で `eval_method_call_full` の `Value::Set` アーム（126 行・最大アーム）を切り出したもの。
// `string_methods.rs` / `object_methods.rs` と同じ「レシーバ 1 種類 = 1 ファイル」の形。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{GeneratorState, Interpreter, Value},
};

impl Interpreter {
    /// `set` のメソッドを評価済み引数で呼ぶ（#63 で切り出し）。
    /// 呼び出し元は `eval_method_call_full` のみ。
    ///
    /// ⚠ 要素の同値判定は**すべて `values_eq`**（`Value` は `Hash`/`Eq` を持たないので
    /// 集合演算は線形走査）。`add` / `union` の重複排除もこれに依存している。
    pub(crate) fn eval_set_method(
        &mut self,
        s: Rc<RefCell<Vec<Value>>>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "__iter__" => {
                Self::expect_no_args_evaled(&evaled, "set", "__iter__")?;
                let items = s.borrow().clone();
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items,
                    index: 0,
                }))))
            }
            "add" => {
                let item = Self::one_arg_evaled(evaled, "set", "add")?;
                let mut s_mut = s.borrow_mut();
                if !s_mut.iter().any(|v| self.values_eq(v, &item)) {
                    s_mut.push(item);
                }
                Ok(Value::None)
            }
            "discard" => {
                let item = Self::one_arg_evaled(evaled, "set", "discard")?;
                let mut s_mut = s.borrow_mut();
                if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, &item)) {
                    s_mut.remove(pos);
                }
                Ok(Value::None)
            }
            "remove" => {
                let item = Self::one_arg_evaled(evaled, "set", "remove")?;
                let mut s_mut = s.borrow_mut();
                if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, &item)) {
                    s_mut.remove(pos);
                    Ok(Value::None)
                } else {
                    Err(format!("KeyError: {} is not in set", self.display(&item)))
                }
            }
            "pop" => {
                Self::expect_no_args_evaled(&evaled, "set", "pop")?;
                let mut s_mut = s.borrow_mut();
                if s_mut.is_empty() {
                    Err("KeyError: pop from an empty set".to_string())
                } else {
                    Ok(s_mut.remove(0))
                }
            }
            "clear" => {
                Self::expect_no_args_evaled(&evaled, "set", "clear")?;
                s.borrow_mut().clear();
                Ok(Value::None)
            }
            "copy" => {
                Self::expect_no_args_evaled(&evaled, "set", "copy")?;
                Ok(Value::Set(Rc::new(RefCell::new(s.borrow().clone()))))
            }
            "union" => {
                let other = Self::one_arg_evaled(evaled, "set", "union")?;
                let other_items = self.set_other_items(&other, "union")?;
                let mut result = s.borrow().clone();
                for v in other_items {
                    if !result.iter().any(|x| self.values_eq(x, &v)) {
                        result.push(v);
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            "intersection" => {
                let other = Self::one_arg_evaled(evaled, "set", "intersection")?;
                let other_items = self.set_other_items(&other, "intersection")?;
                let result: Vec<Value> = s
                    .borrow()
                    .iter()
                    .filter(|v| other_items.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            "difference" => {
                let other = Self::one_arg_evaled(evaled, "set", "difference")?;
                let other_items = self.set_other_items(&other, "difference")?;
                let result: Vec<Value> = s
                    .borrow()
                    .iter()
                    .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            "symmetric_difference" => {
                let other = Self::one_arg_evaled(evaled, "set", "symmetric_difference")?;
                let other_items = self.set_other_items(&other, "symmetric_difference")?;
                let s_ref = s.borrow();
                let mut result: Vec<Value> = s_ref
                    .iter()
                    .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                    .cloned()
                    .collect();
                for v in &other_items {
                    if !s_ref.iter().any(|x| self.values_eq(x, v)) {
                        result.push(v.clone());
                    }
                }
                Ok(Value::Set(Rc::new(RefCell::new(result))))
            }
            "issubset" => {
                let other = Self::one_arg_evaled(evaled, "set", "issubset")?;
                let other_items = self.set_other_items(&other, "issubset")?;
                let result = s
                    .borrow()
                    .iter()
                    .all(|v| other_items.iter().any(|x| self.values_eq(x, v)));
                Ok(Value::Bool(result))
            }
            "issuperset" => {
                let other = Self::one_arg_evaled(evaled, "set", "issuperset")?;
                let other_items = self.set_other_items(&other, "issuperset")?;
                let s_ref = s.borrow();
                let result = other_items
                    .iter()
                    .all(|v| s_ref.iter().any(|x| self.values_eq(x, v)));
                Ok(Value::Bool(result))
            }
            _ => Err(format!(
                "AttributeError: 'set' object has no method '{method_name}'"
            )),
        }
    }

    /// set 演算の引数（`set` または `list`）を `Vec<Value>` に変換する。
    ///
    /// ⚠ #63 で `method_call.rs` から**ここへ移した**（消費者は set 演算 6 種だけ）。
    fn set_other_items(&self, other: &Value, method: &str) -> Result<Vec<Value>, String> {
        match other {
            Value::Set(o) => Ok(o.borrow().clone()),
            Value::List(l) => Ok(l.borrow().clone()),
            _ => Err(format!(
                "TypeError: set.{method}() argument must be a set or list, not '{}'",
                self.type_name(other)
            )),
        }
    }
}
