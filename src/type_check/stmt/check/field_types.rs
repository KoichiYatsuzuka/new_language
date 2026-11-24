// stmt/check/field_types.rs — 型パラメータ・`Self` を含む型の判定と、宣言されたフィールドの型。

use super::*;

impl TypeChecker {
    /// この型が**テンプレート型変数を含む**か（`T` / `list[T]` / `dict[str, T]` …）。
    ///
    /// ⚠⚠ 含むなら**検査しない**。`from_ann` は大文字始まりの未知の識別子を
    /// `NamedInstance` にするので、型変数と実在のクラスが型の上では区別できない。
    /// 具体型と突き合わせると `class Box[T]: mut v: T` の `self.v = 0` が
    /// 「`T` に `int` を入れた」という**偽のエラー**になる（実測）。
    ///
    /// 実体化後の AST は型変数が具体型へ置換済みなので、検査したい形はそちらで見られる。
    pub(crate) fn mentions_type_param(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(n) => self.state.is_type_param(n),
            InferredType::GenericInstance { args, .. } => {
                args.iter().any(|a| self.mentions_type_param(a))
            }
            InferredType::ListOf(t)
            | InferredType::FixedListOf(t)
            | InferredType::ListLikeOf(t)
            | InferredType::SetOf(t)
            | InferredType::FrozenSetOf(t)
            | InferredType::TypeValOf(t) => self.mentions_type_param(t),
            InferredType::DictOf(k, v) => {
                self.mentions_type_param(k) || self.mentions_type_param(v)
            }
            InferredType::Result(a, b) => {
                self.mentions_type_param(a) || self.mentions_type_param(b)
            }
            InferredType::Union(ts) | InferredType::Intersection(ts) | InferredType::Tuple(ts) => {
                ts.iter().any(|t| self.mentions_type_param(t))
            }
            // ⚠ 関数型・イテレータ型の内側も見る（タスク 2-15。`function[T]->T` を取りこぼしていた）。
            InferredType::IteratorOf(t) => self.mentions_type_param(t),
            InferredType::Function { params, return_type, .. } => {
                params.iter().flatten().any(|p| self.mentions_type_param(&p.ty))
                    || self.mentions_type_param(return_type)
            }
            _ => false,
        }
    }

    /// `ty` が trait `trait_name` の**テンプレート引数名**を含むか（タスク 5.2d）。
    ///
    /// ⚠ [`Self::mentions_type_param`] は「いま見えている型変数」しか見ない。
    /// trait を実装したクラスの中からは基底 trait の型変数は見えないので、
    /// レジストリから引いた名前と突き合わせる必要がある。
    pub(super) fn mentions_trait_param(&self, trait_name: &str, ty: &InferredType) -> bool {
        let Some(params) = self.registry.template_params(trait_name) else {
            return false;
        };
        if params.is_empty() {
            return false;
        }
        Self::mentions_any_name(ty, params)
    }

    /// `ty` が `names` のいずれかを**クラス名の位置**に含むか（再帰）。
    fn mentions_any_name(ty: &InferredType, names: &[String]) -> bool {
        match ty {
            InferredType::NamedInstance(n) => names.iter().any(|p| p == n),
            InferredType::GenericInstance { args, .. } => {
                args.iter().any(|a| Self::mentions_any_name(a, names))
            }
            InferredType::ListOf(t)
            | InferredType::FixedListOf(t)
            | InferredType::ListLikeOf(t)
            | InferredType::SetOf(t)
            | InferredType::FrozenSetOf(t)
            | InferredType::IteratorOf(t)
            | InferredType::TypeValOf(t) => Self::mentions_any_name(t, names),
            InferredType::DictOf(k, v) => {
                Self::mentions_any_name(k, names) || Self::mentions_any_name(v, names)
            }
            InferredType::Result(a, b) => {
                Self::mentions_any_name(a, names) || Self::mentions_any_name(b, names)
            }
            InferredType::Union(ts) | InferredType::Intersection(ts) | InferredType::Tuple(ts) => {
                ts.iter().any(|t| Self::mentions_any_name(t, names))
            }
            _ => false,
        }
    }

    /// 型注釈中の `Self` を現在のクラスへ解決する（クラス外ならそのまま）。
    pub(super) fn resolve_self_type(&self, ty: InferredType) -> InferredType {
        match (&ty, self.state.current_class()) {
            (InferredType::SelfType, Some(cls)) => Self::instance_type_of(cls),
            _ => ty,
        }
    }

    /// クラス名からインスタンスの型を作る。
    ///
    /// ⚠⚠ 単相化したクラス（`Box[int]`）は **`GenericInstance` の形にそろえる**（タスク 2-8 段階 2）。
    /// 注釈 `Box[int]` もテンプレート呼び出し `Box[int](..)` の結果も `GenericInstance` なので、
    /// `self` だけが `NamedInstance("Box[int]")` だと同じ型が 2 つの形を持ち、`return self` が
    /// 「`Box[int]` を返すはずが `Box[int]` を返している」という偽陽性になる。
    pub(crate) fn instance_type_of(cls: &str) -> InferredType {
        if cls.contains('[') {
            if let Some(t @ InferredType::GenericInstance { .. }) = InferredType::from_ann(cls) {
                return t;
            }
        }
        InferredType::NamedInstance(cls.to_string())
    }

    /// クラス `class_name` のフィールド `field` の**宣言型**を引く。
    ///
    /// ⚠⚠ フィールドの宣言は**2 つのテーブルに分かれて**入っている:
    /// - 自クラスと基底クラスが宣言したもの … `class_field_details`
    /// - **基底 trait** が宣言したもの        … `trait_field_details`
    ///
    /// 前者しか見ないと `class Wolf(Creature)` の `w.hp`（`hp` は `Creature` 由来）が
    /// 引けず、型検査が素通りする。`build_field_index` が実行時に
    /// 「trait のフィールドを先頭に、own を後ろに」と**1 つのスロット列へ畳んでいる**のと
    /// 同じものを、静的側でも 1 つに見せるための関数。
    /// 【計測用・一時】`declared_field_type` の別モジュール向け入口。
    pub(crate) fn declared_field_type_pub(&self, class_name: &str, field: &str) -> Option<InferredType> {
        self.declared_field_type(class_name, field)
    }

    pub(super) fn declared_field_type(&self, class_name: &str, field: &str) -> Option<InferredType> {
        // own（＋クラス継承）を先に見る。own の宣言が trait の宣言を上書きするため。
        if let Some((_, ty)) = self.collect_class_field_details(class_name).get(field) {
            return Some(ty.clone());
        }
        // 基底 trait を宣言順に辿る。複数 trait が同名フィールドを持つ場合は
        // 実行時も同一スロットを共有する（`build_field_index`）ので、最初の 1 件でよい。
        for base in self.registry.class_bases(class_name).unwrap_or(&[]) {
            if let Some((_, ty)) = self
                .registry
                .trait_field_details(base)
                .and_then(|m| m.get(field))
            {
                // ⚠⚠ **trait の型変数を具体型へ置換する**（タスク 9.9）。
                //    `trait Holder[T]: let item: T` を `class IntBox(Holder[int])` が
                //    継承したとき、以前はここが `T` を**そのまま**返していたので
                //    `self.item = 3` が `declared 'T' but got 'int'` になっていた。
                //    ⚠ 置換表は `class_base_args`（基底ごとの具体型引数）から作る。
                //      パーサは同じ情報を持っていたが AST へ載せていなかった。
                return Some(self.subst_trait_params(base, class_name, ty));
            }
        }
        None
    }

    /// 基底 `base`（trait）の**型変数を、`class_name` が渡した具体型へ**置換する（9.9）。
    ///
    /// ⚠ 型引数が書かれていない（`class C(Holder)` や非テンプレート trait）なら
    /// 何も置換せずに返す。「置換できない」を `Unresolved` へ倒さないのは、
    /// 具体型のメンバー（`T` を含まないフィールド）の検査まで消してしまうため
    /// （`class_and_subst` が同じ理由で同じ判断をしている）。
    fn subst_trait_params(&self, base: &str, class_name: &str, ty: &InferredType) -> InferredType {
        let Some(params) = self.registry.template_params(base) else {
            return ty.clone();
        };
        let args = self.registry.class_base_args(class_name, base);
        if params.is_empty() || params.len() != args.len() {
            return ty.clone();
        }
        let subst: std::collections::HashMap<String, InferredType> = params
            .iter()
            .cloned()
            .zip(
                args.iter()
                    .map(|a| InferredType::from_ann(a).unwrap_or(InferredType::Unresolved)),
            )
            .collect();
        Self::subst_type_params(ty, &subst)
    }
}
