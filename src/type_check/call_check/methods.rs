// type_check/call_check/methods.rs — メソッド呼び出しの検査: Self 型パラメータ・組み込みのメンバー・protocol のメソッド・クラスのメソッドのシグネチャ。

use super::*;

impl TypeChecker {
    /// メソッド呼び出しの引数を検査し、メソッドの戻り値型を返す。
    ///
    /// `self`/`cls` は呼び出し元が暗黙的に渡すため、`effective_count = arg_data.len() + 1`
    /// で実際のパラメータ数と照合する。
    pub(in crate::type_check) fn check_self_type_params(
        &mut self,
        cls_name: &str,
        method_name: &str,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) -> Option<InferredType> {
        let sigs = self
            .registry
            .class_methods(cls_name)
            .and_then(|m| m.get(method_name))
            .cloned()?;
        // Static methods have no implicit receiver; instance/class methods add +1 for self/cls
        let is_static = self.registry.is_static_method(cls_name, method_name);
        self.check_method_sigs(cls_name, method_name, &sigs, is_static, arg_data, args)
    }

    /// 組み込みの値の表のメンバー（`xs.__len__()` / `(5).__index__()` / `[1, 2].count(1)`・pandas_import_plan.md の 6-6）の
    /// 呼び出し。引く名前は実行時の `call_builtin_member` と同じ（特殊メソッドと `count` / `index`・`str` / `bytes` の
    /// `count` / `index` は種類ごとの表のまま）。表に無ければ `None`。
    pub(super) fn builtin_member_call(
        &mut self,
        recv_ty: &InferredType,
        method: &str,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) -> Option<InferredType> {
        use crate::type_check::builtin_members::{self as bm, BuiltinKind, Member};
        if !(method.starts_with("__") || matches!(method, "count" | "index")) {
            return None;
        }
        let shape = bm::shape_of(recv_ty)?;
        let member = bm::members_of(shape.kind).iter().copied().find(|m| m.name() == method)?;
        if matches!(member, Member::Count | Member::Index) && matches!(shape.kind, BuiltinKind::Str | BuiltinKind::Bytes) {
            return None;
        }
        let sigs = member.sigs(&shape);
        Some(self.check_method_sigs(&recv_ty.to_string(), method, &sigs, true, arg_data, args).unwrap_or(InferredType::Unresolved))
    }

    /// protocol の型の値のメソッド呼び出し（`h.get()`・`h: Holder[int]`・pandas_import_plan.md の 6-5）。protocol の要求の
    /// シグネチャ（型引数で読んだもの）で引数を検査し、戻り値の型を返す。
    /// ⚠ 以前は protocol の名前をクラスの名前として引き、クラスのメソッドの表に無いので戻り値が `Unresolved` だった
    ///   （型引数の無い protocol でも `fn f(h: Pr) -> str: return h.get()` が通った）。
    pub(super) fn check_protocol_method_call(
        &mut self,
        proto_name: &str,
        args: &[InferredType],
        method_name: &str,
        call_args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
    ) -> InferredType {
        let Some((fields, methods)) = self.protocol_members(proto_name, args) else {
            return InferredType::Unresolved;
        };
        if let Some(sigs) = methods.get(method_name) {
            // 要求のシグネチャは `self` を含まない（静的メソッドと同じ数え方）
            return self.check_method_sigs(proto_name, method_name, sigs, true, arg_data, call_args)
                .unwrap_or(InferredType::Unresolved);
        }
        // 関数を入れたフィールドを呼ぶ形（`h.f(x)`）
        match fields.get(method_name).map(|(_, t)| t.clone()) {
            Some(InferredType::Function { params, return_type, .. }) => {
                if let Some(fn_params) = params {
                    self.check_fn_type_call(&format!("{proto_name}.{method_name}"), call_args, arg_data, &fn_params);
                }
                *return_type
            }
            // 呼べない値を入れたフィールド・型の分からないフィールド
            Some(ty) => {
                self.check_callable(&ty);
                InferredType::Unresolved
            }
            None => InferredType::Unresolved,
        }
    }

