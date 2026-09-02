// functions/deepcopy.rs — 値のディープコピー: deep_copy_value / deep_copy_unfrozen。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        InstanceData,
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
                let new_dict = Self::dict_copy_with(&d.borrow(), Self::deep_copy_value);
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
            // ⚠⚠ **`Set` と `Tuple` の腕が無く、共有されていた**（bug_fix.md B9）。
            //
            // `Tuple` については「`TupleData` は不変なので共有で問題なし」と書かれていたが、
            // **不変なのは容れ物だけで要素は可変**。実測で
            // `mut t = (xs,); mut u = t; u[0].append(9)` が `t` と `xs` の両方を変えた。
            // `Set` は容れ物そのものが可変なので、共有すると
            // `let s = {1}; mut t = s; t.add(9)` で **`let` の `s` が変わる**（実測）。
            // ⇒ どちらも**要素を再帰コピーして新しい実体**にする。
            Value::Set(items) => Value::Set(Rc::new(RefCell::new(
                items.borrow().iter().cloned().map(Self::deep_copy_value).collect(),
            ))),
            Value::Tuple(t) => {
                let vals: Vec<Value> =
                    t.all_values().iter().cloned().map(Self::deep_copy_value).collect();
                Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vals,
                    t.all_types().to_vec(),
                )))
            }
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
                let new_dict = Self::dict_copy_with(&d.borrow(), Self::deep_copy_unfrozen);
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
            // ⚠ `deep_copy_value` と**同じ理由**で `Set` / `Tuple` も複製する（B9）。
            //   片方だけ直すと「`copy()` では独立するのに `let` 束縛では共有される」
            //   という食い違いになる。
            Value::Set(items) => Value::Set(Rc::new(RefCell::new(
                items.borrow().iter().cloned().map(Self::deep_copy_unfrozen).collect(),
            ))),
            Value::Tuple(t) => {
                let vals: Vec<Value> =
                    t.all_values().iter().cloned().map(Self::deep_copy_unfrozen).collect();
                Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vals,
                    t.all_types().to_vec(),
                )))
            }
            other => other,
        }
    }
}
