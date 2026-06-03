#![allow(dead_code)]

use crate::ast::{CallArg, Expr};

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::{FnSig, FnTypeParam, InferredType};
use super::TypeChecker;

impl TypeChecker {
    /// 関数呼び出し式の型を推論し、引数の型・個数・Self 型パラメータを検査する。
    pub(super) fn infer_call(&mut self, func: &Expr, args: &[CallArg]) -> InferredType {
        // __freeze__ may only be invoked via the `freeze` keyword, never as a direct call.
        let freeze_span = match func {
            Expr::Attr { attr, span, .. } if attr == "__freeze__" => Some(span.clone()),
            Expr::Ident(name) if name == "__freeze__" => None,
            _ => {
                // Not a __freeze__ call — proceed normally.
                #[allow(clippy::needless_return)]
                return self.infer_call_inner(func, args);
            }
        };
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::DirectFreezeCall,
            span: freeze_span,
        });
        return InferredType::Unresolved;
    }

    fn infer_call_inner(&mut self, func: &Expr, args: &[CallArg]) -> InferredType {
        let method_call_info: Option<(String, String)> =
            if let Expr::Attr { object, attr, span } = func {
                let obj_ty = match object.as_ref() {
                    Expr::Ident(n) => self
                        .lookup(n)
                        .map(|v| v.ty.clone())
                        .unwrap_or(InferredType::Unresolved),
                    _ => InferredType::Unresolved,
                };
                if let InferredType::NamedInstance(cls_name) = obj_ty {
                    let is_static = self
                        .class_static_methods
                        .get(&cls_name)
                        .map(|s| s.contains(attr.as_str()))
                        .unwrap_or(false);
                    if is_static {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::StaticMethodOnInstance {
                                method_name: attr.clone(),
                                class_name: cls_name.clone(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    Some((cls_name, attr.clone()))
                } else {
                    None
                }
            } else {
                None
            };

        let func_name = match func {
            Expr::Ident(name) => Some(name.clone()),
            Expr::Attr { attr, .. } => Some(attr.clone()),
            _ => None,
        };
        let func_type = self.infer(func);

        let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
        for arg in args.iter() {
            match arg {
                CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                CallArg::Keyword { name, value } => {
                    arg_data.push((Some(name.clone()), self.infer(value)))
                }
            }
        }

        match func_type {
            InferredType::Function {
                params: Some(fn_params),
                return_type,
            } => {
                let fname = func_name.as_deref().unwrap_or("<function>").to_string();
                let ret = *return_type;
                self.check_fn_type_call(&fname, args, &arg_data, &fn_params);
                return ret;
            }
            InferredType::Function { params: None, .. } => {
                return InferredType::Any;
            }
            _ => {}
        }

        if let Some((ref cls_name, ref method_name)) = method_call_info {
            self.check_self_type_params(cls_name, method_name, &arg_data);
        } else if let Some(ref fname) = func_name {
            self.check_call_args(fname, &arg_data);
        }

        if let Some(ref fname) = func_name {
            if self.known_class_names.contains(fname.as_str()) {
                return InferredType::NamedInstance(fname.clone());
            }
        }

        func_name
            .as_deref()
            .and_then(|n| self.fn_sigs.get(n))
            .and_then(|sigs| {
                let call_count = arg_data.len();
                let matching: Vec<_> = sigs
                    .iter()
                    .filter(|s| call_count >= s.required_count && call_count <= s.params.len())
                    .collect();
                if matching.len() == 1 {
                    matching[0].return_type.clone()
                } else {
                    None
                }
            })
            .unwrap_or(InferredType::Unresolved)
    }

    /// `Self` 型パラメータの制約を検査する。
    pub(super) fn check_self_type_params(
        &mut self,
        cls_name: &str,
        method_name: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let sigs = match self
            .class_method_sigs
            .get(cls_name)
            .and_then(|m| m.get(method_name))
            .cloned()
        {
            Some(s) => s,
            None => return,
        };
        let effective_count = arg_data.len() + 1;
        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| effective_count >= s.required_count && effective_count <= s.params.len())
            .cloned()
            .collect();
        if count_matching.len() != 1 {
            return;
        }
        let sig = &count_matching[0];
        for (arg_idx, (_, arg_ty)) in arg_data.iter().enumerate() {
            let param_idx = arg_idx + 1;
            if let Some((param_name, Some(InferredType::SelfType))) = sig.params.get(param_idx) {
                if let InferredType::NamedInstance(got_cls) = arg_ty {
                    if got_cls != cls_name {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::SelfTypeMismatch {
                                method: method_name.to_string(),
                                param_name: param_name.clone(),
                                expected_class: cls_name.to_string(),
                                got_class: got_cls.clone(),
                            },
                            span: None,
                        });
                    }
                }
            }
        }
    }

    /// 名前付き関数呼び出しの引数個数・型・キーワード引数名を検査する。
    pub(super) fn check_call_args(
        &mut self,
        fname: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let sigs = match self.fn_sigs.get(fname).cloned() {
            Some(s) => s,
            None => return,
        };
        let call_count = arg_data.len();
        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| call_count >= s.required_count && call_count <= s.params.len())
            .cloned()
            .collect();

        if count_matching.is_empty() {
            if sigs.len() == 1 {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgCountMismatch {
                        func_name: fname.to_string(),
                        expected_min: sigs[0].required_count,
                        expected_max: sigs[0].params.len(),
                        got: call_count,
                    },
                    span: None,
                });
            } else {
                let available = sigs.iter().map(|s| s.params.len()).collect();
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoMatchingOverload {
                        func_name: fname.to_string(),
                        got: call_count,
                        available,
                    },
                    span: None,
                });
            }
            return;
        }
        if count_matching.len() > 1 {
            return;
        }

        let sig = &count_matching[0];
        let mut positional_idx = 0usize;
        for (key, arg_ty) in arg_data {
            match key {
                Some(kwarg_name) => {
                    match sig.params.iter().position(|(n, _)| n == kwarg_name) {
                        None => self.report_error(StaticTypeError {
                            kind: TypeErrorKind::UnknownKeywordArg {
                                func_name: fname.to_string(),
                                arg_name: kwarg_name.clone(),
                            },
                            span: None,
                        }),
                        Some(param_pos) => {
                            if let Some(expected) = &sig.params[param_pos].1 {
                                if !self.type_matches(arg_ty, expected) {
                                    self.report_error(StaticTypeError {
                                        kind: TypeErrorKind::CallArgTypeMismatch {
                                            func_name: fname.to_string(),
                                            param_index: param_pos,
                                            expected: expected.clone(),
                                            got: arg_ty.clone(),
                                        },
                                        span: None,
                                    });
                                }
                            }
                        }
                    }
                }
                None => {
                    if let Some((_, param_ty)) = sig.params.get(positional_idx) {
                        if let Some(expected) = param_ty {
                            if !self.type_matches(arg_ty, expected) {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::CallArgTypeMismatch {
                                        func_name: fname.to_string(),
                                        param_index: positional_idx,
                                        expected: expected.clone(),
                                        got: arg_ty.clone(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                    positional_idx += 1;
                }
            }
        }
    }

    /// 関数型変数の呼び出し検査：引数個数・型・キーワード名・`mut` 引数の可変性を検査する。
    pub(super) fn check_fn_type_call(
        &mut self,
        func_name: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
        params: &[FnTypeParam],
    ) {
        if arg_data.len() != params.len() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CallArgCountMismatch {
                    func_name: func_name.to_string(),
                    expected_min: params.len(),
                    expected_max: params.len(),
                    got: arg_data.len(),
                },
                span: None,
            });
            return;
        }

        let mut positional_idx = 0usize;
        for (i, (key, arg_ty)) in arg_data.iter().enumerate() {
            let arg_expr = args[i].expr();
            match key {
                Some(kwarg_name) => match params.iter().position(|p| &p.name == kwarg_name) {
                    None => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::UnknownKeywordArg {
                            func_name: func_name.to_string(),
                            arg_name: kwarg_name.clone(),
                        },
                        span: None,
                    }),
                    Some(param_pos) => {
                        let param = &params[param_pos];
                        if param.ty != InferredType::Any && !self.type_matches(arg_ty, &param.ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: param_pos,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                },
                None => {
                    if let Some(param) = params.get(positional_idx) {
                        if param.ty != InferredType::Any && !self.type_matches(arg_ty, &param.ty) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: positional_idx,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    positional_idx += 1;
                }
            }
        }
    }

    /// 式が可変変数の参照かどうかを判定する。
    pub(super) fn is_mutable_expr(&self, expr: &Expr) -> bool {
        if let Expr::Ident(name) = expr {
            self.lookup(name).map(|v| v.mutable).unwrap_or(false)
        } else {
            false
        }
    }
}
