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
    /// 呼び出し名スタックへ push する（#12）。
    /// プールに空きバッファがあれば確保せず詰め直す。
    #[inline]
    fn push_call_name(&mut self, fn_name: &str) {
        match self.call_name_pool.pop() {
            Some(mut buf) => {
                buf.clear();
                buf.push_str(fn_name);
                self.call_stack.push(buf);
            }
            None => self.call_stack.push(fn_name.to_string()),
        }
    }

    /// 呼び出し名スタックから pop し、バッファをプールへ返す（#12）。
    #[inline]
    fn pop_call_name(&mut self) {
        if let Some(buf) = self.call_stack.pop() {
            self.call_name_pool.push(buf);
        }
    }

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

    /// fn_val に対応する VM Chunk を取得（なければコンパイルしてキャッシュ）。
    /// キャッシュキーは `Rc::as_ptr`。`Weak` 検証でアドレス再利用（テンプレート一時 fn_val）を弾く。
    fn get_or_compile_chunk(&mut self, fn_val: &Rc<FnValue>) -> Option<Rc<crate::vm::Chunk>> {
        let key = Rc::as_ptr(fn_val) as usize;
        match self.vm_chunks.get(&key) {
            Some((weak, cached)) if weak.upgrade().is_some() => cached.clone(),
            _ => {
                // 不変キャプチャの名前を渡す（#27-d）。コンパイラが末尾に slot を採番し、
                // 呼び出し側が `chunk.captured_slots` を見て値を書き込む。
                // 可変キャプチャを含む場合は `vm_eligible` が偽なのでここへ来ない。
                // 不変キャプチャは slot へ、**可変キャプチャはセルへ**（#27-d 段階 2b）。
                let mut captures: Vec<String> = Vec::new();
                let mut mut_captures: Vec<String> = Vec::new();
                for (n, c) in &fn_val.captured_env {
                    match c {
                        CapturedVar::Immutable(_) => captures.push(n.clone()),
                        CapturedVar::Mutable(_) => mut_captures.push(n.clone()),
                    }
                }
                let compiled = crate::vm::compile_fn(
                    &fn_val.params,
                    &fn_val.body,
                    self.annotations.clone(),
                    &captures,
                    &mut_captures,
                )
                .map(Rc::new);
                if crate::interpreter::tw_stats::enabled() {
                    crate::interpreter::tw_stats::record_compile("fn", compiled.is_some());
                }
                self.vm_chunks
                    .insert(key, (Rc::downgrade(fn_val), compiled.clone()));
                compiled
            }
        }
    }

    /// ジェネレータ本体に対応する VM Chunk を取得（なければコンパイルしてキャッシュ, タスク #8）。
    /// `get_or_compile_chunk` の `GeneratorFnValue` 版。`Weak` でアドレス再利用を弾く。
    fn get_or_compile_gen_chunk(
        &mut self,
        gen_fn: &Rc<GeneratorFnValue>,
    ) -> Option<Rc<crate::vm::Chunk>> {
        let key = Rc::as_ptr(gen_fn) as usize;
        match self.vm_gen_chunks.get(&key) {
            Some((weak, cached)) if weak.upgrade().is_some() => cached.clone(),
            _ => {
                let compiled = crate::vm::compile_fn(
                    &gen_fn.params,
                    &gen_fn.body,
                    self.annotations.clone(),
                    &[], // ジェネレータのクロージャ化は未対応（従来どおり）
                    &[],
                )
                .map(Rc::new);
                if crate::interpreter::tw_stats::enabled() {
                    crate::interpreter::tw_stats::record_compile("gen", compiled.is_some());
                }
                self.vm_gen_chunks
                    .insert(key, (Rc::downgrade(gen_fn), compiled.clone()));
                compiled
            }
        }
    }

    /// VM の `Yield` op 用: 値をジェネレータの yield 収集バッファ（`GENERATOR_YIELDS`）へ追加する。
    /// ツリーウォークの `Stmt::Yield`（dispatch.rs）と同一意味論。収集が無効（`None`）なら何もしない。
    pub(crate) fn vm_yield_push(&self, val: Value) {
        GENERATOR_YIELDS.with(|y| {
            if let Some(yields) = y.borrow_mut().as_mut() {
                yields.push(val);
            }
        });
    }

    /// バインド済みのジェネレータ本体を VM で実行し、eager 収集した yield 値から `Value::Generator` を作る。
    /// yield 収集は `GENERATOR_YIELDS`（ツリーウォークと共有）を使うので意味論一致。エラーは生の `Err` を
    /// 伝播（`exec_generator_evaled` のツリーウォーク経路と同じ・RAISE_SENTINEL は `current_exception` 設定済み）。
    fn run_vm_generator(
        &mut self,
        chunk: &crate::vm::Chunk,
        bindings: Vec<(String, Value, bool, bool)>,
        self_val: &Option<Value>,
    ) -> Result<Value, String> {
        GENERATOR_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        // 共有バッファへ locals を確保し、バインディングを slot へ詰める（self は slot 0）。
        let mut buf = std::mem::take(&mut self.vm_stack);
        let base = buf.len();
        buf.resize(base + chunk.n_locals, Value::None);
        for (i, (_, val, _, _)) in bindings.into_iter().enumerate() {
            if i < chunk.n_locals {
                buf[base + i] = val;
            }
        }
        // ジェネレータメソッド: アクセス制御・Self 依存ディスパッチのため current_class を張る。
        let prev_class = self.current_class.take();
        if let Some(Value::Instance(inst_rc)) = self_val {
            self.current_class = Some(inst_rc.borrow().class.clone());
        }
        let result = crate::vm::run(self, chunk, &mut buf, base, None);
        self.current_class = prev_class;
        buf.truncate(base);
        self.vm_stack = buf;
        // エラー時も含めて必ず yield 値を回収してクリーンアップする。
        let yields = GENERATOR_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        match result {
            Ok(_) => Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: yields,
                index: 0,
            })))),
            Err(e) => Err(e),
        }
    }

    /// バインド済みバッファで VM Chunk を実行し、戻り値／例外フレームを組み立てる（fast/general 共通）。
    /// `buf[base..base+n_locals]` にパラメータが束縛済み。`current_class` はメソッド実行のため一時設定する。
    // 引数が多いのは VM フレームの起動に要る素材がそのまま並んでいるから
    // （chunk・バッファ・base・self・表示名・呼び出し位置・捕捉環境）。
    #[allow(clippy::too_many_arguments)]
    fn run_vm_method(
        &mut self,
        chunk: &crate::vm::Chunk,
        mut buf: Vec<Value>,
        base: usize,
        self_val: &Option<Value>,
        fn_name: &str,
        call_span: Option<Span>,
        // クロージャの捕捉環境（#27-d 段階 2b）。可変キャプチャのセルを共有するために渡す。
        captured_env: Option<&std::collections::HashMap<String, CapturedVar>>,
    ) -> Result<Value, String> {
        let prev_class = self.current_class.take();
        if let Some(Value::Instance(inst_rc)) = self_val {
            self.current_class = Some(inst_rc.borrow().class.clone());
        }
        self.push_call_name(fn_name);
        let result = crate::vm::run(self, chunk, &mut buf, base, captured_env);
        self.pop_call_name();
        self.current_class = prev_class;
        buf.truncate(base);
        self.vm_stack = buf;
        // ⚠ `build_caller_frame` は**エラー経路でしか使わない**ので遅延させる（#12）。
        // 以前は成功時にも毎回作っており、その中で `get_context_lines` が
        // **ソース 5 行を `join` して String を確保**していた（＋呼び出し元名の String clone、
        // ＋`span.file.to_string()`）。呼び出しごとに 3 回のヒープ確保を無駄に払っていた。
        match result {
            Ok(v) => Ok(v),
            Err(e) if e.as_str() == RAISE_SENTINEL => {
                if self.current_exception.is_some() {
                    let caller_frame = self.build_caller_frame(call_span.as_ref());
                    if let Some(raised) = self.current_exception.as_mut() {
                        raised.frames.push(caller_frame);
                    }
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
                    raised.frames.push(self.build_caller_frame(call_span.as_ref()));
                    self.current_exception = Some(raised);
                    Err(RAISE_SENTINEL.to_string())
                } else {
                    Err(msg)
                }
            }
        }
    }

    /// 高速バインド（タスク #4）: 単純シグネチャの VM 呼び出しで `bind_args` の Vec 確保・名前 clone・
    /// copy/cast の各パスを飛ばし、引数を直接バッファへ束縛する。コピー意味論は `bind_args` ＋ copy ループ
    /// と同一（self 非 mut → deep_copy、let パラメータ + mut 引数 → copy_value）。
    ///
    /// 対応条件（いずれか外れれば `Ok(None)` で一般経路へ）: 可変長なし・デフォルトなし・キーワード引数なし・
    /// 実引数数が仮引数数（self 除く）と完全一致・キャスト不要（let パラメータの型注釈と Instance 引数の
    /// クラス名が一致 or 非 Instance）。
    fn try_fast_bind(
        &mut self,
        fn_val: &Rc<FnValue>,
        chunk: &crate::vm::Chunk,
        evaled: &[(Option<String>, Value, bool)],
        self_val: &Option<Value>,
    ) -> Result<Option<(Vec<Value>, usize)>, String> {
        let params = &fn_val.params;
        let has_self = self_val.is_some() && params.first().is_some_and(|p| p.name == "self");
        let bind_params = if has_self { &params[1..] } else { &params[..] };

        // 単純シグネチャの判定。
        if params.iter().any(|p| p.variadic || p.default.is_some()) {
            return Ok(None);
        }
        if evaled.len() != bind_params.len() {
            return Ok(None);
        }
        if evaled.iter().any(|(k, _, _)| k.is_some()) {
            return Ok(None);
        }
        // キャストの可能性（let パラメータ + 型注釈 + クラス名不一致の Instance 引数）があれば一般経路へ。
        for (p, (_, val, _)) in bind_params.iter().zip(evaled.iter()) {
            if !p.mutable {
                if let (Some(ta), Value::Instance(rc)) = (&p.type_ann, val) {
                    if &rc.borrow().class.name != ta {
                        return Ok(None);
                    }
                }
            }
        }

        // 直接バインド。
        let mut buf = std::mem::take(&mut self.vm_stack);
        let base = buf.len();
        buf.resize(base + chunk.n_locals, Value::None);
        let mut slot = 0usize;
        if has_self {
            let sv = self_val.as_ref().unwrap();
            // 非 mut self は deep_copy（bind_args と同一。エイリアス／変異から呼び出し元を保護）。
            let self_bound = if params[0].mutable {
                sv.clone()
            } else {
                Self::deep_copy_value(sv.clone())
            };
            if slot < chunk.n_locals {
                buf[base + slot] = self_bound;
            }
            slot += 1;
        }
        for (p, (_, val, arg_mut)) in bind_params.iter().zip(evaled.iter()) {
            // let パラメータ + mut 引数 → copy_value（copy ループと同一）。それ以外は共有。
            let v = if !p.mutable && *arg_mut {
                self.copy_value(val.clone())?
            } else {
                val.clone()
            };
            if slot < chunk.n_locals {
                buf[base + slot] = v;
            }
            slot += 1;
        }
        Self::bind_captures(fn_val, chunk, &mut buf, base);
        Ok(Some((buf, base)))
    }

    /// クロージャの不変キャプチャをフレームの slot へ書き込む（#27-d）。
    ///
    /// ツリーウォークが `captured_env` を base スコープへ注入するのと同じ位置づけ。
    /// **名前で引く**ので `captured_env`（HashMap）の反復順に依存しない。
    /// キャプチャの無い関数では `captured_slots` が空なのでループごと消える。
    ///
    /// ⚠ 値は `clone` するだけでよい。`CapturedVar::Immutable` は
    /// `capture_env` が**生成時に deep_copy 済み**（クロージャ定義時のスナップショット）で、
    /// 呼び出しごとにコピーし直すのはツリーウォークの意味論と違う。
    fn bind_captures(
        fn_val: &Rc<FnValue>,
        chunk: &crate::vm::Chunk,
        buf: &mut [Value],
        base: usize,
    ) {
        for (name, slot) in &chunk.captured_slots {
            if let Some(CapturedVar::Immutable(v)) = fn_val.captured_env.get(name) {
                let idx = base + *slot as usize;
                if idx < buf.len() {
                    buf[idx] = v.clone();
                }
            }
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
        // ── VM チャンクを1回だけ取得（fast/general 両経路で共有） ──
        // 対象: フリー関数（self なし）＋ インスタンスメソッド（self=Instance）。非 Python・クロージャなし。
        // #1 完了により **デバッグ中でも VM を使う**（`vm/run.rs` の `run_stepping` が
        // 文境界で停止判定する）。以前はここでデバッグ中の VM を丸ごと無効化していた。
        // クロージャは**キャプチャの種類を問わず** VM に載る（#27-d 段階 2b）。
        // 不変は `captured_slots` の slot へ値を書き込み、可変は `captured_cells` の
        // セル index へ **`Rc` を共有**する（外側との書き戻りが保たれる）。
        let vm_eligible = self.vm_mode != crate::vm::VmMode::Off
            && !fn_val.is_python
            && matches!(self_val, None | Some(Value::Instance(_)));
        // 診断フック（#27）: **なぜ VM に載せなかったか**を計上する。
        // `vm_eligible` が偽だと `compile_fn` を呼ばないので bail 統計に現れず、
        // 「クロージャがどれだけツリーウォークへ落ちているか」が測れなかった。
        if crate::interpreter::tw_stats::enabled() && !vm_eligible && self.vm_mode != crate::vm::VmMode::Off {
            let why = if fn_val.is_python { "python" } else { "self-kind" };
            crate::interpreter::tw_stats::record_ineligible(why);
        }
        let chunk_opt: Option<Rc<crate::vm::Chunk>> = if vm_eligible {
            self.get_or_compile_chunk(&fn_val)
        } else {
            None
        };
        // #25: `--vm=force` はフォールバック禁止。関数本体が載らなければ止める。
        // ⚠ `vm_eligible` が偽（クロージャ等）も**失敗として扱う**。そこを見逃すと
        //    「bail 0 なのにツリーウォークが残る」というゲートの穴になる（#27 の `vm_ineligible` 20 件）。
        if self.vm_mode != crate::vm::VmMode::Off && chunk_opt.is_none() {
            return Err(format!(
                "VmForceError: cannot compile function '{}' to bytecode",
                fn_val.name
            ));
        }
        // ── 高速バインド（タスク #4）: 単純シグネチャ + キャスト不要なら bind_args を介さず直接実行 ──
        if let Some(chunk) = &chunk_opt {
            if let Some((buf, base)) = self.try_fast_bind(&fn_val, chunk, evaled, &self_val)? {
                return self.run_vm_method(chunk, buf, base, &self_val, fn_name, call_span, Some(&fn_val.captured_env));
            }
        }

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
            // 第1パス: キャスト対象を特定する（self は除外）。params は借用のみ（clone しない）。
            let mut cast_targets: Vec<(usize, String, Value)> = Vec::new();
            for (idx, (name, val, mutable, _)) in bindings.iter().enumerate() {
                if *mutable || name == "self" {
                    continue;
                }
                let type_ann = fn_val
                    .params
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

        // ── Phase V: バイトコード VM 経路（一般バインド）── 上で取得済みの chunk_opt を再利用。
        // fast-bind に載らなかった呼び出し（キャスト・キーワード引数・デフォルト等）はここで
        // bindings（bind_args + copy + cast 済み）から buffer を埋めて実行する。
        if let Some(chunk) = &chunk_opt {
            let mut buf = std::mem::take(&mut self.vm_stack);
            let base = buf.len();
            buf.resize(base + chunk.n_locals, Value::None);
            for (i, (_, val, _, _)) in bindings.iter().enumerate() {
                if i < chunk.n_locals {
                    buf[base + i] = val.clone();
                }
            }
            Self::bind_captures(&fn_val, chunk, &mut buf, base); // #27-d
            return self.run_vm_method(chunk, buf, base, &self_val, fn_name, call_span, Some(&fn_val.captured_env));
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
                dict.set(Value::str(k), v);
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

        self.push_call_name(fn_name);
        // 診断フック（#10）: ここから先はツリーウォークの関数本体（最上位と区別して計上する）。
        let _tw_guard = crate::interpreter::tw_stats::enabled()
            .then(crate::interpreter::tw_stats::FnBodyGuard::new);
        let result = self.exec_block(&fn_val.body);
        drop(_tw_guard);
        self.pop_call_name();

        LOOP_DEPTH.with(|d| *d.borrow_mut() = prev_loop_depth);

        // アクセス制御コンテキストを復元する
        self.current_class = prev_class;

        // フレームを復元する: base とブロックを切り捨てて呼び出し元の状態に戻す（Vec 確保なし）。
        self.scopes.truncate(saved_len);
        self.frame_floor = saved_floor;

        // 呼び出し元フレームは**エラー経路でしか使わない**ので、ここでは作らない（#12）。
        // 作ると `get_context_lines` がソース 5 行を join して String を確保する。

        // 例外が ExecResult::Raise として直接返ってきた場合: 呼び出し元フレームを追加してセンチネルを返す
        if let Ok(ExecResult::Raise(mut raised)) = result {
            let caller_frame = self.build_caller_frame(call_span.as_ref());
            raised.frames.push(caller_frame);
            self.current_exception = Some(raised);
            return Err(RAISE_SENTINEL.to_string());
        }

        // 例外センチネルが Err として伝播してきた場合（ネストした関数からの raise）: 呼び出し元フレームを追加する
        if let Err(ref e) = result {
            if e.as_str() == RAISE_SENTINEL {
                if self.current_exception.is_some() {
                    let caller_frame = self.build_caller_frame(call_span.as_ref());
                    if let Some(ref mut raised) = self.current_exception {
                        raised.frames.push(caller_frame);
                    }
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
                raised.frames.push(self.build_caller_frame(call_span.as_ref()));
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

    /// 評価済み引数でジェネレータメソッドを実行する（VM の CallMethod 用）。`exec_generator` の本体。
    pub(crate) fn exec_generator_evaled(
        &mut self,
        gen_fn: Rc<GeneratorFnValue>,
        evaled: Vec<(Option<String>, Value, bool)>,
        self_val: Option<Value>,
    ) -> Result<Value, String> {
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

        // ── VM 経路（タスク #8）: 本体をバイトコードで実行し yield を eager 収集する ──
        // 対象: フリージェネレータ（self なし）＋ Instance レシーバのジェネレータメソッド。
        // クロージャキャプチャあり・非対応構文（`Self` 参照等）はツリーウォークへフォールバック。
        let vm_eligible = self.vm_mode != crate::vm::VmMode::Off
            && gen_fn.captured_env.is_empty()
            && matches!(self_val, None | Some(Value::Instance(_)));
        if vm_eligible {
            if let Some(chunk) = self.get_or_compile_gen_chunk(&gen_fn) {
                return self.run_vm_generator(&chunk, bindings, &self_val);
            }
        }
        // #3: フォールバックは撤去済み（ジェネレータ本体）。`Off` 以外は必ず VM で走る。
        if self.vm_mode != crate::vm::VmMode::Off {
            return Err(format!(
                "VmForceError: cannot compile generator '{}' to bytecode",
                gen_fn.name
            ));
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
        // 診断フック（#10）: ジェネレータ本体もツリーウォークの「関数本体内」として計上する。
        let _tw_guard = crate::interpreter::tw_stats::enabled()
            .then(crate::interpreter::tw_stats::FnBodyGuard::new);
        let exec_result = self.exec_block(&gen_fn.body);
        drop(_tw_guard);
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
