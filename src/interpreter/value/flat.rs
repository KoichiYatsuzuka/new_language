// value/flat.rs — フラット凍結リストのレイアウト: FlatFieldTy / FlatListData / FlatLayout。

use {
    std::cell::RefCell, std::rc::Rc,
};
use super::*;


// ---------------------------------------------------------------------------
// Flat-frozen list layout
// ---------------------------------------------------------------------------

/// フラットリストの各フィールドの型。
/// Int/Float はプリミティブ 8-byte フィールド。Struct は再帰的な SWD クラスフィールド。
#[derive(Debug, Clone)]
pub enum FlatFieldTy {
    Int,
    Float,
    /// 別の SWD クラスをインラインに展開したフィールド。
    Struct(Rc<FlatLayout>),
}


impl FlatFieldTy {
    pub fn stride(&self) -> usize {
        match self {
            FlatFieldTy::Int | FlatFieldTy::Float => 8,
            FlatFieldTy::Struct(sub) => sub.stride,
        }
    }
}


impl PartialEq for FlatFieldTy {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int, Self::Int) | (Self::Float, Self::Float) => true,
            (Self::Struct(a), Self::Struct(b)) => a.class_name == b.class_name,
            _ => false,
        }
    }
}


/// FrozenList の可変状態（バイト列・長さ・確保済みサイズ）。
/// `data` はフラット byte 列（stride バイト × allocated_size 要素分を確保、先頭 len 要素が有効）。
#[derive(Debug, Clone)]
pub struct FlatListData {
    /// フラット byte 列。`allocated_size * stride` バイトを確保し、先頭 `len * stride` が有効データ。
    pub data: Vec<u8>,
    /// 有効要素数（論理長）。
    pub len: usize,
    /// 確保済み要素数（容量）。`len <= allocated_size` が常に成立する。
    pub allocated_size: usize,
}


/// FrozenList の平坦メモリレイアウト記述。
/// `fields` はアルファベット順。全フィールドが SWD 型（int/float または別の SWD クラス）のみ。
#[derive(Debug, Clone)]
pub struct FlatLayout {
    pub class_name: String,
    /// (フィールド名, 型) のアルファベット順リスト。
    pub fields: Vec<(String, FlatFieldTy)>,
    /// 要素1つあたりのバイト数。各フィールドの stride() の合計。
    pub stride: usize,
    /// 再構成用クラス定義。
    pub class: Rc<ClassValue>,
}


impl FlatLayout {
    /// フラット配列インデックス `idx` の要素を `Value::Instance` として再構成する。
    pub fn reconstruct_item(&self, data: &[u8], idx: usize) -> Value {
        let base = idx * self.stride;
        self.reconstruct_at(data, base)
    }

    /// バイト列の `byte_base` 位置からこのレイアウトのインスタンスを再構成する。
    fn reconstruct_at(&self, data: &[u8], byte_base: usize) -> Value {
        let mut inst = InstanceData::new_empty(self.class.clone(), INST_IMMUTABLE);
        let mut offset = byte_base;
        for (field_name, field_ty) in &self.fields {
            let val = match field_ty {
                FlatFieldTy::Float => {
                    let bytes: [u8; 8] = data[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
                    offset += 8;
                    Value::Float(f64::from_le_bytes(bytes))
                }
                FlatFieldTy::Int => {
                    let bytes: [u8; 8] = data[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
                    offset += 8;
                    Value::Int(i64::from_le_bytes(bytes))
                }
                FlatFieldTy::Struct(sub) => {
                    let v = sub.reconstruct_at(data, offset);
                    offset += sub.stride;
                    v
                }
            };
            if let Some(&idx) = self.class.field_index.get(field_name.as_str()) {
                inst.store_field(idx, val, false);
            }
        }
        Value::Instance(Rc::new(RefCell::new(inst)))
    }
}
