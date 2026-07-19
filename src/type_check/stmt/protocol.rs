// stmt/protocol.rs — プロトコル適合性検査: check_protocol_conformance とクラスのフィールド/メソッドシグネチャ収集・照合。

use {
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, TypeErrorKind},
    crate::type_check::types::InferredType,
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
        let proto = match self.known_protocols.get(proto_name).cloned() {
            Some(p) => p,
            None => return, // 未知のプロトコルは無視
        };

        let class_name = match ty {
            InferredType::NamedInstance(cls) => cls.clone(),
            InferredType::Any => return, // Any は全プロトコルを満たす
            InferredType::Protocol(p) => {
                // 別プロトコル型 — そのプロトコルがすべての要件を満たすか確認
                let other_proto = match self.known_protocols.get(p.as_str()).cloned() {
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
        if let Some(bases) = self.class_bases.get(class_name) {
            for base in bases.clone() {
                let base_fields = self.collect_class_field_details(&base);
                result.extend(base_fields);
            }
        }
        // 自クラスのフィールドで上書き
        if let Some(details) = self.class_field_details.get(class_name) {
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
        if let Some(bases) = self.class_bases.get(class_name) {
            for base in bases.clone() {
                let base_methods = self.collect_class_method_sigs(&base);
                result.extend(base_methods);
            }
        }
        if let Some(methods) = self.class_method_sigs.get(class_name) {
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
