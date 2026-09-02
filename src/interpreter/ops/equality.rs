// ops/equality.rs — 値の等価判定: values_eq / values_ref_eq。

use {
    std::rc::Rc,
    crate::interpreter::{Interpreter, Value},
};

impl Interpreter {
    /// `values_eq` の再帰の深さ上限。超えたら `RecursionError`。
    ///
    /// Python の既定の再帰上限（1000）に相当する役割だが、こちらは**ネイティブスタック**を
    /// 溢れさせないための値なので小さめに取る。実用上の入れ子の深さより十分大きい。
    const EQ_MAX_DEPTH: u32 = 200;

    /// 2つの値が等値かどうかを判定する（`==` / `!=` 演算子および包含検査で使用）。
    ///
    /// - プリミティブ型（int/float/str/bool/None）は値で比較。int と float は昇格して比較
    /// - `Instance` は参照が同一なら真、違えば同じクラス名＋全フィールドを再帰比較
    /// - `Class` は参照の同一性（ポインタ比較）で判定
    /// - `Tuple` / `List` / `Set` / `Dict` は要素を再帰的に比較
    /// - 異なる型同士（例: int と str）は常に `false`
    ///
    /// ⚠⚠ **`Result` を返すのは循環参照のため**（B2-a）。`List` / `Dict` の腕を足すまでは
    /// 再帰が必ず底を打っていた（`List` に腕が無く `_ => false` で止まっていた）が、
    /// 足した以上、**両方が循環している 2 つの値**を比べるとスタックが溢れる。
    /// Python も同じ構図で、①同一性の高速パス ②再帰深度上限 → `RecursionError` で処理する
    /// （`hash()` の側に循環検出が無いのは hashable ⇒ immutable ⇒ 循環を作れないから）。
    ///
    /// ⚠⚠ **深さで打ち切って `false` を返してはいけない。** 等しいものを「等しくない」と
    /// 答える＝サイレントな誤答で、B2 で消したはずのバグがそのまま戻る。打ち切りは必ずエラー。
    ///
    /// - `a`, `b`: 比較する2つの値
    ///
    /// 戻り値: `Ok(true)` — 等値、`Ok(false)` — 非等値、`Err` — 再帰が深すぎる
    pub(crate) fn values_eq(&self, a: &Value, b: &Value) -> Result<bool, String> {
        self.values_eq_at(a, b, 0)
    }

