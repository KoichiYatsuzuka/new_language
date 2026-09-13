use std::collections::HashSet;

use super::types::InferredType;
use super::TypeChecker;

/// 値の渡し方（タスク 3.3）。整合性検査の厳しさを決める。
///
/// ⚠ 以前は `param_mutable: bool` を引き回していたが、`true`/`false` が呼び出し側で
/// 何を意味するか読めなかった。**書き戻しの有無**は自明キャストを許すかどうかを
/// 左右する本質的な区別なので、名前を付けて渡す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Aliasing {
    /// 値渡し。自明キャスト（`int` → `float`）を許す。
    ByValue,
    /// **書き戻しあり**（`mut` 引数）。記憶域を共有するので自明キャストを許さない。
    WriteBack,
}

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
        let aliasing = if param_mutable { Aliasing::WriteBack } else { Aliasing::ByValue };
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
        // ⚠ 判定本体は `types_compatible` に一本化してある（タスク 3.3）。
        //    `resolve_protocols` は上で済ませたので二重には効かない（冪等）。
        self.types_compatible(&got, &expected, aliasing)
    }

    /// 整合性検査（**Kind 1 / Kind 2**）の**唯一の述語**（タスク 3.3）。報告はしない。
    ///
    /// # 3 分類のどこに当たるか
    ///
    /// | Kind | 期待型 | この関数の中での担当 |
    /// |---|---|---|
    /// | **1**（同一 or 自明キャスト） | プリミティブ・コレクション・クラス・`new_type`・`enum`・テンプレート実体・`function` | [`Self::type_matches`] / [`Self::type_matches_exact`] |
    /// | **2**（アップキャスト） | `trait` / `protocol` / `Intersection` | `type_matches_exact` 内の `satisfies_protocol` / `class_implements_trait` / `Intersection` アーム |
    ///
    /// ⚠ **Kind は「検査地点」ではなく「期待型の種類」で決まる。** 地点は期待型を渡すだけで、
    /// どちらの Kind になるかはこの関数が判断する。だから入口は 1 つで足りる。
    /// ⚠ **Kind 3**（演算子）は被演算子が 2 つで結果型も返すため別系統
    /// （[`Self::check_binop`] / `infer_binop_result`）。
    ///
    /// # なぜ一本化したか
    ///
    /// ⚠⚠ 以前は **`mut` 引数なら自明キャストを許さない**という規則が
    /// `check_expected` と `param_type_matches` の **2 箇所に別々に書かれていた**。
    /// 同じ問いに 2 つの実装がある状態で、片方だけ直すとずれる。
    /// ⚠⚠ さらに `resolve_protocols` を呼ぶ経路と呼ばない経路が混在していたため、
    /// **容器の内側の protocol が経路によって 2 通りに壊れていた**（タスク 1.3 で実測）。
    /// ⇒ 両方をこの関数に閉じ込めた。新しい検査地点はこれを呼ぶだけでよい。
    pub(super) fn types_compatible(
        &self,
        got: &InferredType,
        expected: &InferredType,
        aliasing: Aliasing,
    ) -> bool {
        // ⚠ protocol 名を `Protocol` へ寄せる。**両側**に掛けること（`got` 側を忘れると
        //    「`Pr` という名のクラス」を探しに行って自分自身に不適合という嘘が出る）。
        let expected = self.resolve_protocols(expected);
        let got = self.resolve_protocols(got);
        match aliasing {
            // ⚠ `mut` 引数（write-back）は `int` → `float` の拡大を許さない。
            //    C ABI の `double*` のように呼び先が呼び元の記憶域へ書き戻す引数では、
            //    拡大は「値の変換」ではなく**記憶域の型詐称**になる
            //    （`cpp_prim_ptr_int_arg_type_mismatch` が実際に検出した）。
            Aliasing::WriteBack => self.type_matches_exact(&got, &expected),
            Aliasing::ByValue => self.type_matches(&got, &expected),
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
        // ⚠ `Never`（⊥）はあらゆる型へアップキャストできる（D-11・タスク 4.1）。
        //    空コレクションリテラルの要素型にしか現れない。
        if *arg_ty == InferredType::Never {
            return true;
        }
        if *expected == InferredType::Any {
            return true;
        }
        if arg_ty == expected {
            return true;
        }
        // ── protocol は**構造的に**判定する（タスク 1.3）─────────────────────
        //
        // ⚠⚠ **以前は「任意の `NamedInstance` を受理」していた**（「適合チェックは別途
        //    実施するため」というコメント付き）。その「別途」は `check_expected` が
        //    **期待型が最上位 `Protocol` のときだけ**行うので、**容器の内側では誰も
        //    検査していなかった**。実測で漏れていた形:
        //      fn f() -> Option[Pr]: return No(1)        # 非適合クラスが通る
        //      h.items = [No(1)]   （items: list[Pr]）    # 同じ
        // ⚠ 逆に `let xs: list[HasN] = [Dog(2)]` は `resolve_protocols` を通らない経路で
        //    `HasN` が `NamedInstance` のまま**名前**比較され、**構造的に満たしているのに
        //    弾かれていた**（偽陽性）。経路によって 2 通りに壊れていた。
        if let InferredType::Protocol(proto_name) = expected {
            return self.satisfies_protocol(arg_ty, proto_name.as_str());
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
        // ── 関数型の比較（D-13・タスク 2.2）─────────────────────────────────
        //
        // ⚠⚠ **引数名は比較に使わない。** `FnTypeParam` は `PartialEq` を derive しており
        //    `name` も含むため、素の `arg_ty == expected` では
        //    `function{let param1:int}->int`（注釈由来）と
        //    `function{let y:int}->int`（実体由来）が**名前違いだけで不一致**になる
        //    （実測で 5 例題が落ちた）。
        // ⚠ **引数は反変・戻り値は共変**（D-13）。引数を共変にすると
        //    `let f: function[Any]->int = narrow`（narrow は int を受ける）が通り、
        //    `f("s")` で int 宣言の引数へ str が渡る（不健全）。
        if let InferredType::Function { params: e_params, return_type: e_ret } = expected {
            if let InferredType::Function { params: a_params, return_type: a_ret } = arg_ty {
                // 戻り値は共変（`Any` が期待なら何でも通る）
                if !self.type_matches_exact(a_ret, e_ret) {
                    return false;
                }
                return match (a_params, e_params) {
                    // 期待が素の `function`（シグネチャ未指定）なら引数は問わない
                    (_, None) => true,
                    // 実引数側のシグネチャが不明なら判定材料が無いので通す（保守的側）
                    (None, Some(_)) => true,
                    (Some(a), Some(e)) => {
                        if a.len() != e.len() {
                            return false;
                        }
                        // 引数は**反変**なので `expected → arg` の向きで照合する
                        a.iter()
                            .zip(e.iter())
                            .all(|(ap, ep)| self.type_matches_exact(&ep.ty, &ap.ty))
                    }
                };
            }
        }
        match (arg_ty, expected) {
            // ⚠ テンプレート実体 → 素のテンプレート名は**型引数を忘れる方向**なので
            //    アップキャストとして許す（タスク 2.1）。`list[int]` → `list` と同じ扱い。
            //    例: `mut bare: Box = Box[str]("x")`
            //    ⚠⚠ **逆は許さない**（`Box` → `Box[int]` は情報が増えるダウンキャスト）。
            //    素の容器の双方向特例（下の `(List, ListOf(_))` 等）は D-3 / タスク 4.1 で
            //    撤去する予定なので、ここを**双方向にしないこと**。
            (
                InferredType::GenericInstance { name: a, .. },
                InferredType::NamedInstance(e),
            ) if a == e => return true,
            // ⚠ enum のメンバー型 `enum_item_X` → enum 型 `X` はアップキャスト
            //    （メンバーはその enum に属する）。タスク 2.3。
            //    `let m: Color = Color.Green` という**自然な綴り**を通すために要る
            //    （2.3 でメンバーに型を付けたので、これが無いと書けなくなる）。
            //    ⚠⚠ **逆は許さない**（`Color` → `enum_item_Color` はダウンキャスト）。
            //    ⚠ `enum_item_` は型検査・実行時の両方が使う内部名（`build_enum_classes`）。
            (InferredType::NamedInstance(a), InferredType::NamedInstance(e))
                if a.strip_prefix("enum_item_") == Some(e.as_str()) =>
            {
                return true
            }
            // ── 素の容器と型引数つき容器は**一方向だけ**（D-3・タスク 4.1）──────
            //
            // ⚠⚠ **以前は双方向だった**（`(List, ListOf(_)) => true` も在った）。素の `list` が
            //    `list[任意]` と適合するため、要素型を捨てることが「**何でも通す**」になっていた
            //    （根本原因②の実装本体）。タスク 2.7 で推論側を直し、ここで規則を片方向にした。
            // ⚠ 許すのは「**型引数を忘れる**」方向だけ（`list[int]` → `list`）。
            //    逆（`list` → `list[int]`）は情報が増えるダウンキャストなので許さない。
            (InferredType::ListOf(_), InferredType::List) => return true,
            (InferredType::ListOf(a), InferredType::ListOf(e)) => return self.type_matches_exact(a, e),
            // fixed_list compatibility
            (InferredType::FixedListOf(_), InferredType::FixedList) => return true,
            (InferredType::FixedListOf(a), InferredType::FixedListOf(e)) => return self.type_matches_exact(a, e),
            // list_like accepts list or fixed_list (with or without inner type)
            (a, InferredType::ListLike) if is_list_like(a) => return true,
            (a, InferredType::ListLikeOf(e)) if is_list_like(a) => {
                let a_inner = match a {
                    InferredType::ListOf(i) | InferredType::FixedListOf(i) => Some(i.as_ref()),
                    _ => None,
                };
                // ⚠ 要素型を持たない側（素の `list` / `fixed_list`）から `list_like[T]` へは
                //    **情報が増えるダウンキャスト**なので許さない（D-3・タスク 4.1）。
                //    以前は `is_none_or` で通していた（上の 4 つの双方向特例と同じ形）。
                return match a_inner {
                    Some(ai) => self.type_matches_exact(ai, e),
                    None => false,
                };
            }
            (InferredType::SetOf(_), InferredType::Set) => return true,
            (InferredType::SetOf(a), InferredType::SetOf(e)) => return self.type_matches_exact(a, e),
            (InferredType::DictOf(_, _), InferredType::Dict) => return true,
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
