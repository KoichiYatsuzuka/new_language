// exec/exceptions_async.rs — 例外処理・非同期・イベントの実行: freeze / raise / try、async 代入、イベント購読・解除、外部イベントの排出。

use {
    crate::ast::{
        ExceptHandler, Expr,
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
                ("file", Value::Str(span.file.to_string())),
                ("line", Value::Int(span.line as i64)),
                ("col", Value::Int(span.col as i64)),
                ("code_context", Value::Str(context.clone())),
                ("Error::file", Value::Str(span.file.to_string())),
                ("Error::line", Value::Int(span.line as i64)),
                ("Error::col", Value::Int(span.col as i64)),
                ("Error::code_context", Value::Str(context)),
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
                ("file", Value::Str(span.file.to_string())),
                ("line", Value::Int(span.line as i64)),
                ("col", Value::Int(span.col as i64)),
                ("code_context", Value::Str(context.clone())),
                ("Error::file", Value::Str(span.file.to_string())),
                ("Error::line", Value::Int(span.line as i64)),
                ("Error::col", Value::Int(span.col as i64)),
                ("Error::code_context", Value::Str(context)),
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
    /// それ以外は内部エラーを RaisedError へ変換する（`exec_try` と同じ）。`current_exception` を
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

    /// `try / except / finally` 文を実行する。例外を捕捉してハンドラを実行し、finally ブロックは常に実行する。
    pub(crate) fn exec_try(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        finally_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        let body_result = self.exec_scoped_block(body);

        let mut converted_internal = false;
        let raise_opt: Option<RaisedError> = match &body_result {
            Ok(ExecResult::Raise(r)) => Some(r.clone()),
            Err(e) if e.as_str() == RAISE_SENTINEL => self.current_exception.clone(),
            Err(e) => {
                let msg = e.clone();
                let r = self.make_internal_raised_error(&msg);
                if r.is_some() {
                    converted_internal = true;
                }
                r
            }
            _ => None,
        };

        let mut final_result: Result<ExecResult, String> = body_result;

        if let Some(raised) = raise_opt {
            let mut handled = false;
            for handler in handlers {
                let matches = match &handler.exc_type {
                    None => true,
                    Some(type_name) => {
                        if let Value::Instance(ref inst_rc) = raised.exception {
                            Self::exc_matches(&inst_rc.borrow().class, type_name)
                        } else {
                            false
                        }
                    }
                };
                if matches {
                    let prev_exc = self.current_exception.clone();
                    self.current_exception = Some(raised.clone());

                    self.push_scope();
                    if let Some(alias) = &handler.name {
                        let exc_val = raised.exception.clone();
                        self.declare_var(alias.clone(), Var::new(exc_val, false));
                    }
                    let handler_result = self.exec_block(&handler.body);
                    self.pop_scope();

                    self.current_exception = prev_exc;
                    final_result = handler_result;
                    handled = true;
                    break;
                }
            }
            if !handled && converted_internal {
                // 内部エラーから変換された RaisedError がどのハンドラにもマッチしなかった場合:
                // ExecResult::Raise として上位に伝播させ、トレースバック表示が機能するようにする
                final_result = Ok(ExecResult::Raise(raised));
            }
        }

        if let Some(finally) = finally_body {
            let finally_result = self.exec_scoped_block(finally);
            match finally_result {
                Ok(ExecResult::Normal) => {}
                Ok(signal) => return Ok(signal),
                Err(e) => return Err(e),
            }
        }

        final_result
    }

    // ---------------------------------------------------------------------------
    // Async
    // ---------------------------------------------------------------------------

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
        mgr_rc.borrow_mut().add_task(stmts.to_vec(), env);
        Ok(ExecResult::Normal)
    }

    // ---------------------------------------------------------------------------
    // External event queue draining (C#/Go bridge)
    // ---------------------------------------------------------------------------

    /// 外部イベントキュー（C#/Go ブリッジが ar_event_fire() で書き込んだもの）をすべて処理する。
    pub(crate) fn drain_external_events(&mut self) -> Result<(), String> {
        let events: Vec<crate::interpreter::event_loop::ExternalEvent> = {
            let mut guard = self.external_event_queue.lock().unwrap();
            guard.drain(..).collect()
        };
        for ev in events {
            let sig_rc = self.external_handler_registry.get(&ev.handler_id).cloned();
            if let Some(sig_rc) = sig_rc {
                // データは MessagePack でシリアライズされているが、現時点では str として渡す。
                let val = Value::Str(String::from_utf8_lossy(&ev.data).into_owned());
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
