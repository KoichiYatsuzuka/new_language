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
        Self::path_root_ident(expr)
    }

    /// アクセスパスの**根になっている識別子**を返す（`a` / `a[0][1]` / `o.f.g` → `a` / `o`）。
    ///
    /// ⚠ 「その値が誰のものか」を決めるのに使う。規則 1（**要素は変数の属性を再帰的に
    /// 引き継ぐ**）を実装する土台で、`let a` の要素は `let`、`mut a` の要素は `mut`。
    /// ⚠ 根が識別子でない式（リテラル・呼び出しの戻り値）は `None`。
    /// **その場合は一時値なので、誰とも共有していない**＝可変性を問う意味が無い。
    pub(super) fn path_root_ident(expr: &Expr) -> Option<&str> {
        match expr {
            Expr::Ident { name, .. } => Some(name.as_str()),
            Expr::Subscript { object, .. } => Self::path_root_ident(object),
            Expr::Attr { object, .. } => Self::path_root_ident(object),
            _ => None,
        }
    }

    /// レシーバの**中身を書き換える**組み込みコレクションメソッド。
    ///
    /// ⚠⚠ **ここに足し忘れると「`let` なのに書き換えられる」穴が開く。**
    /// `src/interpreter/classes/` の `method_call.rs` / `set_methods.rs` /
    /// `frozen_list_methods.rs` に変更メソッドを足したら**必ずここにも足すこと**。
    pub(super) const MUTATING_COLLECTION_METHODS: &'static [&'static str] =
        &["append", "pop", "add", "clear", "discard", "remove"];

    /// アクセスパスの根の**可変性**。根が識別子でない（一時値）／未宣言なら `None`。
    pub(super) fn path_is_mutable(&self, expr: &Expr) -> Option<bool> {
        let name = Self::path_root_ident(expr)?;
        self.lookup(name).map(|info| info.mutable)
    }

    /// `let` の値に対する**変更メソッド**の呼び出しを弾く（bug_fix.md B8）。
    ///
    /// ⚠⚠ 添字代入（`a[0] = 9`）は元から弾いていたのに、**メソッド経由
    /// （`a.append(9)`）だけ素通り**していた。同じ「書き換え」なのに入口で扱いが割れていた。
    ///
    /// ⚠ **レシーバの型がコレクションのときだけ**検査する。ユーザー定義クラスは
    /// `mut self` メソッドの実行時ガード（`INST_IMMUTABLE`）が別にあるので、ここで
    /// メソッド名だけを見て弾くと `let` インスタンスの `remove()` のような**正しい
    /// 呼び出しまで落ちる**。
    ///
    /// ⚠ **判定はパスの根**（規則 1: 要素は根の属性を再帰的に引き継ぐ）。
    /// `let o = K([1]); o.xs.append(9)` も `let w = [[1]]; w[0].append(9)` も弾く。
    /// ⚠ 根が識別子でない式（リテラル・戻り値）は**一時値**なので通す。
    ///
    /// ⚠ 型が `Unresolved`（注釈が供給されない import モジュール本体など）のときは
    /// **検査しない**。誤検出を出さない代わりに、そこだけ穴が残る。
    pub(super) fn check_mutating_method_receiver(
        &mut self,
        object: &Expr,
        attr: &str,
        obj_ty: &InferredType,
        span: &Span,
    ) {
        if !Self::MUTATING_COLLECTION_METHODS.contains(&attr) {
            return;
        }
        let is_collection = matches!(
            obj_ty,
            InferredType::List
                | InferredType::ListOf(_)
                | InferredType::FixedList
                | InferredType::FixedListOf(_)
                | InferredType::ListLike
                | InferredType::ListLikeOf(_)
                | InferredType::Set
                | InferredType::SetOf(_)
                | InferredType::Dict
                | InferredType::DictOf(_, _)
        );
        if !is_collection {
            return;
        }
        if self.path_is_mutable(object) != Some(false) {
            return;
        }
        let root_name = Self::path_root_ident(object).unwrap_or("").to_string();
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::MutatingMethodOnImmutable {
                method: attr.to_string(),
                root_name,
            },
            span: Some(span.clone()),
        });
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
            let is_self_in_init = matches!(object.as_ref(), Expr::Ident { name: n, .. } if n == "self")
                && self.state.current_fn() == Some("__init__");
            if is_self_in_init {
                return;
            }
            let class_name_opt: Option<String> = if matches!(object.as_ref(), Expr::Ident { name: n, .. } if n == "self")
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
