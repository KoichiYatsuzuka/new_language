use crate::ast::Expr;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// デコレータ式の型シグネチャを検査する。
    pub(super) fn check_decorator(
        &mut self,
        decorator: &Expr,
        target_is_fn: bool,
        target_name: &str,
    ) {
        self.infer(decorator);

        let dec_name = match decorator {
            Expr::Ident(name) => name.clone(),
            _ => return,
        };

        let expected_what = if target_is_fn { "function" } else { "type" };
        let target_kind = if target_is_fn { "function" } else { "class" };

        let is_fn_type = |ty: &InferredType| matches!(ty, InferredType::Function { .. });
        let is_type_type =
            |ty: &InferredType| matches!(ty, InferredType::TypeVal | InferredType::TypeValOf(_));
        let kind_matches = |ty: &InferredType| {
            if target_is_fn {
                is_fn_type(ty)
            } else {
                is_type_type(ty)
            }
        };

        // --- Case 1: 関数デコレータ ---
        if let Some(sigs) = self.fn_sigs.get(&dec_name).cloned() {
            if sigs.len() != 1 {
                return;
            }
            let sig = sigs[0].clone();

            match sig.params.first() {
                None => {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::InvalidDecorator {
                            reason: format!(
                                "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                 must have at least one parameter of '{expected_what}' type"
                            ),
                        },
                        span: None,
                    });
                }
                Some((_, Some(first_param_ty))) => {
                    if !kind_matches(first_param_ty) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::InvalidDecorator {
                                reason: format!(
                                    "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                     first parameter must be '{expected_what}' type, got '{first_param_ty}'"
                                ),
                            },
                            span: None,
                        });
                    }
                }
                Some((_, None)) => {}
            }

            if let Some(return_ty) = &sig.return_type.clone() {
                if !kind_matches(return_ty) {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::InvalidDecorator {
                            reason: format!(
                                "decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                 return type must be '{expected_what}', got '{return_ty}'"
                            ),
                        },
                        span: None,
                    });
                }
            }
            return;
        }

        // --- Case 2: クラスデコレータ ---
        if self.known_class_names.contains(dec_name.as_str()) {
            let cls_methods = match self.class_method_sigs.get(&dec_name).cloned() {
                Some(m) => m,
                None => return,
            };

            if let Some(init_sigs) = cls_methods.get("__init__").cloned() {
                if init_sigs.len() == 1 {
                    if let Some((_, second_ty_opt)) = init_sigs[0].params.get(1) {
                        if let Some(second_ty) = second_ty_opt {
                            if !kind_matches(second_ty) {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::InvalidDecorator {
                                        reason: format!(
                                            "class decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                             '__init__' second parameter must be '{expected_what}' type, got '{second_ty}'"
                                        ),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
            }

            if let Some(call_sigs) = cls_methods.get("__call__").cloned() {
                if call_sigs.len() == 1 {
                    if let Some(return_ty) = &call_sigs[0].return_type.clone() {
                        if !kind_matches(return_ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::InvalidDecorator {
                                    reason: format!(
                                        "class decorator '{dec_name}' applied to {target_kind} '{target_name}': \
                                         '__call__' return type must be '{expected_what}', got '{return_ty}'"
                                    ),
                                },
                                span: None,
                            });
                        }
                    }
                }
            }
        }
    }

    /// 型ガード式の右辺に書かれた型名文字列を [`InferredType`] に変換する。
    pub(super) fn type_from_guard_name(name: &str) -> InferredType {
        match name {
            "int" => InferredType::Int,
            "float" => InferredType::Float,
            "str" => InferredType::Str,
            "bool" => InferredType::Bool,
            "None" => InferredType::None,
            "Undefined" => InferredType::Undefined,
            other => InferredType::NamedInstance(other.to_string()),
        }
    }
}
