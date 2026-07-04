// value/collections.rs — コレクション値型: SliceValue / TupleData / DictData / DictKey。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::fmt,
    std::path::PathBuf, std::rc::Rc, std::sync::atomic::{AtomicU32, Ordering}, std::sync::Arc,
    indexmap::IndexMap,
    crate::ast::{Accessibility, Param, Stmt},
    crate::interpreter::async_mgr,
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// Value storage types
// ---------------------------------------------------------------------------

/// タプル値の内部ストレージ。
/// 内部表現（並列 Vec）はプライベートであり、公開 API（`get` / `len` / `element_type` など）のみが安定。
/// 内部フィールドは将来自由に変更できる。
///
/// スライス値: `begin:end:step` の内部表現。
/// `tuple[Optional[Index], Optional[Index], Optional[int]]` に相当する。
/// begin/end は `Index` インスタンスまたは `None`、step は `int` または `None`。
#[derive(Debug, Clone)]
pub struct SliceValue {
    pub begin: Option<Value>,
    pub end: Option<Value>,
    pub step: Option<Value>,
}


/// - `values`: 実値の順序付きリスト（実行時は任意の型）
/// - `types`: 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）
#[derive(Debug)]
#[allow(dead_code)]
pub struct TupleData {
    /// 要素値の順序付きリスト（実行時は任意の型）。
    pub values: Vec<Value>,
    /// 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）。
    pub types: Vec<String>,
}


#[allow(dead_code)]
impl TupleData {
    /// 実値リストと型名リストから新しい `TupleData` を構築する。
    ///
    /// - `values`: 要素値のリスト
    /// - `types`: 各要素のランタイム型名のリスト（`values` と同じ長さであること）
    pub fn new(values: Vec<Value>, types: Vec<String>) -> Self {
        Self { values, types }
    }

    /// タプルの要素数を返す。
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// タプルが空（要素数0）なら `true` を返す。
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 指定インデックスの要素値を返す。インデックスが範囲外なら `None`。
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// 指定インデックスの要素のランタイム型名を返す。インデックスが範囲外なら `None`。
    pub fn element_type(&self, index: usize) -> Option<&str> {
        self.types.get(index).map(|s| s.as_str())
    }

    /// すべての要素値をスライスとして返す。
    pub fn all_values(&self) -> &[Value] {
        &self.values
    }

    /// すべての要素型名をスライスとして返す。
    pub fn all_types(&self) -> &[String] {
        &self.types
    }
}


/// 辞書値の内部ストレージ。
/// `IndexMap` で挿入順を保持しつつ O(1) ルックアップを提供する。
/// アクセスには `get` / `set` メソッドを使用すること。
///
/// - `key_type`: 有効なキーの型名。型なし辞書は `"Any"`
/// - `item_type`: 有効な値の型名。型なし辞書は `"Any"`
#[derive(Debug)]
pub struct DictData {
    /// 有効なキーの型名。型なし辞書は `"Any"`。
    pub key_type: String,
    /// 有効な値の型名。型なし辞書は `"Any"`。
    pub item_type: String,
    map: IndexMap<DictKey, Value>,
}


/// `IndexMap` のキーとして使用するラッパー。`Value` のプリミティブ部分のみハッシュ可能。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum DictKey {
    Int(i64),
    Str(String),
    Bool(bool),
    None,
}


impl DictKey {
    /// `Value` を `DictKey` に変換する。ハッシュ不可能な型（リスト・インスタンス等）は `None` を返す。
    fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Int(n) => Some(DictKey::Int(*n)),
            Value::Float(f) => {
                // 整数値の float (e.g. 1.0) は Int キーとして扱う（Python 互換）
                if f.fract() == 0.0 && f.is_finite() {
                    Some(DictKey::Int(*f as i64))
                } else {
                    None
                }
            }
            Value::Str(s) => Some(DictKey::Str(s.clone())),
            Value::Bool(b) => Some(DictKey::Bool(*b)),
            Value::None => Some(DictKey::None),
            _ => None,
        }
    }
}


impl DictData {
    /// 空の型付き辞書を生成する。
    pub fn new(key_type: String, item_type: String) -> Self {
        Self {
            key_type,
            item_type,
            map: IndexMap::new(),
        }
    }

    /// 指定したキーに対応する値を返す。キーが存在しない場合は `None`。
    pub fn get(&self, key: &Value) -> Option<Value> {
        DictKey::from_value(key).and_then(|k| self.map.get(&k).cloned())
    }

    /// キーと値を追加、またはキーが既に存在する場合は値を更新する。
    pub fn set(&mut self, key: Value, value: Value) {
        if let Some(k) = DictKey::from_value(&key) {
            self.map.insert(k, value);
        }
        // unhashable key (e.g. instance) silently ignored — same as before
    }

    /// すべてのキーを `Value` リストとして返す（挿入順）。
    pub fn all_keys(&self) -> Vec<Value> {
        self.map
            .keys()
            .map(|k| match k {
                DictKey::Int(n) => Value::Int(*n),
                DictKey::Str(s) => Value::Str(s.clone()),
                DictKey::Bool(b) => Value::Bool(*b),
                DictKey::None => Value::None,
            })
            .collect()
    }

    /// すべての値をクローンしてリストとして返す（挿入順）。
    pub fn all_items(&self) -> Vec<Value> {
        self.map.values().cloned().collect()
    }

    /// キー・値のペアを挿入順で走査するイテレータ。
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&DictKey, &Value)> {
        self.map.iter()
    }

    /// 指定キーを辞書から削除する。存在しない場合は何もしない。
    // pub(super) fn remove(&mut self, key: &Value) {
    //     if let Some(k) = DictKey::from_value(key) {
    //         self.map.shift_remove(&k);
    //     }
    // }

    /// エントリ数を返す。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 辞書が空なら `true`。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
