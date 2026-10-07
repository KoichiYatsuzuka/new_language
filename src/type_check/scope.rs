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

    /// 変数を束縛したことを記録する（[`super::BindingRecord`]・VS Code 拡張の hover / inlay 用）。
    /// 記録しない検査（CLI）では何もしない。`declare` の**直前**に、束縛する型で呼ぶ。
    ///
    /// ⚠ 単相化した具体化（`Box[int]` / `f[str]`）の本体では記録しない。具体化はテンプレートの
    ///   本体と同じ位置を型引数ごとに検査するので、記録すると 1 つの宣言に型が何通りも付く。
    ///   ソースに書かれているのはテンプレートの本体なので、その型（型変数のまま）を残す。
    pub(super) fn note_binding(&mut self, name: &str, ty: &InferredType) {
        if self.bindings.is_none() || self.in_instance_decl() {
            return;
        }
        let pos = self.bind_pos.clone();
        if let Some(records) = self.bindings.as_mut() {
            records.push(super::BindingRecord { pos, name: name.to_string(), ty: ty.clone() });
        }
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

    /// **`block:` / `if:` / `match:` 式の本体**として `f` を実行する（タスク 5.2）。
    /// `expected` はその式の `->T` 注釈で、内側の `block_return` の照合先になる。
    ///
    /// ⚠ `with_barrier` と必ず**組にして**使うこと（`block_return` の可否と照合先は
    /// 同じ 1 つの構文が決めるので、片方だけ積むと食い違う）。
    pub(super) fn with_block_expr<R>(
        &mut self,
        expected: Option<InferredType>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.state.push_block_expr_expected(expected);
        let r = self.with_barrier(f);
        self.state.pop_block_expr_expected();
        r
    }

    /// **`for`/`while` 式の本体**として `f` を実行する（タスク 5.2）。
    /// `expected` はその式の `->list[T]` 注釈**そのもの**（要素を取り出すのは
    /// `loop_yield` 側）。`block_return` の可否は `with_loop_expr` が見る。
    pub(super) fn with_loop_expr_yielding<R>(
        &mut self,
        expected: Option<InferredType>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.state.push_block_expr_expected(expected);
        let r = self.with_loop_expr(f);
        self.state.pop_block_expr_expected();
        r
    }

    /// **関数・`gen` の本体**として `f` を実行する（タスク 5.2）。
    ///
    /// ⚠⚠ **`None` を積む**（継承しない）。入れ子 `fn` の `block_return` /
    /// `loop_yield` が外側のブロック式の注釈と照合されると嘘の判定になる
    /// （`current_fn_return` / `in_gen_body` を張り替えているのと同じ理由）。
    pub(super) fn with_fn_body<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.state.push_block_expr_expected(None);
        let r = self.with_barrier(f);
        self.state.pop_block_expr_expected();
        r
    }

    /// 静的型エラーをエラーリストに追加する。
    ///
    /// ⚠ 位置を持たない誤りには**今検査している文の位置**を付ける（フェーズ10 10-17・`check_stmt`）。
    ///   以前は表の `File` が `<unknown>` になっていた（`let z: str = s` は右辺の識別子が位置を持たない）。
    pub(super) fn report_error(&mut self, mut err: StaticTypeError) {
        if err.span.is_none() {
            err.span = self.stmt_pos.clone();
        }
        let from_instance = self.in_instance_decl();
        self.diags.report_error(err, from_instance);
    }

    /// 静的型警告を警告リストに追加する。
    ///
    /// ⚠⚠ **単相化した具体化の本体では警告を出さない**（タスク 2-8 段階 2）。型変数に関係しない
    /// 警告はテンプレートの本体の検査で必ず出る。具体化でだけ出る警告は型引数が決めたもので、
    /// 作者が書いたものではない（`Box[Drawable]` の `get() -> T` が「protocol を返している」
    /// と警告された・実測）。
    pub(super) fn report_warning(&mut self, mut w: StaticTypeWarning) {
        if self.in_instance_decl() {
            return;
        }
        // 位置を持たない警告には今の文の位置を付ける（`report_error` と同じ・10-17）。
        if w.span.is_none() {
            w.span = self.stmt_pos.clone();
        }
        self.diags.report_warning(w);
    }

    /// 今、単相化した具体化（`Box[int]` / `ident[str]`）の本体を検査しているか（タスク 2-8 段階 2）。
    fn in_instance_decl(&self) -> bool {
        self.state.current_class().is_some_and(|c| c.contains('['))
            || self.state.current_fn().is_some_and(|f| f.contains('['))
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

    /// アクセスパスの**可変性**。根が識別子でない（一時値）／未宣言なら `None`。
    ///
    /// ⚠⚠ **経路の途中に `const` があれば不変**（[`Self::const_on_path`]）。`mut c` の
    ///   `c.L`（`L` はクラスの `const`）も、`m.L`（モジュールの `const`）も書き換えられない。
    ///   以前は根の束縛だけを見ていたので `c.L.append(2)` / `f(c.L)`（`mut` の仮引数）が通っていた。
    /// ⚠⚠ **根が型の値・モジュールなら「値の束縛」ではない**（`check_attr_assign` と同じ区別）。
    ///   `Counter.items`（`static mut`）は書ける。以前はクラス名の束縛が不変なので
    ///   `Counter.items.append(1)` が誤って弾かれていた。
    pub(super) fn path_is_mutable(&mut self, expr: &Expr) -> Option<bool> {
        if self.const_on_path(expr).is_some() {
            return Some(false);
        }
        let name = Self::path_root_ident(expr)?;
        let info = self.lookup(name)?;
        if matches!(
            info.ty,
            InferredType::TypeValOf(_) | InferredType::Namespace(..) | InferredType::PyNamespace(_)
        ) {
            return None;
        }
        Some(info.mutable)
    }

    /// 型だけが欲しい推論（**診断を出さない**）。
    ///
    /// ⚠ 書き込み先の経路（`C.L` の `C`）を調べるために、本来の検査とは別にもう一度推論する。
    ///   診断をそのまま残すと、同じ誤り（未定義の名前など）が二重に出る。
    pub(super) fn infer_quietly(&mut self, expr: &Expr) -> InferredType {
        let mark = self.diags.mark();
        let ty = self.infer(expr);
        self.diags.rollback(mark);
        ty
    }

    /// `object.attr` の `attr` の**持ち主**（`const` を持ちうるもの）と、それが型の値経由か。
    ///
    /// - `self` → 今のクラス
    /// - 型の値（`Counter` / `Color` / `Box[int]`）→ そのクラス（型の値経由）
    /// - インスタンス（`c` / `Box[int]` の値 / trait 型の値）→ そのクラス・trait
    /// - Arrow のモジュール（`m` / 別名 `t`）→ そのモジュール（エディタではメンバーが確定しないので見ない）
    pub(super) fn member_owner(&mut self, object: &Expr) -> Option<(MemberOwner, bool)> {
        if matches!(object, Expr::Ident { name, .. } if name == "self") {
            return self.state.current_class().map(|c| (MemberOwner::Class(c.to_string()), false));
        }
        match self.infer_quietly(object) {
            InferredType::TypeValOf(inner) => match *inner {
                InferredType::NamedInstance(c) => Some((MemberOwner::Class(c), true)),
                ref generic @ InferredType::GenericInstance { .. } => self
                    .registry
                    .instance_class(&generic.to_string())
                    .map(|c| (MemberOwner::Class(c.clone()), true)),
                _ => None,
            },
            InferredType::Namespace(_, Some(closed)) => Some((MemberOwner::Module(closed.name), false)),
            ty => self.class_and_subst(&ty).map(|(c, _)| (MemberOwner::Class(c), false)),
        }
    }

    /// `owner.member` が `const` か（クラス・trait・enum は [`Self::is_const_member`]、モジュールは `module_consts`）。
    pub(super) fn owner_has_const(&self, owner: &MemberOwner, member: &str) -> bool {
        match owner {
            MemberOwner::Class(c) => self.is_const_member(c, member),
            MemberOwner::Module(m) => self.module_consts.get(m).is_some_and(|s| s.contains(member)),
        }
    }

    /// 書き込み先の**経路の途中にある `const`**（持ち主の名前・メンバー名）。無ければ `None`。
    ///
    /// `C.L[0]` / `c.O.x` の `c.O` / `self.L` / `m.L` のように、経路のどこかが `const` なメンバーなら、
    /// その先の要素・フィールドも書き換えられない（規則 1「要素・フィールドは根の属性を継ぐ」を
    /// 経路の途中の `const` に広げたもの）。根の識別子そのもの（`const X`）は束縛の不変性が見る。
    pub(super) fn const_on_path(&mut self, path: &Expr) -> Option<(String, String)> {
        match path {
            Expr::Attr { object, attr, .. } => {
                if let Some((owner, _)) = self.member_owner(object) {
                    if self.owner_has_const(&owner, attr) {
                        return Some((owner.name().to_string(), attr.clone()));
                    }
                }
                self.const_on_path(object)
            }
            Expr::Subscript { object, .. } => self.const_on_path(object),
            _ => None,
        }
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
        // ⚠ `const` の中身（`C.L.append(..)` / `self.L.append(..)` / `m.L.append(..)`）は、
        //   根の束縛が `mut` でも書き換えられない。
        if let Some((owner, member)) = self.const_on_path(object) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ModifyConst { owner, member },
                span: Some(span.clone()),
            });
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

    /// 変更できない値に対する **`mut self` メソッド**の呼び出しを弾く（フェーズ10 10-15）。
    ///
    /// [`Self::check_mutating_method_receiver`]（組み込みコレクションの `append` など）のクラス版。
    /// 判定はメソッドの**名前ではなく `self` の宣言**で行う（`mut self` のメソッドだけが書き換える）。
    /// 以前は実行時のガード（`INST_IMMUTABLE`）任せで、次の形が静的に止まらなかった:
    ///   - `let b = Box(1)` / `freeze b` の後の `b.set(2)` → 実行時の `TypeError`
    ///   - `let` 仮引数・`mut` でない `self` の先のフィールド（`self.b.set(5)`）→ **写しを黙って書き換えて
    ///     何も起きない**（実行時にも止まらない・実測）
    ///
    /// ⚠ 多重定義に `mut self` でないものが 1 つでもあれば通す。実行時はそれを選ぶ
    ///   （`call_instance_method_evaled` の不変性フィルタ）。
    /// ⚠ `gen` メソッドは通す。実行時の `gen` メソッドの呼び出しは不変性フィルタより前に振り分けられ、
    ///   `mut self` でも止まらない。シグネチャの戻り値がジェネレータのものは `gen` とみなして見送る
    ///   （`-> generator` の `fn` も見送ることになるが、誤検出はしない側に倒す）。
    /// ⚠ 判定はパスの根（`path_is_mutable`）。根が識別子でない一時値は通す。
    pub(super) fn check_mut_self_method_receiver(
        &mut self,
        object: &Expr,
        class_name: &str,
        method: &str,
        span: &Span,
    ) {
        let Some(sigs) = self.registry.class_methods(class_name).and_then(|m| m.get(method)) else {
            return;
        };
        let all_mut_self = !sigs.is_empty()
            && sigs.iter().all(|sig| {
                let takes_mut_self = sig.params.first().is_some_and(|(n, _)| n == "self")
                    && sig.param_mutable.first() == Some(&true);
                let is_gen = matches!(
                    &sig.return_type,
                    Some(InferredType::IteratorOf(_))
                ) || matches!(&sig.return_type, Some(InferredType::NamedInstance(n)) if n == "generator");
                takes_mut_self && !is_gen
            });
        if !all_mut_self {
            return;
        }
        if self.path_is_mutable(object) != Some(false) {
            return;
        }
        let root_name = Self::path_root_ident(object).unwrap_or("").to_string();
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::MutatingMethodOnImmutable {
                method: method.to_string(),
                root_name,
            },
            span: Some(span.clone()),
        });
    }

    /// `class_name` のメンバー（フィールド・メソッド）`member_name` へのアクセスが現在のコンテキストで
    /// 許可されているか検査する。
    ///
    /// ⚠⚠ **メソッドも見る**（フェーズ10 10-6）。以前は先頭で「フィールドでなければ見ない」と
    ///   返していたので、`private:` の下のメソッドを**クラスの外から呼べた**（静的にも実行時にも
    ///   止まらなかった・実測）。アクセス指定はフィールドもメソッドも同じ表（`member_access`）に
    ///   入っていて、載っていないメンバーは public なので、表を引くだけでよい。
    pub(super) fn check_member_access_static(
        &mut self,
        class_name: &str,
        member_name: &str,
        span: Option<Span>,
    ) {
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

    /// 属性・添字への代入（複合代入も）の書き込み先を検査する。
    ///
    /// 1. `obj.attr = ..` で `attr` が **`const` なメンバー**（[`Self::owner_has_const`]）なら `AssignToConst`。
    ///    クラス名経由（`Counter.LIMIT`）・インスタンス経由（`c.LIMIT`）・`__init__` の中の
    ///    `self.LIMIT`・trait の `const`・enum のメンバー・モジュールの `const`（`m.K`）のどれでも同じ。
    /// 2. 書き込む先の**入れ物の経路に `const` がある**（`C.L[0] = ..` / `C.O.x = ..` / `m.L[0] = ..`）なら
    ///    `ModifyConst`（[`Self::const_on_path`]）。
    /// 3. `obj.attr = ..` で `attr` が `let` フィールドなら `AssignToImmutableField`（`__init__` の中の `self` は除く）。
    ///
    /// ⚠⚠ **1・2 は `__init__` の免除より先に見る**。`__init__` で許すのは `let` フィールドの初回代入で、
    ///    `const` は `__init__` の中でも書き換えられない（実行時も `TypeError: cannot assign to class
    ///    variable`）。以前は `__init__` の中を丸ごと免除していたので `self.LIMIT = 7` が素通りし、
    ///    クラス名経由の `Counter.LIMIT = 5` も（`NamedInstance` しか見ていなかったので）素通りしていた。
    pub(super) fn check_immutable_field_assign(&mut self, target: &Expr) {
        // ⚠ 添字（`Expr::Subscript`）は位置を持たないので、文の位置で知らせる（`report_error` が補う）。
        let (object, attr, span): (&Expr, Option<&str>, Option<Span>) = match target {
            Expr::Attr { object, attr, span, .. } => (object.as_ref(), Some(attr.as_str()), Some(span.clone())),
            Expr::Subscript { object, .. } => (object.as_ref(), None, None),
            _ => return,
        };
        let owner = match attr {
            Some(_) => self.member_owner(object),
            None => None,
        };
        if let (Some((o, _)), Some(attr)) = (&owner, attr) {
            if self.owner_has_const(o, attr) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::AssignToConst { owner: o.name().to_string(), member: attr.to_string() },
                    span,
                });
                return;
            }
        }
        if let Some((owner, member)) = self.const_on_path(object) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ModifyConst { owner, member },
                span,
            });
            return;
        }
        let (Some((MemberOwner::Class(class_name), via_type_value)), Some(attr)) = (owner, attr) else {
            return;
        };
        // ⚠ 型の値経由で書けるのは `static mut` だけ。`let` / `mut` フィールドを型の値経由で
        //   書く形（`Counter.own = 1`）はここでは判定しない（以前と同じ）。
        let is_self = matches!(object, Expr::Ident { name: n, .. } if n == "self");
        if via_type_value || (is_self && self.state.current_fn() == Some("__init__")) {
            return;
        }
        if self.registry.field_is_mutable(&class_name, attr) == Some(false) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::AssignToImmutableField {
                    field_name: attr.to_string(),
                    class_name,
                },
                span,
            });
        }
    }

    /// `owner.member` が **`const` なメンバー**か（代入できないメンバー）。
    ///
    /// - クラスの `const`（`FieldKind::Const`・基底クラスの分も含む）
    /// - trait の `const`（クラスが実装する trait・trait 型の値）
    /// - enum のメンバー（`registry.enum_members`）。メンバーは値で、**暗黙に `const`**
    ///   （`implementation_plans/enum_member_type_plan.md` 5-1〜5-3）
    ///
    /// ⚠ enum のメンバーの `value` は `const` ではなく、メンバーごとの不変のフィールド（`let`）。
    pub(super) fn is_const_member(&self, owner: &str, member: &str) -> bool {
        if self.registry.enum_members(owner).is_some_and(|m| m.contains_key(member)) {
            return true;
        }
        if self
            .collect_class_field_details(owner)
            .get(member)
            .is_some_and(|(kind, _)| matches!(kind, crate::ast::FieldKind::Const))
        {
            return true;
        }
        self.trait_has_const(owner, member, 0)
    }

    /// `name`（trait・クラス）自身か、その基底の trait が `member` を `const` として宣言しているか。
    /// ⚠ trait のフィールドはクラスの表（`class_field_details`）ではなく `trait_field_details` にある。
    fn trait_has_const(&self, name: &str, member: &str, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        if self
            .registry
            .trait_field_details(name)
            .and_then(|f| f.get(member))
            .is_some_and(|(kind, _)| matches!(kind, crate::ast::FieldKind::Const))
        {
            return true;
        }
        self.registry
            .class_bases(name)
            .unwrap_or(&[])
            .iter()
            .any(|b| self.trait_has_const(b, member, depth + 1))
    }
}

/// `const` を持ちうるメンバーの持ち主（[`TypeChecker::member_owner`]）。
pub(super) enum MemberOwner {
    /// クラス・trait・enum（名前）。
    Class(String),
    /// Arrow のモジュール（名前 `a.b`）。
    Module(String),
}

impl MemberOwner {
    /// 誤りに出す持ち主の名前。
    pub(super) fn name(&self) -> &str {
        match self {
            MemberOwner::Class(n) | MemberOwner::Module(n) => n,
        }
    }
}
