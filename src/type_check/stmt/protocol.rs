// stmt/protocol.rs — プロトコル適合性検査: check_protocol_conformance とクラスのフィールド/メソッドシグネチャ収集・照合。

use {
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, TypeErrorKind},
    crate::type_check::types::{FnSig, InferredType},
    crate::type_check::TypeChecker,
};

/// protocol の要求を満たさない理由 1 つ（[`TypeChecker::check_protocol_conformance`] の報告と
/// [`TypeChecker::satisfies_protocol`] の判定が共有する・pandas_import_plan.md の 6-5）。
pub(crate) struct ProtocolMiss {
    /// 誤りに出す型の名前（クラスならクラスの名前）。`None` は照合の文脈（`argument to `f`` など）。
    type_name: Option<String>,
    reason: String,
}

/// 照合の相手が持つメンバー（[`TypeChecker::conformer_of`]）。
enum Conformer {
    /// フィールドとメソッド。`owner` は誤りに出す型の名前（クラスの名前・protocol なら `None`）。
    Members {
        owner: Option<String>,
        fields: std::collections::HashMap<String, (crate::ast::FieldKind, InferredType)>,
        methods: std::collections::HashMap<String, Vec<FnSig>>,
    },
    /// 判定材料が無い（`Any` / `Unresolved` / 単相化されないテンプレートの具体化など）。満たすとみなす。
    Opaque,
    /// 照合できない種類の型（理由）。
    Mismatch(String),
}

