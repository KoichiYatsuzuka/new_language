use crate::ast::{Expr, MatchPattern, UnaryOp};

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// 式の型を推論して [`InferredType`] を返す。副作用として型エラーを収集する場合がある。
    pub(super) fn infer(&mut self, expr: &Expr) -> InferredType {
        match expr {
            // --- リテラル ---
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::ImaginaryLit(_) => InferredType::Complex,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
            Expr::Undefined => InferredType::Undefined,
            Expr::List(elems) => {
                if elems.is_empty() {
                    InferredType::List
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    let first = &types[0];
                    if *first != InferredType::Unresolved && types.iter().all(|t| t == first) {
                        InferredType::ListOf(Box::new(first.clone()))
                    } else {
                        InferredType::List
                    }
                }
            }
            Expr::Set(elems) => {
                if elems.is_empty() {
                    InferredType::Set
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    let first = &types[0];
                    if *first != InferredType::Unresolved && types.iter().all(|t| t == first) {
                        InferredType::SetOf(Box::new(first.clone()))
                    } else {
                        InferredType::Set
                    }
                }
            }
            Expr::Tuple(exprs) => {
                let types: Vec<InferredType> = exprs.iter().map(|e| self.infer(e)).collect();
                InferredType::Tuple(types)
            }

            // --- 属性アクセス ---
            Expr::Attr { object, attr, span } => {
                let obj_ty = self.infer(object);
                let class_name_opt = if let InferredType::NamedInstance(cls) = &obj_ty {
                    Some(cls.clone())
                } else {
                    None
                };
                match &obj_ty {
                    InferredType::Any => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::OperationOnAny {
                            op: "attribute access".to_string(),
                        },
                        span: Some(span.clone()),
                    }),
                    InferredType::Union(_) | InferredType::Result(_, _) => {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnUnion {
                                union_type: obj_ty.to_string(),
                                op: "attribute access".to_string(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    // Intersection 型のメンバーアクセスはダウンキャストなしで許可する
                    InferredType::Intersection(_) => {}
                    _ => {}
                }
                if let Some(class_name) = class_name_opt {
                    self.check_member_access_static(&class_name, attr, Some(span.clone()));
                }
                // Namespace (imported module) — return the member's type directly.
                if let InferredType::Namespace(ref members) = obj_ty {
                    return members.get(attr.as_str()).cloned().unwrap_or(InferredType::Unresolved);
                }
                // PyNamespace: unknown members are dynamically typed → Any.
                if let InferredType::PyNamespace(ref members) = obj_ty {
                    return members.get(attr.as_str()).cloned().unwrap_or(InferredType::Any);
                }
                InferredType::Unresolved
            }
            Expr::TraitAccess { object, .. } => {
                self.infer(object);
                InferredType::Unresolved
            }

            // --- 関数呼び出し ---
            Expr::Call { func, args, .. } => self.infer_call(func, args),

            // --- 識別子 ---
            Expr::Ident(name) => self
                .lookup(name)
                .map(|v| v.ty.clone())
                .unwrap_or(InferredType::Unresolved),

            // --- local::name 変数 ---
            Expr::LocalVar(name) => {
                let key = format!("local::{}", name);
                self.lookup(&key)
                    .map(|v| v.ty.clone())
                    .unwrap_or(InferredType::Unresolved)
            }

            // --- 単項演算子 ---
            Expr::UnaryOp { op, operand } => {
                let ty = self.infer(operand);
                let op_str = match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => "not",
                    UnaryOp::BitNot => "~",
                };
                match &ty {
                    InferredType::Any => {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnAny {
                                op: op_str.to_string(),
                            },
                            span: None,
                        });
                        return InferredType::Unresolved;
                    }
                    InferredType::Union(_) => {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnUnion {
                                union_type: ty.to_string(),
                                op: op_str.to_string(),
                            },
                            span: None,
                        });
                        return InferredType::Unresolved;
                    }
                    _ => {}
                }
                match op {
                    UnaryOp::Not => InferredType::Bool,
                    UnaryOp::Neg => match ty {
                        InferredType::Int => InferredType::Int,
                        InferredType::Float => InferredType::Float,
                        InferredType::Complex => InferredType::Complex,
                        _ => InferredType::Unresolved,
                    },
                    UnaryOp::BitNot => InferredType::Int,
                }
            }

            // --- 二項演算子 ---
            Expr::BinOp {
                op,
                left,
                right,
                span,
            } => {
                let lt = self.infer(left);
                let rt = self.infer(right);
                self.check_binop(op, &lt, &rt, span.clone());
                Self::infer_binop_result(op, &lt, &rt)
            }

            // --- テンプレート実体化 ---
            Expr::TemplateInstantiate { base, .. } => {
                self.infer(base);
                InferredType::Unresolved
            }

            // --- 辞書・サブスクリプト ---
            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    InferredType::Dict
                } else {
                    let key_types: Vec<InferredType> =
                        pairs.iter().map(|(k, _)| self.infer(k)).collect();
                    let val_types: Vec<InferredType> =
                        pairs.iter().map(|(_, v)| self.infer(v)).collect();
                    let first_k = &key_types[0];
                    let first_v = &val_types[0];
                    if *first_k != InferredType::Unresolved
                        && *first_v != InferredType::Unresolved
                        && key_types.iter().all(|t| t == first_k)
                        && val_types.iter().all(|t| t == first_v)
                    {
                        InferredType::DictOf(Box::new(first_k.clone()), Box::new(first_v.clone()))
                    } else {
                        InferredType::Dict
                    }
                }
            }
            Expr::Subscript { object, index } => {
                let obj_ty = self.infer(object);
                let idx_ty = self.infer(index);
                match obj_ty {
                    InferredType::ListOf(elem) | InferredType::FixedListOf(elem) | InferredType::ListLikeOf(elem) => *elem,
                    InferredType::SetOf(elem) => *elem,
                    InferredType::DictOf(_, val) => *val,
                    InferredType::Tuple(types) => {
                        // リテラル整数インデックスなら対応する要素型を返す
                        if let Expr::Int(n) = index.as_ref() {
                            let i = *n as usize;
                            types.into_iter().nth(i).unwrap_or(InferredType::Unresolved)
                        } else {
                            InferredType::Unresolved
                        }
                    }
                    InferredType::Str => {
                        // 文字列の添字は文字列を返す
                        if matches!(idx_ty, InferredType::Int) { InferredType::Str } else { InferredType::Unresolved }
                    }
                    _ => InferredType::Unresolved,
                }
            }
            Expr::Slice { begin, end, step } => {
                if let Some(e) = begin {
                    self.infer(e);
                }
                if let Some(e) = end {
                    self.infer(e);
                }
                if let Some(e) = step {
                    self.infer(e);
                }
                InferredType::NamedInstance("slice".to_string())
            }

            // --- 型ガード式 ---
            Expr::IsType { expr, .. } => {
                self.infer(expr);
                InferredType::Bool
            }

            // --- mustbe 動的型アサーション ---
            Expr::MustBe { expr, guard_type, span } => {
                self.infer(expr);
                let resolved = InferredType::from_ann(guard_type).unwrap_or(InferredType::Unresolved);
                // コレクション型パラメータ・関数シグネチャは実行時に未チェック → 警告
                let warn_kind = match &resolved {
                    InferredType::ListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "list".to_string(),
                    }),
                    InferredType::FixedListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "fixed_list".to_string(),
                    }),
                    InferredType::ListLikeOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "list_like".to_string(),
                    }),
                    InferredType::SetOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "set".to_string(),
                    }),
                    InferredType::DictOf(_, _) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "dict".to_string(),
                    }),
                    InferredType::Tuple(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                        guard_type: guard_type.clone(),
                        outer_type: "tuple".to_string(),
                    }),
                    InferredType::Function { params, return_type } => {
                        let has_params = params.as_ref().is_some_and(|p| !p.is_empty());
                        let has_ret = **return_type != InferredType::Any;
                        if has_params || has_ret {
                            Some(TypeWarningKind::MustBeFunctionSignatureUnchecked {
                                guard_type: guard_type.clone(),
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
            Expr::Block { stmts, return_type } => {
                let saved_depth = self.state.enter_barrier();
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
                self.state.exit_barrier(saved_depth);
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::IfExpr {
                branches,
                else_body,
                return_type,
            } => {
                let saved_depth = self.state.enter_barrier();
                for (cond, body) in branches {
                    self.infer(cond);
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
                if let Some(body) = else_body {
                    self.push_scope();
                    self.check_stmts(body);
                    self.pop_scope();
                }
                self.state.exit_barrier(saved_depth);
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::ForExpr {
                iter,
                body,
                return_type,
                ..
            } => {
                self.infer(iter);
                self.state.enter_loop_expr();
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.state.exit_loop_expr();
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::WhileExpr {
                cond,
                body,
                return_type,
            } => {
                self.infer(cond);
                self.state.enter_loop_expr();
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.state.exit_loop_expr();
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::MatchExpr {
                subject,
                arms,
                return_type,
            } => {
                let saved_depth = self.state.enter_barrier();
                self.infer(subject);
                for arm in arms {
                    if let MatchPattern::Case(e) = &arm.pattern {
                        self.infer(e);
                    }
                    self.push_scope();
                    self.check_stmts(&arm.body);
                    self.pop_scope();
                }
                self.state.exit_barrier(saved_depth);
                if let Some(t) = return_type {
                    InferredType::from_ann(t).unwrap_or(InferredType::Unresolved)
                } else {
                    InferredType::Unresolved
                }
            }
            Expr::Cast { type_name, .. } => {
                InferredType::from_ann(type_name).unwrap_or(InferredType::Unresolved)
            }
            Expr::DebugVar(_) => InferredType::Unresolved,
        }
    }
}
