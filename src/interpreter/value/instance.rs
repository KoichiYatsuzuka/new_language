// ⚠ 評価コアビルド（`native` 無効）では FFI 経路が全部落ちるので、raw レイアウトの読み書き補助が
//   未使用になる。型（`NativeFnRef` / `RawLayout` など）は `Value` が参照するため
//   残す必要があり、**関数だけ** dead になる。⇒ そのビルドに限り警告を抑える。
#![cfg_attr(not(feature = "native"), allow(dead_code))]
// value/instance.rs — インスタンスデータと C ABI raw レイアウト: InstanceData(オフセット参照アクセサ)、RawWidth/RawFieldDesc/RawLayout、シャドウ変換ヘルパー、InstanceData フラグ定数。

use std::rc::Rc;
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

// ---------------------------------------------------------------------------
// 先頭の塊の並び（copy_alloc_speed_plan.md 8-2・D-9・D-13）
// ---------------------------------------------------------------------------

/// インスタンスの先頭の塊（[`InstanceData::raw`]）の並びの**唯一の定義**。
///
/// [`InstanceData`] の読み書きと、コンパイル済みコードが番地から直接読み書きする道（フェーズ 8・
/// `partial_compiler/llvm_codegen`）の両方がこの定数を使う。⚠ 並びを変えるときはここだけを変えること
/// （単体テスト `direct_access_matches_field_value` が、生成コードと同じ手順の読み書きと `field_value` /
/// `store_field` の一致を見ている）。
///
/// ```text
/// 語 0:  bit 0〜31 = class_id ・ bit 32〜63 = flags
///        flags: bit 31 不変 ・ bit 30 raw レイアウト ・ bit 29 例外 ・ bit 28 new_type ・ bit 0〜23 初期化の印
/// 語 1〜: raw レイアウトのクラスのフィールド（C の並び・宣言順）
/// ```
pub mod block {
    /// 先頭の語のうち `flags` が始まる bit。
    pub const FLAGS_SHIFT: u32 = 32;
    /// 先頭の語のうち `class_id` の部分。
    pub const CLASS_ID_MASK: u64 = 0xFFFF_FFFF;
    /// フィールドの領域が始まる位置（塊の先頭からのバイト数）。
    pub const FIELDS_OFFSET: usize = 8;
    /// 先頭の語の「不変のインスタンス」の bit（`INST_IMMUTABLE`）。語を符号つきで読むと負になる。
    pub const IMMUTABLE: u64 = (super::INST_IMMUTABLE as u64) << FLAGS_SHIFT;

    /// 先頭の語の、フィールド `idx` の初期化の印。
    #[inline]
    pub const fn init_bit(idx: usize) -> u64 {
        1u64 << (FLAGS_SHIFT as usize + idx)
    }

    /// フィールドの位置（塊の先頭からのバイト数）。`byte_offset` は [`super::RawFieldDesc::byte_offset`]。
    #[inline]
    pub const fn field_offset(byte_offset: usize) -> usize {
        FIELDS_OFFSET + byte_offset
    }
}


