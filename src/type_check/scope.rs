use crate::ast::{Accessibility, Expr};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind};
use super::types::{InferredType, ModuleVars, VarAttr, VarInfo};
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

    /// 型ガードで絞り込んだ変数を今のスコープに宣言し直す。**付け替え・中身の書き換えの可否は元の
    /// 束縛のまま**（`from m import X` の `X` は付け替えられないが中身は書き換えられることがある）。
    pub(super) fn declare_narrowed(&mut self, name: String, ty: InferredType) {
        let (mutable, contents_mutable) =
            self.lookup(&name).map(|i| (i.mutable, i.contents_mutable)).unwrap_or((false, false));
        self.state.declare_with_contents(name, ty, mutable, contents_mutable);
    }

    /// `from m import X` で取り込んだ名前を宣言する。付け替えはできず、中身は元の宣言の属性に従う
    /// （[`VarAttr::Imported`]）。
    pub(super) fn declare_imported(&mut self, name: String, ty: InferredType, contents_mutable: bool) {
        self.state.declare_with_contents(name, ty, false, contents_mutable);
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

    /// アクセスパスの**可変性**（中身を書き換えられるか）。根が識別子でない（一時値）／未宣言なら `None`。
    ///
    /// ⚠⚠ **経路の途中に書き換えられないものがあれば不変**（[`Self::readonly_on_path`]）。`mut c` の
    ///   `c.L`（`L` はクラスの `const`）も、`g.L`（モジュールの `const` / `let`）も書き換えられない。
    ///   以前は根の束縛だけを見ていたので `c.L.append(2)` / `f(c.L)`（`mut` の仮引数）が通っていた。
    /// ⚠⚠ **根が型の値・モジュールなら「値の束縛」ではない**（`check_attr_assign` と同じ区別）。
    ///   `Counter.items`（`static mut`）・`g.ML`（モジュールの `mut`）は書ける。
    /// ⚠ 根の変数は**中身を書き換えられるか**（`VarInfo::contents_mutable`）で見る。
    pub(super) fn path_is_mutable(&mut self, expr: &Expr) -> Option<bool> {
        if self.readonly_on_path(expr).is_some() {
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
        Some(info.contents_mutable)
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

    /// `object.attr` の `attr` の**持ち主**と、それが型の値経由か。
    ///
    /// - `self` → 今のクラス
    /// - 型の値（`Counter` / `Color` / `Box[int]`）→ そのクラス（型の値経由）
    /// - インスタンス（`c` / `Box[int]` の値 / trait 型の値）→ そのクラス・trait
    /// - モジュール（`g` / 別名 `t`）→ そのモジュール（メンバーとグローバル変数の属性）
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
            InferredType::Namespace(members, _, vars) => Some((
                MemberOwner::Module { members: members.into_keys().collect(), vars: *vars },
                false,
            )),
            ty => self.class_and_subst(&ty).map(|(c, _)| (MemberOwner::Class(c), false)),
        }
    }

    /// 書き込み先の**経路の途中にある、書き換えられないもの**。無ければ `None`。
    ///
    /// - クラス・trait の `const`・enum のメンバー（[`Self::is_const_member`]）→ [`ReadOnly::Const`]
    /// - モジュールのグローバル変数（`g.X`）→ **属性で決める**（[`VarAttr`]）: `const` は
    ///   [`ReadOnly::Const`]、`let` と中身を書き換えられない取り込んだ名前は [`ReadOnly::Immutable`]、
    ///   `mut` は書き換えられる
    ///
    /// `C.L[0]` / `c.O.x` の `c.O` / `self.L` / `g.L` のように、経路のどこかが書き換えられないなら、
    /// その先の要素・フィールドも書き換えられない（規則 1「要素・フィールドは根の属性を継ぐ」を
    /// 経路の途中に広げたもの）。根の識別子そのもの（`const X`）は束縛の属性（`VarInfo`）が見る。
    pub(super) fn readonly_on_path(&mut self, path: &Expr) -> Option<ReadOnly> {
        match path {
            Expr::Attr { object, attr, .. } => {
                match self.member_owner(object) {
                    Some((MemberOwner::Class(c), _)) if self.is_const_member(&c, attr) => {
                        return Some(ReadOnly::Const { owner: c, member: attr.clone() });
                    }
                    Some((MemberOwner::Module { vars, .. }, _)) => {
                        match vars.attrs.get(attr.as_str()) {
                            Some(VarAttr::Const) => {
                                return Some(ReadOnly::Const { owner: vars.name, member: attr.clone() });
                            }
                            Some(a) if !a.contents_mutable() => {
                                return Some(ReadOnly::Immutable { name: format!("{}.{attr}", vars.name) });
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
                self.readonly_on_path(object)
            }
            Expr::Subscript { object, .. } => self.readonly_on_path(object),
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
        // ⚠ `const` の中身（`C.L.append(..)` / `self.L.append(..)` / `g.L.append(..)`）と、モジュールの
        //   `let` の中身（`g.LL.append(..)`）は、根の束縛が `mut` でも書き換えられない。
        match self.readonly_on_path(object) {
            Some(ReadOnly::Const { owner, member }) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ModifyConst { owner, member },
                    span: Some(span.clone()),
                });
                return;
            }
            Some(ReadOnly::Immutable { name }) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::MutatingMethodOnImmutable { method: attr.to_string(), root_name: name },
                    span: Some(span.clone()),
                });
                return;
            }
            None => {}
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

    /// 属性・添字への代入（複合代入も）の書き込み先を検査する。**可否は属性で決める**。
    ///
    /// 1. `obj.attr = ..` で `attr` が書き換えられないメンバーなら:
    ///    - クラス・trait の `const`・enum のメンバー（[`Self::is_const_member`]）→ `AssignToConst`
    ///      （クラス名経由・インスタンス経由・`__init__` の中の `self.LIMIT` のどれでも）
    ///    - モジュールのグローバル変数（`g.X`）→ 属性で: `const` は `AssignToConst`、`let` と取り込んだ名前は
    ///      `AssignToImmutable`、`mut` は代入できる。変数でないメンバー（関数・クラス）も付け替えられない
    /// 2. 書き込む先の**入れ物の経路に書き換えられないもの**がある（`C.L[0] = ..` / `C.O.x = ..` /
    ///    `g.LL[0] = ..`）なら `ModifyConst` / `AssignToImmutable`（[`Self::readonly_on_path`]）。
    /// 3. `obj.attr = ..` で `attr` が `let` フィールドなら `AssignToImmutableField`（`__init__` の中の `self` は除く）。
    ///
    /// ⚠⚠ **1・2 は `__init__` の免除より先に見る**。`__init__` で許すのは `let` フィールドの初回代入で、
    ///    `const` は `__init__` の中でも書き換えられない（実行時も `TypeError: cannot assign to class
    ///    variable`）。
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
            let kind = match o {
                MemberOwner::Class(c) if self.is_const_member(c, attr) => {
                    Some(TypeErrorKind::AssignToConst { owner: c.clone(), member: attr.to_string() })
                }
                MemberOwner::Class(_) => None,
                MemberOwner::Module { members, vars } => match vars.attrs.get(attr) {
                    Some(VarAttr::Const) => {
                        Some(TypeErrorKind::AssignToConst { owner: vars.name.clone(), member: attr.to_string() })
                    }
                    Some(VarAttr::Mut) => None,
                    Some(VarAttr::Let | VarAttr::Imported { .. }) => {
                        Some(TypeErrorKind::AssignToImmutable { name: format!("{}.{attr}", vars.name) })
                    }
                    // 変数でないメンバー（関数・クラス・サブモジュール）は付け替えられない。
                    None if members.contains(attr) => {
                        Some(TypeErrorKind::AssignToImmutable { name: format!("{}.{attr}", vars.name) })
                    }
                    None => None,
                },
            };
            if let Some(kind) = kind {
                self.report_error(StaticTypeError { kind, span });
                return;
            }
        }
        match self.readonly_on_path(object) {
            Some(ReadOnly::Const { owner, member }) => {
                self.report_error(StaticTypeError { kind: TypeErrorKind::ModifyConst { owner, member }, span });
                return;
            }
            Some(ReadOnly::Immutable { name }) => {
                self.report_error(StaticTypeError { kind: TypeErrorKind::AssignToImmutable { name }, span });
                return;
            }
            None => {}
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

/// メンバーの持ち主（[`TypeChecker::member_owner`]）。
pub(super) enum MemberOwner {
    /// クラス・trait・enum（名前）。
    Class(String),
    /// モジュール。`members` はメンバーの名前（変数・関数・クラス…）、`vars` はグローバル変数の属性。
    Module { members: std::collections::HashSet<String>, vars: ModuleVars },
}

/// 書き込み先の経路の途中にある、書き換えられないもの（[`TypeChecker::readonly_on_path`]）。
pub(super) enum ReadOnly {
    /// `const`（クラス・trait の `const`・enum のメンバー・モジュールの `const`）。`owner.member`。
    Const { owner: String, member: String },
    /// 中身を書き換えられない変数（モジュールの `let`・取り込んだ名前）。`name` は `g.LL` の形。
    Immutable { name: String },
}
