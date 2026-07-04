// value/instance.rs — インスタンスデータと C ABI raw レイアウト: InstanceData(オフセット参照アクセサ)、RawWidth/RawFieldDesc/RawLayout、シャドウ変換ヘルパー、InstanceData フラグ定数。

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
// InstanceData flags (u32 in InstanceData.flags)
// ---------------------------------------------------------------------------

/// `let` バインドされたインスタンス: 全フィールドが不変、`mut self` メソッド呼び出し禁止。
pub const INST_IMMUTABLE: u32 = 0x80000000;

/// `raw_fields: Vec<u8>` による int/float フラット バッファが有効。
pub const INST_HAS_RAW_LAYOUT: u32 = 0x40000000;

/// 例外クラスのインスタンス（高速例外型チェック用）。
pub const INST_IS_EXCEPTION: u32 = 0x20000000;

/// `new_type` ラッパーのインスタンス（高速 new_type 判定用）。
pub const INST_IS_NEW_TYPE: u32 = 0x10000000;

/// bits 23-0: `raw_fields` の初期化済みスロットを示すビットマップ（最大 24 スロット）。
pub const INST_FIELD_INIT_MASK: u32 = 0x00FF_FFFF;


// ---------------------------------------------------------------------------
// Raw field layout (C ABI 準拠のフラット格納 — for_claude/c_abi_interop.md P1)
// ---------------------------------------------------------------------------

/// raw ブロック内の1フィールドの格納形式。
/// Arrow の `int`/`float` は 8 バイト、C ABI 型（int32 等）は宣言幅で格納される。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawWidth {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
}


impl RawWidth {
    pub fn byte_width(self) -> usize {
        match self {
            RawWidth::I8 | RawWidth::U8 => 1,
            RawWidth::I16 | RawWidth::U16 => 2,
            RawWidth::I32 | RawWidth::U32 | RawWidth::F32 => 4,
            RawWidth::I64 | RawWidth::U64 | RawWidth::F64 => 8,
        }
    }
    pub fn is_float(self) -> bool {
        matches!(self, RawWidth::F32 | RawWidth::F64)
    }
    /// 型注釈文字列から格納形式を導出する。プリミティブでない場合は `None`。
    pub fn from_ann(ann: &str) -> Option<RawWidth> {
        match ann {
            "int" => Some(RawWidth::I64),
            "float" => Some(RawWidth::F64),
            "int8" => Some(RawWidth::I8),
            "int16" => Some(RawWidth::I16),
            "int32" => Some(RawWidth::I32),
            "int64" => Some(RawWidth::I64),
            "uint8" => Some(RawWidth::U8),
            "uint16" => Some(RawWidth::U16),
            "uint32" => Some(RawWidth::U32),
            "uint64" => Some(RawWidth::U64),
            "float32" => Some(RawWidth::F32),
            "float64" => Some(RawWidth::F64),
            _ => None,
        }
    }
}


/// raw ブロック内の1フィールドの位置と形式（スロットインデックス順）。
#[derive(Debug, Clone, Copy)]
pub struct RawFieldDesc {
    /// フィールド領域先頭（ブロック先頭 + 8）からのバイトオフセット。C アラインメント規則で計算。
    pub byte_offset: usize,
    pub width: RawWidth,
}


/// クラスの raw ブロックレイアウト記述子。
/// 全 own フィールドがプリミティブ（int/float/C ABI 型）かつ trait 継承なし、
/// フィールド数 ≤ 24（`INST_FIELD_INIT_MASK` の制約）のクラスにのみ付与される。
#[derive(Debug, Clone)]
pub struct RawLayout {
    /// スロットインデックス → 記述子（宣言順 = field_index 順）。
    pub fields: Vec<RawFieldDesc>,
    /// フィールド領域の総バイト数（末尾パディング込み、8 の倍数に切り上げ）。
    pub total_bytes: usize,
}


impl RawLayout {
    /// (名前, 型注釈) の宣言順リストからレイアウトを構築する。
    /// 非プリミティブフィールドを含む場合や 24 フィールド超は `None`。
    pub fn from_fields(fields: &[(String, String)]) -> Option<RawLayout> {
        if fields.is_empty() || fields.len() > 24 {
            return None;
        }
        let mut descs = Vec::with_capacity(fields.len());
        let mut offset = 0usize;
        for (_, ann) in fields {
            let width = RawWidth::from_ann(ann)?;
            let w = width.byte_width();
            // C のアラインメント規則: オフセットを型幅に切り上げ
            offset = (offset + w - 1) / w * w;
            descs.push(RawFieldDesc { byte_offset: offset, width });
            offset += w;
        }
        let total_bytes = (offset + 7) / 8 * 8;
        Some(RawLayout { fields: descs, total_bytes })
    }
}


