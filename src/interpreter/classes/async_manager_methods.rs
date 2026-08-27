// classes/async_manager_methods.rs — `AsyncManager` メソッドのディスパッチ。
//
// #63 で `eval_method_call_full` の `Value::AsyncManager` アーム（76 行）を切り出したもの。
//
// ⚠ `wait_for_finish` は**ここだけがスレッドを待つ**（ポーリング + `thread::sleep`）。
// D5 の share-nothing 設計により、タスクは submit 時に deep-clone されて別スレッドで
// ツリーウォーク実行される（`Stmt::AsyncAssign` → `exec_async_assign`）。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::interpreter::{
        async_mgr::AsyncManagerData, Interpreter, RaisedError, Value, RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// `AsyncManager` のメソッドを評価済み引数で呼ぶ（#63 で切り出し）。
    /// 呼び出し元は `eval_method_call_full` のみ。
    pub(crate) fn eval_async_manager_method(
        &mut self,
        mgr_rc: Rc<RefCell<AsyncManagerData>>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "all_done" => {
                Self::expect_no_args_evaled(&evaled, "AsyncManager", "all_done")?;
                let all = mgr_rc.borrow().all_done();
                Ok(Value::Bool(all))
            }
            "wait_for_finish" => {
                // wait_for_finish(await_interval_msec = 100)
                let interval_ms: u64 = match evaled.as_slice() {
                    [] => 100,
                    [(key, Value::Int(n), _)]
                        if key.is_none() || key.as_deref() == Some("await_interval_msec") =>
                    {
                        (*n).max(1) as u64
                    }
                    _ => return Err("TypeError: wait_for_finish() takes at most 1 argument (await_interval_msec)".to_string()),
                };

                loop {
                    let (done, abort_triggered) = {
                        let mut mgr = mgr_rc.borrow_mut();
                        mgr.poll_completed();
                        mgr.try_schedule_pub();
                        let done = mgr.all_done();
                        let abort = mgr.raise_immediately && mgr.first_error().is_some();
                        (done, abort)
                    };

                    if done {
                        break;
                    }

                    if abort_triggered {
                        // Cancel remaining pending tasks then wait for running ones
                        mgr_rc.borrow_mut().cancel_pending();
                        // Keep polling until all running threads finish
                        loop {
                            {
                                let mut mgr = mgr_rc.borrow_mut();
                                mgr.poll_completed();
                                if mgr.all_done() {
                                    break;
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
                        }
                        break;
                    }

                    std::thread::sleep(std::time::Duration::from_millis(interval_ms));
                }

                // Propagate first error if raise_immediately, as a catchable raise
                let first_err = {
                    let mgr = mgr_rc.borrow();
                    if mgr.raise_immediately {
                        mgr.first_error()
                    } else {
                        None
                    }
                };
                if let Some(e) = first_err {
                    self.current_exception = Some(RaisedError {
                        exception: Value::str(e),
                        frames: vec![],
                    });
                    return Err(RAISE_SENTINEL.to_string());
                }

                Ok(Value::None)
            }
            _ => Err(format!(
                "AttributeError: 'AsyncManager' has no method '{method_name}'"
            )),
        }
    }
}
