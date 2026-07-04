// functions/args.rs — 呼び出し引数の評価と束縛: eval_call_args / bind_args / bind_args_relaxed。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::{CallArg, Param},
    crate::token::Span,
    crate::interpreter::{
        CapturedVar, DictData, ExecResult, FnValue, GeneratorFnValue, GeneratorState, InstanceData,
        Interpreter, StackFrame, Value, Var, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
        RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// 呼び出し引数リスト（AST の `CallArg`）を評価して `(name, value, is_mutable)` トリプルのリストを返す。
    ///
    /// - 位置引数: `(None, value, is_mutable)` として格納
    /// - キーワード引数: `(Some(name), value, is_mutable)` として格納
    /// - `is_mutable`: 引数が `mut` 変数（または式）から来ている場合は `true`。
    ///   `let` 変数の識別子なら `false`。それ以外（リテラル・式・コンストラクタ等）は保守的に `true`。
    ///
    /// - `call_args`: 評価前の呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Vec<(Option<String>, Value, bool)>)` — 評価済み引数リスト。`Err` — 評価エラー
    pub(crate) fn eval_call_args(
        &mut self,
        call_args: &[CallArg],
    ) -> Result<Vec<(Option<String>, Value, bool)>, String> {
        use crate::ast::Expr;
        let mut result = Vec::new();
        for arg in call_args {
            match arg {
                CallArg::Positional(e) => {
                    // Ident が let 変数を指す場合のみ is_mutable = false（コピー省略可）
                    let is_mutable = match e {
                        Expr::Ident(name) => self.get_var(name)
                            .map(|v| v.is_mutable())
                            .unwrap_or(true),
                        _ => true,
                    };
                    result.push((None, self.eval(e)?, is_mutable));
                }
                CallArg::Keyword { name, value } => {
                    let is_mutable = match value {
                        Expr::Ident(vname) => self.get_var(vname)
                            .map(|v| v.is_mutable())
                            .unwrap_or(true),
                        _ => true,
                    };
                    result.push((Some(name.clone()), self.eval(value)?, is_mutable));
                }
                // 可変長引数: 各要素を評価してリストに集約し、特殊キー "..." で渡す
                CallArg::Variadic(exprs) => {
                    let mut vals = Vec::new();
                    for e in exprs {
                        vals.push(self.eval(e)?);
                    }
                    result.push((
                        Some("...".to_string()),
                        Value::List(Rc::new(RefCell::new(vals))),
                        true, // variadic は保守的に mutable 扱い
                    ));
                }
            }
        }
        Ok(result)
    }

    /// 評価済み引数リストを仮引数リストにバインドして `(name, value, param_mutable, arg_is_mutable)` の
    /// 4要素タプルのリストを返す。
    ///
    /// - `param_mutable`: パラメータが `mut` 宣言されているか
    /// - `arg_is_mutable`: 引数が `mut` 変数から来ているか（デフォルト値は保守的に `true`）
    ///
    /// バインドルール:
    /// - `self_val` が `Some` かつ先頭パラメータが `self` の場合: `self` を先にバインド
    /// - 位置引数: 順番にパラメータスロットに割り当てる
    /// - キーワード引数: パラメータ名で検索してスロットに割り当てる
    /// - 未割り当てスロットでデフォルト値がある場合はデフォルト値を使用する
    /// - 引数数が範囲外の場合や重複キーワードは `TypeError` を返す
    pub(crate) fn bind_args(
        params: &[Param],
        evaled: &[(Option<String>, Value, bool)],
        self_val: Option<Value>,
        defaults: &[Option<Value>],
    ) -> Result<Vec<(String, Value, bool, bool)>, String> {
        let mut result = Vec::new();

        // self_val が Some かつ先頭パラメータが "self" なら先にバインドして残りのパラメータを取得する
        let (params_to_bind, defaults_to_bind) =
            if let (Some(sv), Some(p)) = (&self_val, params.first()) {
                if p.name == "self" {
                    // let self（不変レシーバ）はディープコピーして元オブジェクトの変更を防ぐ
                    let self_to_bind = if p.mutable {
                        sv.clone()
                    } else {
                        Self::deep_copy_value(sv.clone())
                    };
                    // self の arg_is_mutable は特別扱い（常にコピー済みなので false でよい）
                    result.push(("self".to_string(), self_to_bind, p.mutable, false));
                    (&params[1..], &defaults[1..])
                } else {
                    (params, defaults)
                }
            } else {
                (params, defaults)
            };

        // 可変長パラメータを分離する（末尾にのみ存在する）
        let variadic_idx = params_to_bind.iter().position(|p| p.variadic);
        let (non_variadic_params, non_variadic_defaults) = if let Some(vi) = variadic_idx {
            (&params_to_bind[..vi], &defaults_to_bind[..vi])
        } else {
            (params_to_bind, defaults_to_bind)
        };

        // evaled から可変長引数エントリを分離する
        let variadic_value: Option<Value> = evaled
            .iter()
            .find(|(k, _, _)| k.as_deref() == Some("..."))
            .map(|(_, v, _)| v.clone());
        let non_variadic_evaled: Vec<&(Option<String>, Value, bool)> = evaled
            .iter()
            .filter(|(k, _, _)| k.as_deref() != Some("..."))
            .collect();

        // デフォルト値なしのパラメータ数（必須引数数）と最大引数数を計算する
        let required_count = non_variadic_defaults.iter().filter(|d| d.is_none()).count();
        let max_count = non_variadic_params.len();
        if non_variadic_evaled.len() < required_count || non_variadic_evaled.len() > max_count {
            if required_count == max_count {
                return Err(format!(
                    "TypeError: function takes {} argument(s), got {}",
                    max_count,
                    non_variadic_evaled.len()
                ));
            } else {
                return Err(format!(
                    "TypeError: function takes {} to {} argument(s), got {}",
                    required_count,
                    max_count,
                    non_variadic_evaled.len()
                ));
            }
        }

        // パラメータスロットを用意して位置引数・キーワード引数を割り当てる
        let mut slots: Vec<Option<Value>> = vec![None; non_variadic_params.len()];
        // 各スロットに対応する引数の is_mutable。デフォルト値埋めは保守的に true
        let mut slot_is_mutable: Vec<bool> = vec![true; non_variadic_params.len()];
        let mut positional_idx = 0usize;

        for (key, val, is_mut) in &non_variadic_evaled {
            match key {
                None => {
                    // 位置引数: 次のスロットに順番に割り当てる
                    slots[positional_idx] = Some((*val).clone());
                    slot_is_mutable[positional_idx] = *is_mut;
                    positional_idx += 1;
                }
                Some(name) => {
                    // キーワード引数: パラメータ名でスロットを検索して割り当てる
                    let pos = non_variadic_params
                        .iter()
                        .position(|p| p.name == *name)
                        .ok_or_else(|| {
                            format!("TypeError: unexpected keyword argument '{name}'")
                        })?;
                    if slots[pos].is_some() {
                        return Err(format!("TypeError: argument '{name}' given twice"));
                    }
                    slots[pos] = Some((*val).clone());
                    slot_is_mutable[pos] = *is_mut;
                }
            }
        }

        // 未割り当てスロットはデフォルト値で埋める。デフォルト値は保守的に mutable 扱い
        for (i, slot) in slots.into_iter().enumerate() {
            let param = &non_variadic_params[i];
            let v = match slot {
                Some(v) => v,
                None => match &non_variadic_defaults[i] {
                    Some(dv) => dv.clone(),
                    None => return Err(format!("TypeError: missing argument '{}'", param.name)),
                },
            };
            result.push((param.name.clone(), v, param.mutable, slot_is_mutable[i]));
        }

        // 可変長パラメータのバインド: local::args に渡す。保守的に mutable 扱い
        if let Some(vi) = variadic_idx {
            let variadic_param = &params_to_bind[vi];
            let local_args_val = variadic_value.unwrap_or(Value::None);
            result.push(("local::args".to_string(), local_args_val, variadic_param.mutable, true));
        }

        Ok(result)
    }

    /// Python 関数用の引数バインド。`bind_args` と同じだが、パラメータリストにないキーワード引数を
    /// エラーにせず `extra_kwargs` として返す。引数個数の検査は位置引数のみで行う。
    ///
    /// 戻り値: `Ok((bindings, extra_kwargs))` — `bindings` は通常通り、`extra_kwargs` は余分な kwarg の (名前, 値) リスト
    pub(crate) fn bind_args_relaxed(
        params: &[Param],
        evaled: &[(Option<String>, Value, bool)],
        self_val: Option<Value>,
        defaults: &[Option<Value>],
    ) -> Result<(Vec<(String, Value, bool, bool)>, Vec<(String, Value)>), String> {
        let mut result = Vec::new();
        let mut extra_kwargs = Vec::new();

        let (params_to_bind, defaults_to_bind) =
            if let (Some(sv), Some(p)) = (&self_val, params.first()) {
                if p.name == "self" {
                    result.push(("self".to_string(), sv.clone(), p.mutable, false));
                    (&params[1..], &defaults[1..])
                } else {
                    (params, defaults)
                }
            } else {
                (params, defaults)
            };

        // 可変長パラメータを分離する
        let variadic_idx = params_to_bind.iter().position(|p| p.variadic);
        let (non_variadic_params, non_variadic_defaults) = if let Some(vi) = variadic_idx {
            (&params_to_bind[..vi], &defaults_to_bind[..vi])
        } else {
            (params_to_bind, defaults_to_bind)
        };

        // evaled から可変長引数エントリを分離する
        let variadic_value: Option<Value> = evaled
            .iter()
            .find(|(k, _, _)| k.as_deref() == Some("..."))
            .map(|(_, v, _)| v.clone());
        let non_variadic_evaled: Vec<&(Option<String>, Value, bool)> = evaled
            .iter()
            .filter(|(k, _, _)| k.as_deref() != Some("..."))
            .collect();

        let mut slots: Vec<Option<Value>> = vec![None; non_variadic_params.len()];
        let mut slot_is_mutable: Vec<bool> = vec![true; non_variadic_params.len()];
        let mut positional_idx = 0usize;

        for (key, val, is_mut) in &non_variadic_evaled {
            match key {
                None => {
                    if positional_idx >= non_variadic_params.len() {
                        return Err(format!(
                            "TypeError: function takes {} positional argument(s), got too many",
                            non_variadic_params.len()
                        ));
                    }
                    slots[positional_idx] = Some((*val).clone());
                    slot_is_mutable[positional_idx] = *is_mut;
                    positional_idx += 1;
                }
                Some(name) => match non_variadic_params.iter().position(|p| p.name == *name) {
                    Some(pos) => {
                        if slots[pos].is_some() {
                            return Err(format!("TypeError: argument '{name}' given twice"));
                        }
                        slots[pos] = Some((*val).clone());
                        slot_is_mutable[pos] = *is_mut;
                    }
                    None => {
                        extra_kwargs.push((name.clone(), (*val).clone()));
                    }
                },
            }
        }

        for (i, slot) in slots.into_iter().enumerate() {
            let v = match slot {
                Some(v) => v,
                None => match &non_variadic_defaults[i] {
                    Some(dv) => dv.clone(),
                    None => {
                        return Err(format!(
                            "TypeError: missing argument '{}'",
                            non_variadic_params[i].name
                        ))
                    }
                },
            };
            result.push((non_variadic_params[i].name.clone(), v, non_variadic_params[i].mutable, slot_is_mutable[i]));
        }

        // 可変長パラメータのバインド
        if let Some(vi) = variadic_idx {
            let variadic_param = &params_to_bind[vi];
            let local_args_val = variadic_value.unwrap_or(Value::None);
            result.push(("local::args".to_string(), local_args_val, variadic_param.mutable, true));
        }

        Ok((result, extra_kwargs))
    }

    // --- オーバーロード解決 ---

}
