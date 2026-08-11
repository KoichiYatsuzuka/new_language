// eval/subscript.rs — 添字アクセス・代入: value_matches_type、subscript 取得(スライス含む)、setitem、反復対象の収集。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        Interpreter, SliceValue, TupleData, Value,
    },
};
use super::*;

impl Interpreter {
    /// 値が宣言された型名と互換性があるかを確認する。
    ///
    /// 特別ルール:
    /// - `"Any"` はすべての型を受け入れる
    /// - `"float"` には `int` 値も受け入れる（アップキャスト）
    /// - それ以外のユーザー定義型はクラス名で比較する
    ///
    /// - `val`: チェック対象の値
    /// - `type_name`: 宣言された型名
    ///
    /// 戻り値: `true` — 互換あり、`false` — 型不一致
    pub(crate) fn value_matches_type(val: &Value, type_name: &str) -> bool {
        match type_name {
            "Any" => true,
            "int" => matches!(val, Value::Int(_)),
            "float" => matches!(val, Value::Float(_) | Value::Int(_)),
            "str" => matches!(val, Value::Str(_)),
            "bool" => matches!(val, Value::Bool(_)),
            "None" => matches!(val, Value::None),
            "list" => matches!(val, Value::List(_)),
            "fixed_list" => matches!(val, Value::FrozenList { .. }),
            "list_like" => matches!(val, Value::List(_) | Value::FrozenList { .. }),
            "dict" => matches!(val, Value::Dict(_)),
            "set" => matches!(val, Value::Set(_)),
            "tuple" => matches!(val, Value::Tuple(_)),
            _ if type_name.starts_with("list[") => matches!(val, Value::List(_)),
            _ if type_name.starts_with("fixed_list[") => matches!(val, Value::FrozenList { .. }),
            _ if type_name.starts_with("list_like[") => matches!(val, Value::List(_) | Value::FrozenList { .. }),
            _ if type_name.starts_with("dict[") => matches!(val, Value::Dict(_)),
            _ if type_name.starts_with("set[") => matches!(val, Value::Set(_)),
            _ if type_name.starts_with("tuple[") => matches!(val, Value::Tuple(_)),
            _ => {
                if let Value::Instance(inst) = val {
                    inst.borrow().class.name == type_name
                } else {
                    false
                }
            }
        }
    }