// ---------------------------------------------------------------------------
// Raw field layout (C ABI 準拠のフラット格納 — .claude/skills/c-abi-interop/SKILL.md P1)
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
    /// 幅の番号（[`direct_field_info`] に入れる。生成コードの表と番地を返すコールバックの照合に使う・8-2）。
    pub fn code(self) -> u64 {
        match self {
            RawWidth::I8 => 1,
            RawWidth::I16 => 2,
            RawWidth::I32 => 3,
            RawWidth::I64 => 4,
            RawWidth::U8 => 5,
            RawWidth::U16 => 6,
            RawWidth::U32 => 7,
            RawWidth::U64 => 8,
            RawWidth::F32 => 9,
            RawWidth::F64 => 10,
        }
    }

    pub fn byte_width(self) -> usize {
        match self {
            RawWidth::I8 | RawWidth::U8 => 1,
            RawWidth::I16 | RawWidth::U16 => 2,
            RawWidth::I32 | RawWidth::U32 | RawWidth::F32 => 4,
            RawWidth::I64 | RawWidth::U64 | RawWidth::F64 => 8,
        }
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
            offset = offset.div_ceil(w) * w;
            descs.push(RawFieldDesc { byte_offset: offset, width });
            offset += w;
        }
        let total_bytes = offset.div_ceil(8) * 8;
        Some(RawLayout { fields: descs, total_bytes })
    }

    /// クラスの raw レイアウト（**持つかどうかの唯一の判定**・8-2）: trait を実装せず（`no_bases`）、インスタンスの
    /// フィールドが全部このクラスのもので（`field_count` が `own_fields` の数と同じ）、全部が数値（[`RawWidth::from_ann`]）、
    /// 24 個以下。実行時のクラスの定義（`exec/definitions.rs`）と、コンパイル済みコードの静的な並び
    /// （[`static_class_layout`]）が同じこの関数を通る。
    pub fn for_class(no_bases: bool, own_fields: &[(String, String)], field_count: usize) -> Option<RawLayout> {
        if !no_bases || own_fields.len() != field_count {
            return None;
        }
        RawLayout::from_fields(own_fields)
    }

    /// C の sizeof（最後のフィールドの終わりを、いちばん大きいフィールドの幅に切り上げたもの・4-6）。配列の要素の間隔。
    /// ⚠ `total_bytes` は 8 の倍数に切り上げてあるので C の sizeof とは限らない（`unsigned char` 4 つは C では 4 バイト）。
    pub fn c_size(&self) -> usize {
        let align = self.fields.iter().map(|f| f.width.byte_width()).max().unwrap_or(1);
        let end = self.fields.iter().map(|f| f.byte_offset + f.width.byte_width()).max().unwrap_or(0);
        end.div_ceil(align) * align
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

    /// C の構造体のバイト列から、コンストラクタを呼ばずにインスタンスを作る（値返し・構造体の配列の展開・4-6）。
    /// クラスは C と同じ raw レイアウトを持つこと。全フィールドを初期化済み・書き換え可能にする（C の構造体のクラスは
    /// フィールドがすべて `mut`）。
    pub fn from_c_bytes(class: Rc<ClassValue>, bytes: &[u8]) -> Result<InstanceData, String> {
        let Some(layout) = class.raw_layout.clone() else {
            return Err(format!("TypeError: class '{}' does not have the C layout of a struct", class.name));
        };
        let size = layout.c_size();
        if bytes.len() < size {
            return Err(format!(
                "RuntimeError: struct '{}' needs {size} bytes, but {} were given",
                class.name,
                bytes.len()
            ));
        }
        // 符号なし 64 ビットの 2^63 以上は Arrow の `int` に入らないので、境界で OverflowError（1-6 と同じ。写したまま
        // にすると、フィールドを読んだときに黙って負になる）
        for f in &layout.fields {
            if matches!(f.width, RawWidth::U64) {
                let mut b8 = [0u8; 8];
                b8.copy_from_slice(&bytes[f.byte_offset..f.byte_offset + 8]);
                if u64::from_le_bytes(b8) > i64::MAX as u64 {
                    return Err(format!("OverflowError: {}", super::U64_OVERFLOW_MSG));
                }
            }
        }
        let mut inst = InstanceData::new_empty(class, 0);
        inst.raw_bytes_mut()[..size].copy_from_slice(&bytes[..size]);
        // 初期化済みの印（初期化のビットマップ・raw レイアウトは 24 フィールドまで）
        let n = layout.fields.len();
        inst.flags_or(if n >= 32 { u32::MAX } else { (1u32 << n) - 1 });
        Ok(inst)
    }

    /// ヘッダブロックを構築する（raw フィールド領域 `extra_bytes` バイト付き）。
    #[inline]
    pub fn make_raw_block(class_id: u32, flags: u32, extra_bytes: usize) -> Box<[u64]> {
        let mut v = vec![0u64; 1 + extra_bytes.div_ceil(8)];
        v[0] = (class_id as u64) | ((flags as u64) << block::FLAGS_SHIFT);
        v.into_boxed_slice()
    }

    #[inline]
    pub fn flags(&self) -> u32 {
        (self.raw[0] >> block::FLAGS_SHIFT) as u32
    }

    #[inline]
    pub fn set_flags(&mut self, f: u32) {
        self.raw[0] = (self.raw[0] & block::CLASS_ID_MASK) | ((f as u64) << block::FLAGS_SHIFT);
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
        unsafe { std::slice::from_raw_parts(ptr.add(block::FIELDS_OFFSET), (self.raw.len() - 1) * 8) }
    }

    #[inline]
    pub(crate) fn raw_bytes_mut(&mut self) -> &mut [u8] {
        let ptr = self.raw.as_mut_ptr() as *mut u8;
        unsafe { std::slice::from_raw_parts_mut(ptr.add(block::FIELDS_OFFSET), (self.raw.len() - 1) * 8) }
    }

    /// スロット `idx` が初期化済みかどうか（raw クラスは init ビットマップ、boxed は Some 判定）。
    #[inline]
    pub fn slot_initialized(&self, idx: usize) -> bool {
        if self.has_raw_layout() {
            self.raw[0] & block::init_bit(idx) != 0
        } else {
            matches!(self.boxed_fields.get(idx), Some(Some(_)))
        }
    }

    /// 属性読み出しの IC の命中判定（**唯一の実装**・native_object_access_plan.md 2-1）。
    ///
    /// `packed` は `AttrCache` の中身（VM・ツリーウォークは `Cell`、ネイティブは DLL 内の `AtomicU64`）。
    /// class_id が当たれば `(slot, アクセスレベル)`、ほかは `None`（呼ぶ側が `get_attr_val` のミス経路へ回す）。
    /// `public_only` なら public 以外も `None`（VM・ネイティブはアクセスの検査をミス経路に任せる。
    /// ツリーウォークは命中経路で `check_access_level` する）。値は呼ぶ側が [`Self::field_value`] で読む
    /// （未初期化の slot は `None` でミス経路へ）。
    /// `name` はデバッグビルドの照合にだけ使う（release では呼ばれないので名前の表を引かない）。
    /// ⚠ 以前は VM の `GetAttr` / `GetAttrLocal` とツリーウォークの `eval_attr` が同じ判定を各自で書いていた。
    /// ⚠⚠ **値の読み出しまでここに入れない**。`#[inline(always)]` で `field_value`（raw レイアウトの幅ごとの
    ///   変換を含む）まで `exec_op` に展開され、`bench_field_access` が 6% 遅くなった（実測・2-1）。
    #[inline(always)]
    pub(crate) fn ic_slot<'n>(
        &self,
        packed: u64,
        public_only: bool,
        name: impl FnOnce() -> &'n str,
    ) -> Option<(usize, u8)> {
        let (idx, access) = crate::ast::AttrCache::probe(packed, self.class.class_id)?;
        if public_only && access != crate::ast::AttrCache::PUBLIC {
            return None;
        }
        debug_assert_eq!(
            self.class.field_index.get(name()).copied(),
            Some(idx),
            "attr IC slot mismatch on class_id {}",
            self.class.class_id
        );
        Some((idx, access))
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
            // 記述子（`Copy`）だけを写す（並びの `Rc` を写さない・copy_alloc_speed_plan.md 7-10）
            let Some(desc) = self.class.raw_layout.as_ref().and_then(|l| l.fields.get(idx).copied()) else { return false };
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
            self.raw[0] |= block::init_bit(idx); // 初期化の印
            true
        } else {
            if idx >= self.boxed_fields.len() {
                return false;
            }
            // ── boxed レイアウトの型検査（A-3）──────────────────────────────
            //
            // ⚠⚠ **以前はここが無検査だった。** raw レイアウト経路（上）は「値がスロット形式に
            //    合うか」を見るのに、boxed 経路は受け取った値をそのまま入れていた。
            //    raw レイアウトが付くのは「trait 継承なし・全フィールドが int/float 系」の
            //    ときだけなので、`str` が 1 つ混ざる／trait を 1 つ実装する／
            //    **テンプレートクラスである**だけで検査が消え、静的検査が届かない経路
            //    （FFI・ネイティブコールバック・`Any` 経由）から**黙って不整合な型**が
            //    入っていた。
            //
            // ⚠ 判定できる種別だけを見る。`TypeTag::Other`（`list[T]`・クラス名・`Union` 等）と
            //    `Any` は通す — 誤検知で正しいコードを落とすより取りこぼす方へ倒す
            //    （`ffi_boundary` の「保守的側」と同じ方針）。
            let tag = self
                .class
                .field_tags
                .get(idx)
                .copied()
                .unwrap_or(crate::vm::op::TypeTag::Other);
            // `int` → `float` の昇格（案 B）。raw 経路が `(RawWidth::F64, Value::Int)` で
            // 行っているのと同じ規則をここでも適用する。
            let val = match (tag, &val) {
                (crate::vm::op::TypeTag::Float, Value::Int(n)) => Value::Float(*n as f64),
                _ => val,
            };
            // ⚠ `int` と `uint` は**相互に許容する**。`TypeTag::matches` は
            //    `uint` に `Value::UInt` を要求するが、Arrow で `Value::UInt` が生まれるのは
            //    ハンドル値と uint 同士の除算だけで、`let x: uint = 5` も
            //    `f(v: uint)` も**値としては `Value::Int` を束縛する**
            //    （変数・引数・返り値のどの束縛点も `uint` 注釈に対して `Value::Int` を通す）。
            //    ここだけ厳格にすると実バグを 1 つも塞がずに正常なコードを落とすので、
            //    整数族は通し、`str`/`bool` の混入だけを弾く。
            //    ⚠ `TypeTag::matches` 自体は VM の `MustBe`/`IsType` と共有なので変えない。
            //    （`uint` 注釈と `Value::UInt` のずれ自体は別タスク: 計画 §7）
            let tag_ok = match (tag, &val) {
                (crate::vm::op::TypeTag::Any | crate::vm::op::TypeTag::Other, _) => true,
                (crate::vm::op::TypeTag::Int | crate::vm::op::TypeTag::UInt, v) => {
                    matches!(v, Value::Int(_) | Value::UInt(_))
                }
                (t, v) => t.matches(v),
            };
            if !tag_ok {
                return false;
            }
            // ── `Other` タグの追加判定（A-4）──────────────────────────────
            //
            // `TypeTag` は `int`/`float`/`str`/`bool` しか区別しないので、**クラス型・
            // trait 型・protocol 型のフィールドは `Other` に落ちて素通り**していた。
            // そのため A-3 の後もまだ次が黙って通っていた:
            //
            //     mut xs: list = [Cat(2), 3]   # 要素型なし ⇒ xs[0] は Unresolved
            //     h.pet = xs[0]                # pet: Dog なのに Cat が入る
            //
            // ⚠ **判定するのは値が `Value::Instance` のときだけ。** `CsObject`・
            //    ネイティブハンドル・`None` などは判定材料が無いので通す（取りこぼす方へ倒す）。
            if let Value::Instance(v_rc) = &val {
                let ok = match self.class.field_checks.get(idx) {
                    // ⚠ Arrow はクラス継承を許さない（trait だけが base）ので基本は完全一致。
                    //   ⚠ ただし `Exception` の欄はすべての例外を受ける（`ClassValue::is_a`・10-19）。
                    Some(FieldCheck::Class(expected)) => {
                        let vb = v_rc.borrow();
                        vb.class.is_a(expected)
                            // new_type ラッパは基底名でも受ける（`new_type M: Dog` を Dog へ）。
                            || vb.class.new_type_base.as_deref() == Some(&**expected)
                    }
                    // trait 型は実装クラスを受ける（自身が同名のこともある）。
                    Some(FieldCheck::Trait(expected)) => {
                        let vb = v_rc.borrow();
                        vb.class.name.as_str() == &**expected
                            || vb.class.bases.iter().any(|b| b.as_str() == &**expected)
                    }
                    // protocol は**構造的**に見る（必須メンバーが全て在るか）。
                    Some(FieldCheck::Protocol(req)) => {
                        let vb = v_rc.borrow();
                        req.iter().all(|m| {
                            vb.class.field_index.contains_key(m.as_str())
                                || vb.class.methods.contains_key(m)
                        })
                    }
                    _ => true,
                };
                if !ok {
                    return false;
                }
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
// コンパイル済みコードが番地から直接読み書きするクラス（copy_alloc_speed_plan.md 8-2・D-13）
// ---------------------------------------------------------------------------

/// クラスの宣言から静的に求めた raw レイアウト（コンパイル済みコードが位置と幅を決めるのに使う・D-13 ②）。
#[derive(Debug, Clone)]
pub struct StaticRawClass {
    pub layout: RawLayout,
    /// インスタンスのフィールド（宣言順 = slot 順）: (名前, `mut` か)。
    pub fields: Vec<(String, bool)>,
    /// インスタンスのフィールドが全部 public か。そうでないクラスは番地を返すコールバックが必ず空を返すので
    /// （[`ClassValue::matches_direct_desc`]）、生成コードは直接の道を出さない（無駄なコールバックを呼ばない・8-7）。
    pub all_public: bool,
    /// `__init__` がパーサの合成したもの（[`synthesized_init_fields`]）なら、仮引数の順に格納するフィールドの slot。
    /// 生成コードが数値の引数を番地へ直接書いて作るのに使う（copy_alloc_speed_plan.md 7-11）。⚠ ヒントであり、実行時の
    /// 計画と同じ順かは生成のコールバック（`ar_new_instance`）が確かめる。
    pub auto_init: Option<Vec<usize>>,
}

/// `__init__`（仮引数 `params`・本体 `body`）がパーサの合成したもの（`parser::classes::generate_auto_init_if_needed`）なら、
/// 仮引数（`self` を除く）の順に、格納するフィールドの名前（`self.f = f` の `f`）。合成したものでなければ `None`。
/// 実行時の直接格納の計画（`Interpreter::build_auto_init_plan`）と、コンパイル済みコードの直接の生成（copy_alloc_speed_plan.md
/// 7-11・[`static_class_layout`]）が同じ判定を使う。
/// ⚠ 合成したものに限る（native_object_access_plan.md U-4 案 1）: 利用者が書いた同じ形の `__init__` は今までどおり呼ぶ
///   （トレースバック・デバッガに出る）。目印は文の位置: 合成した文はすべて `Span::unknown()`（行 0）で、パーサが読んだ文は
///   必ず位置を持つ。トレイトのフィールドへの格納（`self::T.f = f`）を含むものは `None`。
pub fn synthesized_init_fields<'p>(params: &'p [crate::ast::Param], body: &[crate::ast::Stmt]) -> Option<Vec<&'p str>> {
    use crate::ast::{Expr, Stmt};
    if !params.first().is_some_and(|p| p.name == "self") {
        return None;
    }
    let params = &params[1..];
    if params.iter().any(|p| p.variadic || p.default.is_some()) || body.len() != params.len() {
        return None;
    }
    body.iter()
        .zip(params)
        .map(|(st, p)| {
            let Stmt::AttrAssign { target: Expr::Attr { object, attr, .. }, value: Expr::Ident { name, .. }, span } = st else {
                return None;
            };
            let self_obj = matches!(object.as_ref(), Expr::Ident { name: s, .. } if s == "self");
            (self_obj && span.line == 0 && attr == &p.name && name == &p.name).then_some(p.name.as_str())
        })
        .collect()
}

