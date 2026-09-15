use crate::ast::{CallArg, Expr};
use crate::type_check::types::InferredType as IT;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::{FnSig, FnTypeParam, InferredType};
use super::type_utils::Site;
use super::TypeChecker;

impl TypeChecker {
    /// 関数呼び出し式の型を推論し、引数の型・個数・Self 型パラメータを検査する。
    pub(super) fn infer_call(
        &mut self,
        func: &Expr,
        args: &[CallArg],
        node_id: u32,
    ) -> InferredType {
        // __freeze__ may only be invoked via the `freeze` keyword, never as a direct call.
        let freeze_span = match func {
            Expr::Attr { attr, span, .. } if attr == "__freeze__" => Some(span.clone()),
            Expr::Ident { name, .. } if name == "__freeze__" => None,
            _ => {
                // Not a __freeze__ call — proceed normally.
                #[allow(clippy::needless_return)]
                return self.infer_call_inner(func, args, node_id);
            }
        };
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::DirectFreezeCall,
            span: freeze_span,
        });
        InferredType::Unresolved
    }

    fn infer_call_inner(&mut self, func: &Expr, args: &[CallArg], node_id: u32) -> InferredType {
        // ── Step 1: method-call detection ──────────────────────────────────────
        // Infer the object type exactly once here. This handles:
        //   - NamedInstance: regular instance method calls (obj.method())
        //   - TypeValOf(NamedInstance): class method calls (ClassName.method())
        //   - Non-Ident objects: method chains (Builder(1).set(5))
        // When method_call_info is Some, we skip self.infer(func) later to avoid
        // double-evaluating the object and duplicating error reports.
        // 組み込みコレクションのレシーバ（型・メソッド名・位置）。`arg_data` が揃ってから使う。
        let mut builtin_recv: Option<(InferredType, String, crate::token::Span)> = None;
        let method_call_info: Option<(String, String)> =
            if let Expr::Attr { object, attr, span, .. } = func {
                let obj_ty = self.infer(object);
                self.check_mutating_method_receiver(object, attr, &obj_ty, span);
                // ⚠ 組み込みコレクションメソッドの引数型は `arg_data` が揃ってから検査する
                //    （タスク 5.2c・検体 `L14`）。レシーバの型をここで控えておく。
                builtin_recv = Some((obj_ty.clone(), attr.clone(), span.clone()));

                // Result[T, E] の is_OK() / is_ERR() は特別扱いして bool を返す。
                // 他のメソッドやアトリビュートアクセスは OperationOnUnion エラーを発生させる。
                if let IT::Result(_, _) = &obj_ty {
                    if (attr == "is_OK" || attr == "is_ERR") && args.is_empty() {
                        return InferredType::Bool;
                    } else {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::OperationOnUnion {
                                union_type: obj_ty.to_string(),
                                op: format!("method/attribute `{attr}`"),
                            },
                            span: Some(span.clone()),
                        });
                        return InferredType::Unresolved;
                    }
                }

                let cls_name_opt: Option<String> = match &obj_ty {
                    InferredType::NamedInstance(cls) => Some(cls.clone()),
                    InferredType::TypeValOf(inner) => {
                        if let InferredType::NamedInstance(cls) = inner.as_ref() {
                            Some(cls.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(cls_name) = cls_name_opt {
                    let is_static = self.registry.is_static_method(&cls_name, attr.as_str());
                    // StaticMethodOnInstance: only report when called on an instance
                    if is_static && matches!(obj_ty, InferredType::NamedInstance(_)) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::StaticMethodOnInstance {
                                method_name: attr.clone(),
                                class_name: cls_name.clone(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    // Member accessibility check (same as infer(Attr) does for NamedInstance)
                    if matches!(obj_ty, InferredType::NamedInstance(_)) {
                        self.check_member_access_static(&cls_name, attr, Some(span.clone()));
                    }
                    Some((cls_name, attr.clone()))
                } else {
                    None
                }
            } else {
                None
            };

        let func_name = match func {
            Expr::Ident { name, .. } => Some(name.clone()),
            Expr::Attr { attr, .. } => Some(attr.clone()),
            _ => None,
        };

        // Only evaluate func type when not a method call. For method calls the object
        // was already inferred above; re-calling self.infer(func) would re-evaluate
        // the object expression and potentially duplicate errors.
        let func_type = if method_call_info.is_none() {
            self.infer(func)
        } else {
            InferredType::Unresolved
        };
        // ⚠ 呼び出せない型を弾く（タスク 7.1・検体 `C13`）。
        self.check_callable(&func_type);

        let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
        for arg in args.iter() {
            match arg {
                CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                CallArg::Keyword { name, value } => {
                    arg_data.push((Some(name.clone()), self.infer(value)))
                }
                // 可変長引数: 各要素の型を推論し、リスト型として "..." キーで登録
                //
                // ⚠⚠ **以前は「全要素が同じ型のときだけ `list[T]`、混ざったら素の `list`」**
                //    だった。素の `list` は下流の可変長検査
                //    （`Some((_, IT::ListOf(elem_ty)))` で受ける）に**当たらない**ので、
                //    `f(... = 1, "s")` のように**型が混ざった瞬間に検査が消えて**いた
                //    （全部 `str` なら捕まるのに 1 つ混ぜると通る・実測。検体 `C15`）。
                //    ⇒ 根本原因②「要素型を捨てると何とでも適合する」がここに残っていた。
                // ⚠ 合成はコレクションリテラルと同じ `join_elem_types`（タスク 2.7）を使う。
                CallArg::Variadic(exprs) => {
                    let elem_types: Vec<InferredType> =
                        exprs.iter().map(|e| self.infer(e)).collect();
                    let list_ty = if elem_types.is_empty() {
                        InferredType::List
                    } else {
                        InferredType::ListOf(Box::new(Self::join_elem_types(elem_types)))
                    };
                    arg_data.push((Some("...".to_string()), list_ty));
                }
            }
        }

        // ⚠ `len()` の引数は「大きさを持つ型」（タスク 7.4）。
        if let Expr::Ident { name, .. } = func {
            if name == "len" && self.lookup("len").is_some() {
                self.check_len_argument(&arg_data);
            }
        }

        // ⚠ `new_type` のコンストラクタ引数（タスク 5.6・検体 `T3`）。
        if let Expr::Ident { name, .. } = func {
            if let Some(original) = self.registry.new_type_original(name).map(str::to_string) {
                self.check_new_type_arg(name, &original, &arg_data);
            }
        }

        // ⚠ 組み込みコレクションメソッドの引数型（タスク 5.2c・検体 `L14`）。
        if let Some((recv_ty, method, span)) = builtin_recv {
            self.check_builtin_collection_method_args(&recv_ty, &method, &arg_data, &span);
        }

        // ── AST 型解決層（#16）── Call 構造化注釈を焼く（arg_data・func_name はここで確定済み）。
        // 呼び先＝シンボル参照（名前）、引数＝(型, 検査指示)。
        // 検査指示（境界検査・点4 の畳み込み）: **直接関数呼び出し（単一シグネチャ・全引数が位置引数）**で、
        // param が具象・arg が動的（`Any`/`Unresolved`）のとき `CheckBefore(param型)`。それ以外は保守的に `None`
        // （overload・メソッド・キーワード/可変長・関数型変数などは次段で精緻化）。
        {
            // パラメータ型の取得（全引数が位置引数のときのみ・単一シグネチャに限る）。
            // overload・キーワード/可変長・関数{params:None}・静的メソッドは静的に param 型が一意でないため None。
            let positional_only = arg_data.iter().all(|(k, _)| k.is_none());
            let param_types: Option<Vec<Option<InferredType>>> = if !positional_only {
                None
            } else if let Some((cls, method)) = &method_call_info {
                // インスタンスメソッド呼び出し: 単一シグネチャ・非 static のとき self(先頭)を除いて対応付け。
                if self.registry.is_static_method(cls, method) {
                    None
                } else {
                    self.registry.class_methods(cls).and_then(|m| m.get(method)).and_then(|sigs| {
                        (sigs.len() == 1).then(|| {
                            sigs[0].params.iter().skip(1).map(|(_, t)| t.clone()).collect()
                        })
                    })
                }
            } else if let IT::Function { params: Some(fn_params), .. } = &func_type {
                // 関数型変数/パラメータ: シグネチャの各 param 型を使う。
                Some(fn_params.iter().map(|p| Some(p.ty.clone())).collect())
            } else if let Expr::Ident { name, .. } = func {
                // 直接グローバル関数呼び出し（単一シグネチャ）。
                self.registry.fn_sigs(name).and_then(|sigs| {
                    (sigs.len() == 1)
                        .then(|| sigs[0].params.iter().map(|(_, t)| t.clone()).collect())
                })
            } else {
                None
            };

            let mut args_ann = Vec::with_capacity(arg_data.len());
            for (i, (_, arg_ty)) in arg_data.iter().enumerate() {
                let tid = self.annotations.intern(arg_ty.clone());
                let directive = match param_types.as_ref().and_then(|p| p.get(i)) {
                    Some(Some(param_ty))
                        if is_specific_param(param_ty) && is_dynamic_arg(arg_ty) =>
                    {
                        let ptid = self.annotations.intern(param_ty.clone());
                        super::annotations::Directive::CheckBefore(ptid)
                    }
                    _ => super::annotations::Directive::None,
                };
                args_ann.push(super::annotations::ArgAnnotation { ty: tid, directive });
            }
            self.annotations.set_call(
                node_id,
                super::annotations::CallInfo {
                    callee: func_name.clone(),
                    args: args_ann,
                },
            );
        }

        // ── テンプレート実体化呼び出し（`f[int](…)` / `Box[int](…)`）────────────
        // ⚠ `func_name` は `Expr::Ident` / `Expr::Attr` しか拾わないので、この形は
        //    下の経路すべてを素通りする。型引数で置換したシグネチャと突き合わせる。
        // ⚠ 戻り値型は従来どおり `Unresolved` のまま返す。`NamedInstance(base)` に
        //    変えるとフィールド型が `T`（= 呼び出し点では未知のクラス名）として下流へ
        //    流れ、`a.v = "s"` などが偽エラーになる（型変数の実体化は別タスク）。
        if let Expr::TemplateInstantiate { base, type_args } = func {
            if let Expr::Ident { name, .. } = base.as_ref() {
                self.check_template_call_args(name, type_args, &arg_data);
                // ⚠ タスク 2.1: 結果型を返す（以前は常に `Unresolved` を捨て返していた）。
                return self.template_call_result_type(name, type_args);
            }
            return InferredType::Unresolved;
        }

        // ⚠⚠ **直接の関数名呼び出しは `check_call_args` に残す**（タスク 2.2）。
        //    2.2 で関数名に `Function` 型を付けたため、`f(1)` のような**直接呼び出し**が
        //    下の `Function` アーム（= 関数**値**用の `check_fn_type_call`）へ迂回し、
        //    可変長引数が `takes 0 argument(s)` になる／`mut` 引数の判定が変わる等で
        //    **13 例題が壊れた**（実測）。`check_fn_type_call` は「シグネチャだけ判っている
        //    関数値」（import したモジュールのメンバ等）用で、`check_call_args` の方が
        //    可変長・既定値・`mut` 引数・オーバーロードを正しく扱う。
        //    ⇒ 名前が `fn_sigs` に在るなら従来どおり下の `check_call_args` へ落とす。
        let direct_fn_call = matches!(func, Expr::Ident { .. })
            && func_name
                .as_deref()
                .is_some_and(|n| self.registry.fn_sigs(n).is_some());
        if !direct_fn_call {
        match func_type {
            InferredType::Function {
                params: Some(fn_params),
                return_type,
            } => {
                let fname = func_name.as_deref().unwrap_or("<function>").to_string();
                let ret = *return_type;
                self.check_fn_type_call(&fname, args, &arg_data, &fn_params);
                return ret;
            }
            InferredType::Function { params: None, return_type } => {
                return *return_type;
            }
            // ── 型値の呼び出し＝変換（タスク 2.4 / 決定 D-6）────────────────────
            //
            // ⚠⚠ **組み込み関数に静的な戻り値型が 1 つも無かった。**
            //      let s: str = float(n)    # 通っていた（結果が `Unresolved` だったため）
            //      let s: str = int("3")    # 同じ
            //    `Unresolved` は `type_matches_exact` の万能受容体なので、変換を経由した
            //    全ての義務が無効化されていた。これは **D-5（暗黙 `int → float` の廃止）の
            //    前提**で、受け皿の `float(n)` が無検査だと穴が閉じずに移動するだけになる。
            //
            // ⚠ **名前を新しく占有しない形で解く。** 既存の `builtin_fns` 機構に
            //    `len` 等を足す案は「グローバル名を占有して `let len = ...` が
            //    already declared になる」ため前任者が見送っており、実際に例題が
            //    `len` を変数名に使っている（実測 3 箇所）。
            //    ⇒ `int` / `float` / `str` / `bool` は**既に `TypeValOf` として登録済み**なので、
            //      「**型値を呼ぶとその型になる**」という一般規則を入れれば足りる。
            //      クラスの `C(..)` は別経路（`NamedInstance` を返す）。
            // ⚠ プリミティブに限る。`TypeValOf(NamedInstance(..))` はクラス・enum・protocol で、
            //    呼び出しの意味がそれぞれ違うので触らない。
            InferredType::TypeValOf(ref inner) => {
                if matches!(
                    **inner,
                    InferredType::Int
                        | InferredType::Float
                        | InferredType::Str
                        | InferredType::Bool
                        | InferredType::Complex
                ) {
                    return (**inner).clone();
                }
            }
            _ => {}
        }
        }

        if let Some((ref cls_name, ref method_name)) = method_call_info {
            // self/cls is passed implicitly; check_self_type_params accounts for the +1
            let ret_ty = self.check_self_type_params(cls_name, method_name, &arg_data);
            return ret_ty.unwrap_or(InferredType::Unresolved);
        } else if let Some(ref fname) = func_name {
            // ⚠⚠ **テンプレートを型引数なしで呼んだ場合はここで打ち切る。**
            //    Arrow に暗黙実体化は無く、実行時も
            //    `TemplateError: template must be called with explicit type arguments` になる。
            //    打ち切らないとシグネチャの型変数（`T`）が具体型と突き合わされて
            //    `argument 0 of 'Box.__init__' expects 'T' but got 'int'` という
            //    **型変数を漏らした読めないメッセージ**になる（実測）。
            if self
                .registry
                .template_params(fname)
                .is_some_and(|p| !p.is_empty())
            {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::TemplateMissingTypeArgs { name: fname.clone() },
                    span: None,
                });
                return InferredType::Unresolved;
            }
            self.check_call_args(fname, &arg_data, args);
        }

        if let Some(ref fname) = func_name {
            if self.registry.is_protocol(fname.as_str()) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::ProtocolInstantiation {
                        protocol_name: fname.clone(),
                    },
                    span: None,
                });
                return InferredType::Protocol(fname.clone());
            }
            if self.registry.is_known_class(fname.as_str()) {
                // ── コンストラクタ引数の検査（0-5）─────────────────────────
                // `C(...)` は上の `check_call_args` では**必ず素通りする**。
                // あれは `fn_sigs` を引くが、クラス名は関数として登録されていないため。
                //
                // ⚠ 自前で仮引数列を組み立てないこと。**パーサが `__init__` を自動生成
                //    している**（`generate_auto_init_if_needed`。並びは
                //    `fn __init__(mut self, trait_field..., class_field...)`）ので、
                //    それは既に `class_method_sigs` に載っている。メソッド呼び出しと
                //    同じ経路へ流せば、並び（trait 由来が先・own が後）も自動的に
                //    実行時の `build_field_index` と一致する。
                // ⚠ `__init__` を持たないクラス（`new_type` ラッパ・組み込み例外）では
                //    `check_self_type_params` が早期 `None` を返すので何も報告されない。
                //
                // ⚠⚠ **外部言語のクラスは対象外**（#27-a の `arrow_class_names`）。
                //    C# スタブは `__init__` を**引数 0 個**で持つが、実行時の生成は
                //    `__cs_bridge_path__` 経由のブリッジ側コンストラクタが行うので、
                //    スタブの `__init__` と実引数は対応しない。除外しないと
                //    `cs_interop_test.ar` が `'Calculator.__init__' takes 0 argument(s)
                //    but 1 were given` で落ちる（実測）。
                if self.registry.arrow_class_names().contains(fname.as_str()) {
                    self.check_self_type_params(fname, "__init__", &arg_data);
                }
                return InferredType::NamedInstance(fname.clone());
            }
        }

        func_name
            .as_deref()
            .and_then(|n| self.registry.fn_sigs(n))
            .and_then(|sigs| {
                let call_count = arg_data
                    .iter()
                    .filter(|(k, _)| k.as_deref() != Some("..."))
                    .count();
                let matching: Vec<_> = sigs
                    .iter()
                    .filter(|s| {
                        call_count >= s.required_count
                            && (s.variadic_type.is_some() || call_count <= s.params.len())
                    })
                    .collect();
                if matching.len() == 1 {
                    matching[0].return_type.clone()
                } else {
                    None
                }
            })
            .unwrap_or(InferredType::Unresolved)
    }

    /// メソッド呼び出しの引数を検査し、メソッドの戻り値型を返す。
    ///
    /// `self`/`cls` は呼び出し元が暗黙的に渡すため、`effective_count = arg_data.len() + 1`
    /// で実際のパラメータ数と照合する。
    pub(super) fn check_self_type_params(
        &mut self,
        cls_name: &str,
        method_name: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        let sigs = self
            .registry
            .class_methods(cls_name)
            .and_then(|m| m.get(method_name))
            .cloned()?;
        // Static methods have no implicit receiver; instance/class methods add +1 for self/cls
        let is_static = self.registry.is_static_method(cls_name, method_name);
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
                self.report_error(StaticTypeError {
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
        for (key, arg_ty) in normal_args.iter() {
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
                self.report_error(StaticTypeError {
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
                        self.report_error(StaticTypeError {
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

    /// 名前付き関数呼び出しの引数個数・型・キーワード引数名を検査する。
    /// ⚠ `args`（引数の**式**）を受け取るのは、`mut` パラメータへ `let` の値を渡していないかを
    /// 見るため（bug_fix.md B11）。型だけでは判定できない — **可変性は束縛の属性であって型ではない**。
    pub(super) fn check_call_args(
        &mut self,
        fname: &str,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
    ) {
        let sigs = match self.registry.fn_sigs(fname).cloned() {
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
                        None => self.report_error(StaticTypeError {
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
                                    self.report_error(StaticTypeError {
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
                            self.report_error(StaticTypeError {
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
                self.report_error(StaticTypeError {
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
            if let Some(InferredType::Protocol(proto_name)) = param_ty_opt {
                let context = format!("argument to `{fname}`");
                self.check_protocol_conformance(arg_ty, &proto_name, None, &context);
            }
        }
    }

    /// 型の中の**テンプレート型変数を具体型へ置換**する（`T` → `int`、`list[T]` → `list[int]`）。
    ///
    /// 実体化呼び出し `Box[int]("s")` の引数を検査するには、シグネチャに書かれた `T` を
    /// 呼び出し点の型引数へ写す必要がある。置換表に無い名前はそのまま残す。
    pub(super) fn subst_type_params(
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
            T::ListOf(t) => T::ListOf(rec(t)),
            T::FixedListOf(t) => T::FixedListOf(rec(t)),
            T::ListLikeOf(t) => T::ListLikeOf(rec(t)),
            T::SetOf(t) => T::SetOf(rec(t)),
            T::TypeValOf(t) => T::TypeValOf(rec(t)),
            T::DictOf(k, v) => T::DictOf(rec(k), rec(v)),
            T::Result(a, b) => T::Result(rec(a), rec(b)),
            T::Union(ts) => T::Union(ts.iter().map(|t| Self::subst_type_params(t, map)).collect()),
            T::Intersection(ts) => {
                T::Intersection(ts.iter().map(|t| Self::subst_type_params(t, map)).collect())
            }
            T::Tuple(ts) => T::Tuple(ts.iter().map(|t| Self::subst_type_params(t, map)).collect()),
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
    fn template_call_result_type(&self, base_name: &str, type_args: &[String]) -> InferredType {
        let Some(tparams) = self.registry.template_params(base_name) else {
            return InferredType::Unresolved; // テンプレートでない名前
        };
        if tparams.len() != type_args.len() {
            return InferredType::Unresolved; // 個数不一致 → 実行時の TemplateError に任せる
        }
        let mut args = Vec::with_capacity(type_args.len());
        let mut map = std::collections::HashMap::new();
        for (p, a) in tparams.iter().zip(type_args.iter()) {
            match InferredType::from_ann(a) {
                Some(t) if !matches!(t, InferredType::Unresolved) => {
                    map.insert(p.clone(), t.clone());
                    args.push(t);
                }
                // 未知の綴りが 1 つでも混ざったら全体を諦める（嘘の型を作らない）
                _ => return InferredType::Unresolved,
            }
        }
        if self.registry.is_known_class(base_name) {
            // テンプレートクラスの実体化 → そのクラスのインスタンス
            InferredType::GenericInstance { name: base_name.to_string(), args }
        } else {
            // テンプレート関数 → 宣言戻り値型を型引数で置換する
            match self.registry.fn_sigs(base_name) {
                // ⚠ オーバーロードは実引数で決まるのでここでは決めない
                Some(sigs) if sigs.len() == 1 => match &sigs[0].return_type {
                    Some(rt) => Self::subst_type_params(rt, &map),
                    None => InferredType::Unresolved,
                },
                _ => InferredType::Unresolved,
            }
        }
    }

    fn check_template_call_args(
        &mut self,
        base_name: &str,
        type_args: &[String],
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let Some(tparams) = self.registry.template_params(base_name) else {
            return; // テンプレートでない名前（実行時に別のエラーになる）
        };
        if tparams.len() != type_args.len() {
            return; // 型引数の個数不一致 → 実行時の TemplateError に任せる
        }
        // 置換表。解釈できない型引数（未知の綴り）が混ざったら検査を諦める。
        let mut map = std::collections::HashMap::new();
        for (p, a) in tparams.iter().zip(type_args.iter()) {
            match InferredType::from_ann(a) {
                Some(t) if !matches!(t, InferredType::Unresolved) => {
                    map.insert(p.clone(), t);
                }
                _ => return,
            }
        }
        // ⚠ 実引数の個数が合わないときは対応付けが嘘になるので見送る（実行時に捕まる）。
        let normal: Vec<_> = arg_data
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();
        // シグネチャを引く。クラスなら自動生成された `__init__`（`self` が先頭）。
        let (candidates, implicit) = if self.registry.is_known_class(base_name) {
            match self
                .registry
                .class_methods(base_name)
                .and_then(|m| m.get("__init__"))
            {
                Some(sigs) => (sigs.clone(), 1usize),
                None => return, // `__init__` が無い（`new_type` ラッパ等）
            }
        } else {
            match self.registry.fn_sigs(base_name) {
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
        for (key, arg_ty) in normal {
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
            let expected = Self::subst_type_params(declared, &map);
            // 置換後もなお型変数が残る（入れ子テンプレート等）なら検査しない。
            if self.mentions_type_param(&expected) {
                continue;
            }
            // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
            let mutable = sig.param_mutable.get(param_idx).copied().unwrap_or(false);
            let ctx = format!("argument to `{base_name}[{}]`", type_args.join(", "));
            if !self.check_expected(arg_ty, &expected, mutable, &ctx) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: format!("{base_name}[{}]", type_args.join(", ")),
                        param_index: param_idx.saturating_sub(implicit),
                        expected,
                        got: arg_ty.clone(),
                    },
                    span: None,
                });
            }
        }
    }

    /// シグネチャの `param_idx` 番目に `arg_ty` を渡せるか。
    ///
    /// ⚠⚠ **`mut` パラメータ（write-back）だけは `int` → `float` の暗黙拡大を許さない。**
    /// 呼び先が呼び元の記憶域へ書き戻す引数（C ABI の `double*` など）では、拡大は
    /// 「値の変換」ではなく**記憶域の型詐称**になる（`int` 変数へ 8 バイトの double が
    /// 書き戻される）。詳細は [`TypeChecker::type_matches`] の doc。
    ///
    /// ⚠ `param_mutable` は `params` と**同じ絞り込み**で並んでいる前提（B11）。
    /// 添字がずれると別の引数の可変性を見る。
    fn param_type_matches(
        &self,
        sig: &FnSig,
        param_idx: usize,
        arg_ty: &InferredType,
        expected: &InferredType,
    ) -> bool {
        // ⚠ 判定は `types_compatible` に委譲する（タスク 3.3）。以前はここに
        //    `check_expected` と**同じ規則をもう 1 つ**書いていたので、片方だけ直すと
        //    ずれる状態だった。
        // ⚠ `let` 仮引数は `__cast__` による受理を許す唯一の地点（タスク 4.3）。
        //    実行時（`interpreter/functions/execution.rs`）が**ここだけ**変換を挿入する。
        let site = if sig.param_mutable.get(param_idx).copied().unwrap_or(false) {
            Site::MutParam
        } else {
            Site::LetParam
        };
        self.types_compatible(arg_ty, expected, site)
    }

    /// オーバーロード候補を**実引数の型**で絞り込む（タスク 5.7・検体 `C12`）。
    ///
    /// ## 何が壊れていたか
    ///
    /// 絞り込みは**引数の個数だけ**で行われ、個数の合う候補が 2 つ以上あると
    /// `count_matching.len() != 1` で**検査を丸ごと諦めて**いた。
    /// ⇒ `fn m(v: int)` と `fn m(v: str)` の 2 本があるクラスで `c.m(1.5)` が**素通り**
    /// （実測: `1.5` を出す）。**オーバーロードを書いた瞬間に型検査が消える**形だった。
    ///
    /// ## 絞り込みの規則
    ///
    /// | 残った候補 | 動作 |
    /// |---|---|
    /// | ちょうど 1 つ | それを使う（以降の引数検査が普通に走る） |
    /// | 0 個 | [`TypeErrorKind::NoOverloadForArgTypes`] を報告 |
    /// | 2 つ以上 | **絞り込まない**（どれを選んでも嘘になりうる） |
    ///
    /// ⚠ 安全側に倒す条件（いずれかに当たれば絞り込まない）:
    /// - キーワード引数・可変長引数が混ざっている（並びが 1 対 1 でない）
    /// - 候補に可変長パラメータを持つものがある
    /// - 実引数に判定材料の無い型（`Unresolved` / `Any`）がある
    ///
    /// `implicit_self` はメソッドのとき `true`（`params[0]` が `self` なので 1 つずらす）。
    fn narrow_overloads_by_arg_types(
        &mut self,
        func_name: &str,
        candidates: Vec<FnSig>,
        arg_data: &[(Option<String>, InferredType)],
        implicit_self: bool,
    ) -> Vec<FnSig> {
        if candidates.len() < 2 {
            return candidates;
        }
        // すべて位置引数でなければ並びが 1 対 1 に対応しない。
        if arg_data.iter().any(|(k, _)| k.is_some()) {
            return candidates;
        }
        if candidates.iter().any(|s| s.variadic_type.is_some()) {
            return candidates;
        }
        let arg_types: Vec<InferredType> = arg_data.iter().map(|(_, t)| t.clone()).collect();
        // ⚠ 判定材料の無い実引数が 1 つでもあれば絞り込まない（どの候補も通りうる）。
        if arg_types
            .iter()
            .any(|t| matches!(t, InferredType::Unresolved | InferredType::Any))
        {
            return candidates;
        }
        let offset = usize::from(implicit_self);
        let accepted: Vec<FnSig> = candidates
            .iter()
            .filter(|sig| {
                arg_types.iter().enumerate().all(|(i, arg_ty)| {
                    match sig.params.get(i + offset).and_then(|(_, t)| t.as_ref()) {
                        // 注釈の無いパラメータは何でも受ける。
                        None => true,
                        Some(expected)
                            if matches!(
                                expected,
                                InferredType::Unresolved | InferredType::Any
                            ) =>
                        {
                            true
                        }
                        Some(expected) => self.param_type_matches(sig, i + offset, arg_ty, expected),
                    }
                })
            })
            .cloned()
            .collect();
        match accepted.len() {
            1 => accepted,
            0 => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoOverloadForArgTypes {
                        func_name: func_name.to_string(),
                        got: arg_types,
                    },
                    span: None,
                });
                Vec::new()
            }
            // 曖昧: どれを選んでも嘘になりうるので絞り込まない。
            _ => candidates,
        }
    }

    /// `len()` の引数が**大きさを持つ型**かを検査する（タスク 7.4）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 大きさを持つ | 持たない |
    /// |---|---|
    /// | `list` ・ `fixed_list` ・ `list_like` ・ `str` ・ `dict` ・ `set` ・ `tuple` | `int` ・ `float` ・ `bool` ・ `None` |
    /// | `__len__` を持つクラス | `__len__` を持たないクラス（`object of type 'object' has no len()`） |
    ///
    /// ⚠ `InferredType` に「大きさを持つ型」は無いので、仮引数の型は `Any` にして
    /// **明らかに持たない型だけ**をここで弾く（7.1 と同じ「判っていて不可なら報告」）。
    /// ⚠ クラスは `__len__` を定義できるので素通しする。
    fn check_len_argument(&mut self, arg_data: &[(Option<String>, InferredType)]) {
        use InferredType as T;
        let [(None, got)] = arg_data else { return };
        let no_len = matches!(
            got,
            T::Int | T::Float | T::Complex | T::Bool | T::None | T::Undefined
        );
        if !no_len {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::NotSized { ty: got.clone() },
            span: None,
        });
    }

    /// 呼び出し対象が呼び出せる型かを検査する（タスク 7.1・検体 `C13`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 呼び出せる | 呼び出せない |
    /// |---|---|
    /// | 関数・ジェネレータ関数・ネイティブ関数 | `int` ・ `float` ・ `str` ・ `bool` |
    /// | クラス名（＝コンストラクタ） | `list` ・ `dict` ・ `set` ・ `tuple` |
    /// | `__call__` を持つクラスのインスタンス | `None` |
    ///
    /// ⚠ クラスのインスタンスは `__call__` を定義できる（実測）ので**素通し**する
    /// （持たない場合は実行時 `AttributeError: 'D' has no method '__call__'`）。
    /// ⚠⚠ ここで報告するのは「**確実に呼べない**」と分かっている型だけ。
    /// `Unresolved` / `Any` / `Namespace` などは判定材料が無いので触らない
    /// （根本原因① の教訓: 「判らない」と「誤り」を取り違えない）。
    fn check_callable(&mut self, func_type: &InferredType) {
        use InferredType as T;
        let not_callable = matches!(
            func_type,
            T::Int
                | T::Float
                | T::Complex
                | T::Str
                | T::Bool
                | T::None
                | T::Undefined
                | T::List
                | T::ListOf(_)
                | T::FixedList
                | T::FixedListOf(_)
                | T::ListLike
                | T::ListLikeOf(_)
                | T::Dict
                | T::DictOf(_, _)
                | T::Set
                | T::SetOf(_)
                | T::Tuple(_)
        );
        if !not_callable {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::NotCallable { ty: func_type.clone() },
            span: None,
        });
    }

    /// `new_type` のコンストラクタ引数を**基底型**と照合する（タスク 5.6・検体 `T3`）。
    ///
    /// ⚠⚠ `new_type Meters: float` に `Meters("s")` が**黙って通っていた**（実測）。
    /// `new_type` は `known_class_names` と `new_type_originals` にしか登録されず、
    /// `__init__` のシグネチャを持たないので、引数の照合先が**そもそも無かった**。
    /// 実行時が見ているのは個数だけ（`function takes 1 argument(s), got 2`）。
    ///
    /// ⚠ 個数違いはここでは扱わない（実行時が報告する）。
    /// ⚠ `Meters(5)` は**エラーになる**。暗黙の `int` → `float` はタスク 4.2 で廃止したので
    /// `Meters(float(5))` と書く。
    ///
    /// ## ⚠⚠ 基底が**クラス**のときは検査しない
    ///
    /// `new_type Kilometers: Meters` の `Meters` が**クラス**のとき、`Kilometers(5)` の `5` は
    /// 「`Meters` を包む値」ではなく **`Meters` のコンストラクタ引数**（レジストリが
    /// `class_method_sigs` を引き継いでいる）。基底型と突き合わせると
    /// `expects 'Meters' but got 'int'` という**偽エラー**になる
    /// （`examples/.../polymorphism.ar` が実際に落ちた）。
    /// ⇒ `new_type` の連鎖を根まで辿り、**根がプリミティブのときだけ**検査する。
    fn check_new_type_arg(
        &mut self,
        name: &str,
        original: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) {
        // `new_type` の連鎖を根まで辿る（`Kg: Meters: float` → `float`）。
        let mut root = original.to_string();
        let mut seen = std::collections::HashSet::new();
        while let Some(next) = self.registry.new_type_original(&root).map(str::to_string) {
            if !seen.insert(next.clone()) {
                break;
            }
            root = next;
        }
        let Some(expected) = InferredType::from_ann(&root) else {
            return;
        };
        // ⚠ 根がクラス（`NamedInstance`）ならコンストラクタを継承しているので検査しない。
        if matches!(
            expected,
            InferredType::Unresolved
                | InferredType::Any
                | InferredType::NamedInstance(_)
                | InferredType::GenericInstance { .. }
        ) {
            return;
        }
        let [(None, got)] = arg_data else { return };
        if matches!(got, InferredType::Unresolved | InferredType::Any) {
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("argument of `{name}`");
        if self.check_expected(got, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::CallArgTypeMismatch {
                func_name: name.to_string(),
                param_index: 0,
                expected,
                got: got.clone(),
            },
            span: None,
        });
    }

    /// **組み込みコレクションのメソッド引数**を要素型と照合する（タスク 5.2c・検体 `L14`）。
    ///
    /// ## 対象は「要素を 1 つ受け取るメソッド」だけ
    ///
    /// | レシーバ | メソッド | 引数 |
    /// |---|---|---|
    /// | `list` / `fixed_list` / `list_like` | `append` | 要素 |
    /// | `set` | `add` ・ `discard` ・ `remove` | 要素 |
    ///
    /// ⚠⚠ **実装側と必ず突き合わせること。** 実在するのは
    /// `src/interpreter/classes/method_call.rs`（list）・`set_methods.rs`（set）・
    /// `frozen_list_methods.rs`（fixed_list）に書かれているものだけで、
    /// `insert` / `extend` / `remove`（list）/ `index` / `count` / `get`（dict）は
    /// **存在しない**（実測: `AttributeError: 'list' object has no method 'insert'`）。
    /// 推測で足すと「実在しないメソッドの引数を検査する」死んだ枝になる。
    ///
    /// ⚠ `union` / `intersection` 等は**集合そのもの**を受け取るので対象外
    /// （要素型と突き合わせると偽エラーになる）。
    ///
    /// ⚠ 弾かなかった場合に何が起きるかは地点で違う（実測）:
    /// `list.append` と `set.add` は**黙って異型を入れる**（`[1, 's']` / `{1, 's'}`）、
    /// `set.discard` は黙って何もしない、`set.remove` は `KeyError`。
    /// どれも「要素型が守られない」ことに変わりはない。
    fn check_builtin_collection_method_args(
        &mut self,
        recv_ty: &InferredType,
        method: &str,
        arg_data: &[(Option<String>, InferredType)],
        span: &crate::token::Span,
    ) {
        use InferredType as T;
        let elem = match (recv_ty, method) {
            (T::ListOf(e), "append")
            | (T::FixedListOf(e), "append")
            | (T::ListLikeOf(e), "append") => e,
            (T::SetOf(e), "add") | (T::SetOf(e), "discard") | (T::SetOf(e), "remove") => e,
            // 要素型の判らない素の `list` / `set`、クラス、その他のメソッドは対象外。
            _ => return,
        };
        // 判定材料が無い要素型（`list[Any]`・型変数）は照合しない。
        if matches!(**elem, T::Unresolved | T::Any | T::Never) || self.mentions_type_param(elem) {
            return;
        }
        // ⚠ 引数が 1 つの位置引数でなければ、実装側が個数エラーを出す形。ここでは触らない。
        let [(None, got)] = arg_data else { return };
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("argument of `{method}`");
        if self.check_expected(got, elem, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::SubscriptTypeMismatch {
                context: format!("argument of `{recv_ty}.{method}()`"),
                expected: (**elem).clone(),
                got: got.clone(),
            },
            span: Some(span.clone()),
        });
    }

    /// 関数型変数の呼び出し検査：引数個数・型・キーワード名・`mut` 引数の可変性を検査する。
    pub(super) fn check_fn_type_call(
        &mut self,
        func_name: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
        params: &[FnTypeParam],
    ) {
        // デフォルトを持つ仮引数は省略できるので、必要数は `has_default` が false の個数。
        let required = params.iter().filter(|p| !p.has_default).count();
        if arg_data.len() < required || arg_data.len() > params.len() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CallArgCountMismatch {
                    func_name: func_name.to_string(),
                    expected_min: required,
                    expected_max: params.len(),
                    got: arg_data.len(),
                },
                span: None,
            });
            return;
        }

        let mut positional_idx = 0usize;
        for (i, (key, arg_ty)) in arg_data.iter().enumerate() {
            let arg_expr = args[i].expr();
            match key {
                Some(kwarg_name) => match params.iter().position(|p| &p.name == kwarg_name) {
                    None => self.report_error(StaticTypeError {
                        kind: TypeErrorKind::UnknownKeywordArg {
                            func_name: func_name.to_string(),
                            arg_name: kwarg_name.clone(),
                        },
                        span: None,
                    }),
                    Some(param_pos) => {
                        let param = &params[param_pos];
                        // ⚠ `mut` 引数（write-back）は拡大を許さない（`type_matches` の doc）。
                        let ok = if param.mutable {
                            self.types_compatible(arg_ty, &param.ty, Site::MutParam)
                        } else {
                            self.types_compatible(arg_ty, &param.ty, Site::LetParam)
                        };
                        if param.ty != InferredType::Any && !ok {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: param_pos,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                },
                None => {
                    if let Some(param) = params.get(positional_idx) {
                        // ⚠ `mut` 引数（write-back）は拡大を許さない（`type_matches` の doc）。
                        let ok = if param.mutable {
                            self.types_compatible(arg_ty, &param.ty, Site::MutParam)
                        } else {
                            self.types_compatible(arg_ty, &param.ty, Site::LetParam)
                        };
                        if param.ty != InferredType::Any && !ok {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: positional_idx,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable && !self.is_mutable_expr(arg_expr) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CallMutParamWithImmutableArg {
                                    func_name: func_name.to_string(),
                                    param_name: param.name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    positional_idx += 1;
                }
            }
        }
    }

    /// 式が可変変数の参照かどうかを判定する。
    pub(super) fn is_mutable_expr(&self, expr: &Expr) -> bool {
        if let Expr::Ident { name, .. } = expr {
            self.lookup(name).map(|v| v.mutable).unwrap_or(false)
        } else {
            false
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
        let param_name = sig.params.get(param_pos).map(|(n, _)| n.clone()).unwrap_or_default();
        self.report_error(StaticTypeError {
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
fn is_dynamic_arg(t: &InferredType) -> bool {
    matches!(t, InferredType::Any | InferredType::Unresolved)
}

/// パラメータ型が「具象（検査に値する特定の型）」か。`Any`/`Unresolved` 以外なら true。
fn is_specific_param(t: &InferredType) -> bool {
    !matches!(t, InferredType::Any | InferredType::Unresolved)
}