/// クラスインスタンスの実行時データ。`Rc<RefCell<InstanceData>>` で共有・可変参照する。
///
/// - `raw`: 連続ブロック。slot 0 = `[class_id: u32][flags: u32]`（リトルエンディアン
///   パッキング: バイト 0-3 = class_id, 4-7 = flags）。raw レイアウトを持つクラスでは
///   slot 1.. にプリミティブフィールドが C ABI レイアウト（宣言順）で続く。
///   外部言語へは `raw.as_ptr() + 8` を構造体先頭として渡せる（Case C レイアウト）。
/// - `class`: このインスタンスが属するクラスの定義（メソッド解決などに使用）
/// - `boxed_fields`: raw レイアウトを持たないクラスのフィールドスロット Vec。
///   `None` = 未初期化スロット。`Some((val, mutable))` = 初期化済み。
///   raw レイアウトを持つクラスでは空。
#[derive(Debug)]
pub struct InstanceData {
    /// ヘッダ + raw フィールドの連続ブロック。最低 1 スロット（ヘッダ）。
    pub raw: Box<[u64]>,
    pub class: Rc<ClassValue>,
    /// 従来形式のフィールドスロット（raw レイアウトなしのクラス用）。
    pub boxed_fields: Vec<Option<(Value, bool)>>,
}


impl InstanceData {
    /// クラス定義に従って空インスタンス（全スロット未初期化）を生成する。
    /// `extra_flags` に `INST_IMMUTABLE` 等を渡せる。クラス由来のフラグ
    /// （is_exception / new_type / raw_layout）は自動で立つ。
    pub fn new_empty(class: Rc<ClassValue>, extra_flags: u32) -> InstanceData {
        let flags = extra_flags
            | if class.is_exception { INST_IS_EXCEPTION } else { 0 }
            | if class.new_type_base.is_some() { INST_IS_NEW_TYPE } else { 0 }
            | if class.raw_layout.is_some() { INST_HAS_RAW_LAYOUT } else { 0 };
        let (raw, boxed_fields) = match &class.raw_layout {
            Some(l) => (
                Self::make_raw_block(class.class_id, flags, l.total_bytes),
                Vec::new(),
            ),
            None => (
                Self::make_raw_block(class.class_id, flags, 0),
                vec![None; class.field_count],
            ),
        };
        InstanceData { raw, class, boxed_fields }
    }

    /// ヘッダブロックを構築する（raw フィールド領域 `extra_bytes` バイト付き）。
    #[inline]
    pub fn make_raw_block(class_id: u32, flags: u32, extra_bytes: usize) -> Box<[u64]> {
        let mut v = vec![0u64; 1 + extra_bytes.div_ceil(8)];
        v[0] = (class_id as u64) | ((flags as u64) << 32);
        v.into_boxed_slice()
    }

    #[inline]
    pub fn class_id(&self) -> u32 {
        self.raw[0] as u32
    }

    #[inline]
    pub fn flags(&self) -> u32 {
        (self.raw[0] >> 32) as u32
    }

    #[inline]
    pub fn set_flags(&mut self, f: u32) {
        self.raw[0] = (self.raw[0] & 0xFFFF_FFFF) | ((f as u64) << 32);
    }

    #[inline]
    pub fn flags_or(&mut self, bits: u32) {
        let f = self.flags() | bits;
        self.set_flags(f);
    }

    #[inline]
    pub fn has_raw_layout(&self) -> bool {
        self.flags() & INST_HAS_RAW_LAYOUT != 0
    }

