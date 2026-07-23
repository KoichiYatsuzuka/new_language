// functions/execution.rs — 関数・ジェネレータの実行: exec_fn_evaled / exec_fn / exec_generator。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::token::Span,
    crate::interpreter::{
        CapturedVar, DictData, ExecResult, FnValue, GeneratorFnValue, GeneratorState,
        Interpreter, StackFrame, Value, Var, BREAK_SENTINEL, GENERATOR_YIELDS, LOOP_DEPTH,
        RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// 呼び出し元スタックフレーム（トレースバック用）を構築する。
    /// `call_stack` から呼び出し元名を、`call_span` から位置とコンテキスト行を取る。
    pub(crate) fn build_caller_frame(&self, call_span: Option<&Span>) -> StackFrame {
        let caller_name = self
            .call_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<module>".to_string());
        match call_span {
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
    }

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
    pub(crate) fn exec_fn_evaled(
        &mut self,
        fn_val: Rc<FnValue>,
        evaled: &[(Option<String>, Value, bool)],
        self_val: Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        // デフォルト値を事前評価する（self パラメータは常に None）。
        // デフォルトを持つ仮引数が1つもなければ Vec 確保を省く（bind_args は空 defaults を許容する）。
        let evaluated_defaults: Vec<Option<Value>> = if fn_val.params.iter().any(|p| p.default.is_some()) {
            let mut v = Vec::with_capacity(fn_val.params.len());
            for p in &fn_val.params {
                v.push(match &p.default {
                    Some(expr) => Some(self.eval(expr)?),
                    None => None,
                });
            }
            v
        } else {
            Vec::new()
        };

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

        // `let` パラメータ（self 除く）に copy() を適用する。
        // - let param + mutable arg → deepcopy（呼び出し元の mut 変数を保護）
        // - let param + let arg   → コピー不要（呼び出し元が不変ならエイリアス共有で安全）
        // - mut param             → コピー不要（参照共有が意図された動作）
        if !fn_val.is_python {
            for binding in &mut bindings {
                let (name, val, param_mutable, arg_is_mutable) = binding;
                if !*param_mutable && name != "self" && *arg_is_mutable {
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
            for (idx, (name, val, mutable, _)) in bindings.iter().enumerate() {
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

        // ── Phase V: バイトコード VM 経路（デュアルモード, D2） ──
        // フリー関数（self なし・クロージャなし・非 Python）だけを対象に、初回にコンパイルして
        // Chunk をキャッシュする。コンパイルできなければ None を焼き込みツリーウォークへ。
        if self_val.is_none()
            && fn_val.captured_env.is_empty()
            && !fn_val.is_python
            && self.vm_mode != crate::vm::VmMode::Off
        {
            let key = Rc::as_ptr(&fn_val) as usize;
            let chunk_opt = match self.vm_chunks.get(&key) {
                Some(c) => c.clone(),
                None => {
                    let compiled =
                        crate::vm::compile_fn(&fn_val.params, &fn_val.body).map(Rc::new);
                    self.vm_chunks.insert(key, compiled.clone());
                    compiled
                }
            };
            if let Some(chunk) = chunk_opt {
                // 共有バッファを借り出し、base.. に locals を確保（per-call 確保なし）。
                let mut buf = std::mem::take(&mut self.vm_stack);
                let base = buf.len();
                buf.resize(base + chunk.n_locals, Value::None);
                for (i, (_, val, _, _)) in bindings.iter().enumerate() {
                    if i < chunk.n_locals {
                        buf[base + i] = val.clone();
                    }
                }
                self.call_stack.push(fn_name.to_string());
                let result = crate::vm::run(self, &chunk, &mut buf, base);
                self.call_stack.pop();
                buf.truncate(base);
                self.vm_stack = buf;
                let caller_frame = self.build_caller_frame(call_span.as_ref());
                return match result {
                    Ok(v) => Ok(v),
                    Err(e) if e.as_str() == RAISE_SENTINEL => {
                        if let Some(raised) = self.current_exception.as_mut() {
                            raised.frames.push(caller_frame);
                        }
                        Err(RAISE_SENTINEL.to_string())
                    }
                    Err(msg) => {
                        if let Some(mut raised) = self.make_internal_raised_error(&msg) {
                            raised.frames.push(StackFrame {
                                file: String::new(),
                                line: 0,
                                col: 0,
                                fn_name: fn_name.to_string(),
                                context: String::new(),
                            });
                            raised.frames.push(caller_frame);
                            self.current_exception = Some(raised);
                            Err(RAISE_SENTINEL.to_string())
                        } else {
                            Err(msg)
                        }
                    }
                };
            }
        }

        // 関数フレームへ切り替える: frame_floor を現在の scopes 長に進め（＝これから push する
        // base スコープの index）、呼び出し元のローカルを隔離する。drain/退避/復元の Vec 確保は不要。
        let saved_floor = self.frame_floor;
        let saved_len = self.scopes.len();
        self.frame_floor = saved_len;
        self.push_scope();

        // クロージャキャプチャ環境を先に注入する（パラメータより低い優先度になるよう先にセット）
        for (name, captured) in &fn_val.captured_env {
            let var = match captured {
                CapturedVar::Immutable(v) => Var::new(v.clone(), false),
                CapturedVar::Mutable(cell) => Var::new_cell(cell.clone()),
            };
            self.declare_var(name.clone(), var);
        }

        for (name, val, mutable, _) in bindings {
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

        // フレームを復元する: base とブロックを切り捨てて呼び出し元の状態に戻す（Vec 確保なし）。
        self.scopes.truncate(saved_len);
        self.frame_floor = saved_floor;

        // Build a caller frame using the call site span (where this function was called from).
        let caller_frame = self.build_caller_frame(call_span.as_ref());

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
    pub(crate) fn exec_fn(
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
    pub(crate) fn exec_generator(
        &mut self,
        gen_fn: Rc<GeneratorFnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        let evaluated_defaults: Vec<Option<Value>> = if gen_fn.params.iter().any(|p| p.default.is_some()) {
            let mut v = Vec::with_capacity(gen_fn.params.len());
            for p in &gen_fn.params {
                v.push(match &p.default {
                    Some(expr) => Some(self.eval(expr)?),
                    None => None,
                });
            }
            v
        } else {
            Vec::new()
        };
        let mut bindings = Self::bind_args(
            &gen_fn.params,
            &evaled,
            self_val.clone(),
            &evaluated_defaults,
        )?;

        // `let` パラメータ（self 除く）に copy() を適用する
        // let param + mutable arg → deepcopy、let param + let arg → スキップ
        for binding in &mut bindings {
            let (name, val, param_mutable, arg_is_mutable) = binding;
            if !*param_mutable && name != "self" && *arg_is_mutable {
                *val = self.copy_value(val.clone())?;
            }
        }

        // yield 収集を有効化する（スレッドローカルに収集先を設定）
        GENERATOR_YIELDS.with(|y| {
            *y.borrow_mut() = Some(Vec::new());
        });

        // exec_fn_evaled と同様に frame_floor を進めて呼び出し元ローカルを隔離する（Vec 確保なし）
        let saved_floor = self.frame_floor;
        let saved_len = self.scopes.len();
        self.frame_floor = saved_len;
        self.push_scope();

        // クロージャキャプチャ環境を注入する
        for (name, captured) in &gen_fn.captured_env {
            let var = match captured {
                CapturedVar::Immutable(v) => Var::new(v.clone(), false),
                CapturedVar::Mutable(cell) => Var::new_cell(cell.clone()),
            };
            self.declare_var(name.clone(), var);
        }

        for (name, val, mutable, _) in bindings {
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
        self.scopes.truncate(saved_len);
        self.frame_floor = saved_floor;

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

}