/// クラスの宣言（`Stmt::ClassDef` の `bases` と `body`）から raw レイアウトを求める。持たないクラスは `None`。
///
/// 実行時（`exec_class_def`）と同じ規則: インスタンスのフィールドは `const` / `static mut` 以外の `Stmt::Field` で、
/// 判定は [`RawLayout::for_class`]。⚠ 静的な答えは**ヒント**で、実行時のクラスと合っているかは番地を返すコールバックが
/// 確かめる（[`ClassValue::matches_direct_desc`]。合わなければ直接の道を使わない）。
pub fn static_class_layout(bases: &[String], body: &[crate::ast::Stmt]) -> Option<StaticRawClass> {
    use crate::ast::{FieldKind, Stmt};
    let mut own: Vec<(String, String)> = Vec::new();
    let mut fields: Vec<(String, bool)> = Vec::new();
    let mut all_public = true;
    for st in body {
        if let Stmt::Field { name, kind, type_ann, access, .. } = st {
            if matches!(kind, FieldKind::Const | FieldKind::StaticMut) {
                continue;
            }
            all_public &= *access == crate::ast::Accessibility::Public;
            own.push((name.clone(), type_ann.clone()));
            fields.push((name.clone(), *kind == FieldKind::Mut));
        }
    }
    let layout = RawLayout::for_class(bases.is_empty(), &own, own.len())?;
    // 合成した `__init__` の格納の順（7-11）。`__init__` が 1 つだけで、格納先がどれもインスタンスのフィールドのとき
    // （`const` / `static mut` への格納は格納ではない・実行時の計画と同じ）
    let mut inits = body.iter().filter_map(|st| match st {
        Stmt::FnDef { name, params, body, .. } if name == "__init__" => Some((params, body)),
        _ => None,
    });
    let auto_init = match (inits.next(), inits.next()) {
        (Some((params, fbody)), None) => synthesized_init_fields(params, fbody).and_then(|names| {
            names.iter().map(|n| fields.iter().position(|(f, _)| f == n)).collect::<Option<Vec<usize>>>()
        }),
        _ => None,
    };
    Some(StaticRawClass { layout, fields, all_public, auto_init })
}