    /// raw フィールド領域（ヘッダ直後）へのバイトスライス。
    #[inline]
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        let ptr = self.raw.as_ptr() as *const u8;
        unsafe { std::slice::from_raw_parts(ptr.add(8), (self.raw.len() - 1) * 8) }
    }

    #[inline]
    pub(crate) fn raw_bytes_mut(&mut self) -> &mut [u8] {
        let ptr = self.raw.as_mut_ptr() as *mut u8;
        unsafe { std::slice::from_raw_parts_mut(ptr.add(8), (self.raw.len() - 1) * 8) }
    }

    /// スロット `idx` が初期化済みかどうか（raw クラスは init ビットマップ、boxed は Some 判定）。
    #[inline]
    pub fn slot_initialized(&self, idx: usize) -> bool {
        if self.has_raw_layout() {
            self.flags() & (1u32 << idx) != 0
        } else {
            matches!(self.boxed_fields.get(idx), Some(Some(_)))
        }
    }

    /// スロット `idx` の値を読み出す（未初期化なら `None`）。raw クラスは幅変換込み。
    pub fn field_value(&self, idx: usize) -> Option<Value> {
        if self.has_raw_layout() {
            if !self.slot_initialized(idx) {
                return None;
            }
            let layout = self.class.raw_layout.as_ref()?;
            let desc = layout.fields.get(idx)?;
            let bytes = self.raw_bytes();
            let o = desc.byte_offset;
            Some(match desc.width {
                RawWidth::I8 => Value::Int(bytes[o] as i8 as i64),
                RawWidth::U8 => Value::Int(bytes[o] as i64),
                RawWidth::I16 => Value::Int(i16::from_le_bytes([bytes[o], bytes[o + 1]]) as i64),
                RawWidth::U16 => Value::Int(u16::from_le_bytes([bytes[o], bytes[o + 1]]) as i64),
                RawWidth::I32 => Value::Int(i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as i64),
                RawWidth::U32 => Value::Int(u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as i64),
                RawWidth::I64 | RawWidth::U64 => {
                    Value::Int(i64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()))
                }
                RawWidth::F32 => Value::Float(f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as f64),
                RawWidth::F64 => Value::Float(f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap())),
            })
        } else {
            self.boxed_fields.get(idx)?.as_ref().map(|(v, _)| v.clone())
        }
    }

    /// スロット `idx` の可変フラグを返す（未初期化スロットは None）。
    pub fn field_mutable(&self, idx: usize) -> Option<bool> {
        if self.has_raw_layout() {
            if !self.slot_initialized(idx) {
                return None;
            }
            if self.flags() & INST_IMMUTABLE != 0 {
                return Some(false);
            }
            Some(self.class.field_mutability_vec.get(idx).copied().unwrap_or(true))
        } else {
            self.boxed_fields.get(idx)?.as_ref().map(|(_, m)| *m)
        }
    }

    /// スロット `idx` へ書き込む（可変性検査なしの生ストア）。
    /// raw クラスで値の型がスロット形式に合わない場合は `false` を返す。
    pub fn store_field(&mut self, idx: usize, val: Value, mutable: bool) -> bool {
        if self.has_raw_layout() {
            let Some(layout) = self.class.raw_layout.clone() else { return false };
            let Some(desc) = layout.fields.get(idx).copied() else { return false };
            let o = desc.byte_offset;
            {
                let bytes = self.raw_bytes_mut();
                match (desc.width, &val) {
                    (RawWidth::I8 | RawWidth::U8, Value::Int(n)) => bytes[o] = *n as u8,
                    (RawWidth::I16 | RawWidth::U16, Value::Int(n)) => {
                        bytes[o..o + 2].copy_from_slice(&(*n as u16).to_le_bytes())
                    }
                    (RawWidth::I32 | RawWidth::U32, Value::Int(n)) => {
                        bytes[o..o + 4].copy_from_slice(&(*n as u32).to_le_bytes())
                    }
                    (RawWidth::I64 | RawWidth::U64, Value::Int(n)) => {
                        bytes[o..o + 8].copy_from_slice(&n.to_le_bytes())
                    }
                    (RawWidth::F32, Value::Float(f)) => {
                        bytes[o..o + 4].copy_from_slice(&(*f as f32).to_le_bytes())
                    }
                    (RawWidth::F64, Value::Float(f)) => {
                        bytes[o..o + 8].copy_from_slice(&f.to_le_bytes())
                    }
                    // int → float フィールドの自動昇格
                    (RawWidth::F32, Value::Int(n)) => {
                        bytes[o..o + 4].copy_from_slice(&(*n as f32).to_le_bytes())
                    }
                    (RawWidth::F64, Value::Int(n)) => {
                        bytes[o..o + 8].copy_from_slice(&(*n as f64).to_le_bytes())
                    }
                    _ => return false,
                }
            }
            self.flags_or(1u32 << idx); // init ビットマップ
            true
        } else {
            if idx >= self.boxed_fields.len() {
                return false;
            }
            self.boxed_fields[idx] = Some((val, mutable));
            true
        }
    }

    /// フィールドスロット総数。
    #[inline]
    pub fn field_count(&self) -> usize {
        self.class.field_count
    }
}


