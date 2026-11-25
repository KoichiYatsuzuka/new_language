// type_check/call_check/args.rs — 呼び出しの引数の検査: 個数・キーワード・展開（`*` / `**`）・引数の型・`mut` 仮引数。

use super::*;

impl TypeChecker {
    /// 名前付き関数呼び出しの引数個数・型・キーワード引数名を検査する。
    /// ⚠ `args`（引数の**式**）を受け取るのは、`mut` パラメータへ `let` の値を渡していないかを
    /// 見るため（bug_fix.md B11）。型だけでは判定できない — **可変性は束縛の属性であって型ではない**。
    /// `mut` パラメータへの実引数として「不変でも構わない」型か（A7-a）。
    ///
    /// ⚠⚠ **B11 の検査が守りたいのは「呼び出し元の `let` 変数が書き変わること」**。
    /// 実測した Arrow の `mut` パラメータの意味論はこうなっている:
    ///
    /// | 実引数 | 呼び出し元へ書き戻るか |
    /// |---|---|
    /// | `mut x: int`（スカラ）| ❌ 値渡し |
    /// | `mut xs: list`（コンテナ）| ✅ 参照 |
    ///
    /// **関数値は書き戻しようがない**（中身を差し替える手段が無い）。それでも弾いていたため、
    /// `m.apply(m.base, n)` のように**関数を引数で渡す形が丸ごと通らなかった**。
    /// ⚠ 見つけたのは Python のコードを Arrow へ変換していた頃（変換器は全パラメータを `mutable: true` にした）だが、
    ///   食い違いは純 Arrow の `fn deco(mut f: function)` でも同じく通らなかった（INF-D と同じ判断）。
    /// ⚠ スカラ（`int` 等）は**対象にしない**。書き戻らないのは同じだが、
    /// そこまで緩めるかは仕様の話で、本件（関数を渡せない）とは別。
    pub(super) fn mut_arg_exempt(ty: &InferredType) -> bool {
        // `Ptr` も書き戻しようがない（Arrow から中身を書き換える手段が無い・D-11）。`T*` の引数へ `let` の
        // `Ptr` を渡せる（ffi_value_exchange_plan.md 3-3）
        match ty {
            InferredType::Function { .. } | InferredType::Ptr(_) => true,
            // `Option[Ptr[T]]`（ポインタの戻り値・4-3）も同じ（`None` も書き戻しようがない）
            InferredType::Union(ts) => ts.iter().all(|t| matches!(t, InferredType::Ptr(_) | InferredType::None)),
            _ => false,
        }
    }

    /// 呼び出し引数に `*xs` / `**d` の展開が含まれるか。
    ///
    /// ⚠⚠ 含まれるときは**引数個数・位置ごとの型検査を降りる**。展開後の個数も
    /// 並び順も実行時にしか決まらないので、位置で対応づける検査が成り立たない。
    /// ⚠ `arg_data` は展開を積まないので、`args` と zip している検査は
    /// **並びがずれる**。降りる判断はその安全性の担保でもある。
    pub(in crate::type_check) fn has_spread_arg(args: &[CallArg]) -> bool {
        args.iter()
            .any(|a| matches!(a, CallArg::Spread(_) | CallArg::KwSpread(_)))
    }

    /// `arg_data` の `i` 番目に当たる実引数の式。引数の誤りをその引数に指させる（diagnostic_position_plan.md の 3-2）。
    ///
    /// ⚠ `arg_data` は展開（`*xs` / `**d`）を積まないので、展開を飛ばして数える。可変長引数（`... = a, b`）は
    ///   引数列の最後にしか書けないので、それを除いた並び（各検査の `normal_args`）の添字もそのまま使える。
    ///   可変長引数そのものは [`Self::variadic_arg_expr`]。
    pub(super) fn arg_expr(args: &[CallArg], i: usize) -> Option<&Expr> {
        args.iter()
            .filter(|a| !matches!(a, CallArg::Spread(_) | CallArg::KwSpread(_) | CallArg::Variadic(_)))
            .nth(i)
            .map(CallArg::expr)
    }

    /// 可変長引数（`... = a, b`）の最初の式。要素型の不一致を指す。
    pub(super) fn variadic_arg_expr(args: &[CallArg]) -> Option<&Expr> {
        args.iter().find(|a| matches!(a, CallArg::Variadic(_))).map(CallArg::expr)
    }

    /// 誤りを式 `at` の錨で報告する。`at` が無い（対応する実引数が見つからない）ときは今の錨のまま。
    pub(super) fn report_error_at_opt(&mut self, at: Option<&Expr>, err: StaticTypeError) {
        match at {
            Some(e) => self.report_error_at(e, err),
            None => self.report_error(err),
        }
    }

