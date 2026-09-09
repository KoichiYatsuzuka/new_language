// stmt/protocol.rs — プロトコル適合性検査: check_protocol_conformance とクラスのフィールド/メソッドシグネチャ収集・照合。

use {
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, TypeErrorKind},
    crate::type_check::types::{FnSig, InferredType},
    crate::type_check::TypeChecker,
};

impl TypeChecker {
    /// 型 `ty` がプロトコル `proto_name` を満たすか検査する。
    /// 満たさない場合は StaticTypeError を記録する。
    pub(crate) fn check_protocol_conformance(
        &mut self,
        ty: &InferredType,
        proto_name: &str,
        span: Option<Span>,
        context: &str,
    ) {
        let proto = match self.registry.protocol(proto_name).cloned() {
            Some(p) => p,
            None => return, // 未知のプロトコルは無視
        };

        let class_name = match ty {
            InferredType::NamedInstance(cls) => cls.clone(),
            InferredType::Any => return, // Any は全プロトコルを満たす
            InferredType::Protocol(p) => {
                // 別プロトコル型 — そのプロトコルがすべての要件を満たすか確認
                let other_proto = match self.registry.protocol(p.as_str()).cloned() {
                    Some(op) => op,
                    None => return,
                };
                // フィールドチェック
                for req_field in &proto.fields {
                    let found = other_proto.fields.iter().find(|f| f.name == req_field.name);
                    if let Some(f) = found {
                        if f.kind != req_field.kind || f.ty != req_field.ty {
                            let reason = format!(
                                "field `{}` has kind/type `{:?}:{:?}` but protocol requires `{:?}:{:?}`",
                                req_field.name, f.kind, f.ty, req_field.kind, req_field.ty
                            );
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::ProtocolConformanceFailed {
                                    type_name: context.to_string(),
                                    protocol_name: proto_name.to_string(),
                                    reason,
                                },
                                span: span.clone(),
                            });
                        }
                    } else {
                        let reason = format!("missing field `{}`", req_field.name);
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::ProtocolConformanceFailed {
                                type_name: context.to_string(),
                                protocol_name: proto_name.to_string(),
                                reason,
                            },
                            span: span.clone(),
                        });
                    }
                }
                return;
            }
            _ => {
                let reason = format!("expected a class instance, got `{ty}`");
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ProtocolConformanceFailed {
                        type_name: context.to_string(),
                        protocol_name: proto_name.to_string(),
                        reason,
                    },
                    span,
                });
                return;
            }
        };

        // クラスのフィールド詳細とメソッドシグネチャを取得（クラス継承チェーンを含む）
        let field_details = self.collect_class_field_details(&class_name);
        let method_sigs = self.collect_class_method_sigs(&class_name);

        // フィールド適合チェック
        for req_field in &proto.fields {
            match field_details.get(&req_field.name) {
                None => {
                    let reason = format!(
                        "missing field `{}` (expected {:?}: {})",
                        req_field.name, req_field.kind, req_field.ty
                    );
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::ProtocolConformanceFailed {
                            type_name: class_name.clone(),
                            protocol_name: proto_name.to_string(),
                            reason,
                        },
                        span: span.clone(),
                    });
                }
                Some((actual_kind, actual_ty)) => {
                    if *actual_kind != req_field.kind {
                        let reason = format!(
                            "field `{}` is `{:?}` but protocol requires `{:?}`",
                            req_field.name, actual_kind, req_field.kind
                        );
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::ProtocolConformanceFailed {
                                type_name: class_name.clone(),
                                protocol_name: proto_name.to_string(),
                                reason,
                            },
                            span: span.clone(),
                        });
                    } else if *actual_ty != req_field.ty && req_field.ty != InferredType::Unresolved {
                        let reason = format!(
                            "field `{}` has type `{}` but protocol requires `{}`",
                            req_field.name, actual_ty, req_field.ty
                        );
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::ProtocolConformanceFailed {
                                type_name: class_name.clone(),
                                protocol_name: proto_name.to_string(),
                                reason,
                            },
                            span: span.clone(),
                        });
                    }
                }
            }
        }

        // メソッド適合チェック
        for req_method in &proto.methods {
            match method_sigs.get(&req_method.name) {
                None => {
                    let reason = format!("missing method `{}`", req_method.name);
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::ProtocolConformanceFailed {
                            type_name: class_name.clone(),
                            protocol_name: proto_name.to_string(),
                            reason,
                        },
                        span: span.clone(),
                    });
                }
                Some(sigs) => {
                    // sigs はオーバーロードリスト; 少なくとも1つがシグネチャ一致であればOK
                    let matches_any = sigs.iter().any(|sig| {
                        Self::method_sig_matches_protocol(sig, req_method)
                    });
                    if !matches_any {
                        let reason = format!(
                            "method `{}` signature does not match protocol requirement",
                            req_method.name
                        );
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::ProtocolConformanceFailed {
                                type_name: class_name.clone(),
                                protocol_name: proto_name.to_string(),
                                reason,
                            },
                            span: span.clone(),
                        });
                    }
                }
            }
        }
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
    pub(crate) fn method_sig_matches_protocol(
        sig: &crate::type_check::types::FnSig,
        req: &crate::type_check::types::ProtocolMethod,
    ) -> bool {
        // self を除くパラメータ
        let non_self_params: Vec<_> = sig.params.iter()
            .filter(|(name, _)| name != "self")
            .collect();
        if non_self_params.len() != req.params.len() {
            return false;
        }
        for ((p_name, p_ty), (r_name, r_mut, r_ty)) in non_self_params.iter().zip(req.params.iter()) {
            if p_name != r_name {
                return false;
            }
            if let Some(pty) = p_ty {
                if pty != r_ty && *r_ty != InferredType::Unresolved {
                    return false;
                }
            }
            let _ = r_mut; // mutability check placeholder — FnSig doesn't store mutable per-param
        }
        // 戻り値型チェック
        if req.return_type != InferredType::Unresolved {
            if let Some(ret) = &sig.return_type {
                if *ret != req.return_type {
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
