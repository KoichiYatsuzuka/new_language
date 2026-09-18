// value/collections.rs — コレクション値型: SliceValue / TupleData / DictData / DictKey。

use indexmap::IndexMap;
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
///
/// ## なぜ `IndexMap<HKey, Value>` なのか（B1-c）
///
/// 以前は `IndexMap<DictKey, Value>` で、`DictKey` は `Int` / `Str` / `Bool` / `None` の
/// 4 種しか持たない **`Value` のプリミティブ射影**だった。Rust の `Hash` / `Eq` トレイトが
/// 要求されるので `Value` をそのまま入れられず、変換できないキーは**黙って捨てられていた**。
///
/// ⚠⚠ **`Hash` / `Eq` トレイト経由では、ユーザー定義の `__hash__` / `__eq__` を
/// 呼ぶことが原理的にできない**（トレイトのメソッドはインタプリタ文脈を受け取れない）。
/// ⇒ `indexmap` の `raw_entry_v1` API を使う。ハッシュ値を自分で渡し、等値判定は
/// **クロージャ**で渡せるので、そこが `&mut Interpreter` を掴める。
/// この API には `K: Hash + Eq` の境界が無いので、`HKey` は素の構造体でよい。
///
/// - `key_type`: 有効なキーの型名。型なし辞書は `"Any"`
/// - `item_type`: 有効な値の型名。型なし辞書は `"Any"`
#[derive(Debug)]
pub struct DictData {
    /// 有効なキーの型名。型なし辞書は `"Any"`。
    pub key_type: String,
    /// 有効な値の型名。型なし辞書は `"Any"`。
    pub item_type: String,
    map: IndexMap<HKey, Value>,
}


/// 辞書のキー。**計算済みのハッシュを一緒に持ち回る**。
///
/// ⚠ ハッシュを保存しておくのが要点。キーにした後で中身が変わっても、
/// 保存済みハッシュとバケットは一致したままなので**テーブルの不変条件は壊れない**
/// （構造的に等しいプローブで引けなくなるだけ）。さらに `set` がキーを**複製**するので、
/// 外から書き換えられる経路そのものが無い。
#[derive(Debug)]
pub(crate) struct HKey {
    /// 挿入時に計算したハッシュ。
    pub(crate) hash: u64,
    /// キーの値そのもの。⚠ `set` が複製したものなので、外部と共有していない。
    pub(crate) key: Value,
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

    /// キー検査の再帰の深さ上限。超えたらエラーにする。
    ///
    /// ⚠ 深く潜りすぎるキーを通すと、この先の `deep_copy_value`（深さ制限なし）で
    /// スタックが溢れる。ここで止めるのが最も浅い防波堤。
    /// ⚠ 元は「循環の疑い」だったが、格納時にディープコピーする規則（L4）が入って
    /// **循環は作れなくなった**。今ここに来るのは素直に深い入れ子だけ。
    const KEY_MAX_DEPTH: u32 = 64;

    /// `key` を辞書のキーとして**使えない理由**を返す。使えるなら `None`。
    ///
    /// ⚠ ここは**仕様上の禁止**だけを見る。「ハッシュできるか」は
    /// [`Interpreter::default_hash`] が全 `Value` に答えるので、型による制限は無い
    /// （B1-c で tuple / list / instance / 関数 / クラス等がすべてキーになった）。
    ///
    /// 禁止しているもの:
    /// - `None` / `Undefined` … 「存在しないことを示す値」はキーにしない（仕様）
    /// - `NaN` … 自分自身と等値にならないので、入れても引けない
    /// - `__eq__` を持つのに `__hash__` を持たないクラスのインスタンス（下記）
    /// - 極端に深い値（⚠ 循環は L4 の格納時複製で作れなくなったので、ここには来ない）
    ///
    /// ⚠ **入れ子も走査する**。`(1, NaN)` をキーにすると、等値にならない要素を含むので
    /// 同じく引けなくなる。黙って引けないキーを作らせない。
    pub fn reject_key(key: &Value) -> Option<String> {
        Self::reject_key_at(key, 0)
    }

