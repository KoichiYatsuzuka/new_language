// functions/overload.rs — オーバーロードのディスパッチと型照合: dispatch_overload(_evaled) / overload_types_match / value_matches_ann。

use {
    std::rc::Rc,
    crate::ast::{CallArg, Param},
    crate::token::Span,
    crate::interpreter::{
        FnValue,
        Interpreter, Value,
    },
};

impl Interpreter {
    /// 呼び出し引数を評価してからオーバーロード候補を解決して実行する。
    /// 引数の評価は一度だけ行い、`dispatch_overload_evaled` に委譲する。
    ///
    /// - `candidates`: オーバーロード候補の関数リスト
    /// - `args`: 呼び出し引数リスト（評価前）
    /// - `self_val`: レシーバインスタンス（メソッド用）
    ///
    /// 戻り値: 選択されたオーバーロードの実行結果
    pub(crate) fn dispatch_overload(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        args: &[CallArg],
        self_val: Option<Value>,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        self.dispatch_overload_evaled(candidates, evaled, self_val, "<overloaded>", call_span)
    }

    /// 評価済み引数リストを用いてオーバーロード候補から適合する関数を選択して実行する。
    ///
    /// 解決アルゴリズム:
    /// 1. `self` を除いた有効引数数でフィルタリング
    /// 2. 引数数が一致する候補が1つなら即実行
    /// 3. 複数一致する場合は型アノテーションと引数型を照合（`overload_types_match`）
    /// 4. 型一致候補が見つからない場合は引数数一致の先頭候補にフォールバック
    ///
    /// - `candidates`: オーバーロード候補リスト
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（メソッド用）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: 選択されたオーバーロードの実行結果。`Err` — 引数数不一致など
    pub(crate) fn dispatch_overload_evaled(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        evaled: Vec<(Option<String>, Value, bool)>,
        self_val: Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        // 可変長引数を除いた通常引数の数
        let call_count = evaled.iter().filter(|(k, _, _)| k.as_deref() != Some("...")).count();
        let has_self = self_val.is_some();

        // `self` パラメータと可変長パラメータを除いた有効引数数の範囲（必須数, 最大数）を返すクロージャ
        let effective_param_range = |f: &FnValue| -> (usize, usize) {
            let self_offset =
                if has_self && f.params.first().map(|p| p.name == "self").unwrap_or(false) {
                    1
                } else {
                    0
                };
            let params = &f.params[self_offset..];
            let non_variadic: Vec<_> = params.iter().filter(|p| !p.variadic).collect();
            let required = non_variadic.iter().filter(|p| p.default.is_none()).count();
            (required, non_variadic.len())
        };

        // 呼び出し引数数が有効範囲に収まる候補のみに絞り込む
        let count_matching: Vec<Rc<FnValue>> = candidates
            .iter()
            .filter(|f| {
                let (req, max) = effective_param_range(f);
                call_count >= req && call_count <= max
            })
            .cloned()
            .collect();

        if count_matching.is_empty() {
            let available: Vec<String> = candidates
                .iter()
                .map(|f| {
                    let (req, max) = effective_param_range(f);
                    if req == max {
                        req.to_string()
                    } else {
                        format!("{req}-{max}")
                    }
                })
                .collect();
            return Err(format!(
                "TypeError: no overload takes {} argument(s) (overloads take: {})",
                call_count,
                available.join(", ")
            ));
        }

        if count_matching.len() == 1 {
            return self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name, call_span);
        }

        // 引数数が複数一致: 型アノテーションと引数型を照合して候補を絞り込む
        for candidate in &count_matching {
            if Self::overload_types_match(candidate, &evaled, &self_val) {
                return self.exec_fn_evaled(candidate.clone(), &evaled, self_val.clone(), fn_name, call_span.clone());
            }
        }

        // 型一致候補なし: 引数数一致の先頭候補にフォールバック
        self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name, call_span)
    }

    /// 関数のすべてのアノテーション付きパラメータが対応する引数値と型一致するか判定する。
    ///
    /// アノテーションのないパラメータは型チェックをスキップする。
    /// 型一致の判定には `value_matches_ann` を使用する。
    ///
    /// - `fn_val`: 型チェック対象の関数定義
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（`Some` の場合は `self` パラメータをスキップ）
    ///
    /// 戻り値: `true` — すべてのアノテーション付きパラメータが一致
    pub(crate) fn overload_types_match(
        fn_val: &FnValue,
        evaled: &[(Option<String>, Value, bool)],
        self_val: &Option<Value>,
    ) -> bool {
        // self_val がある場合は `self` パラメータをスキップして残りを対象にする
        let all_params = if self_val.is_some()
            && fn_val
                .params
                .first()
                .map(|p| p.name == "self")
                .unwrap_or(false)
        {
            &fn_val.params[1..]
        } else {
            &fn_val.params[..]
        };
        // 可変長パラメータを除いた通常パラメータのみを対象にする
        let params: Vec<&Param> = all_params.iter().filter(|p| !p.variadic).collect();

        // evaled から可変長引数エントリを除いた通常引数のみを対象にする
        let non_variadic_evaled: Vec<&(Option<String>, Value, bool)> = evaled
            .iter()
            .filter(|(k, _, _)| k.as_deref() != Some("..."))
            .collect();

        // 各引数をパラメータスロットに割り当てる（bind_args と同様のロジック）
        let mut slots: Vec<Option<&Value>> = vec![None; params.len()];
        let mut positional_idx = 0usize;

        for (key, val, _) in &non_variadic_evaled {
            match key {
                None => {
                    if positional_idx >= params.len() {
                        return false;
                    }
                    slots[positional_idx] = Some(val);
                    positional_idx += 1;
                }
                Some(name) => {
                    if let Some(pos) = params.iter().position(|p| p.name == *name) {
                        slots[pos] = Some(val);
                    } else {
                        return false;
                    }
                }
            }
        }

        // アノテーション付きパラメータについてのみ型一致を確認する
        for (i, slot) in slots.iter().enumerate() {
            if let (Some(val), Some(ann)) = (slot, &params[i].type_ann) {
                if !Self::value_matches_ann(val, ann) {
                    return false;
                }
            }
        }
        true
    }

    /// 値が型アノテーション名と一致するかを判定する（オーバーロード解決用）。
    ///
    /// - `val`: チェック対象の値
    /// - `ann`: パラメータの型アノテーション名
    ///
    /// 戻り値: `true` — 型が一致する
    pub(crate) fn value_matches_ann(val: &Value, ann: &str) -> bool {
        // `tuple` アノテーションは任意の Tuple 値に一致する（要素数・型は問わない）
        if ann == "tuple" && matches!(val, Value::Tuple(_)) {
            return true;
        }
        // `type[X]`: 型値がアノテーション内の型名と一致するか確認する（オーバーロード解決用）
        if let Some(inner) = ann.strip_prefix("type[").and_then(|s| s.strip_suffix(']')) {
            return match val {
                Value::Type(name) => name == inner,
                Value::Class(c) => c.name == inner,
                _ => false,
            };
        }
        matches!(
            (ann, val),
            ("int", Value::Int(_))
                | ("float", Value::Float(_))
                | ("str", Value::Str(_))
                | ("bool", Value::Bool(_))
                | ("None", Value::None)
                | ("list", Value::List(_))
                | ("fixed_list", Value::FrozenList { .. })
                | ("list_like", Value::List(_))
                | ("list_like", Value::FrozenList { .. })
                | ("type", Value::Type(_))
                | ("type", Value::Class(_))
                | ("Self", Value::Instance(_))
        )
    }

}
