#![allow(dead_code)]

use crate::ast::{Expr, FieldKind, MatchPattern, Stmt, TupleTarget};
use crate::token::Span;

use super::errors::{StaticTypeError, TypeErrorKind};
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
            Stmt::Let(name, expr) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Const(name, expr) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, false);
            }
            Stmt::Mut(name, expr) => {
                let ty = self.infer(expr);
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
                            self.declare(name.clone(), ty, false)
                        }
                        TupleTarget::Mut(name) => self.declare(name.clone(), ty, true),
                        TupleTarget::Wildcard => {}
                    }
                }
            }

            // --- 代入 ---
            Stmt::Assign { name, value, span } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                self.infer(value);
            }
            Stmt::CompoundAssign {
                name,
                op: _,
                value,
                span,
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

                    let (narrowed, error_info): (
                        Option<(String, InferredType, bool)>,
                        Option<(String, InferredType, Span)>,
                    ) = match &guard_opt {
                        None => (None, None),
                        Some((var_name, type_name, negated, span)) => {
                            let guard_ty = Self::type_from_guard_name(type_name);
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
                    if let Some((var_name, narrowed_ty, is_mut)) = narrowed {
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
                    if param.name == "self" {
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
                }
                self.declare(name.clone(), InferredType::Unresolved, false);
                self.push_scope();
                for param in params {
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
                    if param.name == "self" {
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
                module,
                alias,
                body,
                ..
            } => {
                let member_types = self.collect_module_types(body);
                let bind_name = alias
                    .clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                self.declare(bind_name, InferredType::Namespace(member_types), false);
            }

            Stmt::FromImport { names, body, .. } => {
                let member_types = self.collect_module_types(body);
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let ty = member_types
                        .get(orig_name.as_str())
                        .cloned()
                        .unwrap_or(InferredType::Unresolved);
                    self.declare(bind_name, ty, false);
                }
            }

            Stmt::AsyncAssign { stmts, .. } => {
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
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
                Stmt::Mut(name, _)
                | Stmt::Let(name, _)
                | Stmt::Const(name, _)
                | Stmt::Static(name, _, _) => {
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
            _ => InferredType::Unresolved,
        }
    }
}
