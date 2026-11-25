// stmt/check/functions.rs — 関数・ジェネレータの定義の検査: 仮引数・既定値・戻り値・ブロック式の値。

use super::*;

impl TypeChecker {
    /// 仮引数の**既定値**の型を、その仮引数の宣言型と突き合わせる（0-12）。
    ///
    /// ⚠⚠ 既定値の式は**推論すらされていなかった**。`check_fn_def` は注釈の有無しか見ず
    /// `param.default` に触れず、`declare_param` も無視していたため
    /// `fn f(let n: int = "wrong")` が静的にも実行時にも通っていた（実測）。
    ///
    /// ⚠ 可変長パラメータは既定値を持てない（パーサが弾く）ので対象外。
    fn check_param_defaults(&mut self, func_name: &str, params: &[Param]) {
        for p in params {
            let Some(expr) = &p.default else { continue };
            // ⚠ 推論は**必ず行う**（注釈が無くても）。注釈テーブルへ焼くため。
            let got = self.infer(expr);
            let Some(declared) = p.type_ann.as_deref().and_then(InferredType::from_ann) else {
                continue; // 注釈なし／解釈できない綴り（欠落は別途 `MissingParamTypeAnn`）
            };
            if matches!(declared, InferredType::Unresolved | InferredType::Any)
                || self.mentions_type_param(&declared)
            {
                continue;
            }
            let ctx = format!("default value of `{}` of `{func_name}`", p.name);
            // ⚠ `mut` 引数でも既定値は**呼び元の記憶域ではない**ので拡大を許してよい
            //    （write-back の相手が居ない）。
            // 不一致は既定値の式を指す（3-3）。
            if self.with_anchor_at(expr, |c| c.check_expected(&got, &declared, false, &ctx)) {
                continue;
            }
            self.report_error_at(expr, StaticTypeError {
                kind: TypeErrorKind::ParamDefaultTypeMismatch {
                    func_name: func_name.to_string(),
                    param_name: p.name.clone(),
                    expected: declared,
                    got,
                },
                span: None,
            });
        }
    }