    /// `*xs` / `**d` の展開元の型を検査する。
    ///
    /// ⚠⚠ **判らないときは黙って通す**が、**明らかに展開できない型はここで捕まえる**。
    /// 実行時（`expand_spread_into`）と同じ意味の検査を静的に行う。
    /// ⚠ `Unresolved` / `Any` はエラーにしない（注釈が供給されない経路があるため）。
    ///
    /// `at` は展開する式（誤りはそこを指す・diagnostic_position_plan.md の 3-2）。
    pub(super) fn check_spread_arg(&mut self, ty: &InferredType, kw: bool, at: &Expr) {
        use InferredType as IT;
        if matches!(ty, IT::Any | IT::Unresolved) {
            return;
        }
        let ok = if kw {
            matches!(ty, IT::Dict | IT::DictOf(_, _))
        } else {
            matches!(
                ty,
                IT::List
                    | IT::ListOf(_)
                    | IT::FixedList
                    | IT::FixedListOf(_)
                    | IT::ListLike
                    | IT::ListLikeOf(_)
                    | IT::Set
                    | IT::SetOf(_)
                    | IT::FrozenSet
                    | IT::FrozenSetOf(_)
                    | IT::Tuple(_)
                    | IT::TupleAny
                    | IT::Str
                    | IT::Bytes
                    | IT::NamedInstance(_)
            )
        };
        if !ok {
            self.report_error_at(at, StaticTypeError {
                kind: TypeErrorKind::SpreadArgNotIterable {
                    got: ty.to_string(),
                    kw,
                },
                span: None,
            });
        }
    }

