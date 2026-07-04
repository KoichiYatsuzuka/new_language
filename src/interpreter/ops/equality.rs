// ops/equality.rs — 値の等価判定: values_eq / values_ref_eq。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::{BinOp, Param, UnaryOp},
    crate::interpreter::str_methods::percent_format,
    crate::interpreter::{Interpreter, Value},
};
#[allow(unused_imports)]
use super::*;

impl Interpreter {
    /// 2つの値が等値かどうかを判定する（`==` / `!=` 演算子および `values_eq` で使用）。
    ///
    /// - プリミティブ型（int/float/str/bool/None）は値で比較。int と float は昇格して比較
    /// - `Instance` と `Class` は参照の同一性（ポインタ比較）で判定
    /// - `Tuple` は要素数と各要素を再帰的に比較
    /// - 異なる型同士（例: int と str）は常に `false`
    ///
    /// - `a`, `b`: 比較する2つの値
    ///
    /// 戻り値: `true` — 等値、`false` — 非等値
    pub(crate) fn values_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::UInt(a), Value::UInt(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Complex(r1, i1), Value::Complex(r2, i2)) => r1 == r2 && i1 == i2,
            // int と float の混在比較: int を float に昇格して比較
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            (Value::Undefined, Value::Undefined) => true,
            // インスタンスの等値判定:
            // enum バリアント (class name が "enum_item_" で始まる) はフィールド値で比較する。
            // それ以外のインスタンスは参照の同一性を先に確認し、
            // 一致しない場合は同じクラスかつ全フィールドが等値であれば真とする。
            (Value::Instance(a), Value::Instance(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let a_borrow = a.borrow();
                let b_borrow = b.borrow();
                if a_borrow.class.name.starts_with("enum_item_")
                    && a_borrow.class.name == b_borrow.class.name
                {
                    let get_value = |inst: &crate::interpreter::InstanceData| {
                        inst.class.field_index.get("value").and_then(|&idx| inst.field_value(idx))
                    };
                    match (get_value(&a_borrow), get_value(&b_borrow)) {
                        (Some(va), Some(vb)) => self.values_eq(&va, &vb),
                        _ => false,
                    }
                } else {
                    // 構造的等値: 同じクラス名かつ全スロットが等値
                    a_borrow.class.name == b_borrow.class.name
                        && a_borrow.field_count() == b_borrow.field_count()
                        && (0..a_borrow.field_count()).all(|i| {
                            match (a_borrow.field_value(i), b_borrow.field_value(i)) {
                                (Some(va), Some(vb)) => self.values_eq(&va, &vb),
                                (None, None) => true,
                                _ => false,
                            }
                        })
                }
            }
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            // タプルは要素数と各要素を再帰的に比較
            (Value::Tuple(a), Value::Tuple(b)) => {
                let av = a.all_values();
                let bv = b.all_values();
                av.len() == bv.len() && av.iter().zip(bv.iter()).all(|(x, y)| self.values_eq(x, y))
            }
            // セットは要素数と各要素の包含関係で比較（順序無関係）
            (Value::Set(a), Value::Set(b)) => {
                let ar = a.borrow();
                let br = b.borrow();
                ar.len() == br.len() && ar.iter().all(|v| br.iter().any(|w| self.values_eq(v, w)))
            }
            _ => false,
        }
    }

    /// `===` 演算子: 参照の同一性のみで等値を判定する。
    ///
    /// - 参照型 (`Instance`, `Class`, `List`, `Dict`, `Set`) は `Rc::ptr_eq` でポインタを比較する。
    /// - 値型 (`Int`, `Float`, `Str` など) は参照の概念がないため `values_eq` と同じ挙動にする。
    pub(crate) fn values_ref_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
            (Value::Dict(a), Value::Dict(b)) => Rc::ptr_eq(a, b),
            (Value::Set(a), Value::Set(b)) => Rc::ptr_eq(a, b),
            _ => self.values_eq(a, b),
        }
    }
}
