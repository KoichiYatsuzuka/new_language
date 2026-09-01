use crate::ast::{CallArg, Expr};
use crate::type_check::types::InferredType as IT;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::{FnSig, FnTypeParam, InferredType};
use super::TypeChecker;

impl TypeChecker {
    /// 関数呼び出し式の型を推論し、引数の型・個数・Self 型パラメータを検査する。
    pub(super) fn infer_call(
        &mut self,
        func: &Expr,
        args: &[CallArg],
        node_id: u32,
    ) -> InferredType {
        // __freeze__ may only be invoked via the `freeze` keyword, never as a direct call.
        let freeze_span = match func {
            Expr::Attr { attr, span, .. } if attr == "__freeze__" => Some(span.clone()),
            Expr::Ident { name, .. } if name == "__freeze__" => None,
            _ => {
                // Not a __freeze__ call — proceed normally.
                #[allow(clippy::needless_return)]
                return self.infer_call_inner(func, args, node_id);
            }
        };
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::DirectFreezeCall,
            span: freeze_span,
        });
        InferredType::Unresolved
    }

    fn infer_call_inner(&mut self, func: &Expr, args: &[CallArg], node_id: u32) -> InferredType {
        // ── Step 1: method-call detection ──────────────────────────────────────
        // Infer the object type exactly once here. This handles:
        //   - NamedInstance: regular instance method calls (obj.method())
        //   - TypeValOf(NamedInstance): class method calls (ClassName.method())
        //   - Non-Ident objects: method chains (Builder(1).set(5))
        // When method_call_info is Some, we skip self.infer(func) later to avoid
        // double-evaluating the object and duplicating error reports.
        let method_call_info: Option<(String, String)> =
            if let Expr::Attr { object, attr, span, .. } = func {
                let obj_ty = self.infer(object);

                // Result[T, E] の is_OK() / is_ERR() は特別扱いして bool を返す。
                // 他のメソッドやアトリビュートアクセスは OperationOnUnion エラーを発生させる。
                if let IT::Result(_, _) = &obj_ty {
                    if (attr == "is_OK" || attr == "is_ERR") && args.is_empty() {
                        return InferredType::Bool;
                    } else {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnUnion {
                                union_type: obj_ty.to_string(),
                                op: format!("method/attribute `{attr}`"),
                            },
                            span: Some(span.clone()),
                        });
                        return InferredType::Unresolved;
                    }
                }

                let cls_name_opt: Option<String> = match &obj_ty {
                    InferredType::NamedInstance(cls) => Some(cls.clone()),
                    InferredType::TypeValOf(inner) => {
                        if let InferredType::NamedInstance(cls) = inner.as_ref() {
                            Some(cls.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(cls_name) = cls_name_opt {
                    let is_static = self.registry.is_static_method(&cls_name, attr.as_str());
                    // StaticMethodOnInstance: only report when called on an instance
                    if is_static && matches!(obj_ty, InferredType::NamedInstance(_)) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::StaticMethodOnInstance {
                                method_name: attr.clone(),
                                class_name: cls_name.clone(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    // Member accessibility check (same as infer(Attr) does for NamedInstance)
                    if matches!(obj_ty, InferredType::NamedInstance(_)) {
                        self.check_member_access_static(&cls_name, attr, Some(span.clone()));
                    }
                    Some((cls_name, attr.clone()))
                } else {
                    None
                }
            } else {
                None
            };

        let func_name = match func {
            Expr::Ident { name, .. } => Some(name.clone()),
            Expr::Attr { attr, .. } => Some(attr.clone()),
            _ => None,
        };

        // Only evaluate func type when not a method call. For method calls the object
        // was already inferred above; re-calling self.infer(func) would re-evaluate
        // the object expression and potentially duplicate errors.
        let func_type = if method_call_info.is_none() {
            self.infer(func)
        } else {
            InferredType::Unresolved
        };

        let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
        for arg in args.iter() {
            match arg {
                CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                CallArg::Keyword { name, value } => {
                    arg_data.push((Some(name.clone()), self.infer(value)))
                }
                // 可変長引数: 各要素の型を推論し、リスト型として "..." キーで登録
                CallArg::Variadic(exprs) => {
                    let elem_types: Vec<InferredType> =
                        exprs.iter().map(|e| self.infer(e)).collect();
                    let list_ty = if elem_types.is_empty() {
                        InferredType::List
                    } else {
                        let first = &elem_types[0];
                        if elem_types.iter().all(|t| t == first) {
                            InferredType::ListOf(Box::new(first.clone()))
                        } else {
                            InferredType::List
                        }
                    };
                    arg_data.push((Some("...".to_string()), list_ty));
                }
            }
        }

        // ── AST 型解決層（#16）── Call 構造化注釈を焼く（arg_data・func_name はここで確定済み）。
        // 呼び先＝シンボル参照（名前）、引数＝(型, 検査指示)。
        // 検査指示（境界検査・点4 の畳み込み）: **直接関数呼び出し（単一シグネチャ・全引数が位置引数）**で、
        // param が具象・arg が動的（`Any`/`Unresolved`）のとき `CheckBefore(param型)`。それ以外は保守的に `None`
        // （overload・メソッド・キーワード/可変長・関数型変数などは次段で精緻化）。
        {
            // パラメータ型の取得（全引数が位置引数のときのみ・単一シグネチャに限る）。
            // overload・キーワード/可変長・関数{params:None}・静的メソッドは静的に param 型が一意でないため None。
            let positional_only = arg_data.iter().all(|(k, _)| k.is_none());
            let param_types: Option<Vec<Option<InferredType>>> = if !positional_only {
                None
            } else if let Some((cls, method)) = &method_call_info {
                // インスタンスメソッド呼び出し: 単一シグネチャ・非 static のとき self(先頭)を除いて対応付け。
                if self.registry.is_static_method(cls, method) {
                    None
                } else {
                    self.registry.class_methods(cls).and_then(|m| m.get(method)).and_then(|sigs| {
                        (sigs.len() == 1).then(|| {
                            sigs[0].params.iter().skip(1).map(|(_, t)| t.clone()).collect()
                        })
                    })
                }
            } else if let IT::Function { params: Some(fn_params), .. } = &func_type {
                // 関数型変数/パラメータ: シグネチャの各 param 型を使う。
                Some(fn_params.iter().map(|p| Some(p.ty.clone())).collect())
            } else if let Expr::Ident { name, .. } = func {
                // 直接グローバル関数呼び出し（単一シグネチャ）。
                self.registry.fn_sigs(name).and_then(|sigs| {
                    (sigs.len() == 1)
                        .then(|| sigs[0].params.iter().map(|(_, t)| t.clone()).collect())
                })
            } else {
                None
            };

            let mut args_ann = Vec::with_capacity(arg_data.len());
            for (i, (_, arg_ty)) in arg_data.iter().enumerate() {
                let tid = self.annotations.intern(arg_ty.clone());
                let directive = match param_types.as_ref().and_then(|p| p.get(i)) {
                    Some(Some(param_ty))
                        if is_specific_param(param_ty) && is_dynamic_arg(arg_ty) =>
                    {
                        let ptid = self.annotations.intern(param_ty.clone());
                        super::annotations::Directive::CheckBefore(ptid)
                    }
                    _ => super::annotations::Directive::None,
                };
                args_ann.push(super::annotations::ArgAnnotation { ty: tid, directive });
            }
            self.annotations.set_call(
                node_id,
                super::annotations::CallInfo {
                    callee: func_name.clone(),
                    args: args_ann,
                },
            );
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
            InferredType::Function { params: None, return_type } => {
                return *return_type;
            }
            _ => {}
        }

        if let Some((ref cls_name, ref method_name)) = method_call_info {
            // self/cls is passed implicitly; check_self_type_params accounts for the +1
            let ret_ty = self.check_self_type_params(cls_name, method_name, &arg_data);
            return ret_ty.unwrap_or(InferredType::Unresolved);
        } else if let Some(ref fname) = func_name {
            self.check_call_args(fname, &arg_data);
        }

        if let Some(ref fname) = func_name {
            if self.registry.is_protocol(fname.as_str()) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ProtocolInstantiation {
                        protocol_name: fname.clone(),
                    },
                    span: None,
                });
                return InferredType::Protocol(fname.clone());
            }
            if self.registry.is_known_class(fname.as_str()) {
                return InferredType::NamedInstance(fname.clone());
            }
        }

        func_name
            .as_deref()
            .and_then(|n| self.registry.fn_sigs(n))
            .and_then(|sigs| {
                let call_count = arg_data
                    .iter()
                    .filter(|(k, _)| k.as_deref() != Some("..."))
                    .count();
                let matching: Vec<_> = sigs
                    .iter()
                    .filter(|s| {
                        call_count >= s.required_count
                            && (s.variadic_type.is_some() || call_count <= s.params.len())
                    })
                    .collect();
                if matching.len() == 1 {
                    matching[0].return_type.clone()
                } else {
                    None
                }
            })
            .unwrap_or(InferredType::Unresolved)
    }

    /// メソッド呼び出しの引数を検査し、メソッドの戻り値型を返す。
    ///
    /// `self`/`cls` は呼び出し元が暗黙的に渡すため、`effective_count = arg_data.len() + 1`
    /// で実際のパラメータ数と照合する。
    pub(super) fn check_self_type_params(
        &mut self,
        cls_name: &str,
        method_name: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        let sigs = self
            .registry
            .class_methods(cls_name)
            .and_then(|m| m.get(method_name))
            .cloned()?;
        // Static methods have no implicit receiver; instance/class methods add +1 for self/cls
        let is_static = self.registry.is_static_method(cls_name, method_name);
        // 可変長引数エントリを除いた通常引数のみでカウント
        let normal_args: Vec<_> = arg_data
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();
        let variadic_entry = arg_data.iter().find(|(k, _)| k.as_deref() == Some("..."));
        let effective_count = if is_static {
            normal_args.len()
        } else {
            normal_args.len() + 1
        };
        let count_matching: Vec<FnSig> = sigs
            .iter()
            // ⚠ `variadic_type` が Some なら引数の上限は無い（`*args` / `**kwargs` を持つ）。
            .filter(|s| {
                effective_count >= s.required_count
                    && (s.variadic_type.is_some() || effective_count <= s.params.len())
            })
            .cloned()
            .collect();
        if count_matching.is_empty() {
            // Arg count mismatch: for instance/class methods subtract 1 to exclude implicit self/cls
            let implicit = usize::from(!is_static);
            if sigs.len() == 1 {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgCountMismatch {
                        func_name: format!("{cls_name}.{method_name}"),
                        expected_min: sigs[0].required_count.saturating_sub(implicit),
                        expected_max: sigs[0].params.len().saturating_sub(implicit),
                        got: normal_args.len(),
                    },
                    span: None,
                });
            } else {
                let available = sigs
                    .iter()
                    .map(|s| s.params.len().saturating_sub(implicit))
                    .collect();
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoMatchingOverload {
                        func_name: format!("{cls_name}.{method_name}"),
                        got: normal_args.len(),
                        available,
                    },
                    span: None,
                });
            }
            return None;
        }
        if count_matching.len() != 1 {
            return None;
        }
        let sig = &count_matching[0];
        // 可変長引数の型チェック
        if let (Some((_, IT::ListOf(elem_ty))), Some(expected_elem_ty)) =
            (variadic_entry, &sig.variadic_type)
        {
            if !self.type_matches(elem_ty, expected_elem_ty) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: format!("{cls_name}.{method_name}"),
                        param_index: usize::MAX,
                        expected: expected_elem_ty.clone(),
                        got: *elem_ty.clone(),
                    },
                    span: None,
                });
            }
        }
        for (arg_idx, (_, arg_ty)) in normal_args.iter().enumerate() {
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
        sig.return_type.clone()
    }

    /// 名前付き関数呼び出しの引数個数・型・キーワード引数名を検査する。
    pub(super) fn check_call_args(
        &mut self,
        fname: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let sigs = match self.registry.fn_sigs(fname).cloned() {
            Some(s) => s,
            None => return,
        };

        // 可変長引数エントリを分離
        let variadic_entry = arg_data.iter().find(|(k, _)| k.as_deref() == Some("..."));
        let normal_args: Vec<_> = arg_data
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();
        let call_count = normal_args.len();

        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| {
                        call_count >= s.required_count
                            && (s.variadic_type.is_some() || call_count <= s.params.len())
                    })
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
        for (key, arg_ty) in &normal_args {
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
                                            got: (*arg_ty).clone(),
                                        },
                                        span: None,
                                    });
                                }
                            }
                        }
                    }
                }
                None => {
                    if let Some((_, Some(expected))) = sig.params.get(positional_idx) {
                        if !self.type_matches(arg_ty, expected) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: fname.to_string(),
                                    param_index: positional_idx,
                                    expected: expected.clone(),
                                    got: (*arg_ty).clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    positional_idx += 1;
                }
            }
        }

        // 可変長引数の型チェック
        if let (Some((_, IT::ListOf(elem_ty))), Some(expected_elem_ty)) =
            (variadic_entry, &sig.variadic_type)
        {
            if !self.type_matches(elem_ty, expected_elem_ty) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: fname.to_string(),
                        param_index: usize::MAX, // 可変長引数を示す特殊値
                        expected: expected_elem_ty.clone(),
                        got: *elem_ty.clone(),
                    },
                    span: None,
                });
            }
        }

        // Protocol 型パラメータへの引数の適合チェック
        let mut pos_idx = 0usize;
        for (key, arg_ty) in &normal_args {
            let param_ty_opt = match key {
                Some(kwarg_name) => sig.params.iter().find(|(n, _)| n == kwarg_name).and_then(|(_, t)| t.clone()),
                None => {
                    let t = sig.params.get(pos_idx).and_then(|(_, t)| t.clone());
                    pos_idx += 1;
                    t
                }
            };
            if let Some(InferredType::Protocol(proto_name)) = param_ty_opt {
                let context = format!("argument to `{fname}`");
                self.check_protocol_conformance(arg_ty, &proto_name, None, &context);
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
        // デフォルトを持つ仮引数は省略できるので、必要数は `has_default` が false の個数。
        let required = params.iter().filter(|p| !p.has_default).count();
        if arg_data.len() < required || arg_data.len() > params.len() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CallArgCountMismatch {
                    func_name: func_name.to_string(),
                    expected_min: required,
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
        if let Expr::Ident { name, .. } = expr {
            self.lookup(name).map(|v| v.mutable).unwrap_or(false)
        } else {
            false
        }
    }
}

// ── AST 型解決層（#16）の引数検査指示ヘルパ ──
/// 引数の静的型が「完全に動的（実行時まで型が不明）」か。`Any`/`Unresolved` のとき true。
/// これらは呼び出し前に対象型で検査しないと具象パラメータへ渡せない（境界検査）。
fn is_dynamic_arg(t: &InferredType) -> bool {
    matches!(t, InferredType::Any | InferredType::Unresolved)
}

/// パラメータ型が「具象（検査に値する特定の型）」か。`Any`/`Unresolved` 以外なら true。
fn is_specific_param(t: &InferredType) -> bool {
    !matches!(t, InferredType::Any | InferredType::Unresolved)
}
