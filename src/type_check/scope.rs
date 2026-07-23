use crate::ast::{Accessibility, Expr};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind};
use super::types::{InferredType, VarInfo};
use super::TypeChecker;

impl TypeChecker {
    // ── CheckState への委譲 ───────────────────────────────────────────────────
    // 呼び出し側（infer.rs / check.rs / call_check.rs 等）が `self.declare(…)` の
    // ままで済むように薄いラッパを置く。実体は state.rs。

    /// 新しいスコープをスタックに積む。
    pub(super) fn push_scope(&mut self) {
        self.state.push_scope();
    }

    /// 現在のスコープをスタックから取り除く。グローバルスコープは取り除かない。
    pub(super) fn pop_scope(&mut self) {
        self.state.pop_scope();
    }

    /// 現在スコープに変数を宣言する。同名の変数があれば上書きする。
    pub(super) fn declare(&mut self, name: String, ty: InferredType, mutable: bool) {
        self.state.declare(name, ty, mutable);
    }

    /// スコープスタックを内側から外側へ走査して変数情報を返す。見つからない場合は `None`。
    pub(super) fn lookup(&self, name: &str) -> Option<&VarInfo> {
        self.state.lookup(name)
    }

    /// `block_return` 障壁の内側で `f` を実行する（`block`/`if`/`match` 式・関数本体用）。
    /// 進入時に深さを 0 にし、`f` の実行後に必ず元の深さへ戻す。enter/exit を1つの
    /// メソッドに閉じ込めることで復元漏れを構造的に防ぐ。
    ///
    /// Drop ガードではなくクロージャで包むのは、`f` が `self.check_stmts()` 等で
    /// `TypeChecker` 全体を可変借用するため、`CheckState` を借用し続ける RAII ガードだと
    /// 借用が衝突するため。
    pub(super) fn with_barrier<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.state.enter_barrier();
        let r = f(self);
        self.state.exit_barrier(saved);
        r
    }

    /// `for`/`while` 式の本体として `f` を実行する（進入時に深さ +1、終了時に -1）。
    pub(super) fn with_loop_expr<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.state.enter_loop_expr();
        let r = f(self);
        self.state.exit_loop_expr();
        r
    }

    /// 静的型エラーをエラーリストに追加する。
    pub(super) fn report_error(&mut self, err: StaticTypeError) {
        self.diags.report_error(err);
    }

    /// 静的型警告を警告リストに追加する。
    pub(super) fn report_warning(&mut self, w: StaticTypeWarning) {
        self.diags.report_warning(w);
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
        if !self.registry.has_field(class_name, member_name) {
            return;
        }

        let access = self.registry.member_access(class_name, member_name);
        match access {
            Accessibility::Public => {}
            Accessibility::Private => {
                if self.state.current_class() == Some(class_name) {
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
                if let Some(cur) = self.state.current_class().map(str::to_string) {
                    if cur == class_name {
                        return;
                    }
                    if self
                        .registry
                        .class_bases(&cur)
                        .is_some_and(|b| b.contains(&class_name.to_string()))
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
        if let Expr::Attr { object, attr, span, .. } = target {
            let is_self_in_init = matches!(object.as_ref(), Expr::Ident(n) if n == "self")
                && self.state.current_fn() == Some("__init__");
            if is_self_in_init {
                return;
            }
            let class_name_opt: Option<String> = if matches!(object.as_ref(), Expr::Ident(n) if n == "self")
            {
                self.state.current_class().map(str::to_string)
            } else {
                let obj_ty = self.infer(object);
                if let InferredType::NamedInstance(cls) = obj_ty {
                    Some(cls)
                } else {
                    None
                }
            };
            if let Some(class_name) = class_name_opt {
                if self.registry.field_is_mutable(&class_name, attr.as_str()) == Some(false) {
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