    /// 再帰の深さを持ち回る [`Interpreter::values_eq`] の実体。
    fn values_eq_at(&self, a: &Value, b: &Value, depth: u32) -> Result<bool, String> {
        if depth > Self::EQ_MAX_DEPTH {
            return Err(
                "RecursionError: maximum recursion depth exceeded while comparing values"
                    .to_string(),
            );
        }
        let d = depth + 1;
        let eq = match (a, b) {
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
                    return Ok(true);
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
                        (Some(va), Some(vb)) => self.values_eq_at(&va, &vb, d)?,
                        _ => false,
                    }
                } else {
                    // 構造的等値: 同じクラス名かつ全スロットが等値
                    if a_borrow.class.name != b_borrow.class.name
                        || a_borrow.field_count() != b_borrow.field_count()
                    {
                        return Ok(false);
                    }
                    let mut all = true;
                    for i in 0..a_borrow.field_count() {
                        let same = match (a_borrow.field_value(i), b_borrow.field_value(i)) {
                            (Some(va), Some(vb)) => self.values_eq_at(&va, &vb, d)?,
                            (None, None) => true,
                            _ => false,
                        };
                        if !same {
                            all = false;
                            break;
                        }
                    }
                    all
                }
            }
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            // タプルは要素数と各要素を再帰的に比較
            (Value::Tuple(a), Value::Tuple(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                let av = a.all_values();
                let bv = b.all_values();
                if av.len() != bv.len() {
                    return Ok(false);
                }
                self.seq_eq(av.iter().zip(bv.iter()), d)?
            }
            // リストは長さと各要素を**位置ごとに**再帰比較（B2）。
            //
            // ⚠ この腕が無く `_ => false` に落ちていたため、`[1,2] == [1,2]` はもちろん
            //   **`a == a`（同一オブジェクト）すら false** だった。参照比較になっていたのでは
            //   なく純粋な抜けで、エラーにならないので気付けなかった。
            // ⚠ `===`（[`Interpreter::values_ref_eq`]）とは**別物**。混ぜないこと。
            (Value::List(a), Value::List(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                let ar = a.borrow();
                let br = b.borrow();
                if ar.len() != br.len() {
                    return Ok(false);
                }
                self.seq_eq(ar.iter().zip(br.iter()), d)?
            }
            // セットは要素数と各要素の包含関係で比較（順序無関係）
            (Value::Set(a), Value::Set(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                let ar = a.borrow();
                let br = b.borrow();
                if ar.len() != br.len() {
                    return Ok(false);
                }
                let mut all = true;
                for v in ar.iter() {
                    let mut found = false;
                    for w in br.iter() {
                        if self.values_eq_at(v, w, d)? {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        all = false;
                        break;
                    }
                }
                all
            }
            // 辞書は要素数と、各キーに対応する値を再帰比較（順序無関係・B2）。
            // ⚠ キーは `DictKey` として既に一意なので、`a` の各キーが `b` にも在って
            //   値が等しいかだけ見ればよい（要素数が同じなら双方向を見る必要はない）。
            (Value::Dict(a), Value::Dict(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                let ar = a.borrow();
                let br = b.borrow();
                if ar.len() != br.len() {
                    return Ok(false);
                }
                let mut all = true;
                for (key, va) in ar.all_keys().into_iter().zip(ar.all_items()) {
                    let same = match br.get(&key) {
                        Some(vb) => self.values_eq_at(&va, &vb, d)?,
                        None => false,
                    };
                    if !same {
                        all = false;
                        break;
                    }
                }
                all
            }
            _ => false,
        };
        Ok(eq)
    }

    /// ペアの列を短絡付きで全比較する。
    /// クロージャの中では `?` が使えないので、`all(..)` の代わりにこれを使う。
    fn seq_eq<'v, I>(&self, pairs: I, depth: u32) -> Result<bool, String>
    where
        I: Iterator<Item = (&'v Value, &'v Value)>,
    {
        for (x, y) in pairs {
            if !self.values_eq_at(x, y, depth)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `hay` に `needle` と等値な要素があるか。
    ///
    /// ⚠ クロージャの中では `?` が使えないので、`.any(|v| values_eq(..))` の代わりに使う
    /// （`values_eq` が `Result` を返す理由は同関数の doc・B2-a）。
    pub(crate) fn contains_eq(&self, hay: &[Value], needle: &Value) -> Result<bool, String> {
        for v in hay {
            if self.values_eq(v, needle)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `hay` の中で `needle` と等値な最初の要素の位置。無ければ `None`。
    pub(crate) fn position_eq(
        &self,
        hay: &[Value],
        needle: &Value,
    ) -> Result<Option<usize>, String> {
        for (i, v) in hay.iter().enumerate() {
            if self.values_eq(v, needle)? {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    /// `hay` の全要素が `other` に含まれるか（`issubset` / `issuperset` の共通形）。
    pub(crate) fn all_contained(&self, hay: &[Value], other: &[Value]) -> Result<bool, String> {
        for v in hay {
            if !self.contains_eq(other, v)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `hay` のうち「`other` に含まれる／含まれない」要素だけを集める（集合演算の共通形）。
    /// `keep_when_present = true` なら積集合、`false` なら差集合。
    pub(crate) fn filter_by_membership(
        &self,
        hay: &[Value],
        other: &[Value],
        keep_when_present: bool,
    ) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        for v in hay {
            if self.contains_eq(other, v)? == keep_when_present {
                out.push(v.clone());
            }
        }
        Ok(out)
    }

    /// `===` 演算子: 参照の同一性のみで等値を判定する。
    ///
    /// - 参照型 (`Instance`, `Class`, `List`, `Dict`, `Set`) は `Rc::ptr_eq` でポインタを比較する。
    /// - 値型 (`Int`, `Float`, `Str` など) は参照の概念がないため `values_eq` と同じ挙動にする。
    pub(crate) fn values_ref_eq(&self, a: &Value, b: &Value) -> Result<bool, String> {
        Ok(match (a, b) {
            (Value::Instance(a), Value::Instance(b)) => Rc::ptr_eq(a, b),
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            (Value::List(a), Value::List(b)) => Rc::ptr_eq(a, b),
            (Value::Dict(a), Value::Dict(b)) => Rc::ptr_eq(a, b),
            (Value::Set(a), Value::Set(b)) => Rc::ptr_eq(a, b),
            _ => self.values_eq(a, b)?,
        })
    }
}
