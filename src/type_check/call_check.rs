use crate::ast::{CallArg, Expr};
use crate::type_check::types::InferredType as IT;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::{FnSig, FnTypeParam, InferredType};
use super::type_utils::Site;
use super::TypeChecker;

mod methods;
mod args;
mod templates;
mod overloads;
mod builtin_calls;
mod py_values;
mod callables;

use self::args::*;

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
            // 属性名を指す（diagnostic_position_plan.md の 3-5）。
            Expr::Attr { attr, span, attr_span, .. } if attr == "__freeze__" => {
                Some(if attr_span.line != 0 { attr_span.clone() } else { span.clone() })
            }
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
        // 受け手の型が protocol（`h: Holder[int]`）なら、その名前と型引数。メソッドの引数と戻り値は protocol の要求の
        // シグネチャで検査する（pandas_import_plan.md の 6-5・`check_protocol_method_call`）。
        let mut protocol_recv: Option<(String, Vec<InferredType>)> = None;
        let method_call_info: Option<(String, String)> =
            if let Expr::Attr { object, attr, span, attr_span, .. } = func {
                // 属性の誤りは**属性名**を指す（`.` ではない・diagnostic_position_plan.md の 3-5）。合成した属性参照（位置 0）は `.`。
                let span = if attr_span.line != 0 { attr_span } else { span };
                let obj_ty = self.infer(object);
                if let InferredType::Protocol(p, pargs) = self.resolve_protocols(&obj_ty) {
                    protocol_recv = Some((p, pargs));
                }
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

                // ⚠⚠ **テンプレートの具体化（`Box[int]`）も受け手として解決する**（タスク 2-8 段階 2）。
                //   以前は `GenericInstance` が下の `_ => None` に落ち、メソッド呼び出しの検査が
                //   **丸ごと飛ばされていた**（型違いの引数・戻り値・存在しないメソッド・アクセス制御・
                //   `static` の誤用がすべて通った。フェーズ10 の発端）。単相化したクラスがあれば
                //   それを普通のクラスとして使う。
                let instance_cls: Option<String> = match &obj_ty {
                    InferredType::GenericInstance { .. } => {
                        self.registry.instance_class(&obj_ty.to_string()).cloned()
                    }
                    _ => None,
                };
                let is_instance = matches!(obj_ty, InferredType::NamedInstance(_)) || instance_cls.is_some();
                let cls_name_opt: Option<String> = match &obj_ty {
                    // protocol の型の値（6-5）は protocol の名前でメンバーの有無を見る（`member_exists`）
                    _ if protocol_recv.is_some() => protocol_recv.as_ref().map(|(p, _)| p.clone()),
                    InferredType::GenericInstance { .. } => instance_cls.clone(),
                    InferredType::NamedInstance(cls) => Some(cls.clone()),
                    InferredType::TypeValOf(inner) => match inner.as_ref() {
                        InferredType::NamedInstance(cls) => Some(cls.clone()),
                        // 具体化を値として使う形（`Stack[int].empty()`・10-9）は単相化した具体クラス。
                        InferredType::GenericInstance { .. } => {
                            self.registry.instance_class(&inner.to_string()).cloned()
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(cls_name) = cls_name_opt {
                    let is_static = self.registry.is_static_method(&cls_name, attr.as_str());
                    // StaticMethodOnInstance: only report when called on an instance
                    // ⚠ Python のクラスの静的メソッドはインスタンスからも呼べる（CPython と同じ・known_bugs.md の #5）。
                    if is_static && is_instance && !self.registry.is_cpython_class(&cls_name) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::StaticMethodOnInstance {
                                method_name: attr.clone(),
                                class_name: cls_name.clone(),
                            },
                            span: Some(span.clone()),
                        });
                    }
                    // アクセス制御（`infer` の属性の読みと同じ）。⚠ クラス経由の呼び出し
                    // （`Cls.m()`・静的メソッド）も同じく見る（10-6）。
                    self.check_member_access_static(&cls_name, attr, Some(span.clone()));
                    // 変更できない受け手への `mut self` メソッド（10-15）。
                    if is_instance {
                        self.check_mut_self_method_receiver(object, &cls_name, attr, span);
                    }
                    // ⚠⚠ **メソッド呼び出しは `infer_attr` を通らない**（オブジェクトを
                    //    二度推論しないよう `self.infer(object)` を直接呼んでいる）。
                    //    ⇒ 存在検査（タスク 7.5）を**ここにも**書く必要がある。
                    //    検体 `M1`（フィールドの読み）は `infer_attr` 側で閉じたが、
                    //    `M2`（メソッドの呼び出し）はこの経路なので別に閉じる。
                    // ⚠ `__getattr__` を上書きした Python のクラスのインスタンスは、無い名前も上書きが答える（読んで呼ぶ・#11）。
                    let answered_by_hook = is_instance && self.getattr_hook_type(&cls_name).is_some();
                    if !self.member_exists(&cls_name, attr) && !answered_by_hook {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::NoSuchMember {
                                class_name: cls_name.clone(),
                                member: attr.clone(),
                            },
                            span: Some(span.clone()),
                        });
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
            // ⚠ モジュールの名前空間のメンバー（`t.Tag(1)` / `t.make(1)`）は修飾名で引く
            //   （フェーズ10 10-8）。素の `Tag` で引くとメインの同名クラスを見てしまう（実測）。
            // ⚠ 呼び先の受け手がモジュールの連鎖（`a.b.f(..)`・`import a.b` は `a` を束縛する）なら、
            //   修飾名 `a.b.f` で引く（CPython 準拠・2026-10-02）。
            Expr::Attr { object, attr, .. } if self.module_path_of_expr(object).is_some() => {
                self.module_path_of_expr(object).map(|p| format!("{p}.{attr}"))
            }
            Expr::Attr { object, attr, .. } => match object.as_ref() {
                Expr::Ident { name, .. } => match self.lookup(name).map(|v| &v.ty) {
                    // ⚠ `import[py]` のモジュール（`PyNamespace`）も修飾名で引く（`zoo.Dog`）。以前は素の `Dog` で
                    //   引いていたので、`z.Dog()` がメインの同名クラスの `__init__` で検査されていた（実測）。
                    Some(InferredType::Namespace(..) | InferredType::PyNamespace(..)) => {
                        // ⚠ 外部言語のモジュール（cpp / cs …）は素の名前で登録したまま。
                        Some(self.registry.module_member(name, attr).unwrap_or_else(|| attr.clone()))
                    }
                    // ⚠ 型の分からない受け手（エディタで読めていない `import` の別名など）の `x.f(..)` を、
                    //   素の名前 `f` でレジストリから引かない（10-8）。引くとメインの同名クラス・関数を
                    //   見てしまう（エディタだけが偽の誤りを出した・`compare_wasm_frontend` で実測）。
                    None | Some(InferredType::Unresolved) => None,
                    _ => Some(attr.clone()),
                },
                _ => Some(attr.clone()),
            },
            _ => None,
        };

        // 値のメソッド `x.f(..)`（受け手がモジュールでも型が分からない名前でもない）の `f` は関数の名前ではない。表示には使うが、
        // 関数の表を引かない（pandas_import_plan.md の 6-1）。⚠ 以前は引いていたので、numpy を import すると pandas の
        // `.mean()` が numpy の同じ名前の関数として検査された。Arrow の `fn upper` があると `"a".upper()` も同じ。
        let method_of_value = match func {
            Expr::Attr { object, .. } if self.module_path_of_expr(object).is_none() => match object.as_ref() {
                Expr::Ident { name, .. } => !matches!(
                    self.lookup(name).map(|v| &v.ty),
                    None | Some(InferredType::Unresolved | InferredType::Namespace(..) | InferredType::PyNamespace(..))
                ),
                _ => true,
            },
            _ => false,
        };

        // Only evaluate func type when not a method call. For method calls the object
        // was already inferred above; re-calling self.infer(func) would re-evaluate
        // the object expression and potentially duplicate errors.
        let func_type = if method_call_info.is_none() {
            // 呼び先の属性式は「読み」ではない（`attr_is_callee` の doc・10-14）。
            self.attr_is_callee = matches!(func, Expr::Attr { .. });
            let t = self.infer(func);
            self.attr_is_callee = false;
            t
        } else {
            InferredType::Unresolved
        };
        // ⚠ 呼び出せない型を弾く（タスク 7.1・検体 `C13`）。
        self.check_callable(&func_type);

        let mut arg_data: Vec<(Option<String>, InferredType)> = Vec::new();
        // `f(*xs)` / `f(**d)` で並ぶ引数の型（展開した要素・値）。Python へ渡す値の検査（2-11）に使う
        let mut spread_elems: Vec<InferredType> = Vec::new();
        for arg in args.iter() {
            match arg {
                CallArg::Positional(e) => arg_data.push((None, self.infer(e))),
                CallArg::Keyword { name, value } => {
                    arg_data.push((Some(name.clone()), self.infer(value)))
                }
                // ★ `f(*xs)` / `f(**d)` — **展開後の個数は実行時に決まる**ので、
                //   ここでは「型は推論するが引数列には積まない」。個数検査は
                //   `has_spread_arg` を見て降りる（下記）。
                //   ⚠ 推論だけは必ず行う（式の中の誤りを見逃さないため）。
                CallArg::Spread(e) => {
                    let ty = self.infer(e);
                    self.check_spread_arg(&ty, false, e);
                    spread_elems.push(self.iter_element_type(&ty));
                }
                CallArg::KwSpread(e) => {
                    let ty = self.infer(e);
                    self.check_spread_arg(&ty, true, e);
                    if let InferredType::DictOf(_, v) | InferredType::PyDict(_, v) = &ty {
                        spread_elems.push((**v).clone());
                    }
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
                    // ⚠⚠ **空のときは `ListOf(Never)`。素の `List` に倒さないこと**（タスク 8.4）。
                    //    素の `List` は下流の可変長検査（`Some((_, IT::ListOf(elem_ty)))` で
                    //    受ける）に**当たらない**ので、そこだけ検査が消える。空リテラルの
                    //    要素型を `Never`（⊥）にしたタスク 4.1 と同じ判断。
                    // ⚠ 現状この枝は**到達しない**。`parse_type_expr` ではなく呼び出し式の
                    //   パーサ（`parser/exprs.rs` の `... = ` の処理）が
                    //   「variadic argument `... = ...` requires at least one expression」で
                    //   空を弾いているため。**それでも素の `List` を置かない**のは、
                    //   パーサ側が緩んだときに「検査が静かに消える」形で露見するのを防ぐため。
                    let list_ty = if elem_types.is_empty() {
                        InferredType::ListOf(Box::new(InferredType::Never))
                    } else {
                        InferredType::ListOf(Box::new(Self::join_elem_types(elem_types)))
                    };
                    arg_data.push((Some("...".to_string()), list_ty));
                }
            }
        }

        // Python の関数・メソッド・クラスへ Arrow の list / dict / set をキャストせずに渡さない（D-12・2-11）。⚠ 呼ぶ先が
        //   Python と型で分かるときだけ（`from m import f` した `f(xs)` などは実行時の `TypeError`）
        let python_callee = builtin_recv.as_ref().is_some_and(|(recv, _, _)| self.is_python_value(recv))
            || matches!(&func_type, InferredType::TypeValOf(inner) if self.is_python_value(inner));
        if python_callee {
            let callee = func_name.clone().unwrap_or_else(|| "the Python call".to_string());
            for (i, (key, ty)) in arg_data.iter().enumerate() {
                let context = match key {
                    Some(k) => format!("keyword argument `{k}` of `{callee}`"),
                    None => format!("argument {i} of `{callee}`"),
                };
                // 誤りはその引数を指す（diagnostic_position_plan.md の 3-2）
                let at = Self::arg_expr(args, i).and_then(|e| e.anchor_span()).cloned();
                self.with_anchor(at, |c| c.check_no_arrow_collection_to_python(ty, context));
            }
            for ty in &spread_elems {
                self.check_no_arrow_collection_to_python(ty, format!("an unpacked argument of `{callee}`"));
            }
        }

        // ⚠ `len()` の引数は「大きさを持つ型」（タスク 7.4）。
        if let Expr::Ident { name, .. } = func {
            if name == "len" && self.lookup("len").is_some() {
                self.check_len_argument(&arg_data, args);
            }
        }

        // ⚠ `new_type` のコンストラクタ引数（タスク 5.6・検体 `T3`）。
        if let Expr::Ident { name, .. } = func {
            if let Some(original) = self.registry.new_type_original(name).map(str::to_string) {
                self.check_new_type_arg(name, &original, &arg_data, args);
            }
        }

        // ⚠ 組み込みコレクションメソッドの引数型（タスク 5.2c・検体 `L14`）。
        let mut builtin_ret: Option<InferredType> = None;
        if let Some((recv_ty, method, span)) = builtin_recv {
            self.check_builtin_collection_method_args(&recv_ty, &method, &arg_data, &span);
            // ★ 戻り値型も分かる範囲で付ける。
            builtin_ret = Self::builtin_collection_method_return(&recv_ty, &method, &arg_data);
            // 表のメンバー（特殊メソッドと `count` / `index`・6-6）は、表のシグネチャで引数と戻り値を検査する
            // （実行時は `call_builtin_member`）
            if builtin_ret.is_none() {
                builtin_ret = self.builtin_member_call(&recv_ty, &method, &arg_data, args);
            }
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
                self.visible_fn_sigs(name).and_then(|sigs| {
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
            if let Some(name) = self.template_base_name(base) {
                let name = name.to_string();
                self.check_template_call_args(&name, type_args, &arg_data, args);
                // ⚠ タスク 2.1: 結果型を返す（以前は常に `Unresolved` を捨て返していた）。
                return self.template_call_result_type(&name, type_args);
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
        // ★ 組み込みコレクションのメソッドで戻り値型が判ったものは、ここで確定させる
        //   （`func_type` は `Unresolved` なので下の分岐では何も付かない）。
        if let Some(ret) = builtin_ret {
            return ret;
        }
        // ★ 組み込み**関数**（`enumerate` / `zip`）の戻り値型。
        //   ⚠⚠ **シャドウを先に見る**。`let enumerate = tagged` のように同名の変数や
        //     関数が宣言されていればそちらが勝つ（`examples/basics/builtin_shadow.ar`）。
        //     `lookup` を落とすとこの例題が壊れる（実際に踏んだ）。
        if let Expr::Ident { name, .. } = func {
            if self.visible_fn_sigs(name).is_none() && self.lookup(name).is_none() {
                // `object()` は Arrow のコードからは呼べない（タスク 6-10 の決定・Python のコードの `object()` は CPython）。
                if name == "object" {
                    self.report_error(StaticTypeError { kind: TypeErrorKind::ObjectCallInArrow, span: None });
                    return InferredType::Unresolved;
                }
                // 引数は宣言（`builtins.ars`）の仮引数で検査する（`check_builtin_call`）。
                self.check_builtin_call(name, args, &arg_data);
                if let Some(ret) = self.builtin_fn_return(name, &arg_data) {
                    return ret;
                }
            }
        }
        // Python の関数の `@overload`（`pd.read_csv(path)` は `DataFrame`・1-3）
        if let Some(ret) = self.select_py_overload(func, args, &arg_data) {
            return ret;
        }
        let direct_fn_call = matches!(func, Expr::Ident { .. })
            && func_name
                .as_deref()
                .is_some_and(|n| self.visible_fn_sigs(n).is_some());
        if !direct_fn_call {
        // 束縛メソッドが不変の受け手を書き換える（`let c` の `let add = c.add` の後の `add(1)`・タスク 1-11）。
        // `c.add(1)` を直接書いたのと同じ誤り。束縛すること自体は誤りにしない。
        if let InferredType::Function { mutates_immutable: Some(m), .. } = &func_type {
            let (method, root_name) = (**m).clone();
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::MutatingMethodOnImmutable { method, root_name },
                span: None,
            });
        }
        match func_type {
            InferredType::Function {
                params: Some(fn_params),
                return_type,
                ..
            } => {
                let fname = func_name.as_deref().unwrap_or("<function>").to_string();
                let ret = *return_type;
                self.check_fn_type_call(&fname, args, &arg_data, &fn_params);
                return ret;
            }
            InferredType::Function { params: None, return_type, .. } => {
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
            // ⚠ **名前を新しく占有しない形で解く。** 大域の名前を占有する組み込みの登録（当時の
            //    `builtin_fns`・2026-09-30 に撤去し `builtins::GLOBAL_FNS` へ移した）に
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
                        | InferredType::Bytes
                        | InferredType::Bool
                        | InferredType::Complex
                ) {
                    return (**inner).clone();
                }
            }
            _ => {}
        }
        }

        if let (Some((_, ref method_name)), Some((pname, pargs))) = (&method_call_info, &protocol_recv) {
            return self.check_protocol_method_call(pname, pargs, method_name, args, &arg_data);
        }
        if let Some((ref cls_name, ref method_name)) = method_call_info {
            // クラス変数に入ったクラスを呼ぶ形（`Outer.Inner()` / `self.Inner()`・フェーズ10 10-18）。
            // 実行時もメソッドが無いときだけクラス変数のクラスを呼ぶ（`Interpreter::class_var_class`）。
            if !self.collect_class_method_sigs(cls_name).contains_key(method_name.as_str()) {
                // クラス側のディスクリプタは `__get__` の戻り値を呼ぶ（タスク 1-12）。
                let member_ty = match self.class_descriptor(cls_name, method_name) {
                    Some(d) => Some(d.read),
                    None => self.declared_field_type_pub(cls_name, method_name),
                };
                match member_ty {
                    Some(InferredType::TypeValOf(inner)) => {
                        if let InferredType::NamedInstance(c) = *inner {
                            return InferredType::NamedInstance(c);
                        }
                    }
                    // 関数を入れたフィールドを呼ぶ形（`self.f(x)`・python_builtins_plan.md のタスク 1-11）。
                    // 実行時は「属性を読んで、その値を呼ぶ」。関数型で引数・戻り値を検査する。
                    Some(InferredType::Function { params, return_type, .. }) => {
                        if let Some(fn_params) = params {
                            self.check_fn_type_call(&format!("{cls_name}.{method_name}"), args, &arg_data, &fn_params);
                        }
                        return *return_type;
                    }
                    // 呼べない値を入れたフィールド（`self.n(1)` の `n: int`）は値を呼ぶのと同じ誤り。
                    Some(ty) => self.check_callable(&ty),
                    None => {}
                }
            }
            // 今の Arrow から正しく呼べない C# のメソッド（コレクションを受け渡す など・2-1）
            if let Some((what, reason)) = self.registry.ffi_unsupported_method(cls_name, method_name) {
                let (what, reason) = (what.to_string(), reason.to_string());
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::FfiUnsupported {
                        what,
                        function: format!("{cls_name}.{method_name}"),
                        reason,
                    },
                    span: None,
                });
            }
            // Python のクラスのメソッドの `@overload`（6-4）
            if let Some(ret) = self.select_py_method_overload(cls_name, method_name, args, &arg_data) {
                return ret;
            }
            // self/cls is passed implicitly; check_self_type_params accounts for the +1
            let ret_ty = self.check_self_type_params(cls_name, method_name, &arg_data, args);
            return ret_ty.unwrap_or(InferredType::Unresolved);
        } else if let Some(fname) = func_name.as_ref().filter(|_| !method_of_value) {
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
                return InferredType::Protocol(fname.clone(), Vec::new());
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
                // 外部言語のクラスも、スタブの `__init__` で検査する（ffi_value_exchange_plan.md 7-31・利用者の決定: 分かる範囲で
                // 検査し、静的に分からない部分は `Any`）。各ブリッジのスタブは実際のコンストラクタを書く: C# は公開の
                // コンストラクタごと（5-1）、Rust の構造体は `new` / 構造体リテラル、Python は `__init__`（7-27。基底が分からない
                // クラスは引数を問わない `*args, **kwargs`）、cpp の構造体はフィールドの並び（7-31）、js-proc は利用者の `.ars`。
                // 引数の型が分からないところはスタブが `Any` と書く。`__init__` の無いスタブは検査しない（何も分からない）。
                // ⚠ 以前は外部言語のクラスをすべて除外していた（#27-a。当時の C# のスタブが `__init__` を引数 0 個で
                //   持っていたため。5-1 で公開のコンストラクタごとに書くようになっていた）。
                // Arrow から構築できない外部言語のクラス（private フィールドで `new` の無い Rust の構造体・2-5、中身の見えない
                // cpp の構造体・4-3）は、引数の数ではなく構築できないことを誤りにする。
                if let Some((what, reason)) = self.registry.ffi_unsupported_method(fname, "__init__") {
                    let (what, reason) = (what.replace(" method", " constructor"), reason.to_string());
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::FfiUnsupported { what, function: format!("{fname}(...)"), reason },
                        span: None,
                    });
                } else {
                    self.check_self_type_params(fname, "__init__", &arg_data, args);
                }
                return InferredType::NamedInstance(fname.clone());
            }
        }

        func_name
            .as_deref()
            .and_then(|n| self.visible_fn_sigs(n))
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
}
