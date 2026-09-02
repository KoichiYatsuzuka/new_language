// ops/hash.rs — すべての `Value` に対する既定のハッシュ: default_hash。
//
// ⚠⚠ **設計規則はただ 1 つ**: [`Interpreter::values_eq`] と 1:1 に対応させること。
//    `values_eq(a, b) == true` ⇒ `default_hash(a) == default_hash(b)` が破れた瞬間に
//    辞書は壊れる（入れたのに引けない）。`equality.rs` の腕を足したら**必ずここも足す**。
//
// ## なぜ Python の mod (2^61 - 1) 準同型が要らないのか
//
// CPython の数値ハッシュは有理数から `Z/P`（P = 2^61-1）への環準同型で、
// `hash(1) == hash(1.0) == hash(True) == hash(Fraction(1,1))` を成立させるためだけに
// あの複雑さがある。Python は `1 == 1.0 == True` が真なので、そうしないと辞書が壊れる。
//
// Arrow は B2-b で**値の同一性を型厳密**にした（`Int(1)` と `Float(1.0)` は等値でない。
// 昇格は「式としての `==`」だけの規則で [`Interpreter::values_eq_expr`] が担う）。
// ⇒ 型が違えば等値にならないので、**判別子 + ビットパターン**で十分。
//
// ## 打ち切ってよい／よくないの非対称性
//
// ⚠ **ハッシュは深さで打ち切ってよい**。衝突は正しさを壊さないし、打ち切りが決定的なら
//   等値な 2 値は同じところで打ち切られて同じハッシュになる。
// ⚠ **`values_eq` は打ち切ってはいけない**（`false` を返すとサイレントな誤答）。
//   ⇒ あちらは深さ上限で `RecursionError`、こちらは黙って浅く畳む。この差は意図的。

// ## 段階（B1-b: 実装、B1-c: 配線）
//
// ⚠ **B1-b では辞書に載せない。** ここは「`values_eq` と 1:1 のハッシュを定義する」段で、
//   実行時の挙動は 1 バイトも変わらない（消費者はテストだけ）。辞書のコンテナ差し替えは
//   B1-c で行う。⇒ それまで未使用の項目が出るので、モジュール単位で dead_code を許す
//   （`type_check::annotations::Directive` と同じ段階分けの書き方）。
#![allow(dead_code)]

use {
    std::collections::hash_map::RandomState,
    std::hash::{BuildHasher, Hasher},
    std::sync::OnceLock,
    crate::interpreter::{Interpreter, Value},
};

/// ハッシュの再帰の深さ上限。超えたら**降下をやめる**（エラーにはしない。上の doc を参照）。
const HASH_MAX_DEPTH: u32 = 64;

/// プロセスごとに 1 度だけ作るハッシュシード。
///
/// ⚠ **固定シードにしてはいけない**。外部入力の文字列キーで HashDoS を食らう
/// （Python が `PYTHONHASHSEED` を既定でランダム化しているのと同じ理由）。
///
/// ⚠ `Interpreter` に持たせず **プロセス全体で 1 つ**にしてあるのは、
/// インタプリタを持たない経路（`Value::deep_clone` / native ABI コールバック）からも
/// 同じシードで引けるようにするため。async のワーカースレッドも同一プロセスなので、
/// deep-clone して送った辞書のハッシュがスレッドを跨いで有効なままになる。
///
/// ⚠ **観測可能な出力は変わらない**。辞書は `IndexMap` で挿入順を保持するので、
/// シードが変わっても印字順・反復順は不変。⇒ ゴールデン系ゲートに影響しない。
fn seed() -> &'static RandomState {
    static SEED: OnceLock<RandomState> = OnceLock::new();
    SEED.get_or_init(RandomState::new)
}

/// バリアント判別子。**型が違えば必ず違う値**を混ぜるために使う。
///
/// ⚠ 値そのものが同じでも型が違えば等値にならない（型厳密）ので、
/// 判別子を混ぜないと `Int(1)` と `Bool(true)`、`[1,2]` と `(1,2)` のような
/// 「等値でないのに同じハッシュ」を無駄に量産する（正しさは壊れないが衝突が増える）。
#[repr(u8)]
enum Tag {
    Int = 1,
    UInt,
    Float,
    Complex,
    Str,
    Bool,
    None,
    Undefined,
    Instance,
    EnumItem,
    Type,
    Trait,
    Protocol,
    Class,
    Tuple,
    List,
    FrozenList,
    Set,
    Dict,
    AsyncStatus,
    Result,
    Slice,
    JsProcFn,
    CsObject,
    Pointer,
    /// 深さ上限に達して降下をやめた印。
    Truncated,
    /// `Slice` の `None` 要素など「無い」ことの印。
    Absent,
}