    /// `obj[key]` の評価。リスト・文字列・タプル・辞書・PyObject・インスタンスに対応する。
    /// `key` が `Value::Slice` の場合はスライス処理を行い、新たなリスト/文字列/タプルを返す。
    pub(crate) fn eval_subscript(&mut self, obj: Value, key: Value) -> Result<Value, String> {
        if let Value::Slice(s) = &key {
            return self.eval_subscript_slice(obj, Rc::clone(s));
        }
        match obj {
            Value::List(items) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let borrowed = items.borrow();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: list index {} out of range", idx));
                }
                Ok(borrowed[actual as usize].clone())
            }
            Value::FrozenList { ref state, ref layout } => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let st = state.borrow();
                let n = st.len as i64;
                let actual = if idx < 0 { n + idx } else { idx };
                if actual < 0 || actual >= n {
                    return Err(format!("IndexError: list index {} out of range", idx));
                }
                Ok(layout.reconstruct_item(&st.data, actual as usize))
            }
            Value::Str(s) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: string indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let chars: Vec<char> = s.chars().collect();
                let len = chars.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: string index {} out of range", idx));
                }
                Ok(Value::str(chars[actual as usize].to_string()))
            }
            Value::Tuple(td) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: tuple indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let vals = td.all_values();
                let len = vals.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!("IndexError: tuple index {} out of range", idx));
                }
                Ok(vals[actual as usize].clone())
            }
            Value::Dict(d) => d
                .borrow()
                .get(&key)
                .ok_or_else(|| format!("KeyError: {}", self.display(&key))),
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__getitem__", vec![(None, key, true)])
            }
            Value::PyObject(ref handle) => crate::interpreter::py_interop::py_getitem(handle, &key),
            _ => Err(format!(
                "TypeError: '{}' object is not subscriptable",
                self.type_name(&obj)
            )),
        }
    }

    /// スライス添字 `obj[begin:end:step]` を評価する。
    /// リスト → 新しいリスト、文字列 → 新しい文字列、タプル → 新しいタプルを返す。
    pub(crate) fn eval_subscript_slice(&mut self, obj: Value, s: Rc<SliceValue>) -> Result<Value, String> {
        let step = match &s.step {
            None => 1i64,
            Some(Value::Int(n)) => *n,
            _ => return Err("TypeError: slice step must be int".to_string()),
        };
        if step == 0 {
            return Err("ValueError: slice step cannot be zero".to_string());
        }
        let begin = index_val_to_i64(&s.begin);
        let end = index_val_to_i64(&s.end);

        match obj {
            Value::List(items) => {
                let borrowed = items.borrow();
                let len = borrowed.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                Ok(Value::List(Rc::new(RefCell::new(
                    indices.into_iter().map(|i| borrowed[i].clone()).collect(),
                ))))
            }
            Value::Str(s_val) => {
                let chars: Vec<char> = s_val.chars().collect();
                let len = chars.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                Ok(Value::str(indices.into_iter().map(|i| chars[i]).collect::<String>()))
            }
            Value::Tuple(td) => {
                let vals = td.all_values();
                let types: Vec<String> =
                    vals.iter().map(|v| self.type_name(v).to_string()).collect();
                let len = vals.len() as i64;
                let indices = compute_slice_indices(len, begin, end, step);
                let new_vals: Vec<Value> = indices.iter().map(|&i| vals[i].clone()).collect();
                let new_types: Vec<String> = indices.iter().map(|&i| types[i].clone()).collect();
                Ok(Value::Tuple(Rc::new(TupleData::new(new_vals, new_types))))
            }
            // カスタムクラス: __getitem__ にスライスオブジェクトを渡して委譲する
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__getitem__", vec![(None, Value::Slice(s), true)])
            }
            _ => Err(format!(
                "TypeError: '{}' object does not support slicing",
                self.type_name(&obj)
            )),
        }
    }

    /// `obj[slice] = rhs` を実行する。
    /// - `Value::List`: Python 互換のスライス代入（step=1 は長さ変更可、step≠1 は同数必須）
    /// - `Value::Instance`: `__setitem__(slice, rhs)` に委譲する
    pub(crate) fn eval_setitem_slice(
        &mut self,
        obj: Value,
        s: Rc<SliceValue>,
        rhs: Value,
    ) -> Result<(), String> {
        let step = match &s.step {
            None => 1i64,
            Some(Value::Int(n)) => *n,
            _ => return Err("TypeError: slice step must be int".to_string()),
        };
        if step == 0 {
            return Err("ValueError: slice step cannot be zero".to_string());
        }
        let begin = index_val_to_i64(&s.begin);
        let end = index_val_to_i64(&s.end);

        match obj {
            Value::List(items) => {
                let new_vals = self.collect_iterable(rhs)?;
                let mut borrowed = items.borrow_mut();
                let len = borrowed.len() as i64;

                if step == 1 {
                    // step=1: Python 互換。置換先の長さと代入元の長さが違っても構わない。
                    let start = normalize_slice_bound_start(begin, len);
                    let stop = normalize_slice_bound_stop(end, len);
                    // start > stop のときは空スライスへの挿入（Python の動作と一致）
                    let stop = stop.max(start);
                    borrowed.splice(start..stop, new_vals);
                } else {
                    // step≠1: 拡張スライス。代入元の要素数がスライスの要素数と一致しなければならない。
                    let indices = compute_slice_indices(len, begin, end, step);
                    if new_vals.len() != indices.len() {
                        return Err(format!(
                            "ValueError: attempt to assign sequence of size {} to extended slice of size {}",
                            new_vals.len(), indices.len()
                        ));
                    }
                    for (new_val, &idx) in new_vals.into_iter().zip(indices.iter()) {
                        borrowed[idx] = new_val;
                    }
                }
                Ok(())
            }
            // カスタムクラス: __setitem__ にスライスオブジェクトと値を渡して委譲する
            Value::Instance(_) => {
                self.eval_method_call_evaled(
                    obj,
                    "__setitem__",
                    vec![(None, Value::Slice(s), true), (None, rhs, true)],
                )?;
                Ok(())
            }
            _ => Err(format!(
                "TypeError: '{}' object does not support slice assignment",
                self.type_name(&obj)
            )),
        }
    }

    /// 任意の反復可能値を `Vec<Value>` に収集する（スライス代入、enumerate、zip で使用）。
    pub(crate) fn collect_iterable(&self, val: Value) -> Result<Vec<Value>, String> {
        match val {
            Value::List(lst) => Ok(lst.borrow().clone()),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                Ok((0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect())
            }
            Value::Tuple(td) => Ok(td.all_values().to_vec()),
            Value::Str(s) => Ok(s.chars().map(|c| Value::str(c.to_string())).collect()),
            Value::Set(items) => Ok(items.borrow().clone()),
            Value::Generator(gen) => {
                let g = gen.borrow();
                Ok(g.values[g.index..].to_vec())
            }
            other => Err(format!(
                "TypeError: '{}' object is not iterable",
                self.type_name(&other)
            )),
        }
    }

    /// `obj[key] = rhs` の実行。リスト・辞書・PyObject・インスタンスに対応する。
    /// `key` が `Value::Slice` の場合はスライス代入 `eval_setitem_slice` に委譲する。
    pub(crate) fn eval_setitem(
        &mut self,
        obj: Value,
        key: Value,
        rhs: Value,
    ) -> Result<(), String> {
        if let Value::Slice(s) = key {
            return self.eval_setitem_slice(obj, s, rhs);
        }
        match obj {
            Value::List(items) => {
                let idx = value_as_index(&key).ok_or_else(|| {
                    format!(
                        "TypeError: list indices must be integers or Index, not '{}'",
                        self.type_name(&key)
                    )
                })?;
                let mut borrowed = items.borrow_mut();
                let len = borrowed.len() as i64;
                let actual = if idx < 0 { len + idx } else { idx };
                if actual < 0 || actual >= len {
                    return Err(format!(
                        "IndexError: list assignment index {} out of range",
                        idx
                    ));
                }
                borrowed[actual as usize] = rhs;
                Ok(())
            }
            Value::Dict(d) => {
                let (key_type, item_type) = {
                    let b = d.borrow();
                    (b.key_type.clone(), b.item_type.clone())
                };
                if key_type != "Any" && !Self::value_matches_type(&key, &key_type) {
                    return Err(format!(
                        "TypeError: dict key type mismatch: expected '{}', got '{}'",
                        key_type,
                        self.type_name(&key)
                    ));
                }
                if item_type != "Any" && !Self::value_matches_type(&rhs, &item_type) {
                    return Err(format!(
                        "TypeError: dict item type mismatch: expected '{}', got '{}'",
                        item_type,
                        self.type_name(&rhs)
                    ));
                }
                d.borrow_mut().set(key, rhs);
                Ok(())
            }
            Value::Instance(_) => {
                self.eval_method_call_evaled(obj, "__setitem__", vec![(None, key, true), (None, rhs, true)])?;
                Ok(())
            }
            Value::PyObject(ref handle) => crate::interpreter::py_interop::py_setitem(handle, &key, &rhs),
            _ => Err(format!(
                "TypeError: '{}' object does not support item assignment",
                self.type_name(&obj)
            )),
        }
    }
}
