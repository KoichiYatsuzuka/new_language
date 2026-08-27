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
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values, index: 0 })))
            }
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                })))
            }
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Tuple(td) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: td.all_values().to_vec(),
                index: 0,
            }))),
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[], None)?,
            Value::PyObject(ref handle) => {
                let items = crate::interpreter::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items,
                    index: 0,
                })))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };
        Ok(generator)
    }
}
