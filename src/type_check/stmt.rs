#![allow(dead_code)]

use crate::ast::{Expr, FieldKind, MatchPattern, Stmt, TupleTarget};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use super::types::{FnTypeParam, InferredType};
use super::TypeChecker;

impl TypeChecker {
    /// 文のスライスを順に型検査する。
    pub(super) fn check_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    /// 単一の文を型検査する。変数宣言・代入・制御構文・定義文・例外処理・import を網羅する。
    pub(super) fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            // --- 変数宣言 ---
            Stmt::Let(name, type_ann, expr) => {
                let rhs_ty = self.infer(expr);
                if rhs_ty == InferredType::Undefined {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: None,
                    });
                }
                if name != "_" && self.lookup(name).is_some() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                        span: None,
                    });
                }
                let ty = self.resolve_declared_type(type_ann.as_deref(), rhs_ty, name, stmt);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Const(name, type_ann, expr) => {
                let rhs_ty = self.infer(expr);
                if rhs_ty == InferredType::Undefined {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: None,
                    });
                }
                if name != "_" && self.lookup(name).is_some() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                        span: None,
                    });
                }
                let ty = self.resolve_declared_type(type_ann.as_deref(), rhs_ty, name, stmt);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Mut(name, type_ann, expr) => {
                let rhs_ty = self.infer(expr);
                if rhs_ty == InferredType::Undefined {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: None,
                    });
                }
                if name != "_" && self.lookup(name).is_some() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                        span: None,
                    });
                }
                let ty = self.resolve_declared_type(type_ann.as_deref(), rhs_ty, name, stmt);
                self.declare(name.clone(), ty, true);
            }
            Stmt::Static(name, expr, _) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, true);
            }
            Stmt::LetTuple {
                targets,
                value,
                span,
            } => {
                let rhs_ty = self.infer(value);

                for target in targets.iter() {
                    if let TupleTarget::Bare(name) = target {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::TupleUnpackMissingQualifier { name: name.clone() },
                            span: Some(span.clone()),
                        });
                    }
                }

                if let InferredType::Tuple(ref elem_types) = rhs_ty {
                    let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
                    let named = targets
                        .iter()
                        .filter(|t| !matches!(t, TupleTarget::Wildcard))
                        .count();
                    let tlen = elem_types.len();
                    let bad = if has_wildcard {
                        named > tlen
                    } else {
                        named != tlen
                    };
                    if bad {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::TupleUnpackArityMismatch {
                                tuple_len: tlen,
                                target_count: named,
                                has_wildcard,
                            },
                            span: Some(span.clone()),
                        });
                    }
                }

                let elem_types = if let InferredType::Tuple(ref v) = rhs_ty {
                    v.clone()
                } else {
                    vec![]
                };
                for (i, target) in targets.iter().enumerate() {
                    let ty = elem_types.get(i).cloned().unwrap_or(InferredType::Any);
                    match target {
                        TupleTarget::Let(name) | TupleTarget::Bare(name) => {
                            if name != "_" && self.lookup(name).is_some() {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                                    span: Some(span.clone()),
                                });
                            }
                            self.declare(name.clone(), ty, false)
                        }
                        TupleTarget::Mut(name) => {
                            if name != "_" && self.lookup(name).is_some() {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                                    span: Some(span.clone()),
                                });
                            }
                            self.declare(name.clone(), ty, true)
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
            }

            // --- 代入 ---
            Stmt::Assign { name, value, span, .. } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                let rhs_ty = self.infer(value);
                if rhs_ty == InferredType::Undefined {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: Some(span.clone()),
                    });
                }
            }
            Stmt::CompoundAssign {
                name,
                op: _,
                value,
                span,
                ..
            } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::AttrAssign { target, value } => {
                if matches!(target, Expr::Subscript { .. }) {
                    if let Some(name) = Self::subscript_root_ident(target) {
                        if let Some(info) = self.lookup(name) {
                            if !info.mutable {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::AssignToImmutable {
                                        name: name.to_string(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
                self.check_immutable_field_assign(target);
                self.infer(target);
                self.infer(value);
            }
            Stmt::AttrCompoundAssign {
                target,
                op: _,
                value,
            } => {
                if matches!(target, Expr::Subscript { .. }) {
                    if let Some(name) = Self::subscript_root_ident(target) {
                        if let Some(info) = self.lookup(name) {
                            if !info.mutable {
                                self.report_error(StaticTypeError {
                                    kind: TypeErrorKind::AssignToImmutable {
                                        name: name.to_string(),
                                    },
                                    span: None,
                                });
                            }
                        }
                    }
                }
                self.check_immutable_field_assign(target);
                self.infer(target);
                self.infer(value);
            }

            // --- 式文 ---
            Stmt::Expr(expr) => {
                self.infer(expr);
            }

            // --- 制御構文 ---
            Stmt::If {
                branches,
                else_body,
            } => {
                for (cond, body) in branches {
                    let guard_opt: Option<(String, String, bool, Span)> = if let Expr::IsType {
                        expr,
                        type_name,
                        negated,
                        span,
                    } = cond
                    {
                        if let Expr::Ident(var_name) = expr.as_ref() {
                            Some((var_name.clone(), type_name.clone(), *negated, span.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    // Result 型ガード: `x.is_OK()` または `x.is_ERR()` を検出して型を絞り込む
                    let result_guard: Option<(String, InferredType, bool)> = {
                        if let Expr::Call { func, args, .. } = cond {
                            if args.is_empty() {
                                if let Expr::Attr { object, attr, .. } = func.as_ref() {
                                    if attr == "is_OK" || attr == "is_ERR" {
                                        if let Expr::Ident(var_name) = object.as_ref() {
                                            self.lookup(var_name).and_then(|info| {
                                                if let InferredType::Result(ok_ty, err_ty) = &info.ty {
                                                    let narrowed_ty = if attr == "is_OK" {
                                                        *ok_ty.clone()
                                                    } else {
                                                        *err_ty.clone()
                                                    };
                                                    Some((var_name.clone(), narrowed_ty, info.mutable))
                                                } else {
                                                    None
                                                }
                                            })
                                        } else { None }
                                    } else { None }
                                } else { None }
                            } else { None }
                        } else { None }
                    };

                    let (narrowed, error_info): (
                        Option<(String, InferredType, bool)>,
                        Option<(String, InferredType, Span)>,
                    ) = match &guard_opt {
                        None => (None, None),
                        Some((var_name, type_name, negated, span)) => {
                            let guard_ty = if self.known_protocols.contains_key(type_name.as_str()) {
                                InferredType::Protocol(type_name.clone())
                            } else {
                                Self::type_from_guard_name(type_name)
                            };
                            let (var_ty, is_mut) = self
                                .lookup(var_name)
                                .map(|v| (v.ty.clone(), v.mutable))
                                .unwrap_or((InferredType::Unresolved, false));

                            if *negated {
                                match &var_ty {
                                    InferredType::Union(types) => {
                                        let remaining: Vec<InferredType> = types
                                            .iter()
                                            .filter(|t| **t != guard_ty)
                                            .cloned()
                                            .collect();
                                        let narrowed_ty = match remaining.len() {
                                            0 => InferredType::Unresolved,
                                            1 => remaining.into_iter().next().unwrap(),
                                            _ => InferredType::Union(remaining),
                                        };
                                        (Some((var_name.clone(), narrowed_ty, is_mut)), None)
                                    }
                                    InferredType::Unresolved => (None, None),
                                    _ => (
                                        None,
                                        Some((var_name.clone(), var_ty.clone(), span.clone())),
                                    ),
                                }
                            } else {
                                // `is TypeName` guard: if var_ty is Intersection, validate guard type
                                if let InferredType::Intersection(isect_types) = &var_ty {
                                    let isect_cloned = isect_types.clone();
                                    self.check_intersection_guard_type(
                                        type_name,
                                        &isect_cloned,
                                        Some(span.clone()),
                                    );
                                }
                                (Some((var_name.clone(), guard_ty, is_mut)), None)
                            }
                        }
                    };

                    self.infer(cond);

                    if let Some((var_name, var_type, span)) = error_info {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::IsNotOnNonUnion { var_name, var_type },
                            span: Some(span),
                        });
                    }

                    self.push_scope();
                    // result_guard が優先、なければ通常の narrowed を使用
                    if let Some((var_name, narrowed_ty, is_mut)) = result_guard.or(narrowed) {
                        self.declare(var_name, narrowed_ty, is_mut);
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
            Stmt::Match { subject, arms, .. } => {
                let subject_ty = self.infer(subject);
                let subject_name: Option<String> = if let Expr::Ident(n) = subject {
                    Some(n.clone())
                } else {
                    None
                };
                for arm in arms {
                    self.push_scope();
                    match &arm.pattern {
                        MatchPattern::Case(expr) => {
                            self.infer(expr);
                        }
                        MatchPattern::IsType(type_name) => {
                            if let Some(ref var_name) = subject_name {
                                let narrowed = Self::type_from_guard_name(type_name);
                                let is_mut =
                                    self.lookup(var_name).map(|v| v.mutable).unwrap_or(false);
                                self.declare(var_name.clone(), narrowed, is_mut);
                            }
                            let _ = subject_ty.clone();
                        }
                    }
                    self.check_stmts(&arm.body);
                    self.pop_scope();
                }
            }
            Stmt::While { cond, body } => {
                self.infer(cond);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::For {
                targets,
                iter,
                body,
            } => {
                self.infer(iter);
                self.push_scope();
                for t in targets {
                    self.declare(t.clone(), InferredType::Unresolved, true);
                }
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::Block(body) => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }

            // --- 関数定義 ---
            Stmt::FnDef {
                name,
                params,
                return_type,
                body,
                decorators,
                ..
            } => {
                for dec in decorators {
                    self.check_decorator(dec, true, name);
                }
                for param in params.iter() {
                    if param.name == "self" || param.variadic {
                        continue;
                    }
                    if param.type_ann.is_none() {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::MissingParamTypeAnn {
                                func_name: name.clone(),
                                param_name: param.name.clone(),
                            },
                            span: None,
                        });
                    }
                }
                if return_type.is_none() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::MissingReturnTypeAnn {
                            func_name: name.clone(),
                        },
                        span: None,
                    });
                } else if let Some(rt) = return_type {
                    if self.known_protocols.contains_key(rt.as_str()) {
                        self.report_warning(StaticTypeWarning {
                            kind: TypeWarningKind::ProtocolReturnType {
                                func_name: name.clone(),
                                protocol_name: rt.clone(),
                            },
                            span: None,
                        });
                    }
                }
                // 交差型を含む関数は部分コンパイルできないため警告を出す
                let has_intersection_type = params.iter().any(|p| {
                    p.type_ann.as_deref()
                        .and_then(InferredType::from_ann)
                        .map_or(false, |ty| matches!(ty, InferredType::Intersection(_)))
                }) || return_type.as_deref()
                    .and_then(InferredType::from_ann)
                    .map_or(false, |ty| matches!(ty, InferredType::Intersection(_)));
                if has_intersection_type {
                    self.report_warning(StaticTypeWarning {
                        kind: TypeWarningKind::IntersectionSkippedCompile {
                            func_name: name.clone(),
                        },
                        span: None,
                    });
                }
                self.declare(name.clone(), InferredType::Unresolved, false);
                self.push_scope();
                for param in params {
                    if param.variadic {
                        // 可変長パラメータ: local::args として Optional[list[T]] を宣言
                        let elem_ty = param
                            .type_ann
                            .as_deref()
                            .and_then(InferredType::from_ann)
                            .unwrap_or(InferredType::Any);
                        let local_args_ty = InferredType::Union(vec![
                            InferredType::ListOf(Box::new(elem_ty)),
                            InferredType::None,
                        ]);
                        self.declare("local::args".to_string(), local_args_ty, param.mutable);
                        continue;
                    }
                    let ty = if param.name == "self" {
                        self.current_class_name
                            .as_ref()
                            .map(|c| InferredType::NamedInstance(c.clone()))
                            .unwrap_or(InferredType::Unresolved)
                    } else {
                        param
                            .type_ann
                            .as_deref()
                            .and_then(InferredType::from_ann)
                            .unwrap_or(InferredType::Unresolved)
                    };
                    self.declare(param.name.clone(), ty, param.mutable);
                }
                let prev_fn = self.current_fn_name.take();
                self.current_fn_name = Some(name.clone());
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.check_stmts(body);
                self.block_return_forbidden_depth = saved_depth;
                self.current_fn_name = prev_fn;
                self.pop_scope();
            }

            // --- クラス・trait 定義 ---
            Stmt::ClassDef {
                name,
                body,
                decorators,
                ..
            } => {
                for dec in decorators {
                    self.check_decorator(dec, false, name);
                }
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                let prev_class = self.current_class_name.replace(name.clone());
                self.check_stmts(body);
                self.current_class_name = prev_class;
                self.pop_scope();
            }
            Stmt::TraitDef { name, body, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::ProtocolDef { name, .. } => {
                // プロトコルは型値としてスコープに登録する（インスタンス化試行を検出するため）
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::Protocol(name.clone()))),
                    false,
                );
                // collect_fn_sigs で already registered in known_protocols
            }

            // --- ジャンプ文 ---
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.infer(e);
                }
            }
            Stmt::BlockReturn(expr, span) => {
                if self.block_return_forbidden_depth > 0 {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::BlockReturnInLoopExpr,
                        span: Some(span.clone()),
                    });
                }
                self.infer(expr);
            }
            Stmt::LoopYield(expr) | Stmt::Yield(expr) => {
                self.infer(expr);
            }

            // --- クラスフィールド宣言 ---
            Stmt::Field {
                name,
                kind,
                type_ann,
                default,
                ..
            } => {
                let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                if let Some(expr) = default {
                    if matches!(kind, FieldKind::Mut | FieldKind::Let) {
                        let kind_str = if matches!(kind, FieldKind::Mut) { "mut" } else { "let" };
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::FieldDefaultNotAllowed {
                                field_name: name.clone(),
                                kind: kind_str.to_string(),
                            },
                            span: None,
                        });
                    }
                    self.infer(expr);
                }
                let mutable = matches!(kind, FieldKind::Mut);
                self.declare(name.clone(), ty, mutable);
            }

            // --- ジェネレータ関数定義 ---
            Stmt::GenDef {
                name,
                params,
                yield_type,
                body,
                ..
            } => {
                for param in params.iter() {
                    if param.name == "self" || param.variadic {
                        continue;
                    }
                    if param.type_ann.is_none() {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::MissingParamTypeAnn {
                                func_name: name.clone(),
                                param_name: param.name.clone(),
                            },
                            span: None,
                        });
                    }
                }
                if yield_type.is_none() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::MissingReturnTypeAnn {
                            func_name: name.clone(),
                        },
                        span: None,
                    });
                }
                self.declare(name.clone(), InferredType::Unresolved, false);
                self.push_scope();
                for param in params {
                    let ty = param
                        .type_ann
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    self.declare(param.name.clone(), ty, param.mutable);
                }
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.check_stmts(body);
                self.block_return_forbidden_depth = saved_depth;
                self.pop_scope();
            }

            // --- new_type 定義 ---
            Stmt::NewTypeDef { name, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- enum 定義 ---
            Stmt::EnumDef { name, .. } => {
                let item_type_name = format!("enum_item_{}", name);
                self.declare(
                    item_type_name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(item_type_name))),
                    false,
                );
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- 副作用のない文 ---
            Stmt::Pass
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Freeze(..)
            | Stmt::BreakPoint { .. }
            | Stmt::DebugLet(..) => {}

            // --- 例外処理 ---
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                for handler in handlers {
                    self.push_scope();
                    if let Some(name) = &handler.name {
                        self.declare(name.clone(), InferredType::Unresolved, true);
                    }
                    self.check_stmts(&handler.body);
                    self.pop_scope();
                }
                if let Some(fb) = finally_body {
                    self.push_scope();
                    self.check_stmts(fb);
                    self.pop_scope();
                }
            }
            Stmt::Raise { exc, span } => {
                if let Some(e) = exc {
                    let ty = self.infer(e);
                    if !self.is_error_instance_type(&ty) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::InvalidRaiseType { got: ty },
                            span: Some(span.clone()),
                        });
                    }
                }
            }

            // --- import ---
            Stmt::Import {
                lang,
                module,
                alias,
                body,
                ..
            } => {
                let member_types = self.collect_module_types(body);
                let bind_name = alias
                    .clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                let ns_ty = if lang == "py" || lang == "py-int" {
                    InferredType::PyNamespace(member_types)
                } else {
                    InferredType::Namespace(member_types)
                };
                self.declare(bind_name, ns_ty, false);
            }

            Stmt::FromImport { lang, names, body, .. } => {
                let member_types = self.collect_module_types(body);
                let is_py = lang == "py" || lang == "py-int";
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let ty = member_types
                        .get(orig_name.as_str())
                        .cloned()
                        .unwrap_or(if is_py { InferredType::Any } else { InferredType::Unresolved });
                    self.declare(bind_name, ty, false);
                }
            }

            Stmt::AsyncAssign { stmts, .. } => {
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
            }

            Stmt::EventSubscribe { .. } | Stmt::EventUnsubscribe { .. } => {
                // イベント購読/解除文: 現時点では型チェックをスキップ
            }
        }
    }

    /// モジュールの tl AST を浅くスキャンして「名前 → 型」マップを返す。
    pub(super) fn collect_module_types(
        &self,
        body: &[Stmt],
    ) -> std::collections::HashMap<String, InferredType> {
        let mut map = std::collections::HashMap::new();
        for stmt in body {
            match stmt {
                Stmt::ClassDef { name, .. } => {
                    map.insert(
                        name.clone(),
                        InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                            name.clone(),
                        ))),
                    );
                }
                Stmt::FnDef { name, params, return_type, .. } => {
                    let ret = return_type
                        .as_deref()
                        .map(Self::type_ann_to_inferred)
                        .unwrap_or(InferredType::Unresolved);
                    let fn_params: Vec<FnTypeParam> = params
                        .iter()
                        .map(|p| FnTypeParam {
                            name: p.name.clone(),
                            mutable: p.mutable,
                            ty: p.type_ann
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Any),
                        })
                        .collect();
                    map.insert(
                        name.clone(),
                        InferredType::Function {
                            params: Some(fn_params),
                            return_type: Box::new(ret),
                        },
                    );
                }
                // Let/Const with a type annotation carry the type (used by Python stubs:
                // `let dumps: function->str` → Function { params: None, return_type: Str }).
                Stmt::Let(name, type_ann, _) | Stmt::Const(name, type_ann, _) => {
                    let ty = type_ann
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    map.insert(name.clone(), ty);
                }
                Stmt::Mut(name, _, _) | Stmt::Static(name, _, _) => {
                    map.insert(name.clone(), InferredType::Unresolved);
                }
                Stmt::LetTuple { targets, .. } => {
                    for t in targets {
                        match t {
                            TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                                map.insert(n.clone(), InferredType::Unresolved);
                            }
                            TupleTarget::Wildcard => {}
                        }
                    }
                }
                _ => {}
            }
        }
        map
    }

    /// プリミティブ型アノテーション文字列を対応する [`InferredType`] に変換する。未知の場合は `Unresolved`。
    pub(super) fn type_ann_to_inferred(s: &str) -> InferredType {
        match s {
            "int" => InferredType::Int,
            "float" => InferredType::Float,
            "str" => InferredType::Str,
            "bool" => InferredType::Bool,
            "None" => InferredType::None,
            "Any" => InferredType::Any,
            _ => InferredType::Unresolved,
        }
    }

    // ---------------------------------------------------------------------------
    // Protocol helpers
    // ---------------------------------------------------------------------------

    /// 型アノテーション付き変数宣言の型を解決する。
    /// アノテーションがプロトコル名の場合、RHS 型の適合チェックを行い Protocol 型を返す。
    pub(super) fn resolve_declared_type(
        &mut self,
        type_ann: Option<&str>,
        rhs_ty: InferredType,
        var_name: &str,
        _stmt: &Stmt,
    ) -> InferredType {
        let ann = match type_ann {
            None => return rhs_ty,
            Some(a) => a,
        };
        // アノテーションがプロトコル名かチェック
        if self.known_protocols.contains_key(ann) {
            let proto_name = ann.to_string();
            self.check_protocol_conformance(&rhs_ty, &proto_name, None, var_name);
            return InferredType::Protocol(proto_name);
        }
        // アノテーションが交差型の場合、メンバー互換性をチェック
        if let Some(InferredType::Intersection(types)) = InferredType::from_ann(ann) {
            let types_cloned = types.clone();
            self.check_intersection_members(&types_cloned, None);
            return InferredType::Intersection(types);
        }
        // アノテーションが Result 型の場合、Ok 型と Err 型が同じでないかチェック
        if let Some(InferredType::Result(ok_ty, err_ty)) = InferredType::from_ann(ann) {
            self.validate_result_type(&ok_ty, &err_ty, None);
            return InferredType::Result(ok_ty, err_ty);
        }
        rhs_ty
    }

    /// 型 `ty` がプロトコル `proto_name` を満たすか検査する。
    /// 満たさない場合は StaticTypeError を記録する。
    pub(super) fn check_protocol_conformance(
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
    fn collect_class_field_details(
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
    fn collect_class_method_sigs(
        &self,
        class_name: &str,
    ) -> std::collections::HashMap<String, Vec<super::types::FnSig>> {
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
    fn method_sig_matches_protocol(
        sig: &super::types::FnSig,
        req: &super::types::ProtocolMethod,
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
