// exec/control_flow.rs — VM が使う反復ヘルパのみ（#33 で制御フロー実装は削除）。
//
// ⚠ 以前はここに `if`/`match`/`while`/`for`/`block:` 文のツリーウォーク実装があったが、
// 制御フローは**すべてバイトコード VM が実行する**ようになったので削除した（#33）。
// 残しているのは `Op::GetIter` が呼ぶ反復子生成だけ。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{GeneratorState, Interpreter, Value},
};

impl Interpreter {
    /// イテラブルな値を `for` 反復用のイテレータ（多くは `Value::Generator`）へ変換する。
    /// 消費者は VM の `Op::GetIter` **だけ**（ツリーウォークの `exec_for_stmt` は #33 で削除）。
    pub(crate) fn make_for_iterator(&mut self, iter_val: Value) -> Result<Value, String> {
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(items.borrow().clone())))),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(values))))
            }
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(chars))))
            }
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(items.borrow().clone())))),
            Value::Tuple(td) => Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(td.all_values().to_vec())))),
            // ★ 辞書の反復は**キー**を返す（Python と同じ既定）。
            //   ⚠ `values()` / `items()` は明示のメソッド。ここは `for k in d:` の形。
            Value::Dict(ref d) => {
                let keys = d.borrow().all_keys();
                Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(keys))))
            }
            // `Code` は 1 行ずつ（設計書 §1.2 / タスク 2-10）。各要素は 1 行だけの `Code`。
            // ⚠ 列への展開（`drain_iterable`）もこの関数を通るので規則は 1 か所。
            Value::Code(ref lines) => Value::Generator(Rc::new(RefCell::new(
                GeneratorState::materialized(crate::meta_expand::code_lines_as_values(lines)),
            ))),
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[], None)?,
            Value::PyObject(ref handle) => {
                let items = crate::interpreter::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState::materialized(items))))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };
        Ok(generator)
    }

    /// イテラブルを**最後まで回して**要素を集める（列への展開の**唯一の**入口・タスク 1-7 / 3-3）。
    ///
    /// `for` と同じ規則（[`Self::make_for_iterator`]）でイテレータにしてから `gen_next` で尽きるまで進める。
    /// 使い手は `list(it)` / `tuple(it)` / `set(it)` / `zip` / `enumerate` / `*` の展開（呼び出し引数・列の表示）/
    /// スライス代入など。
    /// ⚠ 以前の `collect_iterable`（削除済み）はジェネレータの実体化済みの値しか見なかったので、遅延のジェネレータ
    ///   （`gen` 関数）を渡すと**黙って空**になった（`zip(g(), xs)` が `[]`・実測）。進めもしなかった。
    pub(crate) fn drain_iterable(&mut self, val: Value) -> Result<Vec<Value>, String> {
        let type_name = self.type_name(&val).to_string();
        let it = self
            .make_for_iterator(val)
            .map_err(|_| format!("TypeError: '{type_name}' object is not iterable"))?;
        let Value::Generator(state) = it else {
            return Err(format!("TypeError: iter() returned non-iterator of type '{}'", self.type_name(&it)));
        };
        let mut out = Vec::new();
        while let Some(v) = self.gen_next(&state)? {
            out.push(v);
        }
        Ok(out)
    }
}
