use std::collections::HashSet;

use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// `Result[T, E]` 型で T == E の場合に静的エラーを記録する。
    pub(super) fn validate_result_type(
        &mut self,
        ok_ty: &InferredType,
        err_ty: &InferredType,
        span: Option<crate::token::Span>,
    ) {
        if ok_ty == err_ty {
            self.report_error(super::errors::StaticTypeError {
                kind: super::errors::TypeErrorKind::ResultSameTypes {
                    ok_type: ok_ty.clone(),
                    err_type: err_ty.clone(),
                },
                span,
            });
        }
    }

    /// 型から **クラス名と「型変数 → 具体型」の置換表**を取り出す。
    ///
    /// - `NamedInstance("C")`            → `(C, 空の表)`
    /// - `GenericInstance{Box, [int]}`   → `(Box, {T: int})`（`T` は `Box` の宣言順の型変数）
    ///
    /// ⚠⚠ これが A-2 の核心。テンプレートクラスのフィールド・メソッドの型は
    /// レジストリに**置換前**（`T`）で入っているので、`Box[int]` からメンバーを引くときは
    /// この表で置換しないと `T` が使用箇所へ漏れる（偽陽性の原因）。
    ///
    /// ⚠ 型引数の個数が宣言と合わないときは `None`（＝検査を見送る）。合っていない表で
    /// 置換すると別の型変数に別の型を当てる嘘の対応付けになる。
    pub(super) fn class_and_subst(
        &self,
        ty: &InferredType,
    ) -> Option<(String, std::collections::HashMap<String, InferredType>)> {
        match ty {
            InferredType::NamedInstance(c) => {
                // ⚠⚠ **型引数を伴わないテンプレート名**（`mut x: Box` / クラス本体の `self`）。
                //    置換表が作れないが、**解決そのものを諦めてはいけない** — 諦めると
                //    同じクラスの**具体型フィールド**（`Mixed[T]` の `count: int`）の検査まで
                //    消える（`template_type_param_error.ar` の検出が落ちて実測で気づいた）。
                //    ⇒ **型変数だけを `Unresolved` へ写す**表を作る。具体型のメンバーは
                //      そのまま検査され、型変数のメンバーは各検査の
                //      `matches!(expected, Unresolved | Any)` ガードで見送られる。
                let map = self
                    .registry
                    .template_params(c.as_str())
                    .map(|ps| {
                        ps.iter()
                            .cloned()
                            .map(|p| (p, InferredType::Unresolved))
                            .collect()
                    })
                    .unwrap_or_default();
                Some((c.clone(), map))
            }
            InferredType::GenericInstance { name, args } => {
                let tparams = self.registry.template_params(name)?;
                if tparams.len() != args.len() {
                    return None;
                }
                let map = tparams
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect();
                Some((name.clone(), map))
            }
            _ => None,
        }
    }

    /// 型の中の `NamedInstance(n)` のうち **`n` が protocol 名のものを `Protocol(n)` へ寄せる**。
    ///
    /// # なぜ検査時に寄せるのか
    ///
    /// 収集パスの `resolve_protocol_type`（[registry/builder.rs]）は
    /// **`known_protocols` が育っている途中**に走るので、protocol より前に宣言された
    /// クラス・関数では変換が効かない。しかも適用されているのは `fn_sigs`（自由関数）だけで、
    /// `class_method_sigs`（メソッド・自動生成 `__init__`）・戻り値注釈・フィールド型は
    /// **`NamedInstance("Pr")` のまま**だった。
    ///
    /// ⚠⚠ そのままだと `type_matches` が**継承関係として**照合し（`class_implements_trait`）、
    /// protocol は構造的適合なので基底に現れず **必ず false** になる。
    /// ⇒ **適合しているクラスまで弾く偽陽性**が出る（`fn make() -> Pr: return Good(1)` が
    /// `'make' is declared to return 'Pr' but returns 'Good'` になっていた・実測）。
    ///
    /// 検査時点ではレジストリが完成しているので、ここで寄せれば宣言順に依存しない。
    pub(super) fn resolve_protocols(&self, ty: &InferredType) -> InferredType {
        use InferredType as T;
        let rec = |t: &T| Box::new(self.resolve_protocols(t));
        match ty {
            T::NamedInstance(n) if self.registry.is_protocol(n.as_str()) => T::Protocol(n.clone()),
            T::GenericInstance { name, args } => T::GenericInstance {
                name: name.clone(),
                args: args.iter().map(|a| self.resolve_protocols(a)).collect(),
            },
            T::ListOf(t) => T::ListOf(rec(t)),
            T::FixedListOf(t) => T::FixedListOf(rec(t)),
            T::ListLikeOf(t) => T::ListLikeOf(rec(t)),
            T::SetOf(t) => T::SetOf(rec(t)),
            T::DictOf(k, v) => T::DictOf(rec(k), rec(v)),
            T::Result(a, b) => T::Result(rec(a), rec(b)),
            T::Union(ts) => T::Union(ts.iter().map(|t| self.resolve_protocols(t)).collect()),
            T::Tuple(ts) => T::Tuple(ts.iter().map(|t| self.resolve_protocols(t)).collect()),
            _ => ty.clone(),
        }
    }

    /// `got` を `expected` の位置へ渡してよいか。**protocol なら適合検査へ回す。**
    ///
    /// 戻り値が `false` のときだけ呼び出し側がエラーを報告する。
    /// ⚠ protocol の不適合は `check_protocol_conformance` が**自分で報告する**ので
    /// `true` を返す（呼び出し側が重ねて報告しないため）。
    ///
    /// ⚠⚠ `type_matches` は `Protocol` を期待型にすると**任意の `NamedInstance` を通す**。
    /// 適合の判定はそちらに無く、`check_protocol_conformance` が別に呼ばれる前提の作りなので、
    /// この関数を通さずに `type_matches` だけで判定すると**不適合が素通りする**。
    pub(super) fn check_expected(
        &mut self,
        got: &InferredType,
        expected: &InferredType,
        param_mutable: bool,
        context: &str,
    ) -> bool {
        let expected = self.resolve_protocols(expected);
        // ⚠⚠ **`got` 側も寄せる。** protocol 名で注釈された仮引数は `declare_param` が
        //    `NamedInstance("Pr")` として束縛する（`from_ann` は protocol とクラスを
        //    区別しない）。寄せずに適合検査へ渡すと「`Pr` という名のクラス」を探しに行き、
        //    メンバーが 1 つも無いので **`type 'Pr' does not satisfy protocol 'Pr'`** という
        //    自分自身に不適合という嘘のエラーになる（自動生成 `__init__` の
        //    `self.p = p` で実際に出た）。
        let got = self.resolve_protocols(got);
        if let InferredType::Protocol(proto) = &expected {
            self.check_protocol_conformance(&got, proto, None, context);
            return true;
        }
        // ⚠ `mut` 引数（write-back）は `int` → `float` の拡大を許さない（`type_matches` の doc）。
        if param_mutable {
            self.type_matches_exact(&got, &expected)
        } else {
            self.type_matches(&got, &expected)
        }
    }

    /// 引数型 `arg_ty` が期待型 `expected` と互換か。**最上位でのみ** `int` → `float` の
    /// 暗黙拡大を許す（案 B・2026-09-08）。
    ///
    /// # なぜ拡大を入れたか
    ///
    /// これを入れる前は `fn f(let x: float)` に `f(3)` が StaticTypeError だった（実測）。
    /// 一方で**フィールド書き込みだけは昇格していた** — `store_field` の raw レイアウト経路に
    /// `int → float フィールドの自動昇格` アームがあるため。つまり
    /// ```text
    /// let b: float = 3    → 3    （昇格しない）
    /// k.x = 7（x: float）  → 7.0  （昇格する）
    /// ```
    /// という非対称が実在した。受理側をフィールドに合わせて広げ、束縛時の昇格も揃える
    /// （[`crate::interpreter::exec::vars::coerce_binding`]）。
    ///
    /// # ⚠⚠ 拡大を**伝播させてはいけない** 2 つの場所
    ///
    /// 1. **要素型・メンバ型（再帰の内側）。** 実行時の昇格はスカラの `float` 注釈だけが
    ///    対象で、`list[float] = [1, 2]` の要素は `Int` のまま。ここで拡大を許すと
    ///    **静的には通るのに実行時は Int が入っている**状態になる。
    ///    ⇒ 内側は [`Self::type_matches_exact`] を使い、そこから拡大へは戻らない。
    /// 2. **`mut` パラメータ（write-back）。** C ABI の `double*` のように呼び先が
    ///    呼び元の記憶域へ書き戻す引数では、拡大は「値の変換」ではなく**記憶域の型詐称**に
    ///    なる（`int` 変数へ 8 バイトの double が書き戻される）。
    ///    ⇒ 呼び出し検査側が `param_mutable` を見て [`Self::type_matches_exact`] を使う。
    ///    ⚠ この 1 件は `cpp_prim_ptr_int_arg_type_mismatch` が実際に検出した。
    ///
    /// ⚠ 逆方向（`float` → `int`）は情報を落とすので許さない。
    pub(super) fn type_matches(&self, arg_ty: &InferredType, expected: &InferredType) -> bool {
        if matches!(
            (arg_ty, expected),
            (InferredType::Int, InferredType::Float)
        ) {
            return true;
        }
        self.type_matches_exact(arg_ty, expected)
    }

    /// `int` → `float` の拡大を**許さない**互換判定。
    ///
    /// 要素型の再帰と `mut` パラメータはこちらを使う（理由は [`Self::type_matches`] の doc）。
    /// ⚠ この関数の内部再帰は**必ず自分自身**を呼ぶこと。`type_matches` へ戻すと
    /// 拡大が要素型へ漏れる。
    pub(super) fn type_matches_exact(
        &self,
        arg_ty: &InferredType,
        expected: &InferredType,
    ) -> bool {
        if *arg_ty == InferredType::Unresolved {
            return true;
        }
        if *expected == InferredType::Any {
            return true;
        }
        if arg_ty == expected {
            return true;
        }
        // Protocol 型パラメータ: 適合チェックは別途実施するため、ここでは基本的に受け入れる
        if let InferredType::Protocol(proto_name) = expected {
            return matches!(
                arg_ty,
                InferredType::NamedInstance(_) | InferredType::Protocol(_) | InferredType::Any
            ) || {
                let _ = proto_name;
                false
            };
        }
        if *expected == InferredType::TypeVal {
            return matches!(arg_ty, InferredType::TypeValOf(_) | InferredType::TypeVal);
        }
        if let InferredType::TypeValOf(expected_inner) = expected {
            return match arg_ty {
                InferredType::TypeVal => true,
                InferredType::TypeValOf(arg_inner) => {
                    self.type_val_compatible(arg_inner, expected_inner)
                }
                _ => false,
            };
        }
        // list_like accepts both list and fixed_list
        let is_list_like = |t: &InferredType| matches!(
            t, InferredType::List | InferredType::ListOf(_)
              | InferredType::FixedList | InferredType::FixedListOf(_)
        );
        match (arg_ty, expected) {
            // list compatibility
            (InferredType::ListOf(_), InferredType::List) => return true,
            (InferredType::List, InferredType::ListOf(_)) => return true,
            (InferredType::ListOf(a), InferredType::ListOf(e)) => return self.type_matches_exact(a, e),
            // fixed_list compatibility
            (InferredType::FixedListOf(_), InferredType::FixedList) => return true,
            (InferredType::FixedList, InferredType::FixedListOf(_)) => return true,
            (InferredType::FixedListOf(a), InferredType::FixedListOf(e)) => return self.type_matches_exact(a, e),
            // list_like accepts list or fixed_list (with or without inner type)
            (a, InferredType::ListLike) if is_list_like(a) => return true,
            (a, InferredType::ListLikeOf(e)) if is_list_like(a) => {
                let a_inner = match a {
                    InferredType::ListOf(i) | InferredType::FixedListOf(i) => Some(i.as_ref()),
                    _ => None,
                };
                return a_inner.is_none_or(|ai| self.type_matches_exact(ai, e));
            }
            (InferredType::SetOf(_), InferredType::Set) => return true,
            (InferredType::Set, InferredType::SetOf(_)) => return true,
            (InferredType::SetOf(a), InferredType::SetOf(e)) => return self.type_matches_exact(a, e),
            (InferredType::DictOf(_, _), InferredType::Dict) => return true,
            (InferredType::Dict, InferredType::DictOf(_, _)) => return true,
            (InferredType::DictOf(ak, av), InferredType::DictOf(ek, ev)) => {
                return self.type_matches_exact(ak, ek) && self.type_matches_exact(av, ev);
            }
            _ => {}
        }
        if let InferredType::Union(union_types) = expected {
            return union_types.iter().any(|ut| self.type_matches_exact(arg_ty, ut));
        }
        // Intersection型: arg_ty がすべての構成型にマッチする必要がある
        if let InferredType::Intersection(isect_types) = expected {
            return isect_types.iter().all(|it| self.type_matches_exact(arg_ty, it));
        }
        // arg_ty が Intersection の場合: arg_ty のいずれかの構成型が expected にマッチすれば可
        if let InferredType::Intersection(isect_types) = arg_ty {
            return isect_types.iter().any(|it| self.type_matches_exact(it, expected));
        }
        if let InferredType::NamedInstance(class_name) = arg_ty {
            let expected_name = expected.to_string();
            let cast_key = format!("__cast__[{}]", expected_name);
            if let Some(methods) = self.registry.class_methods(class_name.as_str()) {
                if methods.contains_key(&cast_key) {
                    return true;
                }
            }
            // Check class/trait inheritance: Duck(Flyable, Swimmable) satisfies Flyable
            if let InferredType::NamedInstance(_) = expected {
                if self.class_implements_trait(class_name, &expected_name) {
                    return true;
                }
            }
        }
        false
    }

    /// `arg_inner` が `expected_inner` と互換性のある型値かを判定する。
    pub(super) fn type_val_compatible(
        &self,
        arg_inner: &InferredType,
        expected_inner: &InferredType,
    ) -> bool {
        if arg_inner == expected_inner {
            return true;
        }

        let InferredType::NamedInstance(arg_name) = arg_inner else {
            return false;
        };

        let expected_name = expected_inner.to_string();

        let mut current = arg_name.clone();
        let mut seen = std::collections::HashSet::new();
        while let Some(orig_name) = self.registry.new_type_original(&current).map(str::to_string) {
            if !seen.insert(orig_name.clone()) {
                break;
            }
            if orig_name == expected_name {
                return true;
            }
            current = orig_name;
        }

        if let Some(bases) = self.registry.class_bases(arg_name.as_str()) {
            return bases.contains(&expected_name);
        }

        false
    }

    /// `raise` できる型かを判定する。
    pub(super) fn is_error_instance_type(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(class_name) => {
                self.class_implements_trait(class_name, "Error")
            }
            InferredType::Union(types) => types.iter().all(|t| self.is_error_instance_type(t)),
            _ => false,
        }
    }

    /// クラスが指定 trait を実装しているかを基底リストから確認する。
    pub(super) fn class_implements_trait(&self, class_name: &str, trait_name: &str) -> bool {
        let mut stack = vec![class_name.to_string()];
        let mut seen = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            let Some(bases) = self.registry.class_bases(cur.as_str()) else {
                continue;
            };
            if bases.iter().any(|base| base == trait_name) {
                return true;
            }
            stack.extend(bases.iter().cloned());
        }
        false
    }
}
