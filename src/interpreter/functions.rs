// functions.rs — 関数・ジェネレータ・オーバーロード実行
// (exec_fn_evaled / exec_fn / exec_generator / eval_call_args / bind_args /
//  dispatch_overload / dispatch_overload_evaled / overload_types_match / value_matches_ann)
//
// 関数・ジェネレータ関数の実行と、オーバーロード解決ロジックを提供する。
// 実行時には独立したスコープを構築し、関数完了後に呼び出し元のスコープを復元する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{CallArg, Param};
use crate::token::Span;

use super::{
    CapturedVar, DictData, ExecResult, FnValue, GeneratorFnValue, GeneratorState, InstanceData,
    Interpreter, StackFrame, Value, Var, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
    RAISE_SENTINEL,
};

impl Interpreter {
    /// 評価済み引数リストを用いて関数を実行する。
    ///
    /// 実行フロー:
    /// 1. `bind_args` で引数を仮引数にバインド
    /// 2. グローバルスコープ以外を一時退避し、新しいローカルスコープを構築
    /// 3. メソッド呼び出しの場合は `Self` を現在のクラスにバインド
    /// 4. 関数本体を実行
    /// 5. スコープを復元
    /// 6. 例外が伝播している場合はトレースバックフレームを追加
    ///
    /// - `fn_val`: 実行する関数定義
    /// - `evaled`: 評価済み引数リスト（位置引数は `None`、キーワード引数は `Some(name)`）
    /// - `self_val`: レシーバインスタンス（メソッド呼び出し時は `Some`、通常関数は `None`）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: `Ok(Value)` — `return` 値または `None`。`Err(message)` — ランタイムエラーまたは例外センチネル
    pub(super) fn exec_fn_evaled(
        &mut self,
        fn_val: Rc<FnValue>,
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        // デフォルト値を事前評価する（self パラメータは常に None）
        let mut evaluated_defaults: Vec<Option<Value>> = Vec::new();
        for p in &fn_val.params {
            if let Some(ref expr) = p.default {
                evaluated_defaults.push(Some(self.eval(expr)?));
            } else {
                evaluated_defaults.push(None);
            }
        }

        let (mut bindings, extra_kwargs) = if fn_val.is_python {
            Self::bind_args_relaxed(
                &fn_val.params,
                evaled,
                self_val.clone(),
                &evaluated_defaults,
            )?
        } else {
            (
                Self::bind_args(
                    &fn_val.params,
                    evaled,
                    self_val.clone(),
                    &evaluated_defaults,
                )?,
                vec![],
            )
        };

        // `let` パラメータ（self 除く）に copy() を適用する: __copy__ があればそれを、なければ deepcopy を使う
        if !fn_val.is_python {
            for binding in &mut bindings {
                let (name, val, mutable) = binding;
                if !*mutable && name != "self" {
                    *val = self.copy_value(val.clone())?;
                }
            }
        }

        // 自動キャスト: `let` パラメータに型アノテーションがあり、渡された値がインスタンスで
        // かつ型が異なる場合、__cast__[TypeName] メソッドが定義されていれば自動的にキャストする。
        // `mut` パラメータは自動キャストしない。
        {
            let params_ref = fn_val.params.clone();
            // 第1パス: キャスト対象を特定する（self は除外）
            let mut cast_targets: Vec<(usize, String, Value)> = Vec::new();
            for (idx, (name, val, mutable)) in bindings.iter().enumerate() {
                if *mutable || name == "self" {
                    continue;
                }
                let type_ann = params_ref
                    .iter()
                    .find(|p| &p.name == name)
                    .and_then(|p| p.type_ann.as_deref());
                if let (Some(type_ann), Value::Instance(inst_rc)) = (type_ann, val) {
                    let class = inst_rc.borrow().class.clone();
                    if class.name != type_ann {
                        let method_key = format!("__cast__[{}]", type_ann);
                        if self.lookup_method_in_class(&class, &method_key).is_some() {
                            cast_targets.push((idx, type_ann.to_string(), val.clone()));
                        }
                    }
                }
            }
            // 第2パス: 実際にキャストを実行して binding を更新する
            for (idx, type_ann, val) in cast_targets {
                let class = match &bindings[idx].1 {
                    Value::Instance(rc) => rc.borrow().class.clone(),
                    _ => continue,
                };
                let method_key = format!("__cast__[{}]", type_ann);
                let overloads = match self.lookup_method_in_class(&class, &method_key) {
                    Some(ov) => ov,
                    None => continue,
                };
                let cast_result = if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(val), "__cast__", None)?
                } else {
                    self.dispatch_overload(overloads, &[], Some(val), None)?
                };
                bindings[idx].1 = cast_result;
            }
        }

        // グローバルスコープ（インデックス 0）以外を一時退避して関数専用スコープを構築する
        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();

        // クロージャキャプチャ環境を先に注入する（パラメータより低い優先度になるよう先にセット）
        for (name, captured) in &fn_val.captured_env {
            let var = match captured {
                CapturedVar::Immutable(v) => Var::new(v.clone(), false),
                CapturedVar::Mutable(cell) => Var::new_cell(cell.clone()),
            };
            self.declare_var(name.clone(), var);
        }

        for (name, val, mutable) in bindings {
            self.declare_var(name, Var::new(val, mutable));
        }
        // Python 関数: 引数リストにない余分なキーワード引数を kwargs dict に注入する
        if fn_val.is_python && !extra_kwargs.is_empty() {
            let mut dict = DictData::new("str".to_string(), "Any".to_string());
            for (k, v) in extra_kwargs {
                dict.set(Value::Str(k), v);
            }
            self.declare_var(
                "kwargs".to_string(),
                Var::new(Value::Dict(Rc::new(RefCell::new(dict))), false),
            );
        }
        // メソッド実行時: `Self` をレシーバインスタンスのクラスにバインドする
        let prev_class = self.current_class.take();
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var(
                "Self".to_string(),
                Var::new(Value::Class(class.clone()), false),
            );
            self.current_class = Some(class);
        }

        // Reset LOOP_DEPTH so that break/continue cannot escape this function's body
        // and cannot accidentally see loop depth from an outer call site.
        let prev_loop_depth = LOOP_DEPTH.with(|d| {
            let prev = *d.borrow();
            *d.borrow_mut() = 0;
            prev
        });

        self.call_stack.push(fn_name.to_string());
        let result = self.exec_block(&fn_val.body);
        self.call_stack.pop();

        LOOP_DEPTH.with(|d| *d.borrow_mut() = prev_loop_depth);

        // アクセス制御コンテキストを復元する
        self.current_class = prev_class;

        // スコープを復元する（グローバルのみ残してから退避分を追記）
        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // Build a caller frame using the call site span (where this function was called from).
        // The caller's name is the last entry in call_stack after we already popped fn_name.
        let caller_frame = {
            let caller_name = self
                .call_stack
                .last()
                .cloned()
                .unwrap_or_else(|| "<module>".to_string());
            match call_span.as_ref() {
                Some(span) => StackFrame {
                    file: span.file.to_string(),
                    line: span.line,
                    col: span.col,
                    fn_name: caller_name,
                    context: self.get_context_lines(&span.file, span.line, 5),
                },
                None => StackFrame {
                    file: String::new(),
                    line: 0,
                    col: 0,
                    fn_name: caller_name,
                    context: String::new(),
                },
            }
        };

        // 例外が ExecResult::Raise として直接返ってきた場合: 呼び出し元フレームを追加してセンチネルを返す
        if let Ok(ExecResult::Raise(mut raised)) = result {
            raised.frames.push(caller_frame);
            self.current_exception = Some(raised);
            return Err(RAISE_SENTINEL.to_string());
        }

        // 例外センチネルが Err として伝播してきた場合（ネストした関数からの raise）: 呼び出し元フレームを追加する
        if let Err(ref e) = result {
            if e.as_str() == RAISE_SENTINEL {
                if let Some(ref mut raised) = self.current_exception {
                    raised.frames.push(caller_frame);
                }
                return Err(RAISE_SENTINEL.to_string());
            }
            // BREAK_SENTINEL should not escape a function — it means break was inside an eval
            // context (e.g., if expression) within a function that has no enclosing loop.
            if e.as_str() == BREAK_SENTINEL {
                return Err("SyntaxError: 'break' outside for/while loop".to_string());
            }
            // 内部エラー文字列を RaisedError に変換してスタックフレームを付加してから伝播する
            let msg = e.clone();
            if let Some(mut raised) = self.make_internal_raised_error(&msg) {
                // Frame[0]: the function where the error occurred (unknown line within it)
                raised.frames.push(StackFrame {
                    file: String::new(),
                    line: 0,
                    col: 0,
                    fn_name: fn_name.to_string(),
                    context: String::new(),
                });
                // Frame[1]: the caller frame (where this function was called from)
                raised.frames.push(caller_frame);
                self.current_exception = Some(raised);
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        match result? {
            ExecResult::Return(v) => Ok(v),
            ExecResult::Normal => Ok(Value::None),
            ExecResult::BlockReturn(_) | ExecResult::BlockYield(_) => {
                Err("SyntaxError: 'block_return' used outside any block expression".to_string())
            }
            ExecResult::Break => Err("SyntaxError: 'break' outside for/while loop".to_string()),
            ExecResult::Continue => Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Raise(_) => unreachable!("Raise already handled above"),
        }
    }

    /// 呼び出し引数式リストを評価してから関数を実行する。`exec_fn_evaled` の呼び出しラッパー。
    ///
    /// - `fn_val`: 実行する関数定義
    /// - `call_args`: 評価前の呼び出し引数リスト（AST の `CallArg`）
    /// - `self_val`: レシーバインスタンス（メソッド用）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: 関数の実行結果（`exec_fn_evaled` と同じ）
    pub(super) fn exec_fn(
        &mut self,
        fn_val: Rc<FnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        self.exec_fn_evaled(fn_val, &evaled, self_val, fn_name, call_span)
    }

    /// ジェネレータ関数の本体を一括実行し、すべての `yield` 値を収集して `Value::Generator` を返す。
    ///
    /// スレッドローカル `GENERATOR_YIELDS` を `Some(Vec::new())` にセットして yield 収集を有効化し、
    /// 本体実行後に収集した値リストを取り出して `GeneratorState` を構築する。
    ///
    /// - `gen_fn`: 実行するジェネレータ関数定義
    /// - `call_args`: 呼び出し引数リスト（AST の `CallArg`）
    /// - `self_val`: レシーバインスタンス（ジェネレータメソッド用; `None` はスタンドアロン）
    ///
    /// 戻り値: `Ok(Value::Generator)` — 収集済みの yield 値を保持するジェネレータ。
    ///         `Err(message)` — ランタイムエラーまたは例外センチネル
    pub(super) fn exec_generator(
        &mut self,
        gen_fn: Rc<GeneratorFnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        let mut evaluated_defaults: Vec<Option<Value>> = Vec::new();
        for p in &gen_fn.params {
            if let Some(ref expr) = p.default {
                evaluated_defaults.push(Some(self.eval(expr)?));
            } else {
                evaluated_defaults.push(None);
            }
        }
        let mut bindings = Self::bind_args(
            &gen_fn.params,
            &evaled,
            self_val.clone(),
            &evaluated_defaults,
        )?;

        // `let` パラメータ（self 除く）に copy() を適用する
        for binding in &mut bindings {
            let (name, val, mutable) = binding;
            if !*mutable && name != "self" {
                *val = self.copy_value(val.clone())?;
            }
        }

        // yield 収集を有効化する（スレッドローカルに収集先を設定）
        GENERATOR_YIELDS.with(|y| {
            *y.borrow_mut() = Some(Vec::new());
        });

        // exec_fn_evaled と同様にグローバルスコープ以外を退避して独立したスコープで実行する
        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();

        // クロージャキャプチャ環境を注入する
        for (name, captured) in &gen_fn.captured_env {
            let var = match captured {
                CapturedVar::Immutable(v) => Var::new(v.clone(), false),
                CapturedVar::Mutable(cell) => Var::new_cell(cell.clone()),
            };
            self.declare_var(name.clone(), var);
        }

        for (name, val, mutable) in bindings {
            self.declare_var(name, Var::new(val, mutable));
        }
        // ジェネレータメソッド実行時: `Self` をレシーバインスタンスのクラスにバインドする
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var("Self".to_string(), Var::new(Value::Class(class), false));
        }
        let prev_loop_depth = LOOP_DEPTH.with(|d| {
            let prev = *d.borrow();
            *d.borrow_mut() = 0;
            prev
        });
        let exec_result = self.exec_block(&gen_fn.body);
        LOOP_DEPTH.with(|d| *d.borrow_mut() = prev_loop_depth);
        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // エラー時も含めて必ずスレッドローカルをクリーンアップして yield 値を回収する
        let yields = GENERATOR_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());

        if let Err(ref e) = exec_result {
            if e.as_str() == BREAK_SENTINEL {
                return Err("SyntaxError: 'break' outside for/while loop".to_string());
            }
        }

        match exec_result? {
            ExecResult::Normal => {}
            ExecResult::BlockReturn(_) | ExecResult::BlockYield(_) => {
                return Err(
                    "SyntaxError: 'block_return' used outside any block expression".to_string(),
                );
            }
            ExecResult::Break => {
                return Err("SyntaxError: 'break' outside for/while loop".to_string())
            }
            ExecResult::Continue => return Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Return(_) => {} // パーサーが gen 内の return を禁止しているためここには到達しない
            ExecResult::Raise(raised) => {
                self.current_exception = Some(raised);
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
            values: yields,
            index: 0,
        }))))
    }

    /// 呼び出し引数リスト（AST の `CallArg`）を評価して `(name, value)` ペアのリストを返す。
    ///
    /// - 位置引数: `(None, value)` として格納
    /// - キーワード引数: `(Some(name), value)` として格納
    ///
    /// - `call_args`: 評価前の呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Vec<(Option<String>, Value)>)` — 評価済み引数リスト。`Err` — 評価エラー
    pub(super) fn eval_call_args(
        &mut self,
        call_args: &[CallArg],
    ) -> Result<Vec<(Option<String>, Value)>, String> {
        let mut result = Vec::new();
        for arg in call_args {
            match arg {
                CallArg::Positional(e) => result.push((None, self.eval(e)?)),
                CallArg::Keyword { name, value } => {
                    result.push((Some(name.clone()), self.eval(value)?))
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
                    ));
                }
            }
        }
        Ok(result)
    }

    /// 評価済み引数リストを仮引数リストにバインドして `(name, value, mutable)` トリプルのリストを返す。
    ///
    /// バインドルール:
    /// - `self_val` が `Some` かつ先頭パラメータが `self` の場合: `self` を先にバインド
    /// - 位置引数: 順番にパラメータスロットに割り当てる
    /// - キーワード引数: パラメータ名で検索してスロットに割り当てる
    /// - 未割り当てスロットでデフォルト値がある場合はデフォルト値を使用する
    /// - 引数数が範囲外の場合や重複キーワードは `TypeError` を返す
    ///
    /// - `params`: 仮引数リスト
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（`None` の場合は通常の引数バインド）
    /// - `defaults`: `params` と並行な事前評価済みデフォルト値リスト（`self` を含む全パラメータ分）
    ///
    /// 戻り値: `Ok(Vec<(name, value, mutable)>)` — バインド済みリスト。`Err` — 引数エラー
    pub(super) fn bind_args(
        params: &[Param],
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
        defaults: &[Option<Value>],
    ) -> Result<Vec<(String, Value, bool)>, String> {
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
                    result.push(("self".to_string(), self_to_bind, p.mutable));
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
            .find(|(k, _)| k.as_deref() == Some("..."))
            .map(|(_, v)| v.clone());
        let non_variadic_evaled: Vec<&(Option<String>, Value)> = evaled
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
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
        let mut positional_idx = 0usize;

        for (key, val) in &non_variadic_evaled {
            match key {
                None => {
                    // 位置引数: 次のスロットに順番に割り当てる
                    slots[positional_idx] = Some((*val).clone());
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
                }
            }
        }

        // 未割り当てスロットはデフォルト値で埋める。コピーは呼び出し元（exec_fn_evaled / exec_generator）が行う
        for (i, slot) in slots.into_iter().enumerate() {
            let param = &non_variadic_params[i];
            let v = match slot {
                Some(v) => v,
                None => match &non_variadic_defaults[i] {
                    Some(dv) => dv.clone(),
                    None => return Err(format!("TypeError: missing argument '{}'", param.name)),
                },
            };
            result.push((param.name.clone(), v, param.mutable));
        }

        // 可変長パラメータのバインド: local::args に渡す。コピーは呼び出し元が行う
        if let Some(vi) = variadic_idx {
            let variadic_param = &params_to_bind[vi];
            let local_args_val = variadic_value.unwrap_or(Value::None);
            result.push(("local::args".to_string(), local_args_val, variadic_param.mutable));
        }

        Ok(result)
    }

    /// Python 関数用の引数バインド。`bind_args` と同じだが、パラメータリストにないキーワード引数を
    /// エラーにせず `extra_kwargs` として返す。引数個数の検査は位置引数のみで行う。
    ///
    /// 戻り値: `Ok((bindings, extra_kwargs))` — `bindings` は通常通り、`extra_kwargs` は余分な kwarg の (名前, 値) リスト
    pub(super) fn bind_args_relaxed(
        params: &[Param],
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
        defaults: &[Option<Value>],
    ) -> Result<(Vec<(String, Value, bool)>, Vec<(String, Value)>), String> {
        let mut result = Vec::new();
        let mut extra_kwargs = Vec::new();

        let (params_to_bind, defaults_to_bind) =
            if let (Some(sv), Some(p)) = (&self_val, params.first()) {
                if p.name == "self" {
                    result.push(("self".to_string(), sv.clone(), p.mutable));
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
            .find(|(k, _)| k.as_deref() == Some("..."))
            .map(|(_, v)| v.clone());
        let non_variadic_evaled: Vec<&(Option<String>, Value)> = evaled
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();

        let mut slots: Vec<Option<Value>> = vec![None; non_variadic_params.len()];
        let mut positional_idx = 0usize;

        for (key, val) in &non_variadic_evaled {
            match key {
                None => {
                    if positional_idx >= non_variadic_params.len() {
                        return Err(format!(
                            "TypeError: function takes {} positional argument(s), got too many",
                            non_variadic_params.len()
                        ));
                    }
                    slots[positional_idx] = Some((*val).clone());
                    positional_idx += 1;
                }
                Some(name) => match non_variadic_params.iter().position(|p| p.name == *name) {
                    Some(pos) => {
                        if slots[pos].is_some() {
                            return Err(format!("TypeError: argument '{name}' given twice"));
                        }
                        slots[pos] = Some((*val).clone());
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
            result.push((non_variadic_params[i].name.clone(), v, non_variadic_params[i].mutable));
        }

        // 可変長パラメータのバインド
        if let Some(vi) = variadic_idx {
            let variadic_param = &params_to_bind[vi];
            let local_args_val = variadic_value.unwrap_or(Value::None);
            result.push(("local::args".to_string(), local_args_val, variadic_param.mutable));
        }

        Ok((result, extra_kwargs))
    }

    // --- オーバーロード解決 ---

    /// 呼び出し引数を評価してからオーバーロード候補を解決して実行する。
    /// 引数の評価は一度だけ行い、`dispatch_overload_evaled` に委譲する。
    ///
    /// - `candidates`: オーバーロード候補の関数リスト
    /// - `args`: 呼び出し引数リスト（評価前）
    /// - `self_val`: レシーバインスタンス（メソッド用）
    ///
    /// 戻り値: 選択されたオーバーロードの実行結果
    pub(super) fn dispatch_overload(
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
    pub(super) fn dispatch_overload_evaled(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        evaled: Vec<(Option<String>, Value)>,
        self_val: Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        // 可変長引数を除いた通常引数の数
        let call_count = evaled.iter().filter(|(k, _)| k.as_deref() != Some("...")).count();
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
    pub(super) fn overload_types_match(
        fn_val: &FnValue,
        evaled: &[(Option<String>, Value)],
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
        let non_variadic_evaled: Vec<&(Option<String>, Value)> = evaled
            .iter()
            .filter(|(k, _)| k.as_deref() != Some("..."))
            .collect();

        // 各引数をパラメータスロットに割り当てる（bind_args と同様のロジック）
        let mut slots: Vec<Option<&Value>> = vec![None; params.len()];
        let mut positional_idx = 0usize;

        for (key, val) in &non_variadic_evaled {
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
    pub(super) fn value_matches_ann(val: &Value, ann: &str) -> bool {
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

    /// 参照型の値を再帰的にディープコピーして返す。
    ///
    /// `let` パラメータへのバインド時に呼ばれ、元の可変変数（`mut`）が
    /// 関数内部から変更されることを防ぐ。
    ///
    /// 変換規則:
    /// - `Instance`: フィールドを再帰コピーして新しい `InstanceData` を生成する
    /// - `Dict`: キー・値を再帰コピーして新しい `DictData` を生成する
    /// - `List`: 各要素を再帰コピーする
    /// - その他: プリミティブ・不変型はそのまま返す（Rust の clone でコピー済み）
    pub(crate) fn deep_copy_value(val: Value) -> Value {
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let new_fields = inst
                    .fields
                    .iter()
                    .map(|slot| slot.as_ref().map(|(v, m)| (Self::deep_copy_value(v.clone()), *m)))
                    .collect();
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    class: inst.class.clone(),
                    fields: new_fields,
                    immutable: inst.immutable,
                })))
            }
            Value::Dict(d) => {
                let d_ref = d.borrow();
                let mut new_dict = DictData::new(d_ref.key_type.clone(), d_ref.item_type.clone());
                for (k, v) in d_ref.all_keys().into_iter().zip(d_ref.all_items()) {
                    new_dict.set(Self::deep_copy_value(k), Self::deep_copy_value(v));
                }
                Value::Dict(Rc::new(RefCell::new(new_dict)))
            }
            Value::List(items) => Value::List(Rc::new(RefCell::new(
                items
                    .borrow()
                    .iter()
                    .cloned()
                    .map(Self::deep_copy_value)
                    .collect(),
            ))),
            // Tuple は Rc<TupleData> だが TupleData は不変なので共有で問題なし
            // プリミティブ・関数・クラス等はそのまま返す
            other => other,
        }
    }

    /// `copy()` メソッド用のディープコピー。フリーズ状態をリセットして新鮮な可変インスタンスを返す。
    ///
    /// `deep_copy_value` との違い:
    /// - `Instance`: `immutable = false` に設定し、フィールドの可変性をクラス定義から復元する
    ///   （`let` バインドによるフリーズを解除した独立したコピーを生成する）
    /// - `Dict` / `List`: `deep_copy_value` と同様に再帰コピーする
    /// - その他: `deep_copy_value` と同様にそのまま返す
    pub(crate) fn deep_copy_unfrozen(val: Value) -> Value {
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let class = inst.class.clone();
                let new_fields = inst
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(idx, slot)| {
                        slot.as_ref().map(|(v, _)| {
                            // クラス定義の可変性を復元する: field_mutability_vec の元の値を使う
                            let orig_mutable = class.field_mutability_vec
                                .get(idx)
                                .copied()
                                .unwrap_or(true);
                            (Self::deep_copy_unfrozen(v.clone()), orig_mutable)
                        })
                    })
                    .collect();
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    class,
                    fields: new_fields,
                    immutable: false, // フリーズを解除した新鮮なコピー
                })))
            }
            Value::Dict(d) => {
                let d_ref = d.borrow();
                let mut new_dict = DictData::new(d_ref.key_type.clone(), d_ref.item_type.clone());
                for (k, v) in d_ref.all_keys().into_iter().zip(d_ref.all_items()) {
                    new_dict.set(Self::deep_copy_unfrozen(k), Self::deep_copy_unfrozen(v));
                }
                Value::Dict(Rc::new(RefCell::new(new_dict)))
            }
            Value::List(items) => Value::List(Rc::new(RefCell::new(
                items
                    .borrow()
                    .iter()
                    .cloned()
                    .map(Self::deep_copy_unfrozen)
                    .collect(),
            ))),
            other => other,
        }
    }
}
