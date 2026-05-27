#![allow(dead_code)]

use std::collections::HashMap;

use crate::ast::{Accessibility, Expr};
use crate::token::Span;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::{InferredType, VarInfo};
use super::TypeChecker;

impl TypeChecker {
    /// 新しいスコープをスタックに積む。
    pub(super) fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// 現在のスコープをスタックから取り除く。グローバルスコープは取り除かない。
    pub(super) fn pop_scope(&mut self) {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    /// 現在スコープに変数を宣言する。同名の変数があれば上書きする。
    pub(super) fn declare(&mut self, name: String, ty: InferredType, mutable: bool) {
        self.scope_stack
            .last_mut()
            .unwrap()
            .insert(name, VarInfo { ty, mutable });
    }

    /// スコープスタックを内側から外側へ走査して変数情報を返す。見つからない場合は `None`。
    pub(super) fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.scope_stack.iter().rev().find_map(|s| s.get(name))
    }

    /// 静的型エラーをエラーリストに追加する。
    pub(super) fn report_error(&mut self, err: StaticTypeError) {
        self.errors.push(err);
    }

    /// サブスクリプトチェーン `x[i][j]...` のルート識別子名を返す。
    pub(super) fn subscript_root_ident(expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Ident(name) => Some(name.as_str()),
            Expr::Subscript { object, .. } => Self::subscript_root_ident(object),
            _ => None,
        }
    }

    /// `class_name` のフィールド `member_name` へのアクセスが現在のコンテキストで許可されているか検査する。
    pub(super) fn check_member_access_static(
        &mut self,
        class_name: &str,
        member_name: &str,
        span: Option<Span>,
    ) {
        let is_field = self
            .class_fields
            .get(class_name)
            .map(|f| f.contains_key(member_name))
            .unwrap_or(false);
        if !is_field {
            return;
        }

        let access = self
            .class_member_access
            .get(class_name)
            .and_then(|m| m.get(member_name))
            .cloned()
            .unwrap_or(Accessibility::Public);
        match access {
            Accessibility::Public => {}
            Accessibility::Private => {
                if self.current_class_name.as_deref() == Some(class_name) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::PrivateAccessError {
                        member_name: member_name.to_string(),
                        class_name: class_name.to_string(),
                    },
                    span,
                });
            }
            Accessibility::Protected => {
                if let Some(cur) = self.current_class_name.clone() {
                    if cur == class_name {
                        return;
                    }
                    if self
                        .class_bases
                        .get(&cur)
                        .map(|b| b.contains(&class_name.to_string()))
                        .unwrap_or(false)
                    {
                        return;
                    }
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ProtectedAccessError {
                        member_name: member_name.to_string(),
                        class_name: class_name.to_string(),
                    },
                    span,
                });
            }
        }
    }

    /// `obj.attr = val` のとき `attr` が `let` フィールドであれば `AssignToImmutableField` エラーを記録する。
    pub(super) fn check_immutable_field_assign(&mut self, target: &Expr) {
        if let Expr::Attr { object, attr, span } = target {
            let is_self_in_init = matches!(object.as_ref(), Expr::Ident(n) if n == "self")
                && self.current_fn_name.as_deref() == Some("__init__");
            if is_self_in_init {
                return;
            }
            let class_name_opt: Option<String> = if matches!(object.as_ref(), Expr::Ident(n) if n == "self")
            {
                self.current_class_name.clone()
            } else {
                let obj_ty = self.infer(object);
                if let InferredType::NamedInstance(cls) = obj_ty {
                    Some(cls)
                } else {
                    None
                }
            };
            if let Some(class_name) = class_name_opt {
                if let Some(fields) = self.class_fields.get(&class_name) {
                    if fields.get(attr.as_str()) == Some(&false) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::AssignToImmutableField {
                                field_name: attr.clone(),
                                class_name,
                            },
                            span: Some(span.clone()),
                        });
                    }
                }
            }
        }
    }
}