    /// メソッドの呼び出しの引数を、シグネチャ（オーバーロードの並び）で検査して戻り値の型を返す
    /// （[`Self::check_self_type_params`] と protocol の受け手の共通の本体）。`is_static` は `self` を数えないか。
    /// `args` は誤りを指す実引数の式（無ければ今の錨・diagnostic_position_plan.md の 3-2）。
    pub(in crate::type_check) fn check_method_sigs(
        &mut self,
        cls_name: &str,
        method_name: &str,
        sigs: &[FnSig],
        is_static: bool,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) -> Option<InferredType> {
        // 可変長引数エントリを除いた通常引数のみでカウント
        let normal_args: Vec<_> = arg_data
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();
        let variadic_entry = arg_data.iter().find(|(k, _)| k.as_deref() == Some("..."));
        let effective_count = if is_static {
            normal_args.len()
        } else {
            normal_args.len() + 1
        };
        let count_matching: Vec<FnSig> = sigs
            .iter()
            // ⚠ `variadic_type` が Some なら引数の上限は無い（`*args` / `**kwargs` を持つ）。
            .filter(|s| {
                effective_count >= s.required_count
                    && (s.variadic_type.is_some() || effective_count <= s.params.len())
            })
            .cloned()
            .collect();
        if count_matching.is_empty() {
            // Arg count mismatch: for instance/class methods subtract 1 to exclude implicit self/cls
            let implicit = usize::from(!is_static);
            if sigs.len() == 1 {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgCountMismatch {
                        func_name: format!("{cls_name}.{method_name}"),
                        expected_min: sigs[0].required_count.saturating_sub(implicit),
                        expected_max: sigs[0].params.len().saturating_sub(implicit),
                        got: normal_args.len(),
                    },
                    span: None,
                });
            } else {
                let available = sigs
                    .iter()
                    .map(|s| s.params.len().saturating_sub(implicit))
                    .collect();
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoMatchingOverload {
                        func_name: format!("{cls_name}.{method_name}"),
                        got: normal_args.len(),
                        available,
                    },
                    span: None,
                });
            }
            return None;
        }
        // ⚠ 個数で 1 本に絞れないときは**実引数の型**で絞る（タスク 5.7・検体 `C12`）。
        let count_matching = self.narrow_overloads_by_arg_types(
            &format!("{cls_name}.{method_name}"),
            count_matching,
            arg_data,
            !is_static,
        );
        if count_matching.len() != 1 {
            return None;
        }
        let sig = &count_matching[0];
        // 可変長引数の型チェック
        if let (Some((_, IT::ListOf(elem_ty))), Some(expected_elem_ty)) =
            (variadic_entry, &sig.variadic_type)
        {
            // ⚠ これは**要素型同士**の比較で、仮引数への束縛ではない（実行時に
            //    `__cast__` は挟まらない）ので `Site::Other`。
            if !self.types_compatible(elem_ty, expected_elem_ty, Site::Other) {
                self.report_error_at_opt(Self::variadic_arg_expr(args), StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: format!("{cls_name}.{method_name}"),
                        param_index: usize::MAX,
                        expected: expected_elem_ty.clone(),
                        got: *elem_ty.clone(),
                    },
                    span: None,
                });
            }
        }
        // ── 引数の型検査（0-4）─────────────────────────────────────────────
        // ⚠⚠ ここは長らく**個数と `SelfType` しか見ていなかった**ので、インスタンス／
        //    クラス／`static` すべてのメソッドで引数型が素通りしていた（実測）。
        //    検査内容は `check_call_args`（自由関数側）と揃える。
        // ⚠ `self`/`cls` の分だけ添字がずれる。`implicit` は arity 検査と**同じ規約**。
        let implicit = usize::from(!is_static);
        let mut positional_idx = 0usize;
        for (arg_i, (key, arg_ty)) in normal_args.iter().enumerate() {
            let param_idx = match key {
                // キーワード引数は**名前で**引く（位置で数えると別の仮引数を見る）。
                // 未知の名前はここでは報告しない（個数検査の担当外なので黙って飛ばす）。
                Some(kwarg_name) => match sig.params.iter().position(|(n, _)| n == kwarg_name) {
                    Some(i) => i,
                    None => continue,
                },
                None => {
                    let i = positional_idx + implicit;
                    positional_idx += 1;
                    i
                }
            };
            let Some((_, Some(expected))) = sig.params.get(param_idx) else {
                continue;
            };
            // `Self` 型パラメータは下の専用検査（`SelfTypeMismatch`）が見る。
            // ここで `type_matches` に掛けると二重に鳴る。
            if matches!(expected, InferredType::SelfType) {
                continue;
            }
            // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
            let expected = expected.clone();
            let mutable = sig.param_mutable.get(param_idx).copied().unwrap_or(false);
            let ctx = format!("argument to `{cls_name}.{method_name}`");
            if !self.check_expected(arg_ty, &expected, mutable, &ctx) {
                self.report_error_at_opt(Self::arg_expr(args, arg_i), StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: format!("{cls_name}.{method_name}"),
                        // 呼び出し側から見た位置に直す（`self` を数えない）。
                        param_index: param_idx.saturating_sub(implicit),
                        expected,
                        got: (*arg_ty).clone(),
                    },
                    span: None,
                });
            }
        }
        for (arg_idx, (_, arg_ty)) in normal_args.iter().enumerate() {
            let param_idx = arg_idx + implicit;
            if let Some((param_name, Some(InferredType::SelfType))) = sig.params.get(param_idx) {
                if let InferredType::NamedInstance(got_cls) = arg_ty {
                    if got_cls != cls_name {
                        self.report_error_at_opt(Self::arg_expr(args, arg_idx), StaticTypeError {
                            kind: TypeErrorKind::SelfTypeMismatch {
                                method: method_name.to_string(),
                                param_name: param_name.clone(),
                                expected_class: cls_name.to_string(),
                                got_class: got_cls.clone(),
                            },
                            span: None,
                        });
                    }
                }
            }
        }
        sig.return_type.clone()
    }
}
