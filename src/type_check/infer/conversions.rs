// type_check/infer/conversions.rs — `mustbe` とキャスト（`=>`）の検査。

use super::*;

impl TypeChecker {
    /// `mustbe` が**成功しうるか**を検査する（タスク 7.3・検体 `T2`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 元 | 先 | 結果 |
    /// |---|---|---|
    /// | `Any` / `Union` | 具体型 | ✅（これが `mustbe` の本来の用途） |
    /// | trait | 実装クラス | ✅（ダウンキャスト） |
    /// | `int` | `int` | ✅ |
    /// | `int` | `str` | ⛔ |
    /// | **`int`** | **`float`** | ⛔ **数値昇格は効かない** |
    ///
    /// ⚠⚠ **`mustbe` は実行時の型そのものを見る**ので、`==` の昇格ラティス
    /// （`uint → int → float`）は**効かない**。`1 mustbe float` は必ず失敗する。
    /// タスク 5.4（`case` パターン）と規則が違うので混同しないこと。
    ///
    /// ⚠ 判定は**実行時の種別**（`value_matches_type_ann` が見る外側の種別）で行う。
    /// `list[int] mustbe list[str]` は外側が同じなので**素通し**（要素型は既存の警告が扱う）。
    fn check_mustbe_possible(
        &mut self,
        src_ty: &InferredType,
        target_ty: &InferredType,
        span: &Span,
    ) {
        let (Some(a), Some(b)) = (
            Self::runtime_kind_name(src_ty),
            Self::runtime_kind_name(target_ty),
        ) else {
            return;
        };
        // `Ptr` どうしは要素型も実行時に見る（3-3）。要素型の分かっている `Ptr[A]` は `Ptr[B]` にならない
        // （`Ptr[void]` / 素の `Ptr` からは成功しうる・どの `Ptr` も `Ptr[void]` にはなる）
        let ptr_mismatch = match (src_ty, target_ty) {
            (InferredType::Ptr(from), InferredType::Ptr(to)) => !to.accepts(from) && !from.accepts(to),
            _ => false,
        };
        if a == b && !ptr_mismatch {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::MustBeCanNeverSucceed {
                from: src_ty.clone(),
                to: target_ty.clone(),
            },
            span: Some(span.clone()),
        });
    }

    /// 実行時に区別される**種別名**。判定できない型は `None`（＝素通し）。
    ///
    /// ⚠ クラス・trait・protocol・`Any` / `Union` などは `None`。ダウンキャストが
    /// 成立しうるので種別で弾いてはいけない。
    fn runtime_kind_name(ty: &InferredType) -> Option<&'static str> {
        use InferredType as T;
        Some(match ty {
            T::Int => "int",
            T::Float => "float",
            T::Complex => "complex",
            T::Str => "str",
            T::Bytes => "bytes",
            T::Bool => "bool",
            T::None => "None",
            T::Undefined => "Undefined",
            T::Ptr(_) => "Ptr",
            T::List | T::ListOf(_) => "list",
            T::FixedList | T::FixedListOf(_) => "fixed_list",
            T::Dict | T::DictOf(_, _) => "dict",
            T::Set | T::SetOf(_) => "set",
            T::FrozenSet | T::FrozenSetOf(_) => "frozenset",
            T::Tuple(_) => "tuple",
            _ => return None,
        })
    }

    /// `=>` キャストが**成功しうるか**を検査する（タスク 7.3・検体 `T1`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 元 | 先 | 結果 |
    /// |---|---|---|
    /// | クラスのインスタンス | `__cast__[T]` を持つ `T` | ✅ |
    /// | クラスのインスタンス | `__cast__[T]` を**持たない** `T` | ⛔ `'C' is not castable to 'int'` |
    /// | 何でも | **`new_type`** | ✅ |
    /// | `list` | **`fixed_list[T]`** | ✅（平坦化変換） |
    /// | Python のクラスのインスタンス（numpy の配列など） | **`list[T]`** | ✅（実行時に要素を照合する・pandas_import_plan.md の 4-3） |
    /// | それ以外（`1 => float` ・ `"5" => int` ・ `xs => list[int]`） | — | ⛔ `requires an instance or new_type target` |
    ///
    /// ⚠⚠ **プリミティブ同士の `=>` は書けない**（`1 => float` は実行時エラー）。
    /// 変換は `float(1)` のような組み込み関数を使う（決定 D-5 / タスク 2.4）。
    ///
    /// ⚠ 判定材料が無い型（`Unresolved` / `Any` / `Union` …）は素通し。
    /// ⚠ `__cast__` は**継承チェーンを辿って**探す（基底クラス・trait 由来も有効）。
    pub(super) fn check_cast_possible(
        &mut self,
        src_ty: &InferredType,
        target_name: &str,
        target_ty: &InferredType,
        span: &Span,
    ) {
        use InferredType as T;
        if Self::opaque_for_unary(src_ty) && !matches!(src_ty, T::NamedInstance(_) | T::GenericInstance { .. })
        {
            return;
        }
        // `new_type` 先は何からでも作れる。
        if self.registry.new_type_original(target_name).is_some() {
            return;
        }
        // Arrow の list / dict / set と `py_list` / `py_dict` / `py_set` の間（D-12・2-7）
        if let Some(verdict) = self.py_collection_cast(src_ty, target_ty) {
            if let Some(elem) = target_ty.arrow_collection_in_python() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ArrowCollectionInPython { elem, what: "cast".to_string() },
                    span: Some(span.clone()),
                });
                return;
            }
            if let Err(reason) = verdict {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CastCanNeverSucceed { from: src_ty.clone(), to: target_name.to_string(), reason },
                    span: Some(span.clone()),
                });
            }
            return;
        }
        // 解決できない先（trait / protocol / 未知の名前）は判定しない。
        if matches!(target_ty, T::Unresolved | T::Any | T::Protocol(..)) {
            return;
        }
        if self.registry.is_known_trait(target_name) || self.registry.is_protocol(target_name) {
            return;
        }
        // ⚠⚠ **`new_type` からの取り出しは許される**（`Count => int` など）。
        //    `scan_examples` が `other_typing.ar` / `polymorphism.ar` で教えてくれた。
        //    `new_type` のラッパは `__cast__` を持たないが、実行時は基底型へ戻せる。
        if let T::NamedInstance(name) = src_ty {
            if self.registry.new_type_original(name.as_str()).is_some() {
                return;
            }
        }
        // Python のクラスのインスタンス → `list[T]`（`a => list[float]`・numpy の配列など）。写せるかは実行時に決まる（4-3）
        if let (T::NamedInstance(name), T::ListOf(_)) = (src_ty, target_ty) {
            if self.registry.is_cpython_class(name) {
                return;
            }
        }
        // クラスのインスタンス → `__cast__[T]` の有無で決まる。
        if let Some((cls, _)) = self.class_and_subst(src_ty) {
            // ⚠ テンプレートクラスなどで置換表が作れない場合も名前で引ければ十分。
            let key = format!("__cast__[{target_name}]");
            if self.collect_class_method_sigs(cls.as_str()).contains_key(&key) {
                return;
            }
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CastCanNeverSucceed {
                    from: src_ty.clone(),
                    to: target_name.to_string(),
                    reason: format!("no `{key}` method defined"),
                },
                span: Some(span.clone()),
            });
            return;
        }
        // ⚠⚠ `list` ⇄ `fixed_list` は**双方向**に許される（平坦化変換とその復元）。
        //    `scan_examples` が `fixed_list.ar` / `runtime_checks_in_function.ar` で
        //    `fl => list[Vec2]` を落として教えてくれた（片方向しか書いていなかった）。
        let list_like_kind = |t: &T| {
            matches!(
                t,
                T::List | T::ListOf(_) | T::FixedList | T::FixedListOf(_)
                    | T::ListLike | T::ListLikeOf(_)
            )
        };
        if list_like_kind(src_ty) && list_like_kind(target_ty) {
            return;
        }
        // ここまで来たら「インスタンスでもなく、先が new_type でも fixed_list でもない」。
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::CastCanNeverSucceed {
                from: src_ty.clone(),
                to: target_name.to_string(),
                reason: "`=>` requires an instance source or a new_type target".to_string(),
            },
            span: Some(span.clone()),
        });
    }

    /// `=>` の片側が `py_list` / `py_dict` / `py_set` のとき、写せるか（pandas_import_plan.md の D-12・2-7）。どちらでもなければ `None`。
    ///
    /// 写せるのは同じ種類どうし: Arrow の list / dict / set → `py_*`、`py_*` → Arrow の list / dict / set、`py_*` → `py_*`（写さずに
    /// 要素を照合する）。要素はキャスト先の型の深さまで写す（`[[1]] => py_list[py_list[int]]`・[`Self::py_element_convertible`]）。
    fn py_collection_cast(&self, src: &InferredType, dst: &InferredType) -> Option<Result<(), String>> {
        use InferredType as T;
        let is_py = |t: &T| matches!(t, T::PyList(_) | T::PyDict(..) | T::PySet(_));
        (is_py(src) || is_py(dst)).then(|| self.py_collection_convertible(src, dst))
    }

    fn py_collection_convertible(&self, src: &InferredType, dst: &InferredType) -> Result<(), String> {
        use InferredType as T;
        match (src, dst) {
            (T::ListOf(s) | T::FixedListOf(s) | T::ListLikeOf(s) | T::PyList(s), T::PyList(d))
            | (T::PyList(s), T::ListOf(d))
            | (T::SetOf(s) | T::PySet(s), T::PySet(d))
            | (T::PySet(s), T::SetOf(d)) => self.py_element_convertible(s, d),
            (T::DictOf(sk, sv) | T::PyDict(sk, sv), T::PyDict(dk, dv)) | (T::PyDict(sk, sv), T::DictOf(dk, dv)) => {
                self.py_element_convertible(sk, dk)?;
                self.py_element_convertible(sv, dv)
            }
            _ => Err("`=>` converts a py_list / py_dict / py_set from or to the same kind of collection".to_string()),
        }
    }

    /// 要素 `s` を、キャスト先の要素 `d` にできるか。コレクションどうしならさらに写す（[`Self::py_collection_convertible`]）、
    /// タプルは要素ごと、ほかは今の型のまま受かること。判定材料の無い要素（`Any`・空のリテラル `[]` の要素）は実行時に照合する。
    fn py_element_convertible(&self, s: &InferredType, d: &InferredType) -> Result<(), String> {
        use InferredType as T;
        if matches!(s, T::Any | T::Unresolved | T::Never) || self.type_matches_exact(s, d) {
            return Ok(());
        }
        let collection = |t: &T| {
            matches!(
                t,
                T::ListOf(_) | T::FixedListOf(_) | T::ListLikeOf(_) | T::SetOf(_) | T::DictOf(..) | T::PyList(_)
                    | T::PyDict(..) | T::PySet(_)
            )
        };
        match (s, d) {
            _ if collection(s) && collection(d) => self.py_collection_convertible(s, d),
            (T::Tuple(ss), T::Tuple(ds)) if ss.len() == ds.len() => {
                ss.iter().zip(ds).try_for_each(|(s, d)| self.py_element_convertible(s, d))
            }
            _ => Err(format!("an element of type `{s}` cannot become `{d}`")),
        }
    }

    /// `expr mustbe Type` の型を推論する。解決した型を返し、コレクション要素型や
    /// 関数シグネチャは実行時に検査されないため警告する。
    pub(super) fn infer_mustbe(
        &mut self,
        expr: &Expr,
        guard_type: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
        // ⚠ タスク 7.3 で**対象式の型を使う**ようにした（以前は `walk` で捨てていた）。
        //    「成功しうる値が 1 つも無い」表明を弾くのに元の型が要る。
        let src_ty = self.infer(expr);
        let resolved = InferredType::from_ann(guard_type).unwrap_or(InferredType::Unresolved);
        self.check_mustbe_possible(&src_ty, &resolved, span);
        // ── AST 型解決層（#16・段階(a)）──
        // `mustbe` は実行時に対象型で動的検査する（不一致で raise）。よって:
        //   解決型テーブル = 確定後の型（guard_type）／ 検査指示 = CheckBefore(その型)。
        let tid = self.annotations.intern(resolved.clone());
        self.annotations.set_resolved(node_id, resolved.clone());
        self.annotations
            .set_directive(node_id, crate::type_check::annotations::Directive::CheckBefore(tid));
        // コレクション型パラメータ・関数シグネチャは実行時に未チェック → 警告
        let warn_kind = match &resolved {
            InferredType::ListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list".to_string(),
            }),
            InferredType::FixedListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "fixed_list".to_string(),
            }),
            InferredType::ListLikeOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list_like".to_string(),
            }),
            InferredType::SetOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "set".to_string(),
            }),
            InferredType::FrozenSetOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "frozenset".to_string(),
            }),
            InferredType::DictOf(_, _) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "dict".to_string(),
            }),
            InferredType::Tuple(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "tuple".to_string(),
            }),
            InferredType::Function { params, return_type, .. } => {
                let has_params = params.as_ref().is_some_and(|p| !p.is_empty());
                let has_ret = **return_type != InferredType::Any;
                if has_params || has_ret {
                    Some(TypeWarningKind::MustBeFunctionSignatureUnchecked {
                        guard_type: guard_type.to_string(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(kind) = warn_kind {
            self.report_warning(StaticTypeWarning { kind, span: Some(span.clone()) });
        }
        resolved
    }
}