/// `default_hash` が失敗する理由。
#[derive(Debug)]
pub(crate) enum HashError {
    /// `__hash__` を定義したクラスのインスタンスに当たった。
    ///
    /// ⚠ ここでインタプリタを回すことはできない（`default_hash` は
    /// `Value::deep_clone` や `extern "C"` コールバックからも呼ばれる**純粋な経路**）。
    /// ⇒ 呼び出し側がインタプリタを持っているなら `__hash__` をディスパッチし、
    /// 持っていないならエラーとして扱う。実際のディスパッチは B1-c で配線する。
    NeedsDispatch,
}

impl Interpreter {
    /// すべての `Value` に対する既定のハッシュ。
    ///
    /// ⚠ [`Interpreter::values_eq`]（**型厳密**な値の同一性）と 1:1 に対応する。
    /// 式としての比較（[`Interpreter::values_eq_expr`]）の数値昇格は**反映しない** —
    /// 昇格は比較の場所の規則であって、値の同一性ではないため。
    ///
    /// - `v`: ハッシュする値
    ///
    /// 戻り値: `Ok(u64)`、または `__hash__` を持つインスタンスに当たったら
    /// [`HashError::NeedsDispatch`]。
    pub(crate) fn default_hash(v: &Value) -> Result<u64, HashError> {
        let mut h = seed().build_hasher();
        Self::hash_into(v, &mut h, 0)?;
        Ok(h.finish())
    }

