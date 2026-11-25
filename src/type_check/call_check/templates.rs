// type_check/call_check/templates.rs — テンプレートの呼び出し: 型引数の代入・結果の型・引数の検査。

use super::*;

impl TypeChecker {
    /// 型の中の**テンプレート型変数を具体型へ置換**する（`T` → `int`、`list[T]` → `list[int]`）。
    ///
    /// 実体化呼び出し `Box[int]("s")` の引数を検査するには、シグネチャに書かれた `T` を
    /// 呼び出し点の型引数へ写す必要がある。置換表に無い名前はそのまま残す。
    pub(in crate::type_check) fn subst_type_params(
        ty: &InferredType,
        map: &std::collections::HashMap<String, InferredType>,
    ) -> InferredType {
        use InferredType as T;
        let rec = |t: &T| Box::new(Self::subst_type_params(t, map));
        match ty {
            T::NamedInstance(n) => map.get(n).cloned().unwrap_or_else(|| ty.clone()),
            T::GenericInstance { name, args } => T::GenericInstance {
                name: name.clone(),
                args: args.iter().map(|a| Self::subst_type_params(a, map)).collect(),
            },
            // 総称の protocol の型引数（`Holder[T]` の要求を `Holder[int]` で読む・6-5）
            T::Protocol(name, args) => {
                T::Protocol(name.clone(), args.iter().map(|a| Self::subst_type_params(a, map)).collect())
            }
            T::PyList(t) => T::PyList(rec(t)),
            T::PySet(t) => T::PySet(rec(t)),
            T::PyDict(k, v) => T::PyDict(rec(k), rec(v)),
            T::ListOf(t) => T::ListOf(rec(t)),
            T::FixedListOf(t) => T::FixedListOf(rec(t)),
            T::ListLikeOf(t) => T::ListLikeOf(rec(t)),
            T::SetOf(t) => T::SetOf(rec(t)),
            T::FrozenSetOf(t) => T::FrozenSetOf(rec(t)),
            T::TypeValOf(t) => T::TypeValOf(rec(t)),
            T::DictOf(k, v) => T::DictOf(rec(k), rec(v)),
            T::Result(a, b) => T::Result(rec(a), rec(b)),
            T::Union(ts) => T::Union(ts.iter().map(|t| Self::subst_type_params(t, map)).collect()),
            T::Intersection(ts) => {
                T::Intersection(ts.iter().map(|t| Self::subst_type_params(t, map)).collect())
            }
            T::Tuple(ts) => T::Tuple(ts.iter().map(|t| Self::subst_type_params(t, map)).collect()),
            // ⚠ 関数型・イテレータ型の内側も置換する（タスク 2-15）。抜けていたので
            //   `fn adder[T](..) -> function[T]->T` の `adder[int](1)` が `function[T]->T` のまま
            //   外へ出て、`f(2)` が「`T` を期待したのに `int`」という偽のエラーになっていた（実測）。
            T::IteratorOf(t) => T::IteratorOf(rec(t)),
            T::Function { params, return_type, mutates_immutable } => T::Function {
                mutates_immutable: mutates_immutable.clone(),
                params: params.as_ref().map(|ps| {
                    ps.iter()
                        .map(|p| FnTypeParam {
                            ty: Self::subst_type_params(&p.ty, map),
                            ..p.clone()
                        })
                        .collect()
                }),
                return_type: rec(return_type),
            },
            _ => ty.clone(),
        }
    }