/// 生成コードが DLL の中に持つ「このクラスだと思って読み書きする」表の先頭（8-2・D-13 ②）。
/// 直後に `n_fields` 個の [`DirectFieldDesc`] が続く（生成コードは 1 つの定数として出す）。
#[repr(C)]
pub struct DirectClassDesc {
    pub name: *const u8,
    pub name_len: i64,
    pub n_fields: i64,
}

/// [`DirectClassDesc`] に続くフィールド 1 つ分。
#[repr(C)]
pub struct DirectFieldDesc {
    pub name: *const u8,
    pub name_len: i64,
    /// [`direct_field_info`] の形。
    pub info: u64,
}

/// [`DirectFieldDesc::info`] の詰め方（**唯一の定義**）: bit 0〜15 = 塊の先頭からの位置・bit 16〜19 = 幅の番号
/// （[`RawWidth::code`]）・bit 20 = `mut` のフィールド。
pub fn direct_field_info(desc: RawFieldDesc, mutable: bool) -> u64 {
    block::field_offset(desc.byte_offset) as u64 | (desc.width.code() << 16) | (u64::from(mutable) << 20)
}

impl ClassValue {
    /// このクラスが、生成コードの表（`name` と `fields` = (名前, [`direct_field_info`])）のとおりか（8-2・D-13 ②）。
    ///
    /// 番地を返すコールバックが、箇所ごとのセルに覚える前に 1 回だけ確かめる。確かめること: クラスの名前・raw レイアウトを
    /// 持つこと・フィールドの数・フィールドごとの名前と slot・位置・幅・`mut` か・public であること。1 つでも違えば `false`
    /// （生成コードは今までのコールバックの道を使う）。
    /// ⚠ 名前だけで照合しない: 別のモジュールの同じ名前のクラスが来ても、並びが 1 つでも違えば直接の道には入らない。
    pub fn matches_direct_desc(&self, name: &str, fields: &[(&str, u64)]) -> bool {
        let Some(layout) = self.raw_layout.as_ref() else { return false };
        if self.name.as_str() != name || layout.fields.len() != fields.len() || self.field_count != fields.len() {
            return false;
        }
        fields.iter().enumerate().all(|(idx, (fname, info))| {
            self.field_index.get(*fname) == Some(&idx)
                && !self.field_access.contains_key(*fname)
                && self
                    .field_mutability_vec
                    .get(idx)
                    .is_some_and(|m| direct_field_info(layout.fields[idx], *m) == *info)
        })
    }
}


