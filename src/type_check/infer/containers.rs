// type_check/infer/containers.rs — 容器の要素の型・添字・反復の検査。

use super::*;

impl TypeChecker {
    /// `->Type` 注釈があれば解決した型を、なければ `Unresolved` を返す。
    /// `block`/`if`/`for`/`while`/`match` 式の結果型計算で共通に使う。
    /// コレクションリテラルの要素型を**合成**する（タスク 2.7・原因②）。
    ///
    /// - 全要素が同型 → その型
    /// - 混在 → `Union`（重複は畳む）
    /// - 1 つでも `Unresolved` → `Unresolved`（要素型が決められない）
    ///
    /// ⚠⚠ **「捨てる」ことは中立ではない。** 以前は混在を素の `List` に落としていたが、
    /// 素の `List` は `ListOf(任意)` と適合するので「**何でも通す**」になっていた。
    /// 合成すれば `[1, "s"]` は `list[Union[int, str]]` になり、`list[int]` とは適合しない。
    ///
    /// ⚠ `Unresolved` を返す場合は `ListOf(Unresolved)` になる。素の `List` と違い
    /// **構造は保つ**ので、タスク 4.1 で素の容器の双方向特例を撤去しても壊れない。
    pub(in crate::type_check) fn join_elem_types(types: Vec<InferredType>) -> InferredType {
        if types.iter().any(|t| matches!(t, InferredType::Unresolved)) {
            return InferredType::Unresolved;
        }
        let mut uniq: Vec<InferredType> = Vec::new();
        for t in types {
            if !uniq.contains(&t) {
                uniq.push(t);
            }
        }
        match uniq.len() {
            0 => InferredType::Unresolved,
            1 => uniq.remove(0),
            _ => InferredType::Union(uniq),
        }
    }

