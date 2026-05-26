#![allow(dead_code)]

use crate::ast::{Expr, MatchPattern, UnaryOp};

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    pub(super) fn infer(&mut self, expr: &Expr) -> InferredType {
        match expr {
            // --- リテラル ---
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
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
                    InferredType::Union(_) => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::OperationOnUnion {
                            union_type: obj_ty.to_string(),
                            op: "attribute access".to_string(),
                        },
                        span: Some(span.clone()),
                    }),
                    _ => {}
                }
                if let Some(class_name) = class_name_opt {
                    self.check_member_access_static(&class_name, attr, Some(span.clone()));
                }
                // Namespace (imported module) — return the member's type directly.
                if let InferredType::Namespace(ref members) = obj_ty {
                    return members.get(attr.as_str()).cloned().unwrap_or(InferredType::Unresolved);
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
                self.infer(object);
                self.infer(index);
                InferredType::Unresolved
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
            Expr::Block { stmts, return_type } => {
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
                self.block_return_forbidden_depth = saved_depth;
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
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
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
                self.block_return_forbidden_depth = saved_depth;
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
                self.block_return_forbidden_depth += 1;
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.block_return_forbidden_depth -= 1;
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
                self.block_return_forbidden_depth += 1;
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                self.block_return_forbidden_depth -= 1;
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
                let saved_depth = self.block_return_forbidden_depth;
                self.block_return_forbidden_depth = 0;
                self.infer(subject);
                for arm in arms {
                    if let MatchPattern::Case(e) = &arm.pattern {
                        self.infer(e);
                    }
                    self.push_scope();
                    self.check_stmts(&arm.body);
                    self.pop_scope();
                }
                self.block_return_forbidden_depth = saved_depth;
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
