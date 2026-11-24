// type_check/infer/fn_types.rs — 関数・束縛メソッドを値として読んだときの型。

use super::*;

impl TypeChecker {
    /// 名前 `name` で今見える関数のシグネチャ（入れ子の関数を先に引き、無ければ最上位の関数・name_scope_plan.md の 1-7）。
    ///
    /// ⚠ 検査器から関数のシグネチャを名前で引くときは**必ずこれを使う**。レジストリの `fn_sigs` には入れ子の関数が無い。
    pub(in crate::type_check) fn visible_fn_sigs(&self, name: &str) -> Option<&Vec<crate::type_check::types::FnSig>> {
        self.state.lookup_local_fn(name).or_else(|| self.registry.fn_sigs(name))
    }

    /// 名前が関数を指すとき、その**関数値としての型**（タスク 2.2）。
    ///
    /// ⚠ `Expr::Ident` は変数スコープだけを引いていたので、`fn` を名前で参照した値の型が
    /// `Unresolved` になっていた。情報は `fn_sigs`（引数・戻り値）に揃っている。
    ///
    /// ⚠ **オーバーロードは `None` に倒す。** 関数値としては型が 1 つに決まらず、
    /// ここで 1 本を選ぶと嘘になる（呼び出し点は `check_call_args` が個数と型で解決する）。
    /// ⚠ **テンプレート関数も `None`。** 型引数が決まらないとシグネチャが定まらない
    /// （`f[int]` の形は `Expr::TemplateInstantiate` が扱う）。
    pub(in crate::type_check) fn fn_value_type(&self, name: &str) -> Option<InferredType> {
        if self
            .registry
            .template_params(name)
            .is_some_and(|p| !p.is_empty())
        {
            return None;
        }
        let sigs = self.visible_fn_sigs(name)?;
        if sigs.len() != 1 {
            return None; // オーバーロード
        }
        Self::fn_type_of_sig(&sigs[0], 0)
    }

    /// シグネチャの**関数値としての型**（先頭の `skip` 個の仮引数を除く・束縛メソッドは `self` の 1 個）。
    pub(in crate::type_check) fn fn_type_of_sig(sig: &crate::type_check::types::FnSig, skip: usize) -> Option<InferredType> {
        // ⚠⚠ **可変長パラメータを持つ関数は型を作らない。** `FnTypeParam` は
        //    「個数が固定の引数列」しか表せないので、`f(... = 1, 2, 3)` を正しく検査できず
        //    `takes 0 argument(s) but 1 were given` という**嘘のエラー**になる（実測）。
        //    `resolve.rs` の `has_open_arity` と同じ理由・同じ判断。
        if sig.variadic_type.is_some() {
            return None;
        }
        let params = sig
            .params
            .iter()
            .zip(sig.param_mutable.iter())
            .enumerate()
            .skip(skip)
            .map(|(i, ((pname, pty), pmut))| crate::type_check::types::FnTypeParam {
                name: pname.clone(),
                mutable: *pmut,
                // 注釈が無い仮引数は `Any`（`from_ann` 経路と同じ既定）。
                ty: pty.clone().unwrap_or(InferredType::Any),
                has_default: i >= sig.required_count,
            })
            .collect();
        Some(InferredType::Function {
            params: Some(params),
            return_type: Box::new(sig.return_type.clone().unwrap_or(InferredType::Any)),
            mutates_immutable: None,
        })
    }

    /// `obj.method` を**値として**読んだときの型（束縛メソッド・python_builtins_plan.md のタスク 1-11）。
    ///
    /// - クラスのメソッド: 多重定義が 1 つなら `self` を除いたシグネチャの関数型（型引数は置換する）、
    ///   多重定義・可変長ならシグネチャ未確定の関数型（`function`）
    /// - 組み込みの値のメソッド（`xs.append`）: シグネチャ未確定の関数型。戻り値は分かるものだけ
    /// - **受け手が不変**（パスの根が `let` / `freeze`）で、メソッドが受け手を書き換える（`mut self` /
    ///   `MUTATING_COLLECTION_METHODS`）なら `mutates_immutable` を付ける。呼ぶと `MutatingMethodOnImmutable`
    ///   （`c.add(1)` と同じ誤り）。束縛すること自体は誤りにしない
    /// - クラスの値から読む `C.m` は束縛しない関数なので対象外（`None`）
    pub(super) fn bound_method_type(
        &mut self,
        object: &Expr,
        obj_ty: &InferredType,
        attr: &str,
        class_subst: Option<&(String, std::collections::HashMap<String, InferredType>)>,
    ) -> Option<InferredType> {
        // ⚠ 要るときだけ引く（`path_is_mutable` は経路の `const` を見るので `&mut self`）。
        let immutable_root = |this: &mut Self| {
            (this.path_is_mutable(object) == Some(false))
                .then(|| Self::path_root_ident(object).unwrap_or("").to_string())
        };
        let kinds = Self::builtin_method_kinds(obj_ty);
        if !kinds.is_empty() {
            if !kinds.iter().any(|k| crate::type_check::names::is_builtin_value_method(k, attr)) {
                return None;
            }
            let mutates = Self::MUTATING_COLLECTION_METHODS.contains(&attr)
                && !matches!(obj_ty, InferredType::Str | InferredType::IteratorOf(_));
            return Some(InferredType::Function {
                params: None,
                return_type: Box::new(
                    // ⚠ 引数はまだ無い（束縛メソッドの値）。引数で型が決まるメソッド（`get` など）は分からない。
                    Self::builtin_collection_method_return(obj_ty, attr, &[]).unwrap_or(InferredType::Unresolved),
                ),
                mutates_immutable: if mutates {
                    immutable_root(self).map(|root| Box::new((attr.to_string(), root)))
                } else {
                    None
                },
            });
        }
        if matches!(obj_ty, InferredType::TypeValOf(_)) {
            return None;
        }
        let (class, map) = class_subst?;
        let sigs = self.collect_class_method_sigs(class).remove(attr)?;
        if self.registry.is_static_method(class, attr) {
            return None;
        }
        let skip = usize::from(sigs[0].params.first().is_some_and(|(n, _)| n == "self"));
        let ty = match sigs.as_slice() {
            [sig] => Self::fn_type_of_sig(sig, skip),
            _ => None,
        }
        .map(|t| Self::subst_type_params(&t, map))
        .unwrap_or(InferredType::Function {
            params: None,
            return_type: Box::new(InferredType::Unresolved),
            mutates_immutable: None,
        });
        let mutates = Self::all_sigs_mutate_self(&sigs);
        Some(match ty {
            InferredType::Function { params, return_type, .. } => InferredType::Function {
                params,
                return_type,
                mutates_immutable: if mutates {
                    immutable_root(self).map(|root| Box::new((attr.to_string(), root)))
                } else {
                    None
                },
            },
            other => other,
        })
    }
}