// ---------------------------------------------------------------------------
// C ABI 構造体ポインタ引数のシャドウ変換（for_claude/c_abi_interop.md P3）
// ---------------------------------------------------------------------------

/// 2つの raw レイアウトが構造的に完全一致するか（フィールド数・各オフセット・幅・総バイト数）。
/// 一致すればインスタンスの raw ブロックをそのまま C 構造体ポインタとしてゼロコピーで渡せる。
pub fn raw_layouts_compatible(a: &RawLayout, b: &RawLayout) -> bool {
    a.total_bytes == b.total_bytes
        && a.fields.len() == b.fields.len()
        && a.fields
            .iter()
            .zip(b.fields.iter())
            .all(|(x, y)| x.byte_offset == y.byte_offset && x.width == y.width)
}


/// バイト列の `desc` 位置へ `Value` を書き込む（`InstanceData::store_field` と同じ幅変換規則、
/// int→float 昇格あり）。スロット形式に値の型が合わなければ `false`。
fn write_raw_field_bytes(bytes: &mut [u8], desc: RawFieldDesc, val: &Value) -> bool {
    let o = desc.byte_offset;
    match (desc.width, val) {
        (RawWidth::I8 | RawWidth::U8, Value::Int(n)) => bytes[o] = *n as u8,
        (RawWidth::I16 | RawWidth::U16, Value::Int(n)) => {
            bytes[o..o + 2].copy_from_slice(&(*n as u16).to_le_bytes())
        }
        (RawWidth::I32 | RawWidth::U32, Value::Int(n)) => {
            bytes[o..o + 4].copy_from_slice(&(*n as u32).to_le_bytes())
        }
        (RawWidth::I64 | RawWidth::U64, Value::Int(n)) => {
            bytes[o..o + 8].copy_from_slice(&n.to_le_bytes())
        }
        (RawWidth::F32, Value::Float(f)) => {
            bytes[o..o + 4].copy_from_slice(&(*f as f32).to_le_bytes())
        }
        (RawWidth::F64, Value::Float(f)) => bytes[o..o + 8].copy_from_slice(&f.to_le_bytes()),
        (RawWidth::F32, Value::Int(n)) => {
            bytes[o..o + 4].copy_from_slice(&(*n as f32).to_le_bytes())
        }
        (RawWidth::F64, Value::Int(n)) => {
            bytes[o..o + 8].copy_from_slice(&(*n as f64).to_le_bytes())
        }
        _ => return false,
    }
    true
}


/// バイト列の `desc` 位置から `Value` を読み出す（`InstanceData::field_value` と同じ幅変換規則）。
fn read_raw_field_bytes(bytes: &[u8], desc: RawFieldDesc) -> Value {
    let o = desc.byte_offset;
    match desc.width {
        RawWidth::I8 => Value::Int(bytes[o] as i8 as i64),
        RawWidth::U8 => Value::Int(bytes[o] as i64),
        RawWidth::I16 => Value::Int(i16::from_le_bytes([bytes[o], bytes[o + 1]]) as i64),
        RawWidth::U16 => Value::Int(u16::from_le_bytes([bytes[o], bytes[o + 1]]) as i64),
        RawWidth::I32 => Value::Int(i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as i64),
        RawWidth::U32 => Value::Int(u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as i64),
        RawWidth::I64 | RawWidth::U64 => {
            Value::Int(i64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()))
        }
        RawWidth::F32 => Value::Float(f32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as f64),
        RawWidth::F64 => Value::Float(f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap())),
    }
}


/// インスタンスの各フィールドを**宣言順の位置**で `layout` のフィールド位置へ写した
/// 一時バイト列を構築する。未初期化・型不一致のフィールドがあれば `None`。
pub fn build_shadow_raw(inst: &InstanceData, layout: &RawLayout) -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; layout.total_bytes];
    for (i, desc) in layout.fields.iter().enumerate() {
        let val = inst.field_value(i)?;
        if !write_raw_field_bytes(&mut bytes, *desc, &val) {
            return None;
        }
    }
    Some(bytes)
}


/// C 呼び出し後のバイト列を、インスタンスの各フィールドへ**宣言順**で読み戻す。
/// 型不一致で書き込めないフィールドがあれば `false`。
pub fn apply_shadow_raw(inst: &mut InstanceData, layout: &RawLayout, bytes: &[u8]) -> bool {
    for (i, desc) in layout.fields.iter().enumerate() {
        let val = read_raw_field_bytes(bytes, *desc);
        if !inst.store_field(i, val, true) {
            return false;
        }
    }
    true
}