impl TypeChecker {
    /// 型 `ty` がプロトコル `proto_name`（型引数 `args`・総称でなければ空）を満たすか検査する。
    /// 満たさない場合は StaticTypeError を記録する。
    pub(crate) fn check_protocol_conformance(
        &mut self,
        ty: &InferredType,
        proto_name: &str,
        args: &[InferredType],
        span: Option<Span>,
        context: &str,
    ) {
        let Some(misses) = self.protocol_misses(ty, proto_name, args) else {
            return;
        };
        let protocol_name = if args.is_empty() {
            proto_name.to_string()
        } else {
            let parts: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            format!("{proto_name}[{}]", parts.join(", "))
        };
        for m in misses {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ProtocolConformanceFailed {
                    type_name: m.type_name.unwrap_or_else(|| context.to_string()),
                    protocol_name: protocol_name.clone(),
                    reason: m.reason,
                },
                span: span.clone(),
            });
        }
    }

    /// `ty` が protocol `proto_name`（型引数 `args`）を**構造的に満たすか**（純粋な判定・タスク 1.3）。
    ///
    /// ⚠⚠ **`type_matches_exact` から呼ぶための述語版。** あちらは `&self` なので
    /// 診断を出す [`Self::check_protocol_conformance`]（`&mut self`）を呼べない。
    ///
    /// ⚠⚠ **これが無いと容器の内側の protocol が経路によって 2 通りに壊れる**（実測）:
    /// - `type_matches_exact` の `Protocol` アームは **任意の `NamedInstance` を受理**していた
    ///   ので、`fn f() -> Option[Pr]: return No(1)` とフィールド `list[Pr]` への代入が
    ///   **非適合クラスを通していた**（偽陰性）
    /// - 一方 `let xs: list[HasN] = [Dog(2)]` は `Pr` が `NamedInstance` のまま**名前**で
    ///   比較されるので、**構造的に満たしているのに弾かれていた**（偽陽性）
    ///
    /// ⚠ 判定は [`Self::protocol_misses`] の 1 つだけ（報告する版と同じ規則・6-5 で畳んだ）。
    pub(crate) fn satisfies_protocol(&self, ty: &InferredType, proto_name: &str, args: &[InferredType]) -> bool {
        self.protocol_misses(ty, proto_name, args).is_none_or(|m| m.is_empty())
    }

    /// `ty` が protocol を満たさない理由の一覧（満たせば空）。`None` は判定材料が無い（未知の protocol・型引数の個数が
    /// 宣言と合わない）＝満たすとみなす（個数の誤りは注釈の検査 `check_ann_protocol_args` が報告する）。
    ///
    /// ⚠ 自分を参照する protocol（numpy の `_NestedSequence[T]` の `__getitem__ -> T | _NestedSequence[T]`）は、照合中の組に
    ///   戻ってきたら満たすと仮定して止める（共帰納的な判定・[`TypeChecker::protocol_assumptions`]）。
    fn protocol_misses(&self, ty: &InferredType, proto_name: &str, args: &[InferredType]) -> Option<Vec<ProtocolMiss>> {
        let proto = self.registry.protocol(proto_name)?.clone();
        // `typing` / `collections.abc` の抽象クラスを基底に書いた Python のクラス（`class Bag(Sequence[int])`）は名目で満たす
        // （抽象クラスが足すメソッドは型スタブに現れない・pandas_import_plan.md の 6-8）
        if let InferredType::NamedInstance(cls) = ty {
            if self.registry.is_cpython_class(cls) && self.class_inherits(cls, proto_name, 0) {
                return Some(Vec::new());
            }
        }
        let subst = self.protocol_subst(proto_name, args)?;
        let key = format!("{ty} <: {proto_name}{args:?}");
        if self.protocol_assumptions.borrow().contains(&key) {
            return Some(Vec::new());
        }
        self.protocol_assumptions.borrow_mut().push(key);
        let misses = self.protocol_misses_of(ty, &proto, &subst);
        self.protocol_assumptions.borrow_mut().pop();
        misses
    }

    /// クラス `cls` が基底に `base` を（基底の基底もたどって）書いたか。
    fn class_inherits(&self, cls: &str, base: &str, depth: u32) -> bool {
        // ⚠ 深さの上限は循環（壊れたスタブ）の保険
        depth < 20
            && self
                .registry
                .class_bases(cls)
                .is_some_and(|bs| bs.iter().any(|b| b == base || self.class_inherits(b, base, depth + 1)))
    }

    /// 型引数の置き換えの表（protocol の型変数 → 型引数）。型引数を書かない総称の protocol（型テストの `is Holder`）は
    /// 型変数を何でも受ける（`Unresolved`）として読む。個数が宣言と合わなければ `None`。
    pub(crate) fn protocol_subst(
        &self,
        proto_name: &str,
        args: &[InferredType],
    ) -> Option<std::collections::HashMap<String, InferredType>> {
        let params = self.registry.template_params(proto_name).map(<[String]>::to_vec).unwrap_or_default();
        if args.is_empty() {
            return Some(params.into_iter().map(|p| (p, InferredType::Unresolved)).collect());
        }
        if params.len() != args.len() {
            return None;
        }
        Some(params.into_iter().zip(args.iter().cloned()).collect())
    }

    /// protocol のメンバーを型引数で読んだもの（フィールド: 名前 → (種類, 型)・メソッド: 名前 → シグネチャ）。
    /// 値の型が protocol のときのメンバーの型（`members.rs`）と、protocol どうしの照合が使う。
    pub(crate) fn protocol_members(
        &self,
        proto_name: &str,
        args: &[InferredType],
    ) -> Option<(
        std::collections::HashMap<String, (crate::ast::FieldKind, InferredType)>,
        std::collections::HashMap<String, Vec<FnSig>>,
    )> {
        let proto = self.registry.protocol(proto_name)?;
        let subst = self.protocol_subst(proto_name, args)?;
        let sub = |t: &InferredType| Self::subst_type_params(t, &subst);
        let fields = proto.fields.iter().map(|f| (f.name.clone(), (f.kind.clone(), sub(&f.ty)))).collect();
        let methods = proto
            .methods
            .iter()
            .map(|m| {
                let sig = FnSig {
                    param_mutable: m.params.iter().map(|(_, pmut, _)| *pmut).collect(),
                    params: m.params.iter().map(|(pname, _, pty)| (pname.clone(), Some(sub(pty)))).collect(),
                    required_count: m.params.len(),
                    return_type: Some(sub(&m.return_type)),
                    variadic_type: None,
                };
                (m.name.clone(), vec![sig])
            })
            .collect();
        Some((fields, methods))
    }

    /// 照合の相手（`ty`）のメンバー。
    fn conformer_of(&self, ty: &InferredType) -> Conformer {
        let class = |cls: &str| Conformer::Members {
            owner: Some(cls.to_string()),
            fields: self.collect_class_field_details(cls),
            methods: self.collect_class_method_sigs(cls),
        };
        match ty {
            // Any は全プロトコルを満たす。`Unresolved` / `Never` は判定材料が無い。
            InferredType::Any | InferredType::Unresolved | InferredType::Never => Conformer::Opaque,
            InferredType::NamedInstance(cls) => class(cls),
            // ⚠⚠ **テンプレートの具体化（`Box[int]`）も普通のクラスとして適合を見る**
            //   （タスク 2-14）。以前は「クラスのインスタンスではない」と**誤って弾いていた**
            //   （`let s: Sized = Box[int](1)`・実測）。単相化した具体クラスがあればそれを使う。
            //   無ければ（単相化されない具体化）判定材料が無いので見送る。
            InferredType::GenericInstance { .. } => match self.registry.instance_class(&ty.to_string()) {
                Some(cls) => class(cls),
                None => Conformer::Opaque,
            },
            // 別プロトコル型 — そのプロトコルのメンバー（型引数で読んだもの）が要求を満たすか
            InferredType::Protocol(p, pargs) => match self.protocol_members(p, pargs) {
                Some((fields, methods)) => Conformer::Members { owner: None, fields, methods },
                None => Conformer::Opaque,
            },
            // 組み込みの値は、組み込みの表のメンバーで照合する（`list[int]` は `__len__` を持つ・6-6）。
            // ⚠ 以前は「クラスのインスタンスではない」と弾いていた（どの protocol も満たさなかった）。
            _ => match crate::type_check::builtin_members::shape_of(ty) {
                Some(shape) => Conformer::Members {
                    owner: Some(ty.to_string()),
                    fields: std::collections::HashMap::new(),
                    methods: crate::type_check::builtin_members::members_of(shape.kind)
                        .iter()
                        .map(|m| (m.name().to_string(), m.sigs(&shape)))
                        .collect(),
                },
                None => Conformer::Mismatch(format!("expected a class instance, got `{ty}`")),
            },
        }
    }

    /// `ty` が protocol `proto`（型変数の表 `subst`）を満たさない理由。
    fn protocol_misses_of(
        &self,
        ty: &InferredType,
        proto: &crate::type_check::types::ProtocolInfo,
        subst: &std::collections::HashMap<String, InferredType>,
    ) -> Option<Vec<ProtocolMiss>> {
        let (owner, fields, methods) = match self.conformer_of(ty) {
            Conformer::Opaque => return None,
            Conformer::Mismatch(reason) => return Some(vec![ProtocolMiss { type_name: None, reason }]),
            Conformer::Members { owner, fields, methods } => (owner, fields, methods),
        };
        let miss = |reason: String| ProtocolMiss { type_name: owner.clone(), reason };
        let mut misses = Vec::new();
        for req in &proto.fields {
            let req_ty = Self::subst_type_params(&req.ty, subst);
            match fields.get(&req.name) {
                None => misses.push(miss(format!(
                    "missing field `{}` (expected {:?}: {})",
                    req.name, req.kind, req_ty
                ))),
                Some((actual_kind, _)) if *actual_kind != req.kind => misses.push(miss(format!(
                    "field `{}` is `{:?}` but protocol requires `{:?}`",
                    req.name, actual_kind, req.kind
                ))),
                // フィールドは読みも書きもできるので、型は同じでなければならない（両向きに入る）
                Some((_, actual_ty))
                    if req_ty != InferredType::Unresolved
                        && !(self.type_matches_exact(actual_ty, &req_ty) && self.type_matches_exact(&req_ty, actual_ty)) =>
                {
                    misses.push(miss(format!(
                        "field `{}` has type `{}` but protocol requires `{}`",
                        req.name, actual_ty, req_ty
                    )))
                }
                Some(_) => {}
            }
        }
        for req in &proto.methods {
            let req_sig = crate::type_check::types::ProtocolMethod {
                name: req.name.clone(),
                params: req
                    .params
                    .iter()
                    .map(|(n, m, t)| (n.clone(), *m, Self::subst_type_params(t, subst)))
                    .collect(),
                return_type: Self::subst_type_params(&req.return_type, subst),
            };
            match methods.get(&req.name) {
                None => misses.push(miss(format!("missing method `{}`", req.name))),
                // オーバーロードは 1 つでも一致すればよい
                Some(sigs) if !sigs.iter().any(|sig| self.method_sig_matches_protocol(sig, &req_sig)) => misses.push(
                    miss(format!("method `{}` signature does not match protocol requirement", req.name)),
                ),
                Some(_) => {}
            }
        }
        Some(misses)
    }

    /// クラスのフィールド詳細を継承チェーンを辿って収集する。
    pub(crate) fn collect_class_field_details(
        &self,
        class_name: &str,
    ) -> std::collections::HashMap<String, (crate::ast::FieldKind, InferredType)> {
        let mut result = std::collections::HashMap::new();
        // 基底クラスのフィールドを先に収集（上書きされる）
        if let Some(bases) = self.registry.class_bases(class_name) {
            for base in bases {
                let base_fields = self.collect_class_field_details(base);
                result.extend(base_fields);
            }
        }
        // 自クラスのフィールドで上書き
        if let Some(details) = self.registry.class_field_details(class_name) {
            result.extend(details.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        result
    }

    /// クラスのメソッドシグネチャを継承チェーンを辿って収集する。
    pub(crate) fn collect_class_method_sigs(
        &self,
        class_name: &str,
    ) -> std::collections::HashMap<String, Vec<crate::type_check::types::FnSig>> {
        let mut result = std::collections::HashMap::new();
        if let Some(bases) = self.registry.class_bases(class_name) {
            for base in bases {
                let base_methods = self.collect_class_method_sigs(base);
                result.extend(base_methods);
            }
        }
        if let Some(methods) = self.registry.class_methods(class_name) {
            result.extend(methods.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        result
    }

    /// メソッドシグネチャがプロトコルのメソッド要件を満たすか判定する。
    ///
    /// 引数は広くてよく（protocol の型の値を受けられる）、戻り値は狭くてよい（protocol の型として使える）。
    /// ⚠ 以前は型が完全に同じでなければならなかった（6-5 で部分型にした）。総称の protocol の要求（`-> T | Nested[T]`）を
    ///   具体的な型（`-> int`）で満たすのに要る。
    pub(crate) fn method_sig_matches_protocol(
        &self,
        sig: &crate::type_check::types::FnSig,
        req: &crate::type_check::types::ProtocolMethod,
    ) -> bool {
        use crate::type_check::type_utils::Site;
        // self を除くパラメータ
        let has_self = sig.params.first().is_some_and(|(name, _)| name == "self");
        let non_self_params: Vec<_> = sig.params.iter()
            .filter(|(name, _)| name != "self")
            .collect();
        // 要求より多い引数は、省略できるものならよい（`list.index(x, start, stop)` は `index(x)` を満たす・6-6）
        let required = sig.required_count.saturating_sub(usize::from(has_self));
        if non_self_params.len() < req.params.len() || required > req.params.len() {
            return false;
        }
        // 位置だけで渡す引数（名前が空・組み込みの値のメソッド）と特殊メソッド（演算子が呼ぶので名前で渡さない）は名前を見ない
        let dunder = req.name.starts_with("__") && req.name.ends_with("__");
        for ((p_name, p_ty), (r_name, _r_mut, r_ty)) in non_self_params.iter().zip(req.params.iter()) {
            if !dunder && !p_name.is_empty() && !r_name.is_empty() && p_name != r_name {
                return false;
            }
            if let Some(pty) = p_ty {
                if *pty != InferredType::Unresolved
                    && *r_ty != InferredType::Unresolved
                    && !self.types_compatible(r_ty, pty, Site::Other)
                {
                    return false;
                }
            }
        }
        // 戻り値型チェック
        if req.return_type != InferredType::Unresolved {
            if let Some(ret) = &sig.return_type {
                if *ret != InferredType::Unresolved && !self.types_compatible(ret, &req.return_type, Site::Other) {
                    return false;
                }
            }
        }
        true
    }
}

impl TypeChecker {
    /// クラスが宣言した**基底 trait の要求を満たすか**を検査する。
    ///
    /// # protocol との違い
    ///
    /// protocol は構造的適合なので「その型が要求を満たすか」を**使用箇所**で見る。
    /// trait は基底に書く名目的な継承なので、見るべきは**クラス宣言そのもの**。
    /// 「実装し忘れ」はパーサが既に検出している
    /// （`class C must override virtual method m from trait T`）ので、
    /// ここが埋めるのは **同名で宣言はあるが中身が食い違う**場合:
    ///
    /// - trait のフィールドをクラスが**別の型で再宣言**した
    /// - trait のメソッドをクラスが**別のシグネチャで実装**した
    ///
    /// ⚠ どちらも以前は**静的にも実行時にも素通り**していた（実測）。
    /// `mut hp: int` の trait に対しクラスが `mut hp: str` と書けたし、
    /// `fn speak(self, n: int) -> str` を `-> int` で実装できた。
    ///
    /// ⚠ クラスが再宣言していないフィールド・実装していないデフォルトメソッドは
    /// trait のものをそのまま継承するので**検査対象外**。
    pub(crate) fn check_trait_conformance(&mut self, class_name: &str) {
        let Some(bases) = self.registry.class_bases(class_name).map(|b| b.to_vec()) else {
            return;
        };
        for base in bases {
            self.check_one_trait(class_name, &base);
        }
    }

    fn check_one_trait(&mut self, class_name: &str, trait_name: &str) {
        // ⚠⚠ **trait 自身の型変数を積む。** `trait Holder[T]: fn get(self) -> T` に対し
        //    `class IntBox(Holder[int])` は `-> int` で実装するのが正しいが、積まずに
        //    照合すると `int != NamedInstance("T")` で**偽陽性**になる（実測）。
        //    ⚠ 具体型引数（`Holder[int]` の `int`）は `class_bases` が名前しか持たないため
        //      置換できない。⇒ 型変数を含む要求は**照合を見送る**（判らないものを通す）。
        let tparams = self
            .registry
            .template_params(trait_name)
            .map(|p| p.to_vec())
            .unwrap_or_default();
        let saved_tp = self.state.push_type_params(tparams.into_iter());

        // ── フィールド: クラスが再宣言しているものだけ突き合わせる ──
        let req_fields = self
            .registry
            .trait_field_details(trait_name)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        let own_fields = self
            .registry
            .class_field_details(class_name)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        for (fname, (req_kind, req_ty)) in &req_fields {
            let Some((_, (own_kind, own_ty))) = own_fields.iter().find(|(n, _)| n == fname) else {
                continue; // 再宣言していない ＝ trait のものを継承する
            };
            // ⚠ 型変数を含む要求は照合しない（テンプレート trait の実体化前）。
            if self.mentions_type_param(req_ty) || self.mentions_type_param(own_ty) {
                continue;
            }
            if matches!(req_ty, InferredType::Unresolved) {
                continue;
            }
            if own_ty != req_ty {
                let reason = format!(
                    "field `{fname}` is declared `{own_ty}` but trait requires `{req_ty}`"
                );
                self.report_trait_error(class_name, trait_name, reason);
            } else if own_kind != req_kind {
                let reason = format!(
                    "field `{fname}` is declared `{own_kind:?}` but trait requires `{req_kind:?}`"
                );
                self.report_trait_error(class_name, trait_name, reason);
            }
        }

        // ── メソッド: クラスが実装しているものだけ突き合わせる ──
        let req_methods = self
            .registry
            .trait_methods(trait_name)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        let own_methods = self
            .registry
            .class_methods(class_name)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        for (mname, req_sigs) in &req_methods {
            let Some((_, own_sigs)) = own_methods.iter().find(|(n, _)| n == mname) else {
                continue; // 実装していない ＝ trait のデフォルト実装を使う（不足はパーサが検出済み）
            };
            // trait 側・クラス側ともオーバーロードを持ちうる。
            // **どちらか 1 組でも噛み合えば適合**とする（`method_sig_matches_protocol` と同じ寛容さ）。
            let ok = req_sigs.iter().any(|req| {
                own_sigs.iter().any(|own| self.trait_sig_matches(own, req))
            });
            if !ok {
                let reason =
                    format!("method `{mname}` signature does not match the trait declaration");
                self.report_trait_error(class_name, trait_name, reason);
            }
        }
        self.state.pop_type_params(saved_tp);
    }

    fn report_trait_error(&mut self, class_name: &str, trait_name: &str, reason: String) {
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::TraitConformanceFailed {
                class_name: class_name.to_string(),
                trait_name: trait_name.to_string(),
                reason,
            },
            span: None,
        });
    }

    /// クラス側の実装 `own` が trait の宣言 `req` を満たすか。
    ///
    /// ⚠ `self` は比較しない（レシーバの可変性はクラス側の都合で決まる）。
    /// ⚠ 型が取れない側（注釈が解釈できない・型変数）は**照合を見送る**。
    ///   厳しくするより、判らないものを通す方が偽陽性を生まない。
    fn trait_sig_matches(&self, own: &FnSig, req: &FnSig) -> bool {
        let strip = |s: &FnSig| -> Vec<(String, Option<InferredType>)> {
            s.params
                .iter()
                .filter(|(n, _)| n != "self")
                .cloned()
                .collect()
        };
        let (o, r) = (strip(own), strip(req));
        if o.len() != r.len() {
            return false;
        }
        for ((on, ot), (rn, rt)) in o.iter().zip(r.iter()) {
            // ⚠ 引数名も一致させる。Arrow はキーワード引数を持つので、名前が変わると
            //    trait 越しの呼び出し（`obj.speak(n = 1)`）が壊れる。
            if on != rn {
                return false;
            }
            if let (Some(ot), Some(rt)) = (ot, rt) {
                if self.mentions_type_param(ot) || self.mentions_type_param(rt) {
                    continue;
                }
                if ot != rt {
                    return false;
                }
            }
        }
        match (&own.return_type, &req.return_type) {
            (Some(o), Some(r)) => {
                if self.mentions_type_param(o) || self.mentions_type_param(r) {
                    true
                } else {
                    o == r
                }
            }
            _ => true, // どちらかに注釈が無ければ見送る
        }
    }
}
