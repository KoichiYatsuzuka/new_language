// ops/equality.rs — 値の等価判定: values_eq / values_eq_dyn / values_ref_eq。

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

    /// 2つの値が等値かどうかを**型厳密に**判定する。
    ///
    /// - プリミティブ型は**同じ型どうし**のみ比較する。⚠ `Int` と `Float`、`UInt` と `Int` は
    ///   **等値にならない**（B2-b で型厳密化）。数値の昇格は「式としての `==` / `!=`」の規則で、
    ///   [`Interpreter::values_eq_expr`] がそれを担う（`==` / `in` などはそちらを通る）。
    ///   ここを厳密にしておかないと、辞書のキーやハッシュと規則が食い違う。
    /// - `Instance` は参照が同一なら真、違えば同じクラス名＋全フィールドを再帰比較
    /// - `Class` は **`class_id`** で判定（下記の警告を参照）
    /// - `Tuple` / `List` / `Set` / `Dict` / `FrozenList` は要素を再帰的に比較
    /// - 関数・ジェネレータ・ハンドル類は**参照の同一性**で比較
    /// - 異なる型同士（例: int と str）は常に `false`
    ///
    /// ⚠⚠ **`Result` を返すのは再帰が深くなりうるため**（B2-a）。`List` / `Dict` の腕を
    /// 足すまでは再帰が必ず底を打っていた（`List` に腕が無く `_ => false` で止まっていた）。
    ///
    /// ⚠ 元の動機は**循環参照**だったが、格納時にディープコピーする規則（L4）が入って
    /// **循環そのものが作れなくなった**（`a.append(a)` は追加時点のスナップショットを入れる）。
    /// ⇒ 今この上限が効くのは**素直に深い入れ子**だけ。撤去はしていない — 深い入れ子は
    /// 依然として作れるし、上限は安価な安全網だから。
    ///
    /// ⚠⚠ **深さで打ち切って `false` を返してはいけない。** 等しいものを「等しくない」と
    /// 答える＝サイレントな誤答で、B2 で消したはずのバグがそのまま戻る。打ち切りは必ずエラー。
    ///
    /// ⚠ ユーザー定義の `__eq__` は**ここでは効かない**。効かせたい経路は
    /// [`Interpreter::values_eq_dyn`] を使うこと（`apply_binop` / `apply_binop_dyn` と同じ二層構造）。
    ///
    /// - `a`, `b`: 比較する2つの値
    ///
    /// 戻り値: `Ok(true)` — 等値、`Ok(false)` — 非等値、`Err` — 再帰が深すぎる
    pub(crate) fn values_eq(&self, a: &Value, b: &Value) -> Result<bool, String> {
        Self::values_eq_pure(a, b)
    }

    /// [`Interpreter::values_eq`] の**インタプリタ不要版**。
    ///
    /// ⚠ 構造比較は `self` を一切見ない（再帰しているだけ）ので、関連関数として公開する。
    /// 辞書は `Value::deep_clone` や `extern "C"` コールバックからも触られる — そこから
    /// キーの突き合わせをするのにインタプリタは持てない（B1-c）。
    /// ⚠ ユーザー定義の `__eq__` は**効かない**。効かせたい経路は
    /// [`Interpreter::values_eq_dyn`] を使うこと。
    pub(crate) fn values_eq_pure(a: &Value, b: &Value) -> Result<bool, String> {
        Self::values_eq_at(a, b, 0)
    }

    /// 再帰の深さを持ち回る [`Interpreter::values_eq`] の実体。
    fn values_eq_at(a: &Value, b: &Value, depth: u32) -> Result<bool, String> {
        if depth > Self::EQ_MAX_DEPTH {
            return Err(
                "RecursionError: maximum recursion depth exceeded while comparing values"
                    .to_string(),
            );
        }
        let d = depth + 1;
        let eq = match (a, b) {
            // ── プリミティブ（型厳密）─────────────────────────────────────────
            // ⚠ `(Int, Float)` / `(Float, Int)` の腕は **B2-b で意図的に外した**。
            //    以前はここで int を f64 へ昇格していたため、
            //    `9007199254740993 == 9007199254740992.0` が真になる（2^53 超で精度が落ちる）
            //    **非可逆な等値**だった。式としての `==` の昇格は `apply_binop` が行う。
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::UInt(a), Value::UInt(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Complex(r1, i1), Value::Complex(r2, i2)) => r1 == r2 && i1 == i2,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) => true,
            (Value::Undefined, Value::Undefined) => true,

            // ── インスタンス ──────────────────────────────────────────────────
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
                        (Some(va), Some(vb)) => Self::values_eq_at(&va, &vb, d)?,
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
                            (Some(va), Some(vb)) => Self::values_eq_at(&va, &vb, d)?,
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

            // ── 型・トレイト・クラス ──────────────────────────────────────────
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::Trait(a), Value::Trait(b)) => a == b,
            (Value::Protocol(a), Value::Protocol(b)) => a == b,
            // ⚠⚠ **`Rc::ptr_eq` にしてはいけない**（B2-b）。`ClassValue::deep_clone` が
            //    クラスを複製する経路が実在し（async は share-nothing で全値を `deep_clone`
            //    する）、**スレッドを跨いだ瞬間に `C == C` が False になる**。実測で確認済み:
            //    同一スレッドでは True、`mng <- async->bool: block_return C == K` では False。
            //    `class_id` は `alloc_class_id()` が発行する一意 ID で、`deep_clone` が
            //    そのまま引き継ぐ（`ClassValue::deep_clone` を参照）ので、複製を跨いで安定する。
            (Value::Class(a), Value::Class(b)) => a.class_id == b.class_id,

            // ── 順序ありの複合 ────────────────────────────────────────────────
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
                Self::seq_eq(av.iter().zip(bv.iter()), d)?
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
                Self::seq_eq(ar.iter().zip(br.iter()), d)?
            }
            // `fixed_list` も**リストとして**構造比較する（`List` と規則を揃える）。
            // 要素はフラットバイト列なので、`reconstruct_item` で復元してから比べる。
            (
                Value::FrozenList { state: sa, layout: la },
                Value::FrozenList { state: sb, layout: lb },
            ) => {
                if Rc::ptr_eq(sa, sb) {
                    return Ok(true);
                }
                let ra = sa.borrow();
                let rb = sb.borrow();
                if ra.len != rb.len || la.class_name != lb.class_name {
                    return Ok(false);
                }
                let mut all = true;
                for i in 0..ra.len {
                    let x = la.reconstruct_item(&ra.data, i);
                    let y = lb.reconstruct_item(&rb.data, i);
                    if !Self::values_eq_at(&x, &y, d)? {
                        all = false;
                        break;
                    }
                }
                all
            }

            // ── 順序なしの複合 ────────────────────────────────────────────────
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
                        if Self::values_eq_at(v, w, d)? {
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
                for (hk, va) in ar.iter() {
                    // ⚠ **保存済みハッシュをそのまま使う**。取り直す必要が無いだけでなく、
                    //    等値判定に現在の深さ `d` を渡せるので**深さ上限が効いたまま**になる
                    //    （`dict_get_pure` 経由だと深さが 0 に戻り、循環で止まらなくなる）。
                    let idx = br.index_of_with(hk.hash, |stored| {
                        Self::values_eq_at(&hk.key, stored, d)
                    })?;
                    let same = match idx.and_then(|i| br.value_at(i)) {
                        Some(vb) => Self::values_eq_at(va, vb, d)?,
                        None => false,
                    };
                    if !same {
                        all = false;
                        break;
                    }
                }
                all
            }

            // ── 構造を持つその他の値 ──────────────────────────────────────────
            (Value::AsyncStatusVal(a), Value::AsyncStatusVal(b)) => a == b,
            (
                Value::ResultVal { ok: oa, inner: ia },
                Value::ResultVal { ok: ob, inner: ib },
            ) => oa == ob && Self::values_eq_at(ia, ib, d)?,
            (Value::Slice(a), Value::Slice(b)) => {
                if Rc::ptr_eq(a, b) {
                    return Ok(true);
                }
                Self::opt_eq(&a.begin, &b.begin, d)?
                    && Self::opt_eq(&a.end, &b.end, d)?
                    && Self::opt_eq(&a.step, &b.step, d)?
            }
            (Value::JsProcFn(a), Value::JsProcFn(b)) => {
                a.bridge_key == b.bridge_key
                    && a.module_name == b.module_name
                    && a.fn_name == b.fn_name
            }
            // C# 側の実体は `(class_name, handle)` で一意。`Rc` の同一性ではない
            // （同じオブジェクトへのハンドルが別々の `Rc` で来うる）。
            (Value::CsObject(a), Value::CsObject(b)) => {
                a.class_name == b.class_name && a.handle == b.handle
            }

            // ── 参照の同一性で比べる値 ────────────────────────────────────────
            // ⚠ これらの腕が**丸ごと無かった**ため、`_ => false` に落ちて
            //   **`f == f` すら False** だった（実測）。「あらゆる型を dict のキーに」の前提。
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::OverloadedFn(a), Value::OverloadedFn(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| Rc::ptr_eq(x, y))
            }
            (Value::GeneratorFn(a), Value::GeneratorFn(b)) => Rc::ptr_eq(a, b),
            (Value::Generator(a), Value::Generator(b)) => Rc::ptr_eq(a, b),
            (Value::TemplateFn(a), Value::TemplateFn(b)) => Rc::ptr_eq(a, b),
            (Value::TemplateClass(a), Value::TemplateClass(b)) => Rc::ptr_eq(a, b),
            (Value::TemplateGenFn(a), Value::TemplateGenFn(b)) => Rc::ptr_eq(a, b),
            (Value::Namespace(a), Value::Namespace(b)) => Rc::ptr_eq(a, b),
            (Value::FileObject(a), Value::FileObject(b)) => Rc::ptr_eq(a, b),
            (Value::AsyncManager(a), Value::AsyncManager(b)) => Rc::ptr_eq(a, b),
            (Value::Signal(a), Value::Signal(b)) => Rc::ptr_eq(a, b),
            (Value::EventLoop(a), Value::EventLoop(b)) => Rc::ptr_eq(a, b),
            (Value::PyObject(a), Value::PyObject(b)) => std::sync::Arc::ptr_eq(a, b),
            (Value::NativeFunction(a), Value::NativeFunction(b)) => std::sync::Arc::ptr_eq(a, b),

            _ => false,
        };
        Ok(eq)
    }

    /// `Option<Value>` どうしの等値（`Slice` の begin/end/step 用）。
    fn opt_eq(
        a: &Option<Value>,
        b: &Option<Value>,
        depth: u32,
    ) -> Result<bool, String> {
        match (a, b) {
            (None, None) => Ok(true),
            (Some(x), Some(y)) => Self::values_eq_at(x, y, depth),
            _ => Ok(false),
        }
    }

    /// ペアの列を短絡付きで全比較する。
    /// クロージャの中では `?` が使えないので、`all(..)` の代わりにこれを使う。
    fn seq_eq<'v, I>(pairs: I, depth: u32) -> Result<bool, String>
    where
        I: Iterator<Item = (&'v Value, &'v Value)>,
    {
        for (x, y) in pairs {
            if !Self::values_eq_at(x, y, depth)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 数値の昇格ラティス（`uint → int → float`）で等値比較する。
    /// 昇格が要らない（同種）／数値でない組は `None` を返し、通常の経路へ落とす。
    ///
    /// ⚠ **`bool` は昇格対象外**（`1 == True` は False）。
    /// ⚠ `uint` と `int` は**値で**比べる（`as` で潰すと `u64::MAX` 付近が壊れる）。
    /// ⚠⚠ これは「**式としての `==` / `!=`**」だけの規則。[`Interpreter::values_eq`] 自体は
    ///    型厳密なので、辞書のキーや包含検査では `1` と `1.0` は別物として扱われる（B2-b）。
    ///    `int → float` のキャストは 2^53 超で精度を落とすが、それは
    ///    「float へキャストして比べた」結果として正しい。等値の定義を歪めているわけではない。
    fn numeric_eq_promoted(a: &Value, b: &Value) -> Option<bool> {
        match (a, b) {
            (Value::Int(x), Value::Float(y)) => Some((*x as f64) == *y),
            (Value::Float(x), Value::Int(y)) => Some(*x == (*y as f64)),
            (Value::UInt(x), Value::Int(y)) => Some(*y >= 0 && *x == *y as u64),
            (Value::Int(x), Value::UInt(y)) => Some(*x >= 0 && *x as u64 == *y),
            (Value::UInt(x), Value::Float(y)) => Some((*x as f64) == *y),
            (Value::Float(x), Value::UInt(y)) => Some(*x == (*y as f64)),
            _ => None,
        }
    }

    /// **式としての等値**（`==` / `!=` / `in` / `not in` / set 演算が使う）。
    ///
    /// 数値の昇格ラティスを通してから [`Interpreter::values_eq`] に落とす。
    /// ⇒ ユーザーが比較を書いた場所では常に同じ規則になる。
    ///
    /// ⚠⚠ [`Interpreter::values_eq`] との使い分けが B2-b の要点:
    /// - `values_eq` … **値の同一性**（型厳密）。辞書のキーとハッシュがこちらを使う。
    ///   `d[1]` と `d[1.0]` を別キーにするための厳密さ（B1）。
    /// - `values_eq_expr` … **式としての比較**。オペランドをキャストしてから比べる。
    ///
    /// ⚠ 昇格は**比較のオペランドにだけ**掛かり、コンテナの中までは再帰しない
    ///   （「比較の場所でキャストする」規則の素直な帰結）。実測される差:
    ///   `1 == 1.0` は True だが `[1] == [1.0]` は False。
    pub(crate) fn values_eq_expr(&self, a: &Value, b: &Value) -> Result<bool, String> {
        match Self::numeric_eq_promoted(a, b) {
            Some(x) => Ok(x),
            None => self.values_eq(a, b),
        }
    }

    /// `hay` に `needle` と等値な要素があるか。
    ///
    /// ⚠ クロージャの中では `?` が使えないので、`.any(|v| values_eq(..))` の代わりに使う
    /// （`values_eq` が `Result` を返す理由は同関数の doc・B2-a）。
    pub(crate) fn contains_eq(&self, hay: &[Value], needle: &Value) -> Result<bool, String> {
        for v in hay {
            if self.values_eq_expr(v, needle)? {
                return Ok(true);
            }
        }
        Ok(false)
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

    // ── `__eq__` を尊重する動的版（B2-b）─────────────────────────────────────

    /// ユーザー定義の `__eq__` を尊重する等値判定。
    ///
    /// ⚠ [`Interpreter::apply_binop`] / [`Interpreter::apply_binop_dyn`] と**同じ二層構造**に
    /// してある。[`Interpreter::values_eq`] は `&self` の構造比較で、FFI 経路や
    /// `&self` しか持たない集合演算が使う。こちらは `__eq__` の呼び出しでインタプリタを
    /// 回すので `&mut self` が要る。⇒ **`__eq__` が効くべき経路だけ**がこちらを使う。
    ///
    /// ⚠⚠ 以前は `__eq__` が **`==` 演算子でしか効いていなかった**。`in` / set の包含検査は
    /// `values_eq` を直接呼んでいたため、**`a == b` は真なのに `a in [b]` は偽**という
    /// 食い違いが起きていた（実測）。dict の検索は `DictKey`（ハッシュ）で引くので
    /// ここを通らず、`__eq__` は今も効かない — 解消は B1-c（辞書コンテナの差し替え）で。
    pub(crate) fn values_eq_dyn(&mut self, a: &Value, b: &Value) -> Result<bool, String> {
        if let Value::Instance(inst) = a {
            let has_eq = inst.borrow().class.methods.contains_key("__eq__");
            if has_eq {
                let r = self.eval_method_call_evaled(
                    a.clone(),
                    "__eq__",
                    vec![(None, b.clone(), true)],
                )?;
                return match r {
                    Value::Bool(x) => Ok(x),
                    other => Err(format!(
                        "TypeError: __eq__ should return bool, not '{}'",
                        self.type_name(&other)
                    )),
                };
            }
        }
        self.values_eq_expr(a, b)
    }

    /// `hay` に `needle` と等値な要素があるか（`__eq__` を尊重する版）。
    /// ⚠ `__eq__` は**左オペランド**で引くので、`needle` を左にして呼ぶこと。
    pub(crate) fn contains_eq_dyn(
        &mut self,
        hay: &[Value],
        needle: &Value,
    ) -> Result<bool, String> {
        for v in hay {
            if self.values_eq_dyn(needle, v)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `hay` の中で `needle` と等値な最初の要素の位置（`__eq__` を尊重する版）。
    pub(crate) fn position_eq_dyn(
        &mut self,
        hay: &[Value],
        needle: &Value,
    ) -> Result<Option<usize>, String> {
        for (i, v) in hay.iter().enumerate() {
            if self.values_eq_dyn(needle, v)? {
                return Ok(Some(i));
            }
        }
        Ok(None)
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
            _ => self.values_eq_expr(a, b)?,
        })
    }
}