// ---------------------------------------------------------------------------
// C ABI 構造体ポインタ引数のシャドウ変換（.claude/skills/c-abi-interop/SKILL.md P3）
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


#[cfg(test)]
mod c_size_tests {
    use super::*;

    /// C の sizeof（配列の要素の間隔・4-6）。`total_bytes` は 8 の倍数に切り上げてあるので別物。
    #[test]
    fn c_size_is_sizeof() {
        let rgba = RawLayout::from_fields(&[
            ("r".into(), "uint8".into()),
            ("g".into(), "uint8".into()),
            ("b".into(), "uint8".into()),
            ("a".into(), "uint8".into()),
        ])
        .unwrap();
        assert_eq!((rgba.c_size(), rgba.total_bytes), (4, 8));
        let pair = RawLayout::from_fields(&[("x".into(), "int32".into()), ("y".into(), "float64".into())]).unwrap();
        assert_eq!(pair.c_size(), 16);
    }
}


#[cfg(test)]
mod direct_access_tests {
    use super::*;

    /// 生成コードが出すのと同じ手順の読み出し（8-3 の IR はこの順）: 先頭の語の初期化の印を見て、立っていれば
    /// 塊の先頭から `info` の位置を幅どおりに読む。立っていなければ `None`（生成コードは今までのコールバックへ回す）。
    fn direct_read(blk: *const u64, idx: usize, info: u64) -> Option<Value> {
        let head = unsafe { *blk };
        if head & block::init_bit(idx) == 0 {
            return None;
        }
        let at = unsafe { (blk as *const u8).add((info & 0xFFFF) as usize) };
        macro_rules! rd {
            ($t:ty) => {
                unsafe { at.cast::<$t>().read_unaligned() }
            };
        }
        Some(match (info >> 16) & 0xF {
            1 => Value::Int(rd!(i8) as i64),
            2 => Value::Int(rd!(i16) as i64),
            3 => Value::Int(rd!(i32) as i64),
            4 => Value::Int(rd!(i64)),
            5 => Value::Int(rd!(u8) as i64),
            6 => Value::Int(rd!(u16) as i64),
            7 => Value::Int(rd!(u32) as i64),
            8 => Value::Int(rd!(u64) as i64),
            9 => Value::Float(rd!(f32) as f64),
            10 => Value::Float(rd!(f64)),
            other => panic!("unknown width code {other}"),
        })
    }

