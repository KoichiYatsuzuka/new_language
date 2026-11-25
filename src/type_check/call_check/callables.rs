// type_check/call_check/callables.rs — 呼べる値の検査: 呼べるか・new_type の引数・関数型の値の呼び出し。

use super::*;

impl TypeChecker {
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
    pub(super) fn check_callable(&mut self, func_type: &InferredType) {
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
                | T::FrozenSet
                | T::FrozenSetOf(_)
                | T::Bytes
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
    pub(super) fn check_new_type_arg(
        &mut self,
        name: &str,
        original: &str,
        arg_data: &[(Option<String>, InferredType)],
        args: &[CallArg],
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
        self.report_error_at_opt(Self::arg_expr(args, 0), StaticTypeError {
            kind: TypeErrorKind::CallArgTypeMismatch {
                func_name: name.to_string(),
                param_index: 0,
                expected,
                got: got.clone(),
            },
            span: None,
        });
    }

    /// 関数型変数の呼び出し検査：引数個数・型・キーワード名・`mut` 引数の可変性を検査する。
    pub(in crate::type_check) fn check_fn_type_call(
        &mut self,
        func_name: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
        params: &[FnTypeParam],
    ) {
        // ⚠ 展開があると位置で対応づけられない。個数・型検査を降りる。
        if Self::has_spread_arg(args) {
            return;
        }
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
                    None => self.report_error_at(arg_expr, StaticTypeError {
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
                            self.report_error_at(arg_expr, StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: param_pos,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable
                            && self.path_is_mutable(arg_expr) == Some(false)
                            && !Self::mut_arg_exempt(arg_ty)
                        {
                            self.report_error_at(arg_expr, StaticTypeError {
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
                            self.report_error_at(arg_expr, StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: func_name.to_string(),
                                    param_index: positional_idx,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                        if param.mutable
                            && self.path_is_mutable(arg_expr) == Some(false)
                            && !Self::mut_arg_exempt(arg_ty)
                        {
                            self.report_error_at(arg_expr, StaticTypeError {
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
}
