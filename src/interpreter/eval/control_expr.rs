// eval/control_expr.rs — 式としての制御フロー評価: block / if / for / while 式と block_return の捕捉。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        ExecResult, GeneratorState,
        Interpreter, Value, Var, BLOCK_YIELDS, BREAK_SENTINEL, CONTINUE_SENTINEL, LOOP_DEPTH,
        RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// block: 式 / Expr::Block の実体。BLOCK_YIELDS コンテキストを退避・復元しながら実行する。
    pub(crate) fn eval_block_expr(&mut self, stmts: &[crate::ast::Stmt]) -> Result<Value, String> {
        crate::interpreter::tw_stats::record_tls("block-expr");
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));

        self.push_scope();
        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'block_expr: for stmt in stmts {
            match self.exec(stmt) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::BlockReturn(v)) => {
                    block_return_val = Some(v);
                    break 'block_expr;
                }
                Ok(ExecResult::BlockYield(_)) => {} // スレッドローカル経由で収集済み
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some(
                        "SyntaxError: 'return' inside block expression — use 'block_return'"
                            .to_string(),
                    );
                    break 'block_expr;
                }
                Ok(ExecResult::Break) => {
                    // break propagates through block: expressions to reach the enclosing loop
                    early_err = Some(BREAK_SENTINEL.to_string());
                    break 'block_expr;
                }
                Ok(ExecResult::Continue) => {
                    // continue も break と同じく block: 式を貫通して外側ループへ届く（#34）。
                    early_err = Some(CONTINUE_SENTINEL.to_string());
                    break 'block_expr;
                }
                Err(e) => {
                    early_err = Some(e);
                    break 'block_expr;
                }
            }
        }
        self.pop_scope();

        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

    /// IfExpr のブランチ評価ロジック。BLOCK_RETURN_EXPECTED_TYPE の push/pop は呼び出し元が行う。
    pub(crate) fn eval_if_expr_body(
        &mut self,
        branches: &[(crate::ast::Expr, Vec<crate::ast::Stmt>)],
        else_body: &Option<Vec<crate::ast::Stmt>>,
    ) -> Result<Value, String> {
        for (cond, body) in branches {
            let val = self.eval(cond)?;
            if self.eval_truthy(&val)? {
                return self.eval_capture_block_return(body);
            }
        }
        if let Some(body) = else_body {
            return self.eval_capture_block_return(body);
        }
        Ok(Value::None)
    }

    /// if / match 式のボディを実行し、BlockReturn シグナルを値として捕捉して返す。
    /// BLOCK_YIELDS は設定しない（透過的 — 外側の for/while/block 式に yield が届く）。
    pub(crate) fn eval_capture_block_return(
        &mut self,
        stmts: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        self.push_scope();
        let mut result_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'body: for stmt in stmts {
            match self.exec(stmt) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::BlockReturn(v)) => {
                    result_val = Some(v);
                    break 'body;
                }
                Ok(ExecResult::Break) => {
                    // break propagates through if/match expressions to reach the enclosing loop
                    early_err = Some(BREAK_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Continue) => {
                    // ⚠ 以前はこのアームが無く `Ok(other)` へ落ちて**黙って握り潰されて**いた（#34）。
                    // continue も break と同じく if/match 式を貫通して外側ループへ届く。
                    early_err = Some(CONTINUE_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'body;
                }
                Ok(ExecResult::Return(_)) => {
                    early_err = Some(
                        "SyntaxError: 'return' inside block expression — use 'block_return'"
                            .to_string(),
                    );
                    break 'body;
                }
                Ok(other) => {
                    let _ = other;
                }
                Err(e) => {
                    early_err = Some(e);
                    break 'body;
                }
            }
        }
        self.pop_scope();
        if let Some(e) = early_err {
            return Err(e);
        }
        Ok(result_val.unwrap_or(Value::None))
    }

    /// for 式の実体。BLOCK_YIELDS コンテキストと LOOP_DEPTH を管理し、loop_yield でリスト蓄積、block_return で単値返却。
    pub(crate) fn eval_for_expr(
        &mut self,
        target: &str,
        iter_expr: &crate::ast::Expr,
        body: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        let iter_val = self.eval(iter_expr)?;
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                })))
            }
            Value::Generator(_) => iter_val,
            Value::Instance(_) => self.eval_method_call(iter_val, "__iter__", &[], None)?,
            Value::PyObject(ref handle) => {
                let items = crate::interpreter::py_interop::py_collect_iter(handle)?;
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: items,
                    index: 0,
                })))
            }
            _ => return Err("TypeError: object is not iterable".to_string()),
        };

        crate::interpreter::tw_stats::record_tls("loop-expr");
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);

        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'for_loop: loop {
            match self.eval_method_call(generator.clone(), "next", &[], None) {
                Ok(item) => {
                    self.push_scope();
                    self.declare_var(target.to_string(), Var::new(item, true));
                    let result = self.exec_block(body);
                    self.pop_scope();
                    match result {
                        Ok(ExecResult::Normal) => {}
                        Ok(ExecResult::Continue) => continue,
                        // break: exit loop and return accumulated loop_yields (or None)
                        Ok(ExecResult::Break) => break 'for_loop,
                        // block_return None: exit loop with explicit None (ignores yields)
                        Ok(ExecResult::BlockReturn(Value::None)) => {
                            block_return_val = Some(Value::None);
                            break 'for_loop;
                        }
                        Ok(ExecResult::BlockReturn(v)) => {
                            block_return_val = Some(v);
                            break 'for_loop;
                        }
                        Ok(ExecResult::Raise(raised)) => {
                            self.current_exception = Some(raised);
                            early_err = Some(RAISE_SENTINEL.to_string());
                            break 'for_loop;
                        }
                        Ok(ExecResult::Return(v)) => {
                            block_return_val = Some(v);
                            break 'for_loop;
                        } // shouldn't happen
                        Ok(ExecResult::BlockYield(_)) => {}
                        // break from inside an eval context (e.g. if expression body)
                        Err(ref e) if e.as_str() == BREAK_SENTINEL => break 'for_loop,
                        // continue from inside an eval context（#34）
                        Err(ref e) if e.as_str() == CONTINUE_SENTINEL => continue,
                        Err(e) => {
                            early_err = Some(e);
                            break 'for_loop;
                        }
                    }
                }
                Err(ref e) if e.starts_with("EndOfIteration") => break,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

    /// while 式の実体。for 式と同様に BLOCK_YIELDS と LOOP_DEPTH を管理する。
    pub(crate) fn eval_while_expr(
        &mut self,
        cond_expr: &crate::ast::Expr,
        body: &[crate::ast::Stmt],
    ) -> Result<Value, String> {
        crate::interpreter::tw_stats::record_tls("loop-expr");
        let saved = BLOCK_YIELDS.with(|y| y.borrow_mut().take());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = Some(Vec::new()));
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);

        let mut block_return_val: Option<Value> = None;
        let mut early_err: Option<String> = None;

        'while_loop: loop {
            let cond_val = match self.eval(cond_expr) {
                Ok(v) => v,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            };
            match self.eval_truthy(&cond_val) {
                Ok(false) => break,
                Ok(true) => {}
                Err(e) => { early_err = Some(e); break 'while_loop; }
            }

            match self.exec_scoped_block(body) {
                Ok(ExecResult::Normal) => {}
                Ok(ExecResult::Continue) => continue,
                // break: exit loop and return accumulated loop_yields (or None)
                Ok(ExecResult::Break) => break 'while_loop,
                // block_return None: exit loop with explicit None (ignores yields)
                Ok(ExecResult::BlockReturn(Value::None)) => {
                    block_return_val = Some(Value::None);
                    break 'while_loop;
                }
                Ok(ExecResult::BlockReturn(v)) => {
                    block_return_val = Some(v);
                    break 'while_loop;
                }
                Ok(ExecResult::Raise(raised)) => {
                    self.current_exception = Some(raised);
                    early_err = Some(RAISE_SENTINEL.to_string());
                    break 'while_loop;
                }
                Ok(other) => {
                    let _ = other;
                }
                // break from inside an eval context (e.g. if expression body)
                Err(ref e) if e.as_str() == BREAK_SENTINEL => break 'while_loop,
                // continue from inside an eval context（#34）
                Err(ref e) if e.as_str() == CONTINUE_SENTINEL => continue,
                Err(e) => {
                    early_err = Some(e);
                    break;
                }
            }
        }

        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        let yields = BLOCK_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());
        BLOCK_YIELDS.with(|y| *y.borrow_mut() = saved);

        if let Some(e) = early_err {
            return Err(e);
        }
        match block_return_val {
            Some(v) => Ok(v),
            None => {
                if yields.is_empty() {
                    Ok(Value::None)
                } else {
                    Ok(Value::List(Rc::new(RefCell::new(yields))))
                }
            }
        }
    }

}