    pub(in crate::type_check) fn check_call_args(
        &mut self,
        fname: &str,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) {
        // ⚠ 展開があると位置で対応づけられない（上の doc）。個数・型検査を降りる。
        if Self::has_spread_arg(args) {
            return;
        }
        let sigs = match self.visible_fn_sigs(fname).cloned() {
            Some(s) => s,
            None => return,
        };

        // 可変長引数エントリを分離
        let variadic_entry = arg_data.iter().find(|(k, _)| k.as_deref() == Some("..."));
        // ⚠ 引数の**式**も同じ絞り込みで並べる（B11 の可変性検査に要る）。
        //   `arg_data` と `args` は同じ並びなので、zip してから同じ条件で filter する。
        let normal_args: Vec<_> = arg_data
            .iter()
            .zip(args.iter())
            .filter(|((k, _), _)| k.as_deref() != Some("..."))
            .collect();
        let call_count = normal_args.len();

        let count_matching: Vec<FnSig> = sigs
            .iter()
            .filter(|s| {
                        call_count >= s.required_count
                            && (s.variadic_type.is_some() || call_count <= s.params.len())
                    })
            .cloned()
            .collect();

        if count_matching.is_empty() {
            if sigs.len() == 1 {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgCountMismatch {
                        func_name: fname.to_string(),
                        expected_min: sigs[0].required_count,
                        expected_max: sigs[0].params.len(),
                        got: call_count,
                    },
                    span: None,
                });
            } else {
                let available = sigs.iter().map(|s| s.params.len()).collect();
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoMatchingOverload {
                        func_name: fname.to_string(),
                        got: call_count,
                        available,
                    },
                    span: None,
                });
            }
            return;
        }
        // ⚠ 個数で 1 本に絞れないときは**実引数の型**で絞る（タスク 5.7・検体 `C12`）。
        let count_matching =
            self.narrow_overloads_by_arg_types(fname, count_matching, arg_data, false);
        if count_matching.len() != 1 {
            return;
        }

        let sig = &count_matching[0];
        let mut positional_idx = 0usize;
        for ((key, arg_ty), call_arg) in &normal_args {
            match key {
                Some(kwarg_name) => {
                    match sig.params.iter().position(|(n, _)| n == kwarg_name) {
                        None => self.report_error_at(call_arg.expr(), StaticTypeError {
                            kind: TypeErrorKind::UnknownKeywordArg {
                                func_name: fname.to_string(),
                                arg_name: kwarg_name.clone(),
                            },
                            span: None,
                        }),
                        Some(param_pos) => {
                            self.check_mut_param_arg(fname, sig, param_pos, call_arg.expr());
                            if let Some(expected) = &sig.params[param_pos].1 {
                                // ⚠ `mut` 引数（write-back）は拡大を許さない（`type_matches` の doc）。
                                if !self.param_type_matches(sig, param_pos, arg_ty, expected) {
                                    self.report_error_at(call_arg.expr(), StaticTypeError {
                                        kind: TypeErrorKind::CallArgTypeMismatch {
                                            func_name: fname.to_string(),
                                            param_index: param_pos,
                                            expected: expected.clone(),
                                            got: (*arg_ty).clone(),
                                        },
                                        span: None,
                                    });
                                }
                            }
                        }
                    }
                }
                None => {
                    if let Some((_, Some(expected))) = sig.params.get(positional_idx) {
                        // ⚠ `mut` 引数（write-back）は拡大を許さない（`type_matches` の doc）。
                        if !self.param_type_matches(sig, positional_idx, arg_ty, expected) {
                            self.report_error_at(call_arg.expr(), StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: fname.to_string(),
                                    param_index: positional_idx,
                                    expected: expected.clone(),
                                    got: (*arg_ty).clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    self.check_mut_param_arg(fname, sig, positional_idx, call_arg.expr());
                    positional_idx += 1;
                }
            }
        }

        // 可変長引数の型チェック
        if let (Some((_, IT::ListOf(elem_ty))), Some(expected_elem_ty)) =
            (variadic_entry, &sig.variadic_type)
        {
            // ⚠ これは**要素型同士**の比較で、仮引数への束縛ではない（実行時に
            //    `__cast__` は挟まらない）ので `Site::Other`。
            if !self.types_compatible(elem_ty, expected_elem_ty, Site::Other) {
                self.report_error_at_opt(Self::variadic_arg_expr(args), StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: fname.to_string(),
                        param_index: usize::MAX, // 可変長引数を示す特殊値
                        expected: expected_elem_ty.clone(),
                        got: *elem_ty.clone(),
                    },
                    span: None,
                });
            }
        }

        // Protocol 型パラメータへの引数の適合チェック
        let mut pos_idx = 0usize;
        for ((key, arg_ty), _call_arg) in &normal_args {
            let param_ty_opt = match key {
                Some(kwarg_name) => sig.params.iter().find(|(n, _)| n == kwarg_name).and_then(|(_, t)| t.clone()),
                None => {
                    let t = sig.params.get(pos_idx).and_then(|(_, t)| t.clone());
                    pos_idx += 1;
                    t
                }
            };
            if let Some(InferredType::Protocol(proto_name, proto_args)) = param_ty_opt {
                let context = format!("argument to `{fname}`");
                self.check_protocol_conformance(arg_ty, &proto_name, &proto_args, None, &context);
            }
        }
    }

    /// `mut` パラメータへ `let` の値を渡していないか検査する（bug_fix.md B11）。
    ///
    /// ⚠⚠ 関数の `let` / `mut` は「**関数内で値を書き換えるか**」の宣言であって、
    /// 変数束縛の規則（`let → let` だけ共有）とは**別のルール**。
    /// `mut` パラメータに `mut` 変数を渡して呼び出し元まで書き変わるのは正しい仕様だが、
    /// `let` 変数を渡すのはエラーでなければならない。以前は素通りしており、
    /// `fn g(mut v: list)` に `let a` を渡すと **`let` の `a` が書き換わっていた**（実測）。
    ///
    /// ⚠ 判定は**パスの根**で行う（`o.f` / `xs[0]` も根の属性を継ぐ・規則 1）。
    /// ⚠ 根が識別子でない式（リテラル・呼び出しの戻り値）は**一時値**で誰とも共有して
    /// いないので通す。ここを弾くと `g([1, 2])` のような正しい呼び出しまで落ちる。
    fn check_mut_param_arg(
        &mut self,
        fname: &str,
        sig: &FnSig,
        param_pos: usize,
        arg_expr: &Expr,
    ) {
        if !sig.param_mutable.get(param_pos).copied().unwrap_or(false) {
            return;
        }
        if self.path_is_mutable(arg_expr) != Some(false) {
            return;
        }
        // ⚠ 関数値は書き戻しようがないので弾かない（A7-a。`mut_arg_exempt` の doc 参照）。
        //   ⚠ **`FnTypeParam` 経路と同じ規則にすること**。片方だけ緩めると
        //     「同じ関数が呼び方で通ったり落ちたりする」形が戻る（INF-D で潰した失敗）。
        let arg_ty = self.infer(arg_expr);
        if Self::mut_arg_exempt(&arg_ty) {
            return;
        }
        let param_name = sig.params.get(param_pos).map(|(n, _)| n.clone()).unwrap_or_default();
        self.report_error_at(arg_expr, StaticTypeError {
            kind: TypeErrorKind::CallMutParamWithImmutableArg {
                func_name: fname.to_string(),
                param_name,
            },
            span: None,
        });
    }
}

// ── AST 型解決層（#16）の引数検査指示ヘルパ ──
/// 引数の静的型が「完全に動的（実行時まで型が不明）」か。`Any`/`Unresolved` のとき true。
/// これらは呼び出し前に対象型で検査しないと具象パラメータへ渡せない（境界検査）。
pub(super) fn is_dynamic_arg(t: &InferredType) -> bool {
    matches!(t, InferredType::Any | InferredType::Unresolved)
}

/// パラメータ型が「具象（検査に値する特定の型）」か。`Any`/`Unresolved` 以外なら true。
pub(super) fn is_specific_param(t: &InferredType) -> bool {
    !matches!(t, InferredType::Any | InferredType::Unresolved)
}
