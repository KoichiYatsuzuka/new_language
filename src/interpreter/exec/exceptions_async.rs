// exec/exceptions_async.rs — 例外処理・非同期・イベントの実行: freeze / raise / try、async 代入、イベント購読・解除、外部イベントの排出。

use {
    crate::ast::{
        Expr,
        Stmt,
    },
    crate::token::Span,
    crate::interpreter::{
        ExecResult,
        Interpreter, RaisedError,
        StackFrame, Value, Var,
        RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// `freeze name` 文を実行する。変数を不変化し、インスタンスフィールドも再帰的にフリーズする。
    pub(crate) fn exec_freeze(&mut self, name: &str, span: &Span) -> Result<ExecResult, String> {
        let var = self
            .get_var(name)
            .ok_or_else(|| format!("{span}: NameError: '{name}' is not defined"))?;
        if !var.is_mutable() {
            return Err(format!(
                "{span}: TypeError: cannot freeze immutable variable '{name}'"
            ));
        }
        if var.is_closure_cell() {
            return Err(format!(
                "{span}: TypeError: cannot freeze '{name}' because it is captured by a closure"
            ));
        }
        let val = var.get_value();

        let replacement = match &val {
            Value::Instance(ref inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                    if overloads.len() == 1 {
                        self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__", None)?;
                    } else {
                        self.dispatch_overload(overloads, &[], Some(val.clone()), None)?;
                    }
                }
                Self::freeze_instance(inst_rc);
                None
            }
            Value::List(ref rc) => {
                let items = rc.borrow().clone();
                for item in &items {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            Value::Set(ref rc) => {
                let items = rc.borrow().clone();
                for item in &items {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            Value::Dict(ref rc) => {
                let vals = rc.borrow().all_items();
                for v in &vals {
                    self.apply_freeze_to_value(v, true)?;
                }
                None
            }
            Value::Tuple(ref td) => {
                for item in td.all_values() {
                    self.apply_freeze_to_value(item, true)?;
                }
                None
            }
            // fixed_list: trim unused allocated capacity on freeze
            Value::FrozenList { ref state, ref layout } => {
                let mut st = state.borrow_mut();
                let exact = st.len * layout.stride;
                st.data.truncate(exact);
                st.data.shrink_to_fit();
                st.allocated_size = st.len;
                None
            }
            _ => None,
        };

        // If a flat conversion was produced, update the variable value before sealing it.
        if let Some(flat) = replacement {
            self.assign_var(name, flat)
                .map_err(|e| format!("{span}: {e}"))?;
        }
        self.make_var_immutable(name);
        Ok(ExecResult::Normal)
    }

    /// `raise [exc]` 文を実行する。例外値を評価して `ExecResult::Raise` を返す。引数なしは再 raise。
    pub(crate) fn exec_raise(&mut self, exc: &Option<Expr>, span: &Span) -> Result<ExecResult, String> {
        if exc.is_none() {
            match &self.current_exception {
                Some(err) => {
                    let err = err.clone();
                    return Ok(ExecResult::Raise(err));
                }
                None => return Err("RuntimeError: no active exception to re-raise".to_string()),
            }
        }

        let exc_val = self.eval(exc.as_ref().unwrap())?;

        // 例外インスタンスに file / line / col / code_context を直接書き込む
        if let Value::Instance(ref inst_rc) = exc_val {
            let context = self.get_context_lines(&span.file, span.line, 5);
            let cls = inst_rc.borrow().class.clone();
            let mut inst = inst_rc.borrow_mut();
            for (key, val) in [
                ("file", Value::str(span.file.to_string())),
                ("line", Value::Int(span.line as i64)),
                ("col", Value::Int(span.col as i64)),
                ("code_context", Value::str(context.clone())),
                ("Error::file", Value::str(span.file.to_string())),
                ("Error::line", Value::Int(span.line as i64)),
                ("Error::col", Value::Int(span.col as i64)),
                ("Error::code_context", Value::str(context)),
            ] {
                if let Some(&idx) = cls.field_index.get(key) {
                    inst.store_field(idx, val, false);
                }
            }
        }

        let fn_name = self
            .call_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<module>".to_string());
        let frame = StackFrame {
            file: span.file.to_string(),
            line: span.line,
            col: span.col,
            fn_name,
            context: self.get_context_lines(&span.file, span.line, 5),
        };
        Ok(ExecResult::Raise(RaisedError {
            exception: exc_val,
            frames: vec![frame],
        }))
    }

    // ── VM 例外サポート（vm/run.rs のハンドラスタックが使う。current_exception 等は
    //     interpreter モジュール private なのでここにヘルパを置く） ──

    /// VM: `raise expr`。`exec_raise`（bare でない側）と同一意味論。
    /// 例外インスタンスに span フィールドを書き込み、フレーム付き RaisedError を
    /// `current_exception` に設定して RAISE_SENTINEL を返す（呼び出し側が Err で伝播）。
    pub(crate) fn vm_raise(&mut self, exc_val: Value, span: &Span) -> String {
        if let Value::Instance(ref inst_rc) = exc_val {
            let context = self.get_context_lines(&span.file, span.line, 5);
            let cls = inst_rc.borrow().class.clone();
            let mut inst = inst_rc.borrow_mut();
            for (key, val) in [
                ("file", Value::str(span.file.to_string())),
                ("line", Value::Int(span.line as i64)),
                ("col", Value::Int(span.col as i64)),
                ("code_context", Value::str(context.clone())),
                ("Error::file", Value::str(span.file.to_string())),
                ("Error::line", Value::Int(span.line as i64)),
                ("Error::col", Value::Int(span.col as i64)),
                ("Error::code_context", Value::str(context)),
            ] {
                if let Some(&idx) = cls.field_index.get(key) {
                    inst.store_field(idx, val, false);
                }
            }
        }
        let fn_name = self
            .call_stack
            .last()
            .cloned()
            .unwrap_or_else(|| "<module>".to_string());
        let frame = StackFrame {
            file: span.file.to_string(),
            line: span.line,
            col: span.col,
            fn_name,
            context: self.get_context_lines(&span.file, span.line, 5),
        };
        self.current_exception = Some(RaisedError {
            exception: exc_val,
            frames: vec![frame],
        });
        RAISE_SENTINEL.to_string()
    }

    /// VM: bare `raise`（再送出）。`current_exception` があれば RAISE_SENTINEL、なければエラー文字列。
    pub(crate) fn vm_reraise(&mut self) -> String {
        if self.current_exception.is_some() {
            RAISE_SENTINEL.to_string()
        } else {
            "RuntimeError: no active exception to re-raise".to_string()
        }
    }

    /// VM: 捕捉すべき例外 Value を取り出す。`err` が RAISE_SENTINEL なら `current_exception`、
    /// それ以外は内部エラーを RaisedError へ変換する（`exec_try`＝#33 で削除、と同じ）。`current_exception` を
    /// 設定して例外 Value を返す。変換できなければ `None`（＝伝播）。
    pub(crate) fn vm_take_raised(&mut self, err: &str) -> Option<Value> {
        let raised = if err == RAISE_SENTINEL {
            self.current_exception.clone()
        } else {
            self.make_internal_raised_error(err)
        };
        match raised {
            Some(r) => {
                let v = r.exception.clone();
                self.current_exception = Some(r);
                Some(v)
            }
            None => None,
        }
    }

    /// VM: `except TypeName` の型マッチ（`exc_matches` と同一）。
    pub(crate) fn vm_exc_matches(&self, exc: &Value, type_name: &str) -> bool {
        if let Value::Instance(inst_rc) = exc {
            Self::exc_matches(&inst_rc.borrow().class, type_name)
        } else {
            false
        }
    }

    /// `target <- async->T: body` 文を実行する。`AsyncManager` にタスクを追加する。
    pub(crate) fn exec_async_assign(&mut self, target: &str, stmts: &[Stmt]) -> Result<ExecResult, String> {
        let mgr_val = self
            .get_var(target)
            .map(|v| v.get_value())
            .ok_or_else(|| format!("NameError: '{}' is not defined", target))?;

        let mgr_rc = match mgr_val {
            Value::AsyncManager(rc) => rc,
            other => {
                return Err(format!(
                    "TypeError: '<-' operator requires an AsyncManager, got '{}'",
                    self.type_name(&other)
                ))
            }
        };

        let env = crate::interpreter::async_mgr::capture_env(self);
        // #32: worker スレッドは `Interpreter::new()` を作るので、**親の設定を渡す責任がある**。
        // ⚠ `--vm` は #33 で無くなったが、型注釈（`annotations`）の引き継ぎは今も要る
        //    （渡し忘れるとゲートに穴が開く。実際に開いていた）。
        mgr_rc.borrow_mut().add_task(stmts.to_vec(), env);
        Ok(ExecResult::Normal)
    }

    /// VM の `AsyncSubmit` op 用（タスク #9）。VM フレームには `scopes` が無いため、frame から読み出した
    /// 捕捉ローカル `captured` を一時スコープに積んでから `capture_env` を呼び、ツリーウォークの
    /// `exec_async_assign` と**同一の env**（捕捉ローカル + グローバル・mutable/immutable の deep_clone 規則込み）
    /// を組んで AsyncManager にタスクを投入する。捕捉は本体が参照する slot に限定済み（未参照ローカルは載せない）。
    pub(crate) fn vm_async_submit(
        &mut self,
        mgr: Value,
        body: &[Stmt],
        captured: Vec<(String, Value, bool)>,
    ) -> Result<(), String> {
        let mgr_rc = match mgr {
            Value::AsyncManager(rc) => rc,
            other => {
                return Err(format!(
                    "TypeError: '<-' operator requires an AsyncManager, got '{}'",
                    self.type_name(&other)
                ))
            }
        };
        // 捕捉ローカルだけを可視にする一時スコープを積み、capture_env に globals と合成させる。
        // frame_floor を進めることで capture_env が「現関数ローカル = この一時スコープ」とみなす。
        let saved_floor = self.frame_floor;
        let saved_len = self.scopes.len();
        self.frame_floor = saved_len;
        self.push_scope();
        for (name, value, is_mut) in captured {
            self.declare_var(name, Var::new(value, is_mut));
        }
        let env = crate::interpreter::async_mgr::capture_env(self);
        self.scopes.truncate(saved_len);
        self.frame_floor = saved_floor;
        mgr_rc.borrow_mut().add_task(body.to_vec(), env);
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // External event queue draining (C#/Go bridge)
    // ---------------------------------------------------------------------------

    /// 外部イベントキュー（C#/Go ブリッジが ar_event_fire() で書き込んだもの）をすべて処理する。
    pub(crate) fn drain_external_events(&mut self) -> Result<(), String> {
        let events: Vec<crate::interpreter::event_loop::ExternalEvent> = {
            let mut guard = self.events.external_queue.lock().unwrap();
            guard.drain(..).collect()
        };
        for ev in events {
            let sig_rc = self.events.external_handlers.get(&ev.handler_id).cloned();
            if let Some(sig_rc) = sig_rc {
                // データは MessagePack でシリアライズされているが、現時点では str として渡す。
                let val = Value::str(String::from_utf8_lossy(&ev.data).into_owned());
                let handlers = sig_rc.borrow_mut().collect_handlers_for_emit();
                for (h, _) in handlers {
                    self.call_value_with_args(h, vec![val.clone()])?;
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // Event handler subscription / unsubscription
    // ---------------------------------------------------------------------------

    /// `source on handler` / `source once handler` — イベントハンドラを登録する。
    pub(crate) fn exec_event_subscribe(
        &mut self,
        source: &crate::ast::Expr,
        handler: &crate::ast::Expr,
        is_once: bool,
        is_async: bool,
    ) -> Result<ExecResult, String> {
        let source_val = self.eval(source)?;
        let handler_val = self.eval(handler)?;
        self.event_subscribe_evaled(source_val, handler_val, is_once, is_async)
    }

    /// 評価済みの source/handler で購読する（VM の `Op::EventSubscribe` 用・#27-c）。
    pub(crate) fn event_subscribe_evaled(
        &mut self,
        source_val: Value,
        handler_val: Value,
        is_once: bool,
        is_async: bool,
    ) -> Result<ExecResult, String> {
        match source_val {
            Value::Signal(sig_rc) => {
                sig_rc
                    .borrow_mut()
                    .subscribe(handler_val, is_once, is_async);
                Ok(ExecResult::Normal)
            }
            other => Err(format!(
                "TypeError: 'on'/'once' operator requires a Signal, got '{}'",
                self.type_name(&other)
            )),
        }
    }

    /// `source off handler` — ハンドラを解除する。
    pub(crate) fn exec_event_unsubscribe(
        &mut self,
        source: &crate::ast::Expr,
        handler: &crate::ast::Expr,
    ) -> Result<ExecResult, String> {
        let source_val = self.eval(source)?;
        let handler_val = self.eval(handler)?;
        self.event_unsubscribe_evaled(source_val, handler_val)
    }

    /// 評価済みの source/handler で解除する（VM の `Op::EventUnsubscribe` 用・#27-c）。
    pub(crate) fn event_unsubscribe_evaled(
        &mut self,
        source_val: Value,
        handler_val: Value,
    ) -> Result<ExecResult, String> {
        match source_val {
            Value::Signal(sig_rc) => {
                sig_rc.borrow_mut().unsubscribe_by_value(&handler_val);
                Ok(ExecResult::Normal)
            }
            other => Err(format!(
                "TypeError: 'off' operator requires a Signal, got '{}'",
                self.type_name(&other)
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // Module loading
    // ---------------------------------------------------------------------------

}