    /// 同じく書き込み: 不変の印が立っているか `mut` のフィールドでなければ書かない（`false`・今までのコールバックへ）。
    /// 書いたら初期化の印を立てる。`int` の値を `float` のフィールドへ書くときは変換する。
    fn direct_write(blk: *mut u64, idx: usize, info: u64, val: &Value) -> bool {
        let head = unsafe { *blk };
        if head & block::IMMUTABLE != 0 || info & (1 << 20) == 0 {
            return false;
        }
        let at = unsafe { (blk as *mut u8).add((info & 0xFFFF) as usize) };
        macro_rules! wr {
            ($t:ty, $v:expr) => {
                unsafe { at.cast::<$t>().write_unaligned($v) }
            };
        }
        match ((info >> 16) & 0xF, val) {
            (1 | 5, Value::Int(n)) => wr!(u8, *n as u8),
            (2 | 6, Value::Int(n)) => wr!(u16, *n as u16),
            (3 | 7, Value::Int(n)) => wr!(u32, *n as u32),
            (4 | 8, Value::Int(n)) => wr!(i64, *n),
            (9, Value::Float(f)) => wr!(f32, *f as f32),
            (9, Value::Int(n)) => wr!(f32, *n as f32),
            (10, Value::Float(f)) => wr!(f64, *f),
            (10, Value::Int(n)) => wr!(f64, *n as f64),
            _ => return false,
        }
        unsafe { *blk = head | block::init_bit(idx) };
        true
    }

