// stmt/check/branches.rs — 分岐の検査: `if` / `match` / `except` の条件とパターン・型ガードの絞り込み。

use super::*;

impl TypeChecker {
    /// `match` 文を型検査する。`is Type` パターンでは対象変数を各腕スコープ内で絞り込む。
    pub(super) fn check_match(&mut self, subject: &Expr, arms: &[MatchArm]) {
        self.check_match_arms(subject, arms);
    }

    /// 型ガードの型名が存在するかを検査する（**妥当性検査**・タスク 3.4・検体 X6）。
    ///
    /// ⚠⚠ これが無いと `is NoSuchType:` が**黙って通り、腕が永久に死ぬ**。さらに腕の中では
    /// 対象変数がその存在しないクラスへ絞り込まれるので、**メンバーアクセスが全て無検査**に
    /// なる（`d.totally_missing_field` が通っていた・実測）。
    ///
    /// ⚠ 整合性検査（3 分類）ではなく**妥当性検査**。「2 つの型が適合するか」ではなく
    /// 「名前が在るか」を見る別系統（D-14）。
    /// ⚠ **判らない名前は通さない**が、型変数（`fn f[T]` の `T`）は正当なので除く。
    ///
    /// ⚠⚠ **`match` の腕だけでなく `x is T` のすべて**で呼ぶ（フェーズ10 10-10）。以前は `match` の
    ///   腕からしか呼ばれておらず、`if x is NoSuch:` も式の `x is Nope` も黙って通っていた（実測）。
    /// ⚠ import 先を読めていない環境（エディタ）では、名前が無いことを誤りと言えないので見送る
    ///   （`check_ann_names_exist` と同じ判断）。
    pub(crate) fn check_guard_type_exists(&mut self, type_name: &str) {
        if self.registry_incomplete {
            return;
        }
        // プリミティブ・`Any` 等は `from_ann` が解釈できるので、それで判定する。
        // ⚠ `from_ann` は**大文字始まりの未知の識別子をクラス名にする**ので、
        //    `Some(..)` でも「在る」ことの証明にはならない（`NamedInstance` は要確認）。
        let untestable = |outer: Option<&str>| TypeErrorKind::GuardTypeNotTestable {
            type_name: type_name.to_string(),
            outer: outer.map(str::to_string),
        };
        match InferredType::from_ann(type_name) {
            Some(InferredType::NamedInstance(n)) => {
                // ⚠ import していないモジュールの型は書けない（2026-10-02）。
                if self.report_unimported_module_type(&n) {
                    return;
                }
                // テンプレート型変数・クラス・trait・protocol・`generator`（実行時の型名）は正当。
                if self.type_name_exists(n.as_str()) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                    span: None,
                });
            }
            // テンプレートの具体化（`is Stack[int]`・10-10）。単相化した具体クラスなので判定できる。
            // ⚠ テンプレートでない名前・知らない名前に `[..]` が付いていたら未知の型として弾く。
            Some(InferredType::GenericInstance { ref name, .. }) => {
                if self.report_unimported_module_type(name) {
                    return;
                }
                // 総称の protocol（`is Holder[int]`・6-5）は型引数を実行時に確かめられない（CPython の `isinstance` と
                // 同じ）。素の名前（`is Holder`）は構造で確かめる。
                if self.registry.is_protocol(&self.registry.resolve(name)) {
                    self.report_error(StaticTypeError { kind: untestable(Some(name.as_str())), span: None });
                    return;
                }
                let is_template = self
                    .registry
                    .template_params(name)
                    .is_some_and(|p| !p.is_empty());
                if is_template || self.state.is_type_param(name) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                    span: None,
                });
            }
            // `Ptr[T]` は要素型まで実行時に判定できる（ffi_value_exchange_plan.md 3-3・`ptr_ann_matches`）。
            // 構造体の要素は名前が在るか、それ以外は C の型か（注釈と同じ規則）を見る。
            Some(InferredType::Ptr(ref e)) => {
                let crate::type_check::PtrElem::Type(t) = &**e else { return };
                if !matches!(t, InferredType::NamedInstance(_) | InferredType::GenericInstance { .. }) {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::PtrElemNotCType { elem: t.clone(), what: "type guard".to_string() },
                        span: None,
                    });
                    return;
                }
                let mut names = Vec::new();
                t.collect_type_names(&mut names);
                for n in names {
                    if !self.report_unimported_module_type(&n) && !self.type_name_exists(&n) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                            span: None,
                        });
                    }
                }
            }
            // 型引数つきの組み込み型は `is` で判定できない（`GuardTypeNotTestable` の doc）。
            // 容器（`list[int]` …）は外側しか確かめられず、`Union[..]` / 関数型などは実行時の `is`
            // （`value_is_type`・クラス名と組み込みの型名だけを見る）では常に偽になる。
            // ⚠ `[` の無い綴り（`is list` / `is function`）は従来どおり正当。
            Some(ref t) if type_name.contains('[') => {
                let outer = match t {
                    InferredType::ListOf(_) => Some("list"),
                    InferredType::FixedListOf(_) => Some("fixed_list"),
                    InferredType::ListLikeOf(_) => Some("list_like"),
                    InferredType::SetOf(_) => Some("set"),
                    InferredType::FrozenSetOf(_) => Some("frozenset"),
                    InferredType::DictOf(_, _) => Some("dict"),
                    InferredType::Tuple(_) => Some("tuple"),
                    InferredType::IteratorOf(_) => Some("generator"),
                    _ => None,
                };
                self.report_error(StaticTypeError { kind: untestable(outer), span: None });
            }
            // プリミティブ・コレクション・`Union` 等は解釈できた時点で存在が確かめられている。
            Some(_) => {}
            // `from_ann` が解釈できない綴りは未知として扱う。
            None => self.report_error(StaticTypeError {
                kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                span: None,
            }),
        }
    }

    /// `match` の腕を型検査する（**文と式で共有**・タスク 2.9）。
    ///
    /// ⚠⚠ **以前は `match` 式がこの絞り込みを持っていなかった。** `Expr::MatchExpr`
    /// （`infer.rs`）は `Case` の infer と腕本体の検査だけで `IsType` の絞り込みを
    /// していなかったため、**文と式で意味論が違った**（実測）:
    ///
    /// ```arrow
    /// match v:                     # 文 → 絞り込まれる
    ///     is Dog:
    ///         let s: str = v.n     # ✅ int → str でエラー
    ///
    /// let r = match v ->int:       # 式 → 絞り込まれなかった
    ///     is Dog:
    ///         let s: str = v.n     # ⛔ 通っていた
    /// ```
    ///
    /// ⇒ 腕の処理を 1 箇所に集約し、文・式の両方から呼ぶ。**別々に書くと再びずれる。**
    pub(crate) fn check_match_arms(&mut self, subject: &Expr, arms: &[MatchArm]) {
        let subject_ty = self.infer(subject);
        // 絞り込めるのは対象が**単なる識別子**のときだけ（再 `declare` で実装しているため）。
        let subject_name: Option<String> = if let Expr::Ident { name: n, .. } = subject {
            Some(n.clone())
        } else {
            None
        };
        for arm in arms {
            self.push_scope();
            match &arm.pattern {
                MatchPattern::Case(expr) => {
                    // ⚠ パターンの型が subject と決して一致しない＝**腕が死ぬ**（タスク 5.4・検体 X5）。
                    let pat_ty = self.infer(expr);
                    self.check_case_pattern_type(&subject_ty, &pat_ty);
                }
                MatchPattern::IsType(type_name) => {
                    self.check_guard_type_exists(type_name);
                    if let Some(ref var_name) = subject_name {
                        let narrowed = Self::type_from_guard_name(type_name);
                        self.declare_narrowed(var_name.clone(), narrowed);
                    }
                }
            }
            self.check_stmts(&arm.body);
            self.pop_scope();
        }
    }

    /// `except T:` の `T` が例外クラスかを検査する（タスク 7.1・検体 `E2`）。
    ///
    /// ⚠⚠ これは「型が違う」ではなく「**腕が永久に死ぬ**」という妥当性の指摘
    /// （3.4 の `UnknownGuardType` と同じ系統）。実測では `except NotAnError:` は
    /// 一致せず、例外がそのまま伝播して実行時 `Traceback` になる。
    ///
    /// ⚠ 例外クラスの判定は `raise` 側と**同じ述語**（`class_implements_trait(.., "Error")`）を
    /// 使う。組み込み例外は `class_bases = ["Error"]` で登録されており、利用者の
    /// `class MyErr(Error)` も同じ経路で通る。
    pub(super) fn check_except_type(&mut self, type_name: &str) {
        // `except Error` は「すべての例外」のつもりで書かれる。書くべきは `except Exception`（10-19）。
        if type_name == "Error" {
            self.report_error(StaticTypeError { kind: TypeErrorKind::ExceptOnErrorTrait, span: None });
            return;
        }
        // ⚠ import していないモジュールの例外クラスは書けない（2026-10-02）。
        if self.report_unimported_module_type(type_name) {
            return;
        }
        // 未知の名前は「存在しないクラス」として弾く（`UnknownGuardType` と同じ理由）。
        if !self.registry.is_known_class(type_name) {
            // ⚠ trait / protocol 名は `is_known_class` に載らないので、そちらも見てから判断する。
            if self.registry.is_known_trait(type_name) || self.registry.is_protocol(type_name) {
                return;
            }
            // ⚠ import 先を読めていない環境（エディタ）では、名前が無いことを誤りと言えない
            //   （`check_guard_type_exists` と同じ判断）。`except m.Err:`（タスク 4-3）の `m` のクラスが
            //   見えず、正しいプログラムに偽の誤りを出していた（`compare_wasm_frontend` の INVENTED）。
            if self.registry_incomplete {
                return;
            }
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ExceptNotError {
                    type_name: type_name.to_string(),
                    reason: "not a known type",
                },
                span: None,
            });
            return;
        }
        if self.class_implements_trait(type_name, "Error") {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::ExceptNotError {
                type_name: type_name.to_string(),
                reason: "not an exception class",
            },
            span: None,
        });
    }

    /// `if` / `while` の条件が `bool` かを検査する（決定 D-12・タスク 5.5・検体 `K1` / `K2`）。
    ///
    /// ## Arrow は真偽性を使わない
    ///
    /// | 書き方 | 真偽性（旧） | **`bool` 厳密（決定）** |
    /// |---|---|---|
    /// | `if 0:` | 書けた（偽） | **静的エラー** → `if n != 0:` |
    /// | `if xs:`（リスト） | 書けた（空なら偽） | **静的エラー** → `if len(xs) > 0:` |
    /// | `if some_func:` | 書けた（**常に真**） | **静的エラー** |
    ///
    /// 実利は「**常に真になる条件の書き間違い**」（`()` を忘れた `if some_func:` など）を
    /// 静的に捕まえられること。
    ///
    /// ⚠⚠ **`is_truthy` は撤去していない。** `not` / `and` / `or` の評価と、ネイティブ ABI の
    /// 関数表 export（C ABI なので削除・改名すると FFI が壊れる）で要る。D-12 は
    /// 「条件位置に静的検査を足す」ことであって実行時の真偽性の撤去ではない。
    ///
    /// ⚠ 前提はタスク 2.5（D-10）。`and` / `or` が結果型を**被演算子の join** にしたので
    /// `if a and b:`（非 bool）がここで落ちる。結果型が `Bool` 固定のままなら通ってしまう。
    pub(crate) fn check_condition_is_bool(&mut self, cond: &Expr, keyword: &'static str) {
        let ty = self.infer(cond);
        // ⚠ 本当に判らない型だけ見送る。クラスや数値は**弾く対象**（常に真になる書き間違い）。
        if matches!(ty, InferredType::Unresolved | InferredType::Any | InferredType::Never) {
            return;
        }
        if matches!(ty, InferredType::Bool) {
            return;
        }
        // 条件の式を指す（3-3）。錨が無ければ今の錨（`if` 式）か文の位置（`if` / `while`）。
        self.report_error_at(cond, StaticTypeError {
            kind: TypeErrorKind::ConditionNotBool {
                keyword: keyword.to_string(),
                got: ty,
            },
            span: None,
        });
    }

    /// `case` パターンが subject と**一致しうるか**を検査する（タスク 5.4・検体 `X5`）。
    ///
    /// ## これは「型の不一致」ではなく「**腕が死ぬ**」という指摘
    ///
    /// ⚠⚠ タスク 4.4 で `==` の異型比較を厳密化しようとして**撤回した**のを混同しないこと。
    /// `1 == "a"` が `False` を返すのは**仕様**（意味のある答えがある）。一方
    /// `match (a: int): case "s":` は**到達できない分岐**で、`UnknownGuardType`（3.4）と
    /// 同じ「書いた本人の意図と結果が食い違う」系統。
    ///
    /// ## 判定は `==` と同じ規則に合わせる（実測）
    ///
    /// | subject | pattern | 実行時 |
    /// |---|---|---|
    /// | `int` | `1.0` | **一致する**（`uint → int → float` の昇格ラティス） |
    /// | `float` | `1` | **一致する** |
    /// | `int` | `True` | 一致しない（`bool` はラティスの対象外） |
    /// | `str` | `"x"` | 一致する |
    ///
    /// ⚠ クラス（`__eq__` を定義できる）・enum・`Union`・`Any`・`Unresolved` は
    /// **素通し**。判定材料が無いものを弾くと偽陽性になる。
    fn check_case_pattern_type(&mut self, subject_ty: &InferredType, pat_ty: &InferredType) {
        use InferredType as T;
        let opaque = |t: &T| {
            matches!(
                t,
                T::Unresolved
                    | T::Any
                    | T::Never
                    | T::NamedInstance(_)
                    | T::GenericInstance { .. }
                    | T::SelfType
                    | T::Protocol(..)
                    | T::Union(_)
                    | T::Intersection(_)
                    | T::Result(_, _)
                    | T::Namespace(..)
                    | T::PyNamespace(..)
                    | T::TypeVal
                    | T::TypeValOf(_)
            )
        };
        if opaque(subject_ty) || opaque(pat_ty) {
            return;
        }
        // ⚠ 数値は昇格ラティスで比較されるので相互に一致しうる（`bool` は含めない）。
        let numeric = |t: &T| matches!(t, T::Int | T::Float | T::Complex);
        if numeric(subject_ty) && numeric(pat_ty) {
            return;
        }
        // ⚠ 記憶域ではないので `Site::Other`（`__cast__` は挿入されない）。
        //    どちらの向きでも通れば一致しうるとみなす（`list[⊥]` のような下界を通すため）。
        let site = crate::type_check::type_utils::Site::Other;
        if self.types_compatible(pat_ty, subject_ty, site)
            || self.types_compatible(subject_ty, pat_ty, site)
        {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::CasePatternNeverMatches {
                subject: subject_ty.clone(),
                pattern: pat_ty.clone(),
            },
            span: None,
        });
    }

    /// `if` / `elif` / `else` を型検査する。各分岐の条件が型ガード
    /// (`is Type` / `x.is_OK()` / `x.is_ERR()`) のとき、その分岐スコープ内で
    /// 対象変数の型を絞り込む。
    pub(super) fn check_if(&mut self, branches: &[(Expr, Vec<Stmt>)], else_body: &Option<Vec<Stmt>>) {
        for (cond, body) in branches {
            let guard_opt = Self::detect_type_guard(cond);
            // Result 型ガード (`x.is_OK()` / `x.is_ERR()`) は type_guard より優先する。
            let result_guard = self.detect_result_guard(cond);
            let (narrowed, error_info) = self.narrow_by_type_guard(guard_opt);

            // ⚠ 条件は `bool` でなければならない（D-12・検体 K1）。
            self.check_condition_is_bool(cond, "if");

            if let Some((var_name, var_type, span)) = error_info {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::IsNotOnNonUnion { var_name, var_type },
                    span: Some(span),
                });
            }

            self.push_scope();
            if let Some((var_name, narrowed_ty, _is_mut)) = result_guard.or(narrowed) {
                self.declare_narrowed(var_name, narrowed_ty);
            }
            self.check_stmts(body);
            self.pop_scope();
        }
        if let Some(body) = else_body {
            self.push_scope();
            self.check_stmts(body);
            self.pop_scope();
        }
    }

    /// 条件式が `is Type` ガードなら `(変数名, 型名, 否定か, span)` を返す。
    fn detect_type_guard(cond: &Expr) -> Option<(String, String, bool, Span)> {
        let Expr::IsType { expr, type_name, negated, span, .. } = cond else {
            return None;
        };
        let Expr::Ident { name: var_name, .. } = expr.as_ref() else {
            return None;
        };
        Some((var_name.clone(), type_name.clone(), *negated, span.clone()))
    }

    /// 条件式が Result 型ガード (`x.is_OK()` / `x.is_ERR()`) なら、絞り込んだ
    /// `(変数名, 絞り込み後の型, 可変か)` を返す。
    fn detect_result_guard(&self, cond: &Expr) -> Option<(String, InferredType, bool)> {
        let Expr::Call { func, args, .. } = cond else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let Expr::Attr { object, attr, .. } = func.as_ref() else {
            return None;
        };
        if attr != "is_OK" && attr != "is_ERR" {
            return None;
        }
        let Expr::Ident { name: var_name, .. } = object.as_ref() else {
            return None;
        };
        let info = self.lookup(var_name)?;
        let InferredType::Result(ok_ty, err_ty) = &info.ty else {
            return None;
        };
        let narrowed_ty = if attr == "is_OK" {
            *ok_ty.clone()
        } else {
            *err_ty.clone()
        };
        Some((var_name.clone(), narrowed_ty, info.mutable))
    }

    /// `is Type` ガードから分岐スコープの絞り込みを計算する。
    /// 戻り値は `(絞り込み束縛, `is not` を非 Union に使ったときのエラー情報)`。
    #[allow(clippy::type_complexity)]
    fn narrow_by_type_guard(
        &mut self,
        guard_opt: Option<(String, String, bool, Span)>,
    ) -> (
        Option<(String, InferredType, bool)>,
        Option<(String, InferredType, Span)>,
    ) {
        let Some((var_name, type_name, negated, span)) = guard_opt else {
            return (None, None);
        };
        let guard_ty = if self.registry.is_protocol(type_name.as_str()) {
            InferredType::Protocol(type_name.clone(), Vec::new())
        } else {
            Self::type_from_guard_name(&type_name)
        };
        let (var_ty, is_mut) = self
            .lookup(&var_name)
            .map(|v| (v.ty.clone(), v.mutable))
            .unwrap_or((InferredType::Unresolved, false));

        if negated {
            match &var_ty {
                InferredType::Union(types) => {
                    let remaining: Vec<InferredType> =
                        types.iter().filter(|t| **t != guard_ty).cloned().collect();
                    let narrowed_ty = match remaining.len() {
                        0 => InferredType::Unresolved,
                        1 => remaining.into_iter().next().unwrap(),
                        _ => InferredType::Union(remaining),
                    };
                    (Some((var_name, narrowed_ty, is_mut)), None)
                }
                InferredType::Unresolved => (None, None),
                _ => (None, Some((var_name, var_ty.clone(), span))),
            }
        } else {
            // `is TypeName` guard: var_ty が交差型ならガード型の適合を検証する
            if let InferredType::Intersection(isect_types) = &var_ty {
                let isect_cloned = isect_types.clone();
                self.check_intersection_guard_type(&type_name, &isect_cloned, Some(span));
            }
            (Some((var_name, guard_ty, is_mut)), None)
        }
    }
}
