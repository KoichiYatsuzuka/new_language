// type_check/call_check/overloads.rs — オーバーロードの選択: Python のスタブの候補と、引数の型による絞り込み。

use super::*;

impl TypeChecker {
    /// シグネチャの `param_idx` 番目に `arg_ty` を渡せるか。
    ///
    /// ⚠⚠ **`mut` パラメータ（write-back）だけは `int` → `float` の暗黙拡大を許さない。**
    /// 呼び先が呼び元の記憶域へ書き戻す引数（C ABI の `double*` など）では、拡大は
    /// 「値の変換」ではなく**記憶域の型詐称**になる（`int` 変数へ 8 バイトの double が
    /// 書き戻される）。詳細は [`TypeChecker::type_matches`] の doc。
    ///
    /// ⚠ `param_mutable` は `params` と**同じ絞り込み**で並んでいる前提（B11）。
    /// 添字がずれると別の引数の可変性を見る。
    pub(super) fn param_type_matches(
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
    /// `m.f(..)`（`m` は Python のモジュールの別名）の `f` が `@overload` の形を持つなら、CPython の型の検査器と同じく
    /// **最初に当てはまる形**を選び、その形で引数を検査して戻り値の型を返す（pandas_import_plan.md の 1-3）。
    /// どの形にも当てはまらなければ `None`（本体の宣言の型で従来どおり検査する）。
    ///
    /// 当てはまるか: 位置引数の数・キーワードの名前・省略できない引数が揃っているか・`Literal[X]` の引数に同じ値のリテラル・
    /// `None` の引数に `None` を渡したか。⚠ 型の照合は選んだ後に行う（形を選ぶのは、渡した引数の形と値だけ）。
    /// ⚠ `*xs` / `**d` を渡すと形を決められないので選ばない（本体の宣言の型で検査する）。
    pub(super) fn select_py_overload(
        &mut self,
        func: &Expr,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        let Expr::Attr { object, attr, .. } = func else { return None };
        let Expr::Ident { name, .. } = object.as_ref() else { return None };
        if !matches!(self.lookup(name).map(|v| &v.ty), Some(InferredType::PyNamespace(..) | InferredType::Namespace(..))) {
            return None;
        }
        let module = self.py_aliases.get(name)?;
        let key = format!("{module}.{attr}");
        self.select_py_overload_of(&key, &format!("{name}.{attr}"), args, arg_data)
    }

    /// `x.m(..)`（`x` は Python のクラスのインスタンス）のメソッドが `@overload` の形を持つなら、最初に当てはまる形を選ぶ
    /// （[`Self::select_py_overload`] と同じ規則・pandas_import_plan.md の 6-4）。
    pub(super) fn select_py_method_overload(
        &mut self,
        class: &str,
        method: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        let key = format!("{}.{method}", self.registry.resolve(class));
        self.select_py_overload_of(&key, &format!("{class}.{method}"), args, arg_data)
    }

    /// 形の表の `key` の、最初に当てはまる形で引数を検査して戻り値の型を返す。`shown` は表示の名前。
    fn select_py_overload_of(
        &mut self,
        key: &str,
        shown: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        // 引数の型も当てはまる最初の形（CPython の型の検査器と同じ・pandas_import_plan.md の 6-10）。型で当てはまる形が無ければ、
        // 個数などで当てはまる最初の形で検査して誤りを出す。⚠ 以前は型を見なかったので、numpy の `np.array([1.0])` が
        // 2 つ目の形（`_ArrayLike[_SCT]`・要素が配列らしい物）に決まり、型が付いてから偽の誤りになった
        let forms = self.py_overloads.get(key)?;
        let chosen = forms
            .iter()
            .find(|v| Self::py_overload_matches(v, args) && self.py_overload_types_match(v, arg_data))
            .or_else(|| forms.iter().find(|v| Self::py_overload_matches(v, args)))?
            .clone();
        if let Some(fn_params) = &chosen.fn_params {
            self.check_fn_type_call(shown, args, arg_data, fn_params);
        }
        Some(chosen.ret)
    }

    /// 引数の型が `@overload` の形 `v` の仮引数の型に当てはまるか（報告しない・[`Self::select_py_overload_of`]）。可変長を持つ形
    /// （`fn_params` が無い）は型を見ない。
    fn py_overload_types_match(&self, v: &crate::type_check::PyOverload, arg_data: &[(Option<String>, InferredType)]) -> bool {
        let Some(params) = &v.fn_params else { return true };
        let mut positional = 0usize;
        for (key, ty) in arg_data {
            let param = match key.as_deref() {
                Some("...") => continue,
                Some(k) => params.iter().find(|p| p.name == k),
                None => {
                    positional += 1;
                    params.get(positional - 1)
                }
            };
            if let Some(p) = param {
                if !self.types_compatible(ty, &p.ty, Site::Other) {
                    return false;
                }
            }
        }
        true
    }

    /// 呼び出し `args` が `@overload` の形 `v` に当てはまるか（[`Self::select_py_overload`]）。
    fn py_overload_matches(v: &crate::type_check::PyOverload, args: &[CallArg]) -> bool {
        let mut supplied: std::collections::HashMap<&str, &Expr> = std::collections::HashMap::new();
        let mut positional = 0usize;
        for a in args {
            match a {
                CallArg::Positional(e) => {
                    match v.params.get(positional) {
                        Some((p, _)) => {
                            supplied.insert(p.as_str(), e);
                        }
                        None if v.varargs => {}
                        None => return false,
                    }
                    positional += 1;
                }
                CallArg::Keyword { name, value } => {
                    if !v.kwargs && !v.params.iter().any(|(p, _)| p == name) {
                        return false;
                    }
                    supplied.insert(name.as_str(), value);
                }
                // `*xs` / `**d` / 可変長の塊は形を決められない
                _ => return false,
            }
        }
        if v.params.iter().any(|(p, has_default)| !has_default && !supplied.contains_key(p.as_str())) {
            return false;
        }
        for (p, lit) in &v.literals {
            if let Some(e) = supplied.get(p.as_str()) {
                let same = match e {
                    Expr::Bool(b) => lit == if *b { "True" } else { "False" },
                    Expr::None => lit == "None",
                    Expr::Int(n) => *lit == n.to_string(),
                    Expr::Str(s) => {
                        let t = lit.trim_matches(|c| c == '"' || c == '\'');
                        t == &**s && lit.len() >= 2
                    }
                    // リテラルでない値は `Literal[X]` に当てはまらない（CPython の型の検査器と同じ）
                    _ => false,
                };
                if !same {
                    return false;
                }
            }
        }
        for p in &v.none_params {
            if let Some(e) = supplied.get(p.as_str()) {
                if !matches!(e, Expr::None) {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn narrow_overloads_by_arg_types(
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
}
