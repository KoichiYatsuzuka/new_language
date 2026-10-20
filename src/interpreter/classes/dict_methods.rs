// classes/dict_methods.rs — 辞書（dict）のメソッドのディスパッチ: eval_dict_method。
//
// `eval_method_call_full` の `Value::Dict` アームを切り出し、CPython の dict のメソッドを足した
// （python_builtins_plan.md のタスク 3-6）。`set_methods.rs` と同じ「レシーバ 1 種類 = 1 ファイル」の形。
// キーを引く・入れるのは `dict_get` / `dict_index` / `dict_store_checked`（`__hash__` / `__eq__` を尊重し、型つきの
// 辞書は `d[k] = v` と同じ規則で型を確かめる）。誤りの文言は CPython 3.12 と同じ。
//
// ⚠ 受け手を書き換えるメソッド（`pop` / `popitem` / `setdefault` / `update` / `clear`）は、型検査の
//   `MUTATING_COLLECTION_METHODS` にも載せること（`let` の辞書を書き換えさせない）。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{DictData, GeneratorState, Interpreter, Value},
};

impl Interpreter {
    /// `dict` のメソッドを評価済み引数で呼ぶ。呼び出し元は `eval_method_call_full` のみ。
    pub(crate) fn eval_dict_method(
        &mut self,
        d: Rc<RefCell<DictData>>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            // `d.key()` / `d.keys()` — キーのリストを返す
            "key" | "keys" => {
                Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_keys()))))
            }
            // `d.item()` / `d.values()` — 値のリストを返す
            "item" | "values" => {
                Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_items()))))
            }
            // `d.items()` — `(キー, 値)` タプルのリストを返す。
            //
            // ⚠ 多ターゲット `for`（`for k, v in d.items():`）で分解して使う形が主目的。
            //   タプルは `zip` / `enumerate` と**同じ作り方**にする（型名リストを
            //   `type_name` で埋める）。ここがずれると分解の挙動が揃わない。
            // ⚠ CPython のビューと違い**スナップショット**。走査中に元の辞書を
            //   変更してもこの列は変わらない（CPython は `RuntimeError` になる形）。
            "items" => {
                Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                let pairs = d.borrow().all_pairs();
                let tuples: Vec<Value> = pairs.into_iter().map(|(k, v)| self.pair_tuple(k, v)).collect();
                Ok(Value::List(Rc::new(RefCell::new(tuples))))
            }
            // `d.__iter__()` — キーを挿入順に返すジェネレータ（python_builtins_plan.md のタスク 1-4・
            // CPython の `dict_iter`）。⚠ `for k in d:` と同じく**スナップショット**（CPython は反復中に
            // 大きさが変わると `RuntimeError` になる。list の `__iter__` と同じ差・別に要判断）。
            "__iter__" => {
                Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                let keys = d.borrow().all_keys();
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(keys)))))
            }
            // `d.get(key, default=None)` — 無ければ `default`（タスク 3-6）。
            "get" => {
                let (key, default) = Self::dict_key_default_args(evaled, "get")?;
                Ok(self.dict_get(&d, &key)?.unwrap_or(default.unwrap_or(Value::None)))
            }
            // `d.pop(key[, default])` — 取り除いてその値を返す。無ければ `default`、それも無ければ `KeyError`。
            "pop" => {
                let (key, default) = Self::dict_key_default_args(evaled, "pop")?;
                match self.dict_index(&d, &key)? {
                    Some(idx) => Ok(d.borrow_mut().remove_at(idx).map(|(_, v)| v).unwrap_or(Value::None)),
                    None => default.ok_or_else(|| format!("KeyError: {}", self.display_repr(&key))),
                }
            }
            // `d.setdefault(key, default=None)` — あればその値、無ければ `default` を入れて返す。
            // ⚠ 入れた後で**読み直した値**を返す（入れるときに値を写すので、`default` そのものを返すと
            //   `groups.setdefault(k, []).append(w)` が辞書の中のリストに届かない）。
            "setdefault" => {
                let (key, default) = Self::dict_key_default_args(evaled, "setdefault")?;
                if let Some(v) = self.dict_get(&d, &key)? {
                    return Ok(v);
                }
                self.dict_store_checked(&d, key.clone(), default.unwrap_or(Value::None))?;
                Ok(self.dict_get(&d, &key)?.unwrap_or(Value::None))
            }
            // `d.update(other=(), **kw)` — 辞書か 2 要素の組の列から、続けてキーワード引数から入れる（`dict(..)` と同じ）。
            "update" => {
                self.dict_update_from(&d, evaled, "update")?;
                Ok(Value::None)
            }
            // `d.popitem()` — 最後に入れた `(キー, 値)` を取り除いて返す（LIFO・CPython と同じ）。
            "popitem" => {
                Self::dict_no_args(&evaled, "popitem")?;
                let last = d.borrow_mut().pop_last();
                match last {
                    Some((k, v)) => Ok(self.pair_tuple(k, v)),
                    None => Err("KeyError: 'popitem(): dictionary is empty'".to_string()),
                }
            }
            "clear" => {
                Self::dict_no_args(&evaled, "clear")?;
                d.borrow_mut().clear();
                Ok(Value::None)
            }
            // `d.copy()` — 別の辞書。⚠ CPython は浅い写しだが、Arrow は値を深く写す（`mut e = d` と同じ・
            //   `let` の辞書の中のリストを写しの側から書き換えさせない）。
            "copy" => {
                Self::dict_no_args(&evaled, "copy")?;
                Ok(Self::deep_copy_unfrozen(Value::Dict(d)))
            }
            _ => Err(format!("AttributeError: 'dict' object has no method '{method_name}'")),
        }
    }

    /// `(キー, 値)` の組（`items()` / `popitem()`）。`zip` / `enumerate` と同じ作り方（型名リストを `type_name` で埋める）。
    fn pair_tuple(&self, k: Value, v: Value) -> Value {
        let types = vec![self.type_name(&k).to_string(), self.type_name(&v).to_string()];
        Value::Tuple(Rc::new(crate::interpreter::TupleData::new(vec![k, v], types)))
    }

    /// `get` / `pop` / `setdefault` の引数（キーと、あれば既定値）。位置引数だけ（CPython と同じ文言）。
    fn dict_key_default_args(
        evaled: Vec<(Option<String>, Value, bool)>,
        method: &str,
    ) -> Result<(Value, Option<Value>), String> {
        if evaled.iter().any(|(k, _, _)| k.is_some()) {
            return Err(format!("TypeError: dict.{method}() takes no keyword arguments"));
        }
        let n = evaled.len();
        let mut it = evaled.into_iter().map(|(_, v, _)| v);
        match n {
            0 => Err(format!("TypeError: {method} expected at least 1 argument, got 0")),
            1 | 2 => Ok((it.next().expect("n >= 1"), it.next())),
            _ => Err(format!("TypeError: {method} expected at most 2 arguments, got {n}")),
        }
    }

    /// 引数を取らない dict のメソッド（CPython と同じ文言）。
    fn dict_no_args(evaled: &[(Option<String>, Value, bool)], method: &str) -> Result<(), String> {
        match evaled.len() {
            0 => Ok(()),
            n => Err(format!("TypeError: dict.{method}() takes no arguments ({n} given)")),
        }
    }
}
