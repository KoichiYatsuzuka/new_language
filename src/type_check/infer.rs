use crate::ast::{Expr, MatchPattern, UnaryOp};
use crate::token::Span;

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
            Expr::Attr { object, attr, span, node_id, .. } => {
                self.infer_attr(object, attr, span, *node_id)
            }
            Expr::TraitAccess { object, .. } => {
                self.infer(object);
                InferredType::Unresolved
            }

            // --- 関数呼び出し ---
            Expr::Call { func, args, node_id, .. } => {
                let result = self.infer_call(func, args, *node_id);
                // ── AST 型解決層（#16）── 呼び出しの**結果型**を焼く（CallInfo は infer_call_inner が充填）。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }

            // --- 識別子 ---
            // LocalRef はリゾルバ（型検査後に走る）が付ける解決済みローカル参照。
            // 型検査中に現れることはないが、網羅性のため Ident と同じく名前で引く。
            Expr::Ident(name) | Expr::LocalRef { name, .. } => self
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
            Expr::UnaryOp { op, operand } => self.infer_unaryop(op, operand),

            // --- 二項演算子 ---
            Expr::BinOp {
                op,
                left,
                right,
                span,
                node_id,
            } => {
                let lt = self.infer(left);
                let rt = self.infer(right);
                self.check_binop(op, &lt, &rt, span.clone());
                let result = Self::infer_binop_result(op, &lt, &rt);
                // ── AST 型解決層（#16）── 二項演算の**結果型**を焼く（plan A: 特化 op 判定用）。
                // オペランド型は各オペランド node の解決型として別途参照できる。検査指示は無し
                // （動的なら実行時 apply_bin_fast が緩衝。overload 呼び出しは Call 側で扱う）。
                self.annotations.set_resolved(*node_id, result.clone());
                result
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
            Expr::Subscript { object, index, node_id } => {
                let obj_ty = self.infer(object);
                let idx_ty = self.infer(index);
                let result = match obj_ty {
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
                };
                // ── AST 型解決層（#16）── 添字アクセスの**要素結果型**を焼く。
                self.annotations.set_resolved(*node_id, result.clone());
                result
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
            Expr::IsType { expr, node_id, .. } => {
                self.infer(expr);
                // `is` は Bool を返す（検査自体なので指示は不要・narrowing は直後 if 分岐で反映）。
                self.annotations.set_resolved(*node_id, InferredType::Bool);
                InferredType::Bool
            }

            // --- mustbe 動的型アサーション ---
            Expr::MustBe { expr, guard_type, span, node_id } => {
                self.infer_mustbe(expr, guard_type, span, *node_id)
            }
            Expr::Block { stmts, return_type } => {
                self.with_barrier(|c| {
                    c.push_scope();
                    c.check_stmts(stmts);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::IfExpr {
                branches,
                else_body,
                return_type,
            } => {
                self.with_barrier(|c| {
                    for (cond, body) in branches {
                        c.infer(cond);
                        c.push_scope();
                        c.check_stmts(body);
                        c.pop_scope();
                    }
                    if let Some(body) = else_body {
                        c.push_scope();
                        c.check_stmts(body);
                        c.pop_scope();
                    }
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::ForExpr {
                iter,
                body,
                return_type,
                ..
            } => {
                self.infer(iter);
                self.with_loop_expr(|c| {
                    c.push_scope();
                    c.check_stmts(body);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::WhileExpr {
                cond,
                body,
                return_type,
            } => {
                self.infer(cond);
                self.with_loop_expr(|c| {
                    c.push_scope();
                    c.check_stmts(body);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::MatchExpr {
                subject,
                arms,
                return_type,
            } => {
                self.with_barrier(|c| {
                    c.infer(subject);
                    for arm in arms {
                        if let MatchPattern::Case(e) = &arm.pattern {
                            c.infer(e);
                        }
                        c.push_scope();
                        c.check_stmts(&arm.body);
                        c.pop_scope();
                    }
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::Cast { type_name, node_id, .. } => {
                // 挙動不変: object は従来通り infer しない（この arm は type_name のみ使う）。
                let resolved =
                    InferredType::from_ann(type_name).unwrap_or(InferredType::Unresolved);
                // ── AST 型解決層（#16）── cast は動的ディスパッチ（__cast__/変換）を伴うので
                // 解決型＝ターゲット型、検査指示＝CheckBefore(ターゲット型)。
                let tid = self.annotations.intern(resolved.clone());
                self.annotations.set_resolved(*node_id, resolved.clone());
                self.annotations
                    .set_directive(*node_id, super::annotations::Directive::CheckBefore(tid));
                resolved
            }
            Expr::DebugVar(_) => InferredType::Unresolved,
        }
    }

    /// `->Type` 注釈があれば解決した型を、なければ `Unresolved` を返す。
    /// `block`/`if`/`for`/`while`/`match` 式の結果型計算で共通に使う。
    fn ann_or_unresolved(return_type: &Option<String>) -> InferredType {
        return_type
            .as_deref()
            .and_then(InferredType::from_ann)
            .unwrap_or(InferredType::Unresolved)
    }

    /// 属性アクセス `obj.attr` の型を推論する。`Any`/`Union`/`Result` への
    /// アクセスは診断し、名前空間メンバーはその型を直接返す。
    fn infer_attr(
        &mut self,
        object: &Expr,
        attr: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
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
        if let Some(class_name) = &class_name_opt {
            self.check_member_access_static(class_name, attr, Some(span.clone()));
        }
        // 戻り値（従来通り）: Namespace/PyNamespace はメンバ型、それ以外は Unresolved。
        let ret = if let InferredType::Namespace(ref members) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Unresolved)
        } else if let InferredType::PyNamespace(ref members) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Any)
        } else {
            InferredType::Unresolved
        };
        // ── AST 型解決層（#16）── 属性アクセスの型を焼く。**戻り値は不変**（下流の型検査に影響しない）。
        // NamedInstance のフィールドは checker が Unresolved を返すが、registry から実型を引いて注釈に載せる
        // （backend が具象フィールド型＋(class,field)→byte-offset を導出できる）。
        let annot_ty = match &class_name_opt {
            Some(class) => self
                .registry
                .class_field_details(class)
                .and_then(|m| m.get(attr))
                .map(|(_, ty)| ty.clone())
                .unwrap_or_else(|| ret.clone()),
            None => ret.clone(),
        };
        self.annotations.set_resolved(node_id, annot_ty);
        ret
    }

    /// 単項演算子の結果型を推論する。`Any`/`Union` オペランドは診断する。
    fn infer_unaryop(&mut self, op: &UnaryOp, operand: &Expr) -> InferredType {
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

    /// `expr mustbe Type` の型を推論する。解決した型を返し、コレクション要素型や
    /// 関数シグネチャは実行時に検査されないため警告する。
    fn infer_mustbe(
        &mut self,
        expr: &Expr,
        guard_type: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
        self.infer(expr);
        let resolved = InferredType::from_ann(guard_type).unwrap_or(InferredType::Unresolved);
        // ── AST 型解決層（#16・段階(a)）──
        // `mustbe` は実行時に対象型で動的検査する（不一致で raise）。よって:
        //   解決型テーブル = 確定後の型（guard_type）／ 検査指示 = CheckBefore(その型)。
        let tid = self.annotations.intern(resolved.clone());
        self.annotations.set_resolved(node_id, resolved.clone());
        self.annotations
            .set_directive(node_id, super::annotations::Directive::CheckBefore(tid));
        // コレクション型パラメータ・関数シグネチャは実行時に未チェック → 警告
        let warn_kind = match &resolved {
            InferredType::ListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list".to_string(),
            }),
            InferredType::FixedListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "fixed_list".to_string(),
            }),
            InferredType::ListLikeOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list_like".to_string(),
            }),
            InferredType::SetOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "set".to_string(),
            }),
            InferredType::DictOf(_, _) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "dict".to_string(),
            }),
            InferredType::Tuple(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "tuple".to_string(),
            }),
            InferredType::Function { params, return_type } => {
                let has_params = params.as_ref().is_some_and(|p| !p.is_empty());
                let has_ret = **return_type != InferredType::Any;
                if has_params || has_ret {
                    Some(TypeWarningKind::MustBeFunctionSignatureUnchecked {
                        guard_type: guard_type.to_string(),
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
}