    fn same(a: &Option<Value>, b: &Option<Value>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(Value::Int(x)), Some(Value::Int(y))) => x == y,
            (Some(Value::Float(x)), Some(Value::Float(y))) => x.to_bits() == y.to_bits(),
            _ => false,
        }
    }

    const ANNS: [&str; 12] = [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64", "int", "float",
    ];

    fn all_widths_class() -> Rc<ClassValue> {
        let own: Vec<(String, String)> = ANNS.iter().enumerate().map(|(i, a)| (format!("f{i}"), a.to_string())).collect();
        Rc::new(ClassValue {
            field_index: own.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect(),
            field_count: own.len(),
            field_mutability_vec: vec![true; own.len()],
            raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
            ..ClassValue::synthetic("AllWidths", alloc_class_id())
        })
    }

    /// 生成コードと同じ手順（塊の番地・`block` の定数・`direct_field_info`）の読み書きが、`field_value` / `store_field` と
    /// 同じ値になる（8-2）: 幅ごと・未初期化・不変のインスタンス。⚠ 塊の並びか詰め方を変えるとここが落ちる。
    #[test]
    fn direct_access_matches_field_value() {
        let cls = all_widths_class();
        let layout = cls.raw_layout.clone().expect("all fields are numeric");
        let infos: Vec<u64> = layout.fields.iter().map(|d| direct_field_info(*d, true)).collect();
        // 幅ごとの値（符号・切り詰め・丸めが出るもの）
        let samples = [
            Value::Int(-1),
            Value::Int(0x1_23),
            Value::Int(i64::MIN + 5),
            Value::Int(0x7FFF_FFFF_FFFF),
            Value::Float(-2.75),
            Value::Float(1.0e-3),
        ];

        // 1. `store_field` で書いたものを直接読む
        for v in &samples {
            let mut inst = InstanceData::new_empty(cls.clone(), 0);
            for idx in 0..ANNS.len() {
                assert!(same(&direct_read(inst.raw.as_ptr(), idx, infos[idx]), &None), "{} starts uninitialized", ANNS[idx]);
                if !inst.store_field(idx, v.clone(), true) {
                    continue; // `float` の値は整数のフィールドに入らない
                }
                let direct = direct_read(inst.raw.as_ptr(), idx, infos[idx]);
                assert!(same(&direct, &inst.field_value(idx)), "{} <- {v:?}: direct {direct:?}", ANNS[idx]);
            }
        }

        // 2. 直接書いたものを `field_value` で読む（`store_field` で書いたインスタンスと同じ値・同じ可否）
        for v in &samples {
            let mut a = InstanceData::new_empty(cls.clone(), 0);
            let mut b = InstanceData::new_empty(cls.clone(), 0);
            for idx in 0..ANNS.len() {
                let wrote = direct_write(a.raw.as_mut_ptr(), idx, infos[idx], v);
                assert_eq!(wrote, b.store_field(idx, v.clone(), true), "{} <- {v:?}", ANNS[idx]);
                assert!(same(&a.field_value(idx), &b.field_value(idx)), "{} <- {v:?}", ANNS[idx]);
                assert_eq!(a.slot_initialized(idx), wrote);
            }
            assert_eq!(a.raw, b.raw, "the whole block agrees after writing {v:?}");
        }

        // 3. 不変のインスタンス・`let` のフィールドへは直接書かない（今までのコールバックがエラーにする）
        let mut frozen = InstanceData::new_empty(cls.clone(), INST_IMMUTABLE);
        assert!(!direct_write(frozen.raw.as_mut_ptr(), 0, infos[0], &Value::Int(1)));
        assert!(!frozen.slot_initialized(0));
        let mut inst = InstanceData::new_empty(cls.clone(), 0);
        let let_info = direct_field_info(layout.fields[0], false);
        assert!(!direct_write(inst.raw.as_mut_ptr(), 0, let_info, &Value::Int(1)));
        // 不変でも読み出しは直接できる
        assert!(inst.store_field(3, Value::Int(9), true));
        inst.flags_or(INST_IMMUTABLE);
        assert!(same(&direct_read(inst.raw.as_ptr(), 3, infos[3]), &Some(Value::Int(9))));
    }
}