    fn reject_key_at(key: &Value, depth: u32) -> Option<String> {
        if depth > Self::KEY_MAX_DEPTH {
            return Some(
                "TypeError: too deeply nested value cannot be used as a dict key"
                    .to_string(),
            );
        }
        let d = depth + 1;
        match key {
            Value::None => Some("TypeError: None cannot be used as a dict key".to_string()),
            Value::Undefined => {
                Some("TypeError: Undefined cannot be used as a dict key".to_string())
            }
            Value::Float(f) if f.is_nan() => {
                Some("TypeError: NaN cannot be used as a dict key".to_string())
            }
            Value::Complex(re, im) if re.is_nan() || im.is_nan() => {
                Some("TypeError: NaN cannot be used as a dict key".to_string())
            }
            // ⚠⚠ `__eq__` を定義していて `__hash__` を定義していないクラスは、
            //    **等値の規則とハッシュの規則が食い違う**。辞書はハッシュで束ねてから
            //    等値で確かめるので、`__eq__` が「等しい」と言う 2 つが別バケットに落ちて
            //    **黙って別のキーとして入る**（実測: len が 2 になり、引くと見つからない）。
            //    Python は `__eq__` を定義したクラスの `__hash__` を `None` にして
            //    unhashable にすることで防いでいる。Arrow は `__eq__` を `==` や
            //    リストの `in` でも使うのでクラス定義自体は禁止せず、**辞書のキーに
            //    した時点で**弾く。
            Value::Instance(rc) => {
                let inst = rc.borrow();
                let has_eq = inst.class.methods.contains_key("__eq__");
                let has_hash = inst.class.methods.contains_key("__hash__");
                if has_eq && !has_hash {
                    return Some(format!(
                        "TypeError: class '{}' defines __eq__ without __hash__, so it cannot be a dict key",
                        inst.class.name
                    ));
                }
                // `__hash__` を持つならユーザーの規則に従うので、中身は覗かない。
                if has_hash {
                    return None;
                }
                (0..inst.field_count())
                    .filter_map(|i| inst.field_value(i))
                    .find_map(|v| Self::reject_key_at(&v, d))
            }
            Value::Tuple(t) => t.all_values().iter().find_map(|v| Self::reject_key_at(v, d)),
            Value::List(rc) => rc.borrow().iter().find_map(|v| Self::reject_key_at(v, d)),
            Value::Set(rc) => rc.borrow().iter().find_map(|v| Self::reject_key_at(v, d)),
            Value::Dict(rc) => {
                let b = rc.borrow();
                b.all_keys()
                    .iter()
                    .chain(b.all_items().iter())
                    .find_map(|v| Self::reject_key_at(v, d))
            }
            _ => None,
        }
    }

    /// 挿入済みのキーと値を**そのまま**追加する（ハッシュを再計算しない）。
    ///
    /// ⚠ 既存の辞書を複製する経路（`deep_copy_value` / `Value::deep_clone`）専用。
    /// 複製元でキーは既に一意なので突き合わせは不要で、**保存済みハッシュをそのまま使える**。
    /// これが成り立つのは [`Interpreter::default_hash`] が
    /// **インスタンスを構造ハッシュにしている**から（ポインタだと複製で変わってしまう）。
    pub(crate) fn push_prehashed(&mut self, hash: u64, key: Value, value: Value) {
        use indexmap::map::raw_entry_v1::{RawEntryApiV1, RawEntryMut};
        // `|_| false` なので必ず Vacant になる（複製元でキーは一意）。
        match self.map.raw_entry_mut_v1().from_hash(hash, |_| false) {
            RawEntryMut::Vacant(v) => {
                v.insert_hashed_nocheck(hash, HKey { hash, key }, value);
            }
            RawEntryMut::Occupied(_) => unreachable!("`|_| false` は必ず Vacant を返す"),
        }
    }

