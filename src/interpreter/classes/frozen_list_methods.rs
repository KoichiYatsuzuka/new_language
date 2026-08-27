// classes/frozen_list_methods.rs — `fixed_list`（フラットレイアウト配列）メソッドのディスパッチ。
//
// #63 で `eval_method_call_full` の `Value::FrozenList` アーム（65 行）を切り出したもの。
//
// ⚠ `fixed_list` は**インスタンスを raw バイト列に平坦化して持つ**（`FlatLayout`）ので、
// 要素の取り出しは毎回 `layout.reconstruct_item` による再構築になる。
// 設計は c-abi-interop（[.claude/skills/c-abi-interop](../../../.claude/skills/c-abi-interop/SKILL.md)）と同系。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{FlatLayout, FlatListData, GeneratorState, Interpreter, Value},
};

impl Interpreter {
    /// `fixed_list` のメソッドを評価済み引数で呼ぶ（#63 で切り出し）。
    /// 呼び出し元は `eval_method_call_full` のみ。
    pub(crate) fn eval_frozen_list_method(
        &mut self,
        state: Rc<RefCell<FlatListData>>,
        layout: Rc<FlatLayout>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "__iter__" => {
                Self::expect_no_args_evaled(&evaled, "fixed_list", "__iter__")?;
                let st = state.borrow();
                let values = (0..st.len)
                    .map(|i| layout.reconstruct_item(&st.data, i))
                    .collect();
                Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values,
                    index: 0,
                }))))
            }
            "__contains__" => {
                let needle = Self::one_arg_evaled(evaled, "fixed_list", "__contains__")?;
                let st = state.borrow();
                let found = (0..st.len)
                    .map(|i| layout.reconstruct_item(&st.data, i))
                    .any(|v| self.values_eq(&v, &needle));
                Ok(Value::Bool(found))
            }
            "allocated_size" => {
                Self::expect_no_args_evaled(&evaled, "fixed_list", "allocated_size")?;
                Ok(Value::Int(state.borrow().allocated_size as i64))
            }
            "append" => {
                let item = Self::one_arg_evaled(evaled, "fixed_list", "append")?;
                match item {
                    Value::Instance(inst_rc) => {
                        let inst = inst_rc.borrow();
                        if inst.class.name != layout.class_name {
                            return Err(format!(
                                "TypeError: fixed_list.append(): expected instance of '{}', got '{}'",
                                layout.class_name, inst.class.name
                            ));
                        }
                        let mut st = state.borrow_mut();
                        // Grow capacity when full (double, minimum 1)
                        if st.len >= st.allocated_size {
                            let new_cap = (st.allocated_size * 2).max(1);
                            st.data.resize(new_cap * layout.stride, 0);
                            st.allocated_size = new_cap;
                        }
                        // Write each field recursively (alphabetical order)
                        let base_offset = st.len * layout.stride;
                        let mut tmp = Vec::with_capacity(layout.stride);
                        Self::write_flat_instance(&inst, &layout.fields, &mut tmp).ok_or_else(
                            || {
                                format!(
                                "TypeError: fixed_list.append(): field type mismatch for class '{}'",
                                layout.class_name
                            )
                            },
                        )?;
                        st.data[base_offset..base_offset + layout.stride].copy_from_slice(&tmp);
                        st.len += 1;
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: fixed_list.append(): expected class instance, got '{}'",
                        self.type_name(&other)
                    )),
                }
            }
            _ => Err(format!(
                "AttributeError: 'fixed_list' object has no method '{method_name}'"
            )),
        }
    }
}