    /// `return <値>` の型を、宣言された戻り値型と突き合わせる（0-3）。
    ///
    /// ⚠ 宣言が無い / 解決できない関数では何もしない。戻り値注釈そのものが無い場合は
    /// `MissingReturnTypeAnn` が別途出るので、ここで二重に鳴らさない。
    pub(super) fn check_return_type(&mut self, got: &InferredType) {
        let Some(expected) = self.state.current_fn_return().cloned() else {
            return;
        };
        if matches!(expected, InferredType::Unresolved | InferredType::Any) {
            return;
        }
        // ⚠ テンプレート型変数を含む戻り値型は検査しない（`mentions_type_param` の doc）。
        if self.mentions_type_param(&expected) {
            return;
        }
        let func_name = self.state.current_fn().unwrap_or("<fn>").to_string();
        // ⚠ 単相化したクラス（`Box[int]`）のメソッドは**どの具体化か**を名乗る（タスク 2-8 段階 2）。
        //   同じテンプレートの具体化ごとに合否が違う（`Box[int].half` だけが誤り）ので、
        //   メソッド名だけでは直す場所が分からない。⚠ `current_fn` そのものは変えない
        //   （`__init__` の判定に使われている）。
        let func_name = match self.state.current_class() {
            Some(cls) if cls.contains('[') => format!("{cls}.{func_name}"),
            _ => func_name,
        };
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("return value of `{func_name}`");
        if self.check_expected(got, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::ReturnTypeMismatch {
                func_name,
                expected,
                got: got.clone(),
            },
            span: None,
        });
    }

    /// `block_return` / `loop_yield` / `yield` の値を、囲み構文が宣言した型と照合する
    /// （タスク 5.2・検体 `X1`〜`X4` / `C8`）。
    ///
    /// ⚠⚠ **3 つとも「値を囲み構文へ渡す」同じ義務**なので 1 本にした。地点ごとに
    /// 書き分けると、`block_return` だけ直して `loop_yield` が漏れる形になる
    /// （実際に 3 地点とも別々に無検査だった）。
    ///
    /// `expected` が `None` のとき（注釈なし・解決できない注釈・囲みが無い）は照合しない。
    pub(super) fn check_block_expr_value(
        &mut self,
        got: &InferredType,
        expected: Option<InferredType>,
        keyword: &'static str,
        span: Option<Span>,
    ) {
        let Some(expected) = expected else { return };
        // ⚠ テンプレート型変数を含む型は具体型と突き合わせられない（`mentions_type_param`）。
        if self.mentions_type_param(&expected) {
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("value of `{keyword}`");
        if self.check_expected(got, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::BlockExprValueMismatch {
                keyword: keyword.to_string(),
                expected,
                got: got.clone(),
            },
            span,
        });
    }

    /// 関数定義を型検査する。パラメータ・戻り値の注釈欠落や交差型を診断し、
    /// パラメータをスコープに束縛して本体を検査する。
    pub(super) fn check_fn_def(
        &mut self,
        name: &str,
        params: &[Param],
        return_type: Option<&str>,
        body: &[Stmt],
        decorators: &[Expr],
        template_params: &[crate::ast::TemplateParam],
    ) {
        for dec in decorators {
            // 装飾子の誤りは装飾子の式を指す（diagnostic_position_plan.md の 3-4）。
            self.with_anchor_at(dec, |c| c.check_decorator(dec, true, name));
        }
        for param in params.iter() {
            if param.name == "self" || param.variadic {
                continue;
            }
            if param.type_ann.is_none() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::MissingParamTypeAnn {
                        func_name: name.to_string(),
                        param_name: param.name.clone(),
                    },
                    span: None,
                });
            }
        }
        match return_type {
            None => self.report_error(StaticTypeError {
                kind: TypeErrorKind::MissingReturnTypeAnn { func_name: name.to_string() },
                span: None,
            }),
            Some(rt) if self.registry.is_protocol(rt) => {
                self.report_warning(StaticTypeWarning {
                    kind: TypeWarningKind::ProtocolReturnType {
                        func_name: name.to_string(),
                        protocol_name: rt.to_string(),
                    },
                    span: None,
                });
            }
            Some(_) => {}
        }
        // 交差型を含む関数は部分コンパイルできないため警告を出す
        let has_intersection_type = params.iter().any(|p| {
            p.type_ann.as_deref()
                .and_then(InferredType::from_ann)
                .is_some_and(|ty| matches!(ty, InferredType::Intersection(_)))
        }) || return_type
            .and_then(InferredType::from_ann)
            .is_some_and(|ty| matches!(ty, InferredType::Intersection(_)));
        if has_intersection_type {
            self.report_warning(StaticTypeWarning {
                kind: TypeWarningKind::IntersectionSkippedCompile { func_name: name.to_string() },
                span: None,
            });
        }
        // ⚠⚠ **関数名は関数値としての型で宣言する**（タスク 2.2）。
        //    以前は `Unresolved` で宣言していたため、`fn` を名前で参照した値の型が
        //    `Unresolved` になり、`type_matches_exact` の万能受容体として
        //      let x: int = wrong                       # int 変数に関数が入る
        //      let f: function[int]->int = takes_str    # シグネチャ違いが通る
        //    が黙って通っていた。引数・戻り値は `fn_sigs` に揃っている。
        //    ⚠ `fn_value_type` が `None` を返す場合（オーバーロード・テンプレート関数）は
        //    従来どおり `Unresolved`。関数値としての型が 1 つに決まらないため。
        // ⚠ メソッド（クラス・trait の本体の直下）は宣言しない（1-3・`Stmt::Field` の腕と同じ理由）。
        if let Some(i) = self.group_predeclared.iter().position(|n| n == name) {
            // 組の名前として先に宣言済み（`predeclare_fn_group`・3-2）。
            self.group_predeclared.remove(i);
        } else if !self.state.in_class_body() {
            // 入れ子の関数のシグネチャは、ここでこのスコープへ積む（name_scope_plan.md の 1-7・`CheckState::declare_local_fn`）。
            if self.state.in_deferred_body() && template_params.is_empty() {
                let ret = return_type
                    .and_then(InferredType::from_ann)
                    .map(|t| self.registry.resolve_protocol_ann(t));
                let sig = self.registry.sig_of(params, ret);
                self.state.declare_local_fn(name.to_string(), sig);
            }
            let self_ty = self.fn_value_type(name).unwrap_or(InferredType::Unresolved);
            self.declare(name.to_string(), self_ty, false);
        }
        self.push_scope();
        // ⚠ 関数自身の型変数（`fn f[T]`）を積む。囲みクラスの型変数は `ClassDef` 側が
        //    既に積んでいるので、ここでは追加するだけでよい。
        let saved_tp =
            self.state.push_type_params(template_params.iter().map(|p| p.name.clone()));
        // ⚠⚠ **注釈の妥当性検査は型変数を積んだ後で行う**（タスク 8.1 / 8.5）。
        //    `is_type_param` は**スコープ状態**なので、積む前に見ると `fn f[T](x: T)` の
        //    `T` を「存在しない型名」と誤判定する（8.5 の計測で実測した）。
        for param in params.iter() {
            if let Some(ann) = param.type_ann.as_deref() {
                self.check_ann_not_bare(
                    ann,
                    &format!("parameter `{}` of `{name}`", param.name),
                );
            }
        }
        if let Some(rt) = return_type {
            self.check_ann_not_bare(rt, &format!("return type of `{name}`"));
        }
        // ⚠ **既定値の検査は仮引数を宣言する前**に行う。既定値は他の仮引数を参照できない
        //    （参照できてしまうと評価順に依存する）ので、まだ見えていない状態で推論する。
        // ⚠ 型変数は既に積んであるので `mentions_type_param` が効く（`fn g[T](let n: T = …)`）。
        self.check_param_defaults(name, params);
        for param in params {
            self.declare_param(param);
        }
        // ⚠ 戻り値型は**注釈から**取る（0-3）。`current_fn_name` でレジストリを引くと
        //    オーバーロード・メソッド・入れ子関数で一意に定まらない。
        //    `Self` は現在のクラスへ解決しておく（`return Self(...)` を通すため）。
        let declared_ret = return_type
            .and_then(InferredType::from_ann)
            .map(|t| self.resolve_self_type(t));
        let prev_fn = self.state.enter_fn(name.to_string(), declared_ret);
        // ⚠⚠ **継承しない**。`gen` の中の入れ子 `fn` もここを通るので、
        //    false へ張り替えることでそこの `yield` もエラーになる。
        let prev_gen = self.state.enter_gen_body(false);
        // ⚠ ブロック式の照合先も**継承しない**（`with_fn_body` の doc・タスク 5.2）。
        self.state.enter_deferred_body();
        self.with_fn_body(|c| c.check_stmts(body));
        self.state.exit_deferred_body();
        self.state.exit_gen_body(prev_gen);
        self.state.exit_fn(prev_fn);
        self.state.pop_type_params(saved_tp);
        self.pop_scope();
    }

    /// ジェネレータ関数定義を型検査する。
    pub(super) fn check_gen_def(
        &mut self,
        name: &str,
        template_params: &[crate::ast::TemplateParam],
        params: &[Param],
        yield_type: Option<&str>,
        body: &[Stmt],
    ) {
        for param in params.iter() {
            if param.name == "self" || param.variadic {
                continue;
            }
            if param.type_ann.is_none() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::MissingParamTypeAnn {
                        func_name: name.to_string(),
                        param_name: param.name.clone(),
                    },
                    span: None,
                });
            }
        }
        if yield_type.is_none() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::MissingReturnTypeAnn { func_name: name.to_string() },
                span: None,
            });
        }
        // ⚠ `gen` の名前も関数値としての型で宣言する（`check_fn_def` と同じ・フェーズ10 10-7）。
        //   結果の型は `generator[T]`。以前は `Unresolved` だったので、入れ子の `gen` を呼んだ結果に
        //   型が付かなかった。
        // ⚠ メソッド（クラス・trait の本体の直下）は宣言しない（1-3・`Stmt::Field` の腕と同じ理由）。
        if let Some(i) = self.group_predeclared.iter().position(|n| n == name) {
            // 組の名前として先に宣言済み（`predeclare_fn_group`・3-2）。
            self.group_predeclared.remove(i);
        } else if !self.state.in_class_body() {
            // 入れ子の `gen` も入れ子の関数と同じ（1-7）。結果の型はジェネレータ（`registry/builder.rs` の `GenDef` と同じ）。
            if self.state.in_deferred_body() && template_params.is_empty() {
                let ret = yield_type
                    .and_then(InferredType::from_ann)
                    .map(|t| InferredType::IteratorOf(Box::new(self.registry.resolve_protocol_ann(t))));
                let sig = self.registry.sig_of(params, ret);
                self.state.declare_local_fn(name.to_string(), sig);
            }
            let self_ty = self.fn_value_type(name).unwrap_or(InferredType::Unresolved);
            self.declare(name.to_string(), self_ty, false);
        }
        self.push_scope();
        // ⚠⚠ **`gen` 自身の型変数（`gen take[T]`）を積んでから注釈を検査する**（フェーズ10 10-1）。
        //    以前は「`gen` には型変数の投入が無いので、ここで検査してよい」と書いてあったが誤りで、
        //    **投入が漏れていた**（タスク 8.5 の退行）。`gen take[T](xs: list[T])` が
        //    「`T` は知らない型」と誤って弾かれた。囲みクラスの型変数は `ClassDef` 側が積んでいる。
        let saved_tp =
            self.state.push_type_params(template_params.iter().map(|p| p.name.clone()));
        for param in params.iter() {
            if let Some(ann) = param.type_ann.as_deref() {
                self.check_ann_not_bare(
                    ann,
                    &format!("parameter `{}` of gen `{name}`", param.name),
                );
            }
        }
        if let Some(yt) = yield_type {
            self.check_ann_not_bare(yt, &format!("yield type of gen `{name}`"));
        }
        // ⚠⚠ 仮引数は `fn` と同じく宣言する（フェーズ10 10-7）。以前は注釈から型を取るだけで、
        //    `gen` メソッドの `self` が `Unresolved`（何でも受ける）になり、本体の `self.x` を
        //    何も検査していなかった（`let s: str = self.n` も、存在しない `self.nope` も通った・実測）。
        //    既定値の検査も `fn` と同じく宣言の前に行う。
        self.check_param_defaults(name, params);
        for param in params {
            self.declare_param_in(param, true);
        }
        // `gen` 本体の直下だけが `yield` を書ける（B13）。
        let prev_gen = self.state.enter_gen_body(true);
        // ⚠ `gen` の `->T` は**要素型**（`yield` 1 回分の型）。`check_return_type` が
        //    使う `current_fn_return` と同じ場所へ入れて `yield` の照合に使う（タスク 5.2）。
        let prev_fn = self.state.enter_fn(
            name.to_string(),
            yield_type.and_then(InferredType::from_ann),
        );
        self.state.enter_deferred_body();
        self.with_fn_body(|c| c.check_stmts(body));
        self.state.exit_deferred_body();
        self.state.exit_fn(prev_fn);
        self.state.exit_gen_body(prev_gen);
        self.state.pop_type_params(saved_tp);
        self.pop_scope();
    }

    /// 関数パラメータ1つをスコープに束縛する（`self`・可変長・通常引数を区別）。
    fn declare_param(&mut self, param: &Param) {
        self.declare_param_in(param, false);
    }

    /// [`Self::declare_param`] の本体。`in_gen` はジェネレータ関数の引数か。
    ///
    /// ⚠ ジェネレータ関数の引数は Python の物でも `let` で受けられる（`mut` にできない・`gen` の引数は `let` / `const` だけ。
    ///   pandas_import_plan.md の 2-13・D-3 の例外・2026-11-05 の決定）。ほかの引数は `mut` で受ける（D-3）。
    fn declare_param_in(&mut self, param: &Param, in_gen: bool) {
        if param.variadic {
            // 可変長パラメータ: local::args として Optional[list[T]] を宣言
            let elem_ty = param
                .type_ann
                .as_deref()
                .and_then(InferredType::from_ann)
                .unwrap_or(InferredType::Any);
            let local_args_ty = InferredType::Union(vec![
                InferredType::ListOf(Box::new(elem_ty)),
                InferredType::None,
            ]);
            self.declare("local::args".to_string(), local_args_ty, param.mutable);
            return;
        }
        let ty = if param.name == "self" {
            self.state
                .current_class()
                .map(Self::instance_type_of)
                .unwrap_or(InferredType::Unresolved)
        } else {
            param
                .type_ann
                .as_deref()
                .and_then(InferredType::from_ann)
                .unwrap_or(InferredType::Unresolved)
        };
        // Python のオブジェクトを受ける引数は `mut` で書く（D-3・1-2）。⚠ `self` は対象外（受け手の可変性は別の規則）。
        if !param.mutable && param.name != "self" && !in_gen {
            self.require_mut_for_python_object(&param.name, "parameter", &ty);
        }
        self.declare(param.name.clone(), ty, param.mutable);
    }
}
