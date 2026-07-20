// functions/deepcopy.rs — 値のディープコピー: deep_copy_value / deep_copy_unfrozen。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        DictData, InstanceData,
        Interpreter, Value,
    },
};

impl Interpreter {
    /// 参照型の値を再帰的にディープコピーして返す。
    ///
    /// `let` パラメータへのバインド時に呼ばれ、元の可変変数（`mut`）が
    /// 関数内部から変更されることを防ぐ。
    ///
    /// 変換規則:
    /// - `Instance`: フィールドを再帰コピーして新しい `InstanceData` を生成する
    /// - `Dict`: キー・値を再帰コピーして新しい `DictData` を生成する
    /// - `List`: 各要素を再帰コピーする
    /// - その他: プリミティブ・不変型はそのまま返す（Rust の clone でコピー済み）
    pub(crate) fn deep_copy_value(val: Value) -> Value {
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let new_boxed = inst
                    .boxed_fields
                    .iter()
                    .map(|slot| slot.as_ref().map(|(v, m)| (Self::deep_copy_value(v.clone()), *m)))
                    .collect();
                // raw ブロックは POD なので clone = memcpy（flags・raw フィールドすべて保持）
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    raw: inst.raw.clone(),
                    class: inst.class.clone(),
                    boxed_fields: new_boxed,
                })))
            }
            Value::Dict(d) => {
                let d_ref = d.borrow();
                let mut new_dict = DictData::new(d_ref.key_type.clone(), d_ref.item_type.clone());
                for (k, v) in d_ref.all_keys().into_iter().zip(d_ref.all_items()) {
                    new_dict.set(Self::deep_copy_value(k), Self::deep_copy_value(v));
                }
                Value::Dict(Rc::new(RefCell::new(new_dict)))
            }
            Value::List(items) => Value::List(Rc::new(RefCell::new(
                items
                    .borrow()
                    .iter()
                    .cloned()
                    .map(Self::deep_copy_value)
                    .collect(),
            ))),
            // Tuple は Rc<TupleData> だが TupleData は不変なので共有で問題なし
            // プリミティブ・関数・クラス等はそのまま返す
            other => other,
        }
    }

    /// `copy()` メソッド用のディープコピー。フリーズ状態をリセットして新鮮な可変インスタンスを返す。
    ///
    /// `deep_copy_value` との違い:
    /// - `Instance`: `immutable = false` に設定し、フィールドの可変性をクラス定義から復元する
    ///   （`let` バインドによるフリーズを解除した独立したコピーを生成する）
    /// - `Dict` / `List`: `deep_copy_value` と同様に再帰コピーする
    /// - その他: `deep_copy_value` と同様にそのまま返す
    pub(crate) fn deep_copy_unfrozen(val: Value) -> Value {
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let class = inst.class.clone();
                let new_boxed: Vec<Option<(Value, bool)>> = inst
                    .boxed_fields
                    .iter()
                    .enumerate()
                    .map(|(idx, slot)| {
                        slot.as_ref().map(|(v, _)| {
                            // クラス定義の可変性を復元する: field_mutability_vec の元の値を使う
                            let orig_mutable = class.field_mutability_vec
                                .get(idx)
                                .copied()
                                .unwrap_or(true);
                            (Self::deep_copy_unfrozen(v.clone()), orig_mutable)
                        })
                    })
                    .collect();
                // フリーズを解除した新鮮なコピー: INST_IMMUTABLE を除いた既存フラグを継承。
                // raw クラスの可変性は field_mutability_vec + フラグで表現されるため
                // ブロックの memcpy + フラグ操作だけで復元される。
                let mut new_inst = InstanceData {
                    raw: inst.raw.clone(),
                    class,
                    boxed_fields: new_boxed,
                };
                let unfrozen = new_inst.flags() & !crate::interpreter::value::INST_IMMUTABLE;
                new_inst.set_flags(unfrozen);
                Value::Instance(Rc::new(RefCell::new(new_inst)))
            }
            Value::Dict(d) => {
                let d_ref = d.borrow();
                let mut new_dict = DictData::new(d_ref.key_type.clone(), d_ref.item_type.clone());
                for (k, v) in d_ref.all_keys().into_iter().zip(d_ref.all_items()) {
                    new_dict.set(Self::deep_copy_unfrozen(k), Self::deep_copy_unfrozen(v));
                }
                Value::Dict(Rc::new(RefCell::new(new_dict)))
            }
            Value::List(items) => Value::List(Rc::new(RefCell::new(
                items
                    .borrow()
                    .iter()
                    .cloned()
                    .map(Self::deep_copy_unfrozen)
                    .collect(),
            ))),
            other => other,
        }
    }
}
