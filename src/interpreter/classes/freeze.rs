// classes/freeze.rs — インスタンスの freeze とフラットレイアウト化: try_flat_freeze、build_flat_layout_from_instance、write_flat_instance、freeze_instance、apply_freeze_to_value。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        ClassValue, InstanceData,
        Interpreter, Value,
    },
};

impl Interpreter {
    /// インスタンスを不変化する: `immutable = true` にセットし、すべての `mut` フィールドを不変にする。
    ///
    /// `let` バインドされたインスタンスに適用される。以降は `mut self` メソッド呼び出しが禁止される。
    ///
    /// - `inst_rc`: 不変化するインスタンスへの共有参照
    /// 同一クラス・全フィールドが SWD 型（int/float または別の SWD クラス）の
    /// `Value::List` を平坦バイト配列に変換する。
    /// 変換できない場合は `None` を返す。フィールドはアルファベット順で格納される。
    pub(crate) fn try_flat_freeze(items: &[Value]) -> Option<Value> {
        if items.is_empty() {
            return None;
        }
        let first_rc = match &items[0] {
            Value::Instance(rc) => rc.clone(),
            _ => return None,
        };
        let class = first_rc.borrow().class.clone();
        let class_name = class.name.clone();

        let layout = {
            let inst = first_rc.borrow();
            Self::build_flat_layout_from_instance(&inst, class.clone())?
        };

        let mut data: Vec<u8> = Vec::with_capacity(items.len() * layout.stride);
        for item in items {
            let inst_rc = match item {
                Value::Instance(rc) => rc.clone(),
                _ => return None,
            };
            let inst = inst_rc.borrow();
            if inst.class.name != class_name { return None; }
            Self::write_flat_instance(&inst, &layout.fields, &mut data)?;
        }

        let len = items.len();
        Some(Value::FrozenList {
            state: Rc::new(RefCell::new(crate::interpreter::value::FlatListData {
                data,
                len,
                allocated_size: len,
            })),
            layout: Rc::new(layout),
        })
    }

    /// インスタンスの fields から `FlatLayout` を再帰的に構築する。
    pub(crate) fn build_flat_layout_from_instance(
        inst: &InstanceData,
        class: Rc<ClassValue>,
    ) -> Option<crate::interpreter::value::FlatLayout> {
        // Build (slot_idx, name, ty) triples from the field_index map + field slots
        let mut flds_idx: Vec<(usize, String, crate::interpreter::value::FlatFieldTy)> = inst.class.field_index
            .iter()
            .filter(|(k, _)| !k.contains("::"))  // skip qualified aliases
            .filter_map(|(name, &idx)| {
                let val = inst.field_value(idx)?;
                let fty = Self::val_to_flat_field_ty(&val)?;
                Some((idx, name.clone(), fty))
            })
            .collect::<Vec<_>>();
        // Return None if any field is not flat-convertible (already filtered via filter_map, so check emptiness)
        if inst.class.field_count > 0 && flds_idx.len() != inst.class.field_index.iter().filter(|(k, _)| !k.contains("::")).count() {
            return None;
        }
        if flds_idx.is_empty() { return None; }
        // 宣言順 = スロットインデックス順（C ABI 準拠 — .claude/skills/c-abi-interop/SKILL.md P0c）
        flds_idx.sort_by_key(|(idx, _, _)| *idx);
        let flds: Vec<(String, crate::interpreter::value::FlatFieldTy)> =
            flds_idx.into_iter().map(|(_, n, t)| (n, t)).collect();
        let stride: usize = flds.iter().map(|(_, ft)| ft.stride()).sum();
        Some(crate::interpreter::value::FlatLayout {
            class_name: class.name.clone(),
            fields: flds,
            stride,
            class,
        })
    }

    /// 単一の `Value` から `FlatFieldTy` を導出する（再帰的）。
    pub(crate) fn val_to_flat_field_ty(val: &Value) -> Option<crate::interpreter::value::FlatFieldTy> {
        match val {
            Value::Int(_)   => Some(crate::interpreter::value::FlatFieldTy::Int),
            Value::Float(_) => Some(crate::interpreter::value::FlatFieldTy::Float),
            Value::Instance(rc) => {
                let inst = rc.borrow();
                let sub = Self::build_flat_layout_from_instance(&inst, inst.class.clone())?;
                Some(crate::interpreter::value::FlatFieldTy::Struct(Rc::new(sub)))
            }
            _ => None,
        }
    }