    /// テンプレートの実体化呼び出し（`f[int](…)` / `Box[int](…)`）の**引数型**を検査する。
    ///
    /// ⚠⚠ **ここが無いと実体化呼び出しは丸ごと素通りする。** `infer_call_inner` の
    /// `func_name` は `Expr::Ident` / `Expr::Attr` しか拾わず、`Expr::TemplateInstantiate` は
    /// `_ => None` に落ちる。そのため `check_call_args`（自由関数）も
    /// コンストラクタ検査も走らず、`Box[int]("s")` が静的にも実行時にも通っていた（実測）。
    ///
    /// 検査する型は**型引数で置換したもの**。置換しないとシグネチャ側が `T`
    /// （`NamedInstance("T")`）のままで、具体型の実引数と必ず食い違う。
    ///
    /// ⚠ 個数の不一致（型引数・実引数とも）は**ここでは報告しない**。どちらも実行時に
    /// `TemplateError` / `TypeError` で捕まるうえ、個数が合っていないと置換表が作れず
    /// 対応付け自体が嘘になる。⇒ 合っているときだけ型を見る。
    /// テンプレート実体化呼び出し `Base[T1, T2](args)` の**結果型**（タスク 2.1）。
    ///
    /// ⚠⚠ **以前はここが無く、呼び出し点は常に `Unresolved` を返していた。**
    /// `Unresolved` は `type_matches_exact` の万能受容体なので、
    ///
    /// ```text
    /// let x: int = Box[str]("s")      # 通っていた（int 変数に Box が入る）
    /// take(Box[str]("s"))             # take(let b: Box[int]) でも通っていた
    /// ```
    ///
    /// のように**下流の全義務が無効化**されていた。型引数は既に手元にあり
    /// （`check_template_call_args` が引数検査に使っている）、`GenericInstance` と
    /// `subst_type_params` も A-2 / 0-6 で揃っていたので、組んで返すだけでよかった。
    ///
    /// ⚠ 解釈できない型引数・個数不一致・オーバーロードは `Unresolved` に倒す
    /// （取りこぼす方へ。個数不一致は実行時の `TemplateError` が捕まえる）。
    /// 具体化の式（`Box[int]` / `m.Box[int]`）の**テンプレート名**（タスク 2-15）。
    ///
    /// ⚠ `m.Box[int]` は `m` が `import` したモジュールの名前空間で、`Box` をメンバーに持つときだけ
    ///   `Box` と読む。以前は識別子の形しか見ておらず、モジュールのテンプレートの具体化は
    ///   コンストラクタも結果の型も検査されなかった（型が `Unresolved` に落ちていた）。
    /// ⚠ レジストリは `import` の本体の宣言も**素の名前で**集めている（`m.Plain(..)` と同じ扱い）。
    /// 式がモジュール（の連鎖）を指していれば、その Arrow のモジュールの名前（CPython 準拠・2026-10-02）。
    ///
    /// - `a`（`import a.b` / `import a.b as a` の束縛）→ `a` / `a.b`
    /// - `a.b`（`a` がパッケージで、`a.b` がモジュール）→ `a.b`
    ///
    /// ⚠ 束縛が名前空間の型でなければ（同じ綴りのローカル変数など）モジュールとは見なさない。
    pub(in crate::type_check) fn module_path_of_expr(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident { name, .. } => {
                let is_ns = matches!(
                    self.lookup(name).map(|v| &v.ty),
                    Some(InferredType::Namespace(..) | InferredType::PyNamespace(..))
                );
                if !is_ns {
                    return None;
                }
                self.registry.module_of_alias(name).map(str::to_string)
            }
            Expr::Attr { object, attr, .. } => {
                let p = self.module_path_of_expr(object)?;
                let path = format!("{p}.{attr}");
                self.registry.is_module(&path).then_some(path)
            }
            _ => None,
        }
    }

    pub(in crate::type_check) fn template_base_name(&self, base: &Expr) -> Option<String> {
        // ⚠ 名前は今の文脈で修飾名へ引き直す（フェーズ10 10-8）。モジュールのテンプレートは
        //   `tags.Box` の名前で登録されている（`m.Box` は別名 `m` を引き直して `tags.Box`）。
        match base {
            Expr::Ident { name, .. } => Some(self.registry.resolve(name).into_owned()),
            // `a.b.Box[int]`（`import a.b` は `a` を束縛する・CPython 準拠）。
            Expr::Attr { object, attr, .. } if self.module_path_of_expr(object).is_some() => {
                self.module_path_of_expr(object).map(|p| format!("{p}.{attr}"))
            }
            Expr::Attr { object, attr, .. } => {
                let Expr::Ident { name, .. } = object.as_ref() else { return None };
                match self.lookup(name).map(|v| &v.ty) {
                    Some(InferredType::Namespace(members, _, _)) if members.contains_key(attr) => {
                        Some(self.registry.module_member(name, attr).unwrap_or_else(|| attr.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 具体化の式（`Box[int](..)` / `ident[int](..)`）の結果の型（タスク 2.1 / 10-4）。
    ///
    /// ⚠⚠ **型引数で置き換えない**（タスク 10-4）。展開器が具体化ごとに普通の宣言（`ident[int]`）を
    ///   置くので、その宣言の型を読む。以前はテンプレートの宣言を型引数で置き換えて読んでいた
    ///   （単相化と同じことを型検査だけで別にやっていた）。
    /// ⚠ クラスの具体化は `GenericInstance`（型の表現。メンバーは `instance_class` で具体クラスから引く）。
    /// ⚠ 具体化が無い（テンプレートの本体の中の `ident[T](x)`・型変数のまま）ときは `Unresolved`。
    pub(super) fn template_call_result_type(&self, base_name: &str, type_args: &[String]) -> InferredType {
        let Some(tparams) = self.registry.template_params(base_name) else {
            return InferredType::Unresolved; // テンプレートでない名前
        };
        if tparams.len() != type_args.len() {
            return InferredType::Unresolved; // 個数不一致（展開時の TemplateError）
        }
        if self.registry.is_known_class(base_name) {
            let mut args = Vec::with_capacity(type_args.len());
            for a in type_args {
                match InferredType::from_ann(a) {
                    Some(t) if !matches!(t, InferredType::Unresolved) => args.push(t),
                    // 未知の綴りが 1 つでも混ざったら全体を諦める（嘘の型を作らない）
                    _ => return InferredType::Unresolved,
                }
            }
            return InferredType::GenericInstance { name: base_name.to_string(), args };
        }
        // テンプレート関数 → 具体化した関数の宣言の戻り値型
        let inst = crate::template_subst::instance_name(base_name, type_args);
        match self.visible_fn_sigs(&inst) {
            // ⚠ オーバーロードは実引数で決まるのでここでは決めない
            Some(sigs) if sigs.len() == 1 => {
                sigs[0].return_type.clone().unwrap_or(InferredType::Unresolved)
            }
            _ => InferredType::Unresolved,
        }
    }

    pub(super) fn check_template_call_args(
        &mut self,
        base_name: &str,
        type_args: &[String],
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) {
        if self.registry.template_params(base_name).is_none() {
            return; // テンプレートでない名前（実行時に別のエラーになる）
        }
        // ⚠⚠ **具体化した宣言のシグネチャで検査する**（タスク 10-4）。型引数で置き換えない。
        //   具体化が無い（テンプレートの本体の中・型変数のまま、型引数の個数違い）なら見送る
        //   （具体化した本体の側で検査される・個数違いは展開時のエラー）。
        let inst = crate::template_subst::instance_name(base_name, type_args);
        // ⚠ 実引数の個数が合わないときは対応付けが嘘になるので見送る（実行時に捕まる）。
        let normal: Vec<_> = arg_data
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();
        // シグネチャを引く。クラスなら自動生成された `__init__`（`self` が先頭）。
        let (candidates, implicit) = if self.registry.is_known_class(&inst) {
            match self
                .registry
                .class_methods(&inst)
                .and_then(|m| m.get("__init__"))
            {
                Some(sigs) => (sigs.clone(), 1usize),
                None => return, // `__init__` が無い（`new_type` ラッパ等）
            }
        } else {
            match self.visible_fn_sigs(&inst) {
                Some(sigs) => (sigs.clone(), 0usize),
                None => return,
            }
        };
        // ⚠⚠ **オーバーロードは個数で絞る**（`check_self_type_params` と同じ規約）。
        //    以前は `sigs.len() == 1` でしか検査せず、`__init__` が 2 つあるだけで
        //    `Box[int]("wrong")` が素通りしていた（実測）。
        //    絞って 1 本に決まらないときだけ見送る。
        let matching: Vec<&FnSig> = candidates
            .iter()
            .filter(|s| {
                s.variadic_type.is_none() && normal.len() + implicit == s.params.len()
            })
            .collect();
        if matching.len() != 1 {
            return;
        }
        let sig = matching[0].clone();
        let mut positional_idx = 0usize;
        for (arg_i, (key, arg_ty)) in normal.into_iter().enumerate() {
            let param_idx = match key {
                // キーワード引数は名前で引く（位置で数えると別の仮引数を見る）。
                Some(kw) => match sig.params.iter().position(|(n, _)| n == kw) {
                    Some(i) => i,
                    None => continue,
                },
                None => {
                    let i = positional_idx + implicit;
                    positional_idx += 1;
                    i
                }
            };
            let Some((_, Some(declared))) = sig.params.get(param_idx) else {
                continue;
            };
            let expected = declared.clone();
            // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
            let mutable = sig.param_mutable.get(param_idx).copied().unwrap_or(false);
            let ctx = format!("argument to `{inst}`");
            if !self.check_expected(arg_ty, &expected, mutable, &ctx) {
                self.report_error_at_opt(Self::arg_expr(args, arg_i), StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: inst.clone(),
                        param_index: param_idx.saturating_sub(implicit),
                        expected,
                        got: arg_ty.clone(),
                    },
                    span: None,
                });
            }
        }
    }
}