    /// キー・値のペアを挿入順で走査するイテレータ。
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&HKey, &Value)> {
        self.map.iter()
    }

    /// すべてのキーを `Value` リストとして返す（挿入順）。
    pub fn all_keys(&self) -> Vec<Value> {
        self.map.keys().map(|k| k.key.clone()).collect()
    }

    /// すべての値をクローンしてリストとして返す（挿入順）。
    pub fn all_items(&self) -> Vec<Value> {
        self.map.values().cloned().collect()
    }

    /// すべての `(キー, 値)` を**組**にして返す（挿入順）。
    ///
    /// `d.items()` の素材。タプル化（`TupleData` の型名リストが要る）は呼び出し側で行う
    /// —— 要素のランタイム型名は `Interpreter::type_name` でしか取れないため、
    /// `zip` / `enumerate` と同じく**ディスパッチ側**で組む。
    pub fn all_pairs(&self) -> Vec<(Value, Value)> {
        self.map.iter().map(|(k, v)| (k.key.clone(), v.clone())).collect()
    }

    /// エントリ数を返す。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 辞書が空なら `true`。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// ハッシュと等値判定を**外から与えて**引く。
    ///
    /// ⚠ 等値判定がクロージャなのがこの設計の要点。呼び出し側が `&mut Interpreter` を
    /// 掴んだクロージャを渡せるので、ユーザー定義の `__eq__` を呼べる。
    /// ⚠ **共有借用**で呼ぶこと（`index_of_with` の警告と同じ理由）。
    pub(crate) fn get_with(
        &self,
        hash: u64,
        mut eq: impl FnMut(&Value) -> Result<bool, String>,
    ) -> Result<Option<Value>, String> {
        use indexmap::map::raw_entry_v1::RawEntryApiV1;
        // ⚠ クロージャの中で `?` が使えないので、エラーを外に持ち出して後で返す。
        let mut err: Option<String> = None;
        let found = self.map.raw_entry_v1().from_hash(hash, |k| match eq(&k.key) {
            Ok(b) => b,
            Err(e) => {
                if err.is_none() {
                    err = Some(e);
                }
                false
            }
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(found.map(|(_, v)| v.clone()))
    }

    /// ハッシュと等値判定を**外から与えて**、一致するエントリの索引を返す。
    ///
    /// ⚠ **共有借用**で呼ぶこと。等値判定（＝ユーザーの `__eq__`）が同じ辞書を
    /// 読み返しても共有借用どうしなら安全だが、可変借用を握っていると `RefCell` が
    /// パニックする。⇒ 書き込みは「引く」と「書く」を分ける（`Interpreter::dict_set`）。
    pub(crate) fn index_of_with(
        &self,
        hash: u64,
        mut eq: impl FnMut(&Value) -> Result<bool, String>,
    ) -> Result<Option<usize>, String> {
        use indexmap::map::raw_entry_v1::RawEntryApiV1;
        // ⚠ クロージャの中で `?` が使えないので、エラーを外へ持ち出して後で返す。
        let mut err: Option<String> = None;
        let idx = self.map.raw_entry_v1().index_from_hash(hash, |k| match eq(&k.key) {
            Ok(b) => b,
            Err(e) => {
                if err.is_none() {
                    err = Some(e);
                }
                false
            }
        });
        match err {
            Some(e) => Err(e),
            None => Ok(idx),
        }
    }

    /// 索引で値を読む（`index_of_with` が見つけた位置）。
    pub(crate) fn value_at(&self, idx: usize) -> Option<&Value> {
        self.map.get_index(idx).map(|(_, v)| v)
    }

    /// 索引で値を差し替える（`index_of_with` が見つけた位置に書く）。
    pub(crate) fn set_at(&mut self, idx: usize, value: Value) {
        if let Some((_, v)) = self.map.get_index_mut(idx) {
            *v = value;
        }
    }
}