    /// インスタンスのフィールド値を `layout_fields` の順序に従って `data` に書き出す（再帰的）。
    pub(crate) fn write_flat_instance(
        inst: &InstanceData,
        layout_fields: &[(String, crate::interpreter::value::FlatFieldTy)],
        data: &mut Vec<u8>,
    ) -> Option<()> {
        for (field_name, field_ty) in layout_fields {
            let &idx = inst.class.field_index.get(field_name.as_str())?;
            let val = inst.field_value(idx)?;
            match (field_ty, &val) {
                (crate::interpreter::value::FlatFieldTy::Int, Value::Int(n)) => {
                    data.extend_from_slice(&n.to_le_bytes());
                }
                (crate::interpreter::value::FlatFieldTy::Float, Value::Float(f)) => {
                    data.extend_from_slice(&f.to_le_bytes());
                }
                (crate::interpreter::value::FlatFieldTy::Struct(sub_layout), Value::Instance(rc)) => {
                    let sub = rc.borrow();
                    if sub.class.name != sub_layout.class_name { return None; }
                    Self::write_flat_instance(&sub, &sub_layout.fields, data)?;
                }
                _ => return None,
            }
        }
        Some(())
    }

    pub(crate) fn freeze_instance(inst_rc: &Rc<RefCell<InstanceData>>) {
        let mut inst = inst_rc.borrow_mut();
        inst.flags_or(crate::interpreter::value::INST_IMMUTABLE);
        // すべてのフィールドを不変に変更する（raw クラスは INST_IMMUTABLE フラグのみで足りる）
        for (_, mutable) in inst.boxed_fields.iter_mut().flatten() {
            *mutable = false;
        }
    }

    /// フリーズプロトコル。
    ///
    /// `freeze_fields=true` のとき: `__freeze__` フックを呼び出し、インスタンスのフィールドを不変化する。
    ///   `freeze` 文でコレクションを再帰的にフリーズする際に使用する。
    ///
    /// `freeze_fields=false` のとき: `__freeze__` フックのみ呼び出し、フィールドは不変化しない。
    ///   `let` バインドではインスタンスの Rc 参照は共有されるため、フィールドを凍結すると
    ///   他のすべての参照にも影響してしまう。`let` バインドは変数の再バインドを禁止するのみで、
    ///   オブジェクトのフィールドの可変性には影響しない。
    /// `let` に束縛する値を作る: **深いコピーを不変にしたもの**（python_builtins_plan.md のタスク 1-9）。
    ///
    /// `let x = <mut 変数>` / `let x = <式>`（ツリーウォーク・最上位・関数の中の slot）と `freeze x` の
    /// **唯一の実装**。`freeze x` は `let x = x` と同じ結果になる（同じ値・同じ実行時の印）。
    /// ⚠ 以前は 4 通りあった: `freeze` は**コピーせずその場で**不変にし（同じオブジェクトを持つ相手まで
    ///   不変になる）、`let` はコピーしたがコレクションの中のインスタンスは不変にせず、関数の中の
    ///   `let x = <式>` はインスタンスのときしかコピーしなかった（`let item = xs[0]` が `xs[0]` と共有）。
    pub(crate) fn frozen_copy(&mut self, value: Value) -> Result<Value, String> {
        let copied = Self::deep_copy_value(value);
        self.freeze_value_in_place(&copied)?;
        Ok(copied)
    }

    /// 値を**その場で**不変にする（[`Self::frozen_copy`] の中身）。
    ///
    /// - インスタンス: `__freeze__` を呼び、フィールドをすべて不変にする
    /// - list / set / dict の値 / tuple: 中のインスタンスに同じことをする（1 段）
    /// - fixed_list: 使っていない確保分を切り詰める（`allocated_size()` が `len` になる）
    fn freeze_value_in_place(&mut self, val: &Value) -> Result<(), String> {
        match val {
            Value::Instance(_) => self.apply_freeze_to_value(val, true),
            Value::List(rc) => {
                let items = rc.borrow().clone();
                items.iter().try_for_each(|item| self.apply_freeze_to_value(item, true))
            }
            Value::Set(rc) => {
                let items = rc.borrow().clone();
                items.iter().try_for_each(|item| self.apply_freeze_to_value(item, true))
            }
            Value::Dict(rc) => {
                let vals = rc.borrow().all_items();
                vals.iter().try_for_each(|v| self.apply_freeze_to_value(v, true))
            }
            Value::Tuple(td) => td.all_values().iter().try_for_each(|item| self.apply_freeze_to_value(item, true)),
            Value::FrozenList { state, layout } => {
                let mut st = state.borrow_mut();
                let exact = st.len * layout.stride;
                st.data.truncate(exact);
                st.data.shrink_to_fit();
                st.allocated_size = st.len;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn apply_freeze_to_value(&mut self, val: &Value, freeze_fields: bool) -> Result<(), String> {
        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__", None)?;
                } else {
                    self.dispatch_overload(overloads, &[], Some(val.clone()), None)?;
                }
            }
            if freeze_fields {
                Self::freeze_instance(inst_rc);
            }
        }
        Ok(())
    }

    // --- クラスのインスタンス化 ---

}
