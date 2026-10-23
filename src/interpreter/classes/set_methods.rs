// classes/set_methods.rs — 集合(set / frozenset)メソッドのディスパッチ: eval_set_method / eval_frozenset_method。
//
// #63 で `eval_method_call_full` の `Value::Set` アーム（126 行・最大アーム）を切り出したもの。
// `string_methods.rs` / `object_methods.rs` と同じ「レシーバ 1 種類 = 1 ファイル」の形。
// frozenset（python_builtins_plan.md のタスク 3-1）は書き換えないメソッドを set と共有する（`eval_set_query`）。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{GeneratorState, Interpreter, Value},
};

/// set と frozenset に共通の、受け手を書き換えないメソッド（`eval_set_query` が受ける）。
/// ⚠ `type_check::names::BUILTIN_VALUE_METHODS` の `frozenset` の行と同じ並び（片方だけ足さないこと）。
const SET_QUERY_METHODS: &[&str] = &[
    "__iter__", "copy", "union", "intersection", "difference", "symmetric_difference",
    "issubset", "issuperset", "isdisjoint",
];

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
        if SET_QUERY_METHODS.contains(&method_name) {
            let items = s.borrow().clone();
            return self.eval_set_query(items, false, method_name, evaled);
        }
        match method_name {
            "add" => {
                let item = Self::one_arg_evaled(evaled, "set", "add")?;
                // ⚠ `__eq__` を尊重する（B2-b）。`x in s` と同じ規則でないと、
                //    「`in` では既にあると言われるのに `add` すると重複する」ことになる。
                // ⚠⚠ `borrow_mut()` を握ったまま `__eq__` を呼んではいけない。
                //    ハンドラが同じ set に触れた瞬間に RefCell の二重借用でパニックする。
                //    ⇒ 先に複製して探し、書き込みのときだけ借りる（discard / remove も同形）。
                let items = s.borrow().clone();
                if !self.contains_eq_dyn(&items, &item)? {
                    // ⚠ 格納する値は複製する（L4。list の `append` と同じ理由）。
                    // ⚠ 複製は借用の外で（自分自身を渡されたときのパニック回避）。
                    let copied = self.copy_for_store(item);
                    s.borrow_mut().push(copied);
                }
                Ok(Value::None)
            }
            "discard" => {
                let item = Self::one_arg_evaled(evaled, "set", "discard")?;
                // ⚠ `__eq__` を尊重するため、借用を跨がないよう先に複製して探す。
                let items = s.borrow().clone();
                if let Some(pos) = self.position_eq_dyn(&items, &item)? {
                    s.borrow_mut().remove(pos);
                }
                Ok(Value::None)
            }
            "remove" => {
                let item = Self::one_arg_evaled(evaled, "set", "remove")?;
                let items = s.borrow().clone();
                if let Some(pos) = self.position_eq_dyn(&items, &item)? {
                    s.borrow_mut().remove(pos);
                    Ok(Value::None)
                } else {
                    Err(format!("KeyError: {}", self.display_repr(&item)))
                }
            }
            "pop" => {
                Self::expect_no_args_evaled(&evaled, "set", "pop")?;
                let mut s_mut = s.borrow_mut();
                if s_mut.is_empty() {
                    Err("KeyError: 'pop from an empty set'".to_string())
                } else {
                    Ok(s_mut.remove(0))
                }
            }
            "clear" => {
                Self::expect_no_args_evaled(&evaled, "set", "clear")?;
                s.borrow_mut().clear();
                Ok(Value::None)
            }
            _ => Err(format!(
                "AttributeError: 'set' object has no method '{method_name}'"
            )),
        }
    }

    /// `frozenset` のメソッド（python_builtins_plan.md のタスク 3-1）。書き換えないメソッドだけを持つ。
    /// ⚠ `add` などは CPython と同じ `AttributeError: 'frozenset' object has no attribute 'add'`。
    pub(crate) fn eval_frozenset_method(
        &mut self,
        s: Rc<Vec<Value>>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        if method_name == "copy" {
            // 不変なので自分自身を返す（CPython も同じオブジェクトを返す）。
            Self::expect_no_args_evaled(&evaled, "frozenset", "copy")?;
            return Ok(Value::FrozenSet(s));
        }
        if SET_QUERY_METHODS.contains(&method_name) {
            return self.eval_set_query(s.to_vec(), true, method_name, evaled);
        }
        Err(format!("AttributeError: 'frozenset' object has no attribute '{method_name}'"))
    }

    /// set / frozenset に共通の、受け手を書き換えないメソッド。結果の集合は受け手と同じ種類（`frozen`）。
    ///
    /// - `union` / `intersection` / `difference` は引数をいくつでも取り、どの引数も**任意のイテラブル**
    ///   （CPython と同じ。以前は set か list を 1 つだけ受けた）
    /// - `symmetric_difference` / `issubset` / `issuperset` / `isdisjoint` は引数を 1 つ
    fn eval_set_query(
        &mut self,
        items: Vec<Value>,
        frozen: bool,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        let kind = if frozen { "frozenset" } else { "set" };
        let make = |v: Vec<Value>| {
            if frozen {
                Value::FrozenSet(Rc::new(v))
            } else {
                Value::Set(Rc::new(RefCell::new(v)))
            }
        };
        match method_name {
            "__iter__" => {
                Self::expect_no_args_evaled(&evaled, kind, "__iter__")?;
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(items)))))
            }
            "copy" => {
                Self::expect_no_args_evaled(&evaled, kind, "copy")?;
                Ok(make(items))
            }
            "union" | "intersection" | "difference" => {
                let mut result = items;
                for (name, other, _) in evaled {
                    if let Some(n) = name {
                        return Err(format!("TypeError: {kind}.{method_name}() takes no keyword arguments ('{n}' given)"));
                    }
                    let other_items = self.set_other_items(other)?;
                    result = match method_name {
                        "union" => {
                            for v in other_items {
                                if !self.contains_eq(&result, &v)? {
                                    result.push(v);
                                }
                            }
                            result
                        }
                        "intersection" => self.filter_by_membership(&result, &other_items, true)?,
                        _ => self.filter_by_membership(&result, &other_items, false)?,
                    };
                }
                Ok(make(result))
            }
            "symmetric_difference" => {
                let other = Self::one_arg_evaled(evaled, kind, method_name)?;
                let other_items = self.set_other_items(other)?;
                let mut result = self.filter_by_membership(&items, &other_items, false)?;
                for v in &other_items {
                    if !self.contains_eq(&items, v)? && !self.contains_eq(&result, v)? {
                        result.push(v.clone());
                    }
                }
                Ok(make(result))
            }
            "issubset" | "issuperset" | "isdisjoint" => {
                let other = Self::one_arg_evaled(evaled, kind, method_name)?;
                let other_items = self.set_other_items(other)?;
                Ok(Value::Bool(match method_name {
                    "issubset" => self.all_contained(&items, &other_items)?,
                    "issuperset" => self.all_contained(&other_items, &items)?,
                    // 共通の要素が 1 つも無い。
                    _ => self.filter_by_membership(&items, &other_items, true)?.is_empty(),
                }))
            }
            _ => unreachable!("SET_QUERY_METHODS の名前だけが来る"),
        }
    }

    /// set 演算の引数（任意のイテラブル）を `Vec<Value>` に変換する。
    ///
    /// ⚠ #63 で `method_call.rs` から**ここへ移した**（消費者は set 演算だけ）。
    /// ⚠ set / frozenset / list は中身をそのまま、それ以外は `for` と同じ規則で回す（タスク 3-1。
    ///   以前は set か list だけを受け、ほかは `argument must be a set or list` だった）。
    fn set_other_items(&mut self, other: Value) -> Result<Vec<Value>, String> {
        match other {
            Value::Set(o) => Ok(o.borrow().clone()),
            Value::FrozenSet(o) => Ok(o.to_vec()),
            Value::List(l) => Ok(l.borrow().clone()),
            other => self.drain_iterable(other),
        }
    }
}
