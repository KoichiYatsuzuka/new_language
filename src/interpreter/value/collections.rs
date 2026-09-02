// value/collections.rs — コレクション値型: SliceValue / TupleData / DictData / DictKey。

use indexmap::IndexMap;
use std::rc::Rc;
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
    /// `Value::Str` と同じ `Rc<str>`。`d["key"]` の索引でキーを作るたびに
    /// String を確保しないようにするため（#15 / §7.4-1）。
    /// `Hash`/`Eq` は `Rc` が pointee へ委譲するので意味論は `String` 時と同一。
    Str(Rc<str>),
    Bool(bool),
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
            // ⚠ `None` は**キーとして禁止**（仕様・B1-a）。禁止したことで `DictKey::None`
            // が構築されなくなったため、バリアントごと削除してある。
            // ⚠ 判定は [`DictData::reject_key`] と必ず一致させること。
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
    ///
    /// ⚠⚠ **キーにできない値は黙って捨てず必ずエラーにする**（B1-a）。
    /// 以前はここが `if let Some(k) = … { … }` で、変換できないキーを**無言で無視**していた
    /// （「unhashable key silently ignored」というコメント付きの意図的な設計だった）。
    /// その結果 `d[(1, 2)] = v` が例外も出さずに**何も起きない**＝サイレントなデータ消失に
    /// なっていた。⇒ 受け付けられない理由は [`DictData::reject_key`] が必ず文章で返す。
    pub fn set(&mut self, key: Value, value: Value) -> Result<(), String> {
        if let Some(why) = Self::reject_key(&key) {
            return Err(why);
        }
        let k = DictKey::from_value(&key)
            .expect("reject_key が通した値は必ず DictKey に変換できる（両者は同じ判定）");
        self.map.insert(k, value);
        Ok(())
    }

    /// `key` を辞書のキーとして**使えない理由**を返す。使えるなら `None`。
    ///
    /// ⚠ 判定は [`DictKey::from_value`] と**必ず一致させること**。ずれると `set` の
    /// `expect` が落ちる（＝ずれたことがその場で分かる）。
    ///
    /// 区別している 2 種類:
    /// - **恒久的に禁止**（仕様）: `None` / `Undefined` は「存在しないことを示す値」、
    ///   `NaN` は自分自身と等しくないのでキーにできない。
    /// - **まだ未対応**: tuple / list / instance / uint / complex / 非整数 float など。
    ///   `DictKey` が int / str / bool / None の 4 種しか持たないため。B1-b/B1-c で解消予定。
    ///   それまでは**黙って捨てず**「まだ使えない」と知らせる。
    pub fn reject_key(key: &Value) -> Option<String> {
        match key {
            Value::Int(_) | Value::Str(_) | Value::Bool(_) => None,
            // 整数値の float（`1.0`）は Int キーへ正規化されるので受け付ける。
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 => None,
            Value::Float(f) if f.is_nan() => {
                Some("TypeError: NaN cannot be used as a dict key".to_string())
            }
            Value::Float(f) => Some(format!(
                "TypeError: {f} cannot be used as a dict key yet                  (only integral floats are supported)"
            )),
            Value::None => {
                Some("TypeError: None cannot be used as a dict key".to_string())
            }
            Value::Undefined => {
                Some("TypeError: Undefined cannot be used as a dict key".to_string())
            }
            other => Some(format!(
                "TypeError: unhashable type: '{}'",
                crate::interpreter::ops::typecheck::runtime_type_name(other)
            )),
        }
    }

    /// すべてのキーを `Value` リストとして返す（挿入順）。
    pub fn all_keys(&self) -> Vec<Value> {
        self.map
            .keys()
            .map(|k| match k {
                DictKey::Int(n) => Value::Int(*n),
                DictKey::Str(s) => Value::Str(s.clone()),
                DictKey::Bool(b) => Value::Bool(*b),
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
