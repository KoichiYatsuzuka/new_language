// exec/control_flow.rs — 制御構造の実行: if / match / while / for / block 文。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::ast::{
        BinOp, Expr, MatchArm, MatchPattern,
        Stmt,
    },
    crate::interpreter::{
        ExecResult, GeneratorState,
        Interpreter, Value, Var, BREAK_SENTINEL, LOOP_DEPTH,
    },
};
use super::*;

impl Interpreter {
    /// `if / elif / else` 文を実行する。最初に真となった条件のブランチをスコープ付きブロックとして実行する。
    pub(crate) fn exec_if_stmt(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
    ) -> Result<ExecResult, String> {
        for (cond, body) in branches {
            // Result ガード検出: `x.is_OK()` / `x.is_ERR()` の形式を確認する
            let result_rebind: Option<(String, bool)> = extract_result_guard_call(cond);

            let val = self.eval(cond)?;
            if self.eval_truthy(&val)? {
                // ガード節なら x を内部値（unwrap済み）に差し替えたスコープでボディを実行する
                if let Some((var_name, _is_ok)) = result_rebind {
                    let rebind_info = self.get_var(&var_name).and_then(|rv| {
                        if let Value::ResultVal { inner, .. } = rv.get_value() {
                            Some((*inner, rv.is_mutable()))
                        } else {
                            None
                        }
                    });
                    if let Some((inner_val, is_mut)) = rebind_info {
                        self.push_scope();
                        self.declare_var(var_name, Var::new(inner_val, is_mut));
                        let result = self.exec_block(body);
                        self.pop_scope();
                        return result;
                    }
                }
                return self.exec_scoped_block(body);
            }
        }
        if let Some(body) = else_body {
            return self.exec_scoped_block(body);
        }
        Ok(ExecResult::Normal)
    }

    /// `match` 文を実行する。サブジェクトを各アームのパターンと照合し、最初に一致したアームのボディを実行する。
    pub(crate) fn exec_match_stmt(&mut self, subject: &Expr, arms: &[MatchArm]) -> Result<ExecResult, String> {
        let subject_val = self.eval(subject)?;
        for arm in arms {
            let matched = match &arm.pattern {
                MatchPattern::Case(pattern_expr) => {
                    if matches!(pattern_expr, Expr::Ident { name: n, .. } if n == "_") {
                        true
                    } else {
                        let pattern_val = self.eval(pattern_expr)?;
                        let result =
                            self.apply_binop_dyn(&BinOp::Eq, subject_val.clone(), pattern_val)?;
                        matches!(result, Value::Bool(true))
                    }
                }
                MatchPattern::IsType(type_name) => self.value_is_type(&subject_val, type_name),
            };
            if matched {
                return self.exec_scoped_block(&arm.body);
            }
        }
        Ok(ExecResult::Normal)
    }

    /// `while cond: body` 文を実行する。条件が偽になるか `break` が発生するまでボディを繰り返す。
    pub(crate) fn exec_while_stmt(&mut self, cond: &Expr, body: &[Stmt]) -> Result<ExecResult, String> {
        crate::interpreter::tw_stats::record_tls("while-stmt");
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result = (|| {
            loop {
                let val = self.eval(cond)?;
                if !self.eval_truthy(&val)? {
                    break;
                }
                match self.exec_scoped_block(body) {
                    Ok(ExecResult::Break) | Ok(ExecResult::BlockReturn(Value::None)) => break,
                    Ok(ExecResult::Continue) | Ok(ExecResult::Normal) => {}
                    Ok(r) => return Ok(r),
                    Err(ref e) if e.as_str() == BREAK_SENTINEL => break,
                    Err(e) => return Err(e),
                }
            }
            Ok(ExecResult::Normal)
        })();
        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        result
    }

    /// イテラブルな値を `for` 反復用のイテレータ（多くは `Value::Generator`）へ変換する。
    /// `exec_for_stmt`（ツリーウォーク）と VM の `GetIter` op が共有する（意味論一致）。
    pub(crate) fn make_for_iterator(&mut self, iter_val: Value) -> Result<Value, String> {
        let generator = match iter_val {
            Value::List(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::FrozenList { ref state, ref layout } => {
                let st = state.borrow();
                let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState { values, index: 0 })))
            }
            Value::Str(s) => {
                let chars: Vec<Value> = s.chars().map(|c| Value::str(c.to_string())).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                })))
            }
            Value::Set(items) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: items.borrow().clone(),
                index: 0,
            }))),
            Value::Tuple(td) => Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: td.all_values().to_vec(),
                index: 0,
            }))),
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
        Ok(generator)
    }

    /// `for target in iter: body` 文を実行する。イテラブルを展開して各要素でボディを繰り返す。
    pub(crate) fn exec_for_stmt(
        &mut self,
        targets: &[String],
        iter: &Expr,
        body: &[Stmt],
    ) -> Result<ExecResult, String> {
        let iter_val = self.eval(iter)?;
        let generator = self.make_for_iterator(iter_val)?;
        crate::interpreter::tw_stats::record_tls("for-stmt");
        LOOP_DEPTH.with(|d| *d.borrow_mut() += 1);
        let result =
            (|| {
                loop {
                    match self.eval_method_call(generator.clone(), "next", &[], None) {
                        Ok(item) => {
                            self.push_scope();
                            if targets.len() == 1 {
                                self.declare_var(targets[0].clone(), Var::new(item, true));
                            } else {
                                let elems =
                                    match &item {
                                        Value::Tuple(td) => {
                                            if td.len() != targets.len() {
                                                return Err(format!(
                                                    "ValueError: not enough values to unpack \
                                             (expected {}, got {})",
                                                    targets.len(),
                                                    td.len()
                                                ));
                                            }
                                            td.all_values().to_vec()
                                        }
                                        _ => return Err(
                                            "TypeError: cannot unpack non-tuple value in for loop"
                                                .to_string(),
                                        ),
                                    };
                                for (name, val) in targets.iter().zip(elems) {
                                    self.declare_var(name.clone(), Var::new(val, true));
                                }
                            }
                            let result = self.exec_block(body);
                            self.pop_scope();
                            match result {
                                Ok(ExecResult::Break)
                                | Ok(ExecResult::BlockReturn(Value::None)) => break,
                                Ok(ExecResult::Continue) | Ok(ExecResult::Normal) => {}
                                Ok(r) => return Ok(r),
                                Err(ref e) if e.as_str() == BREAK_SENTINEL => break,
                                Err(e) => return Err(e),
                            }
                        }
                        Err(ref e) if e.starts_with("EndOfIteration") => break,
                        Err(e) => return Err(e),
                    }
                }
                Ok(ExecResult::Normal)
            })();
        LOOP_DEPTH.with(|d| *d.borrow_mut() -= 1);
        result
    }

    /// `block: body` 文を実行する。`BlockReturn` を消費し、それ以外の制御フローは外へ伝播させる。
    pub(crate) fn exec_block_stmt(&mut self, body: &[Stmt]) -> Result<ExecResult, String> {
        // All BlockReturn values are absorbed (the block: statement consumes them).
        // Break, Continue, Return, Raise propagate outward to the enclosing loop/function.
        match self.exec_scoped_block(body)? {
            ExecResult::Normal | ExecResult::BlockReturn(_) => Ok(ExecResult::Normal),
            r => Ok(r),
        }
    }

    // ---------------------------------------------------------------------------
    // Function / generator definitions
    // ---------------------------------------------------------------------------

}