    /// 添字 `obj[i]` の**添字の型**を検査する（タスク 5.2b・検体 `L6` / `L7`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 容器 | 添字に許される型 | 外れたとき |
    /// |---|---|---|
    /// | `list` / `fixed_list` / `list_like` / `str` / `tuple` | `int` ・ `Index` ・ スライス | `TypeError` |
    /// | `dict[K, V]` | **`K` と同じ型** | `KeyError`（型エラーではない） |
    /// | `set` | **無し**（添字アクセス不可） | `TypeError` |
    ///
    /// ⚠⚠ **`bool` と `float` は添字にできない**（`xs[True]` も `xs[1.5]` も `TypeError`）。
    /// 真偽値が整数として通る言語の癖で書くと落ちるので、静的に弾く価値がある。
    ///
    /// ⚠⚠ **`dict` のキーに数値昇格は効かない。** `dict[float, int]` に `int` のキーで
    /// 引くと `KeyError` になる（実測）。等値比較は `uint → int → float` で昇格するのに
    /// **辞書引きはハッシュなので昇格しない**という食い違いがあるため、`==` と同じ規則で
    /// 判定してはいけない。
    /// 列リテラル（`list` / `set`）の要素型を集める。`*other` は**展開元の要素型**を持ち込む。
    ///
    /// ⚠⚠ **`Unresolved` を返さない**（`dict_spread_elem_types` と同じ判断）。
    /// `Unresolved` は下流で「万能受容体」になり、要素型の食い違いを素通しさせてしまう。
    /// 判らないときは `Any` に倒す —— こちらは明示ダウンキャストを要求する側なので
    /// 検査が消えない。
    ///
    /// ⚠ 展開元が反復できない型なら**その場で静的エラー**（実行時と同じ意味）。
    pub(super) fn seq_entry_elem_types(&mut self, entries: &[crate::ast::SeqEntry]) -> Vec<InferredType> {
        use InferredType as IT;
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            match e {
                crate::ast::SeqEntry::Item(x) => out.push(self.infer(x)),
                crate::ast::SeqEntry::Spread(x) => {
                    let ty = self.infer(x);
                    out.push(match &ty {
                        IT::ListOf(t) | IT::FixedListOf(t) | IT::ListLikeOf(t) | IT::SetOf(t) | IT::FrozenSetOf(t) => {
                            (**t).clone()
                        }
                        // 要素型を持たない容器。反復はできるので `Any`。
                        IT::List | IT::FixedList | IT::ListLike | IT::Set | IT::FrozenSet | IT::TupleAny => IT::Any,
                        // タプルは位置ごとの型を持つ。展開すると位置が崩れるので合成する。
                        IT::Tuple(ts) => Self::join_elem_types(ts.clone()),
                        // `str` を展開すると 1 文字ずつの `str`。
                        IT::Str => IT::Str,
                        // `bytes` を展開すると 1 バイトずつの `int`（タスク 3-1）。
                        IT::Bytes => IT::Int,
                        IT::Any | IT::Unresolved => IT::Any,
                        // クラスは `__iter__` を持ちうるので通す（実行時に判る）。
                        IT::NamedInstance(_) => IT::Any,
                        // 展開する式を指す（diagnostic_position_plan.md の 3-4。リストの表示は錨を持たない）。
                        other => {
                            self.report_error_at(x, StaticTypeError {
                                kind: TypeErrorKind::SpreadArgNotIterable {
                                    got: other.to_string(),
                                    kw: false,
                                },
                                span: None,
                            });
                            IT::Any
                        }
                    });
                }
            }
        }
        out
    }

    /// 辞書リテラル内の `**src` が持ち込む（キー型, 値型）を返す。
    ///
    /// ⚠⚠ **`Unresolved` を返さない**。`Unresolved` は下流で「万能受容体」として
    /// 振る舞い、`{**b, 3.0: 2}` のようなキー型の食い違いを**素通し**させてしまう。
    /// 判らないときは `Any` に倒す —— `Any` は「明示ダウンキャストを要求する」側なので、
    /// 検査を消さずに済む（`extract_py_type_stubs` の doc と同じ判断）。
    ///
    /// ⚠ 展開元が辞書でなければ**その場で静的エラー**（実行時 `TypeError` と同じ形）。
    pub(super) fn dict_spread_elem_types(
        &mut self,
        src: &Expr,
    ) -> (InferredType, InferredType) {
        let ty = self.infer(src);
        match &ty {
            InferredType::DictOf(k, v) => ((**k).clone(), (**v).clone()),
            // 要素型を持たない素の `dict`。要素は判らないが**辞書ではある**ので
            // `Any` として通す（`Unresolved` にすると検査が消える）。
            InferredType::Dict => (InferredType::Any, InferredType::Any),
            // ⚠ 型が判らない（`Any` / `Unresolved`）ときは検査できないが、
            //   **エラーにもしない**（`import` 越しの値など、注釈が供給されない経路がある）。
            InferredType::Any | InferredType::Unresolved => {
                (InferredType::Any, InferredType::Any)
            }
            // 展開する式を指す（3-4）。
            other => {
                self.report_error_at(src, StaticTypeError {
                    kind: TypeErrorKind::DictSpreadNotADict {
                        got: other.to_string(),
                    },
                    span: None,
                });
                (InferredType::Any, InferredType::Any)
            }
        }
    }

    pub(super) fn check_subscript_index(&mut self, obj_ty: &InferredType, idx_ty: &InferredType) {
        use InferredType as T;
        // 添字が不透明なら判定材料が無い（取りこぼす方へ倒す）。
        // ⚠ `NamedInstance` はここで落ちる。`Index` クラスとスライス（`NamedInstance("slice")`）
        //   も通したいので、クラスは一律で素通しにするのが都合もよい。
        if Self::opaque_for_subscript(idx_ty) {
            return;
        }
        match obj_ty {
            T::List | T::ListOf(_) | T::FixedList | T::FixedListOf(_) | T::ListLike
            | T::ListLikeOf(_) | T::PyList(_) | T::Str | T::Bytes | T::Tuple(_) => {
                self.check_int_position(idx_ty, &format!("index of `{obj_ty}`"));
            }
            T::DictOf(key, _) | T::PyDict(key, _) => {
                // ⚠ キー型が不透明な辞書（`dict[Any, V]`）は判定しない。
                if Self::opaque_for_subscript(key) {
                    return;
                }
                // ⚠ 記憶域の型なので `Site::Other`（`__cast__` は挿入されない）。
                if self.types_compatible(idx_ty, key, crate::type_check::type_utils::Site::Other) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::SubscriptTypeMismatch {
                        context: format!("key of `{obj_ty}`"),
                        expected: (**key).clone(),
                        got: idx_ty.clone(),
                    },
                    span: None,
                });
            }
            T::Set | T::SetOf(_) | T::FrozenSet | T::FrozenSetOf(_) | T::PySet(_) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NotSubscriptable { container: obj_ty.clone() },
                    span: None,
                });
            }
            // 素の `dict`（キー型不明）・クラス（`__getitem__` を定義できる）・
            // `Any` / `Unresolved` などは判定しない。
            _ => {}
        }
    }

    /// 「ここは `int` でなければならない」位置（添字・スライス境界）を検査する。
    pub(super) fn check_int_position(&mut self, ty: &InferredType, context: &str) {
        if Self::opaque_for_subscript(ty) || matches!(ty, InferredType::Int) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::SubscriptTypeMismatch {
                context: context.to_string(),
                expected: InferredType::Int,
                got: ty.clone(),
            },
            span: None,
        });
    }

    /// スライス値の型か（`xs[0:2]` の添字 / `slice(...)` の戻り値）。
    pub(super) fn is_slice_type(ty: &InferredType) -> bool {
        matches!(ty, InferredType::NamedInstance(n) if n == "slice")
    }

    /// 添字まわりの検査で「判定材料が無い」とみなす型。
    ///
    /// ⚠ `NamedInstance` を含めるのが要点。組み込みの `Index` クラス・スライス値
    /// （`NamedInstance("slice")`）・利用者が `__getitem__` を定義したクラスが
    /// ここを通る。`uint` 注釈も現状 `Unresolved` になるのでここで落ちる。
    fn opaque_for_subscript(ty: &InferredType) -> bool {
        use InferredType as T;
        matches!(
            ty,
            T::Unresolved
                | T::Any
                | T::Never
                | T::NamedInstance(_)
                | T::GenericInstance { .. }
                | T::SelfType
                | T::Protocol(..)
                | T::Union(_)
                | T::Intersection(_)
                | T::Namespace(..)
                | T::PyNamespace(..)
        )
    }

    /// 添字の相手がスライスの境界を自分で決める CPython の物か（`Any` も・pandas_import_plan.md 2-3）。
    /// pandas の `df.loc["a":"c"]` のように、境界は int とは限らない（CPython と同じ）。
    pub(super) fn takes_foreign_slices(&self, obj_ty: &InferredType) -> bool {
        match obj_ty {
            InferredType::Any => true,
            InferredType::NamedInstance(n) | InferredType::GenericInstance { name: n, .. } => {
                self.registry.is_cpython_class(n)
            }
            _ => false,
        }
    }

    /// CPython の物の添字（[`Self::takes_foreign_slices`]）: スライスの境界は型を問わない（式は推論して中を検査する）。
    /// `a[1:, 0]` のようにタプルの項がスライスでも同じ。
    pub(super) fn infer_foreign_index(&mut self, index: &Expr) -> InferredType {
        use crate::ast::SeqEntry;
        match index {
            Expr::Slice { begin, end, step } => {
                for e in [begin, end, step].into_iter().flatten() {
                    self.walk(e);
                }
                InferredType::NamedInstance("slice".to_string())
            }
            // 項がどれも式で（`*xs` が無い）、スライスを含むタプル（構文解析が作るのはこの形だけ）
            Expr::Tuple(items)
                if items.iter().all(|i| matches!(i, SeqEntry::Item(_)))
                    && items.iter().any(|i| matches!(i, SeqEntry::Item(Expr::Slice { .. }))) =>
            {
                let parts: Vec<&Expr> =
                    items.iter().filter_map(|i| if let SeqEntry::Item(e) = i { Some(e) } else { None }).collect();
                InferredType::Tuple(parts.into_iter().map(|e| self.infer_foreign_index(e)).collect())
            }
            _ => self.infer(index),
        }
    }

    /// `for` の反復対象が反復可能かを検査する（タスク 7.1・検体 `K3`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 反復できる | 反復できない |
    /// |---|---|
    /// | `list` / `fixed_list` / `list_like` / `set` / `tuple` / `str` | `int` ・ `float` ・ `bool` |
    /// | `range(..)` ・ ジェネレータ | **`dict`** |
    /// | `__iter__` を持つクラス | `__iter__` を持たないクラス（`AttributeError`） |
    ///
    /// ⚠⚠ **`dict` は反復できない**（`for k in d:` は `TypeError: object is not iterable`）。
    /// Python と違うので、ここを取りこぼすと Python の癖で書いた `for k in d:` が
    /// 実行時まで判らない。
    /// ⚠ クラスは `__iter__` を定義できるので素通しする（持たない場合は実行時 `AttributeError`）。
    /// protocol の型の値の添字（`xs[0]`・`xs: Sequence[int]`）。`__getitem__` の要求で引数を検査して戻り値の型を返す。
    /// 要求に無ければ添字を取れない（`Sized` の値など）。
    pub(super) fn protocol_subscript(
        &mut self,
        proto: &str,
        args: &[InferredType],
        obj_ty: &InferredType,
        idx_ty: &InferredType,
    ) -> InferredType {
        match self.protocol_members(proto, args).and_then(|(_, m)| m.get("__getitem__").cloned()) {
            Some(sigs) => self
                .check_method_sigs(&obj_ty.to_string(), "__getitem__", &sigs, true, &[(None, idx_ty.clone())], &[])
                .unwrap_or(InferredType::Unresolved),
            None => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NotSubscriptable { container: obj_ty.clone() },
                    span: None,
                });
                InferredType::Unresolved
            }
        }
    }

    pub(in crate::type_check) fn check_iterable(&mut self, iter_ty: &InferredType, span: Option<Span>) {
        use InferredType as T;
        // 判定できるのは「確実に反復できない」と分かっている型だけ。
        let not_iterable = matches!(
            iter_ty,
            // ⚠ **`dict` は反復できる**（キーが出る・Python と同じ）。以前はここで
            //   弾いていたので `for k in d:` が `'dict[..]' is not iterable` だった。
            T::Int | T::Float | T::Complex | T::Bool | T::None | T::Undefined
        ) || match self.resolve_protocols(iter_ty) {
            // protocol の型の値は `__iter__`（無ければ CPython と同じく `__getitem__`）を要求するか（6-7）
            T::Protocol(p, args) => self
                .protocol_members(&p, &args)
                .is_some_and(|(_, m)| !m.contains_key("__iter__") && !m.contains_key("__getitem__")),
            _ => false,
        };
        if !not_iterable {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::NotIterable { ty: iter_ty.clone() },
            span,
        });
    }
}