    /// `v` を `h` に混ぜ込む。`depth` が上限を超えたら降下をやめる。
    fn hash_into<H: Hasher>(v: &Value, h: &mut H, depth: u32) -> Result<(), HashError> {
        if depth > HASH_MAX_DEPTH {
            h.write_u8(Tag::Truncated as u8);
            return Ok(());
        }
        let d = depth + 1;
        match v {
            // ── プリミティブ（型厳密。判別子で型を分ける）────────────────────
            Value::Int(n) => {
                h.write_u8(Tag::Int as u8);
                h.write_i64(*n);
            }
            Value::UInt(n) => {
                h.write_u8(Tag::UInt as u8);
                h.write_u64(*n);
            }
            Value::Float(f) => {
                h.write_u8(Tag::Float as u8);
                h.write_u64(canonical_f64_bits(*f));
            }
            Value::Complex(re, im) => {
                h.write_u8(Tag::Complex as u8);
                h.write_u64(canonical_f64_bits(*re));
                h.write_u64(canonical_f64_bits(*im));
            }
            Value::Str(s) => {
                h.write_u8(Tag::Str as u8);
                h.write(s.as_bytes());
            }
            Value::Bool(b) => {
                h.write_u8(Tag::Bool as u8);
                h.write_u8(u8::from(*b));
            }
            Value::None => h.write_u8(Tag::None as u8),
            Value::Undefined => h.write_u8(Tag::Undefined as u8),

            // ── インスタンス（構造ハッシュ）──────────────────────────────────
            // ⚠ **ポインタで済ませてはいけない**。`values_eq` は `Rc::ptr_eq` が外れても
            //   「同じクラス名 ＋ 全フィールドが等値」なら真を返す（構造的等値）ので、
            //   ポインタでハッシュすると等値なのにハッシュが違う組が生まれる。
            // ⚠ 構造ハッシュなので **`deep_clone` がハッシュを保存する**。これが
            //   「複製経路は保存済みハッシュをそのままコピーしてよい」根拠になる（B1-c）。
            Value::Instance(rc) => {
                let inst = rc.borrow();
                // ⚠ `__hash__` を持つクラスは呼び出し側に委ねる（純粋な経路では回せない）。
                if inst.class.methods.contains_key("__hash__") {
                    return Err(HashError::NeedsDispatch);
                }
                if inst.class.name.starts_with("enum_item_") {
                    // enum バリアントは `values_eq` が `value` フィールドだけを見る。
                    h.write_u8(Tag::EnumItem as u8);
                    h.write(inst.class.name.as_bytes());
                    match inst.class.field_index.get("value").and_then(|&i| inst.field_value(i)) {
                        Some(val) => Self::hash_into(&val, h, d)?,
                        None => h.write_u8(Tag::Absent as u8),
                    }
                } else {
                    h.write_u8(Tag::Instance as u8);
                    h.write(inst.class.name.as_bytes());
                    h.write_usize(inst.field_count());
                    // ⚠ `values_eq` は未初期化スロット（`None`）も突き合わせるので、こちらも
                    //   「無い」ことを混ぜる。片方だけ見ると等値な組でハッシュがずれる。
                    for i in 0..inst.field_count() {
                        match inst.field_value(i) {
                            Some(val) => Self::hash_into(&val, h, d)?,
                            None => h.write_u8(Tag::Absent as u8),
                        }
                    }
                }
            }

            // ── 型・トレイト・クラス ─────────────────────────────────────────
            Value::Type(s) => {
                h.write_u8(Tag::Type as u8);
                h.write(s.as_bytes());
            }
            Value::Trait(s) => {
                h.write_u8(Tag::Trait as u8);
                h.write(s.as_bytes());
            }
            Value::Protocol(s) => {
                h.write_u8(Tag::Protocol as u8);
                h.write(s.as_bytes());
            }
            // ⚠ `values_eq` が `class_id` で比べる（`Rc::ptr_eq` は async の deep-clone で
            //   壊れる）ので、こちらも `class_id`。ポインタにすると規則がずれる。
            Value::Class(c) => {
                h.write_u8(Tag::Class as u8);
                h.write_u32(c.class_id);
            }

            // ── 順序ありの複合（位置依存）────────────────────────────────────
            Value::Tuple(t) => {
                h.write_u8(Tag::Tuple as u8);
                let vals = t.all_values();
                h.write_usize(vals.len());
                for x in vals {
                    Self::hash_into(x, h, d)?;
                }
            }
            Value::List(rc) => {
                h.write_u8(Tag::List as u8);
                let items = rc.borrow();
                h.write_usize(items.len());
                for x in items.iter() {
                    Self::hash_into(x, h, d)?;
                }
            }
            Value::FrozenList { state, layout } => {
                h.write_u8(Tag::FrozenList as u8);
                h.write(layout.class_name.as_bytes());
                let st = state.borrow();
                h.write_usize(st.len);
                for i in 0..st.len {
                    Self::hash_into(&layout.reconstruct_item(&st.data, i), h, d)?;
                }
            }

            // ── 順序なしの複合（可換に畳む）──────────────────────────────────
            // ⚠ `values_eq` が順序を見ないので、ハッシュも**順序に依存してはいけない**。
            //   ⇒ 要素ごとに撹拌してから XOR で畳む。素の XOR だと重複が打ち消し合って
            //   弱いので、Python の frozenset と同じく 1 段撹拌を挟む。
            Value::Set(rc) => {
                h.write_u8(Tag::Set as u8);
                let items = rc.borrow();
                let mut acc: u64 = 0;
                for x in items.iter() {
                    acc ^= scramble(sub_hash(x, d)?);
                }
                h.write_usize(items.len());
                h.write_u64(acc);
            }
            Value::Dict(rc) => {
                h.write_u8(Tag::Dict as u8);
                let dict = rc.borrow();
                let mut acc: u64 = 0;
                for (k, val) in dict.all_keys().into_iter().zip(dict.all_items()) {
                    // キーと値をまとめて 1 要素として撹拌する（対応関係を保つため）。
                    let mut pair = seed().build_hasher();
                    Self::hash_into(&k, &mut pair, d)?;
                    Self::hash_into(&val, &mut pair, d)?;
                    acc ^= scramble(pair.finish());
                }
                h.write_usize(dict.len());
                h.write_u64(acc);
            }

            // ── 構造を持つその他の値 ─────────────────────────────────────────
            Value::AsyncStatusVal(s) => {
                h.write_u8(Tag::AsyncStatus as u8);
                // ⚠ `AsyncStatus` は `Copy` ではないので、バリアントを数値へ写して混ぜる。
                //   `values_eq` が `==` で比べるのと同じ区別になっていればよい。
                h.write_u8(match s {
                    crate::interpreter::async_mgr::AsyncStatus::Waiting => 0,
                    crate::interpreter::async_mgr::AsyncStatus::Running => 1,
                    crate::interpreter::async_mgr::AsyncStatus::Done => 2,
                });
            }
            Value::ResultVal { ok, inner } => {
                h.write_u8(Tag::Result as u8);
                h.write_u8(u8::from(*ok));
                Self::hash_into(inner, h, d)?;
            }
            Value::Slice(s) => {
                h.write_u8(Tag::Slice as u8);
                for part in [&s.begin, &s.end, &s.step] {
                    match part {
                        Some(x) => Self::hash_into(x, h, d)?,
                        None => h.write_u8(Tag::Absent as u8),
                    }
                }
            }
            Value::JsProcFn(f) => {
                h.write_u8(Tag::JsProcFn as u8);
                h.write(f.bridge_key.as_bytes());
                h.write(f.module_name.as_bytes());
                h.write(f.fn_name.as_bytes());
            }
            Value::CsObject(o) => {
                h.write_u8(Tag::CsObject as u8);
                h.write(o.class_name.as_bytes());
                h.write_i64(o.handle);
            }

            // ── 参照の同一性で比べる値 ───────────────────────────────────────
            // ⚠ `values_eq` がポインタで比べるものは、ハッシュもポインタで取る。
            //   `deep_clone` するとポインタが変わるが、そのとき `values_eq` も等値でなくなる
            //   ので**規則は一致したまま**（`Instance` / `Class` とはここが違う）。
            Value::Function(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::OverloadedFn(v) => {
                h.write_u8(Tag::Pointer as u8);
                h.write_usize(v.len());
                for f in v {
                    h.write_usize(std::rc::Rc::as_ptr(f) as *const () as usize);
                }
            }
            Value::GeneratorFn(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::Generator(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::TemplateFn(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::TemplateClass(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::TemplateGenFn(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::Namespace(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::FileObject(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::AsyncManager(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::Signal(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::EventLoop(rc) => ptr_hash(h, std::rc::Rc::as_ptr(rc) as *const ()),
            Value::PyObject(a) => ptr_hash(h, std::sync::Arc::as_ptr(a) as *const ()),
            Value::NativeFunction(a) => ptr_hash(h, std::sync::Arc::as_ptr(a) as *const ()),
        }
        Ok(())
    }
}

/// 部分ハッシュ（順序なしの畳み込みで、要素ごとに独立した値が要るとき）。
fn sub_hash(v: &Value, depth: u32) -> Result<u64, HashError> {
    let mut h = seed().build_hasher();
    Interpreter::hash_into(v, &mut h, depth)?;
    Ok(h.finish())
}

/// ポインタを 4bit 右回転して混ぜる。
/// アラインメントで下位ビットが常に 0 になるのを散らす（CPython の `_Py_HashPointer` と同じ手）。
fn ptr_hash<H: Hasher>(h: &mut H, p: *const ()) {
    let y = p as usize as u64;
    h.write_u8(Tag::Pointer as u8);
    h.write_u64(y.rotate_right(4));
}

/// XOR で畳む前の撹拌（Python の frozenset と同じ形）。
/// 素の XOR は同じ値どうしが打ち消し合うので、1 段混ぜてから畳む。
fn scramble(x: u64) -> u64 {
    ((x ^ 89_869_747) ^ (x << 16)).wrapping_mul(3_644_798_167)
}

/// `f64` を「等値なら同じビット列」に正規化する。
///
/// ⚠ **`-0.0 == 0.0` は真**（IEEE-754）。ビットパターンをそのまま使うと
/// 等値なのにハッシュが違う組が生まれて辞書が壊れる。⇒ `-0.0` は `0.0` に潰す。
/// ⚠ `NaN` は自分自身と等値にならないので、ハッシュの一致を要求されない。
/// ここでは正規形の 1 つに畳む（辞書のキーとしては別途禁止されている）。
fn canonical_f64_bits(f: f64) -> u64 {
    if f == 0.0 {
        0.0f64.to_bits()
    } else if f.is_nan() {
        f64::NAN.to_bits()
    } else {
        f.to_bits()
    }
}
