// classes/object_methods.rs — 特殊オブジェクトのメソッド実行: Signal / EventLoop / File のメソッド、および evaled 版メソッド呼び出し。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::interpreter::str_methods::{
        regex_findall, regex_match, regex_search, regex_split, regex_sub, str_format,
    },
    crate::interpreter::{
        ByteModeRust, ClassValue, FileOpenModeRust, FnValue, GeneratorState, InstanceData,
        Interpreter, RaisedError, Value, RAISE_SENTINEL,
    },
};
#[allow(unused_imports)]
use super::*;

impl Interpreter {
    /// `Signal[T]` のメソッド呼び出しを処理する。
    pub(crate) fn exec_signal_method(
        &mut self,
        sig_rc: std::rc::Rc<std::cell::RefCell<crate::interpreter::event_loop::SignalData>>,
        method_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        match method_name {
            "emit" => {
                let evaled = self.eval_call_args(args)?;
                let val = match evaled.as_slice() {
                    [(_, v, _)] => v.clone(),
                    [] => Value::None,
                    _ => return Err("TypeError: Signal.emit() takes exactly 1 argument".to_string()),
                };
                // 全ハンドラを取得（is_once のものはリストから除去される）。
                let handlers = sig_rc.borrow_mut().collect_handlers_for_emit();
                let el_rc = self.event_loop_data.clone();
                for (func, is_async) in handlers {
                    if is_async {
                        // 非同期ハンドラ: EventLoop キューに積む。
                        el_rc.borrow_mut().signal_queue.push_back((sig_rc.clone(), val.clone()));
                    } else {
                        // 同期ハンドラ: 即座に呼ぶ。
                        self.call_value_with_args(func, vec![val.clone()])?;
                    }
                }
                Ok(Value::None)
            }
            "emit_async" => {
                let evaled = self.eval_call_args(args)?;
                let val = match evaled.as_slice() {
                    [(_, v, _)] => v.clone(),
                    [] => Value::None,
                    _ => return Err("TypeError: Signal.emit_async() takes exactly 1 argument".to_string()),
                };
                // EventLoop のキューに積むだけ。実際の呼び出しは EventLoop.run() が行う。
                let el_rc = self.event_loop_data.clone();
                el_rc.borrow_mut().signal_queue.push_back((sig_rc.clone(), val));
                Ok(Value::None)
            }
            _ => Err(format!(
                "AttributeError: 'Signal' object has no method '{method_name}'"
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // EventLoop メソッド
    // ---------------------------------------------------------------------------

    /// `EventLoop` のメソッド呼び出しを処理する。
    pub(crate) fn exec_event_loop_method(
        &mut self,
        el_rc: std::rc::Rc<std::cell::RefCell<crate::interpreter::event_loop::EventLoopData>>,
        method_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        match method_name {
            "run" => {
                // EventLoop.run([timeout: float])
                let evaled = self.eval_call_args(args)?;
                let timeout_ms: Option<u64> = match evaled.as_slice() {
                    [] => None,
                    [(key, Value::Float(f), _)]
                        if key.is_none() || key.as_deref() == Some("timeout") =>
                    {
                        Some((f * 1000.0) as u64)
                    }
                    [(key, Value::Int(n), _)]
                        if key.is_none() || key.as_deref() == Some("timeout") =>
                    {
                        Some((*n as u64) * 1000)
                    }
                    _ => return Err("TypeError: EventLoop.run() takes at most 1 argument (timeout: float)".to_string()),
                };

                let deadline = timeout_ms.map(|ms| {
                    std::time::Instant::now() + std::time::Duration::from_millis(ms)
                });

                loop {
                    // 外部イベントキュー（C#/Go ブリッジ）を処理する。
                    self.drain_external_events()?;

                    // Signal の非同期キューと post キューを処理する。
                    let has_work = {
                        let b = el_rc.borrow();
                        b.has_work()
                    };
                    if has_work {
                        // signal_queue エントリを 1 つ取り出して全同期ハンドラを呼ぶ。
                        let entry = el_rc.borrow_mut().signal_queue.pop_front();
                        if let Some((sig_ref, val)) = entry {
                            let handlers = sig_ref.borrow_mut().collect_handlers_for_emit();
                            for (func, _is_async) in handlers {
                                // EventLoop 内では全ハンドラを同期的に処理する（非同期も含む）。
                                self.call_value_with_args(func, vec![val.clone()])?;
                            }
                        }
                        // post キューのコールバックを 1 つ取り出して呼ぶ。
                        let cb = el_rc.borrow_mut().post_queue.pop_front();
                        if let Some(func) = cb {
                            self.call_value_with_args(func, vec![])?;
                        }
                        continue;
                    }

                    // タイムアウトチェック。
                    if let Some(dl) = deadline {
                        if std::time::Instant::now() >= dl {
                            break;
                        }
                    } else {
                        // タイムアウトなし: 作業がなければ終了。
                        break;
                    }

                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(Value::None)
            }
            "post" => {
                // EventLoop.post(fn) — メインスレッドへ処理を投入する。
                let evaled = self.eval_call_args(args)?;
                let func = match evaled.as_slice() {
                    [(_, v, _)] => v.clone(),
                    _ => return Err("TypeError: EventLoop.post() takes exactly 1 argument".to_string()),
                };
                el_rc.borrow_mut().post_queue.push_back(func);
                Ok(Value::None)
            }
            _ => Err(format!(
                "AttributeError: 'EventLoop' object has no method '{method_name}'"
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // FileObject メソッド
    // ---------------------------------------------------------------------------

    /// `FileObject` のメソッド（`read` / `write` / `close` / `seek` 等）を実行する。
    pub(crate) fn exec_file_method(
        &mut self,
        fd_rc: Rc<RefCell<crate::interpreter::FileData>>,
        method_name: &str,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Result<Value, String> {
        match method_name {
            "read" => {
                let backward = file_bool_arg(evaled, "backward", false)?;
                let mut fd = fd_rc.borrow_mut();
                if fd.is_closed {
                    return Err("IOError: I/O operation on closed file".to_string());
                }
                if !backward && fd.pointer == fd.content.len() {
                    return Err("EOFError: EOF".to_string());
                }
                if backward && fd.pointer == 0 {
                    return Err("BOFError: BOF".to_string());
                }
                let data: Vec<u8> = if backward {
                    let result = fd.content[..fd.pointer].to_vec();
                    fd.pointer = 0;
                    result
                } else {
                    let result = fd.content[fd.pointer..].to_vec();
                    fd.pointer = fd.content.len();
                    result
                };
                Ok(bytes_to_value(&data, &fd.byte_mode))
            }
            "read_line" => {
                let backward = file_bool_arg(evaled, "backward", false)?;
                let mut fd = fd_rc.borrow_mut();
                if fd.is_closed {
                    return Err("IOError: I/O operation on closed file".to_string());
                }
                if !backward && fd.pointer == fd.content.len() {
                    return Err("EOFError: EOF".to_string());
                }
                if backward && fd.pointer == 0 {
                    return Err("BOFError: BOF".to_string());
                }
                if !backward {
                    // 次の \n を探してその位置（含む）まで返す
                    let offset = fd.content[fd.pointer..].iter().position(|&b| b == b'\n');
                    let end = match offset {
                        Some(i) => fd.pointer + i + 1,
                        None => fd.content.len(),
                    };
                    let data = fd.content[fd.pointer..end].to_vec();
                    fd.pointer = end;
                    Ok(bytes_to_value(&data, &fd.byte_mode))
                } else {
                    // 現在位置の直前の \n をスキップしてその前の \n を探す
                    let p = fd.pointer;
                    let skip_end = if p > 0 && fd.content[p - 1] == b'\n' {
                        p - 1
                    } else {
                        p
                    };
                    let prev_nl = fd.content[..skip_end].iter().rposition(|&b| b == b'\n');
                    let start = match prev_nl {
                        Some(i) => i + 1,
                        None => 0,
                    };
                    let data = fd.content[start..p].to_vec();
                    fd.pointer = start;
                    Ok(bytes_to_value(&data, &fd.byte_mode))
                }
            }
            "read_letter" => {
                let backward = file_bool_arg(evaled, "backward", false)?;
                let mut fd = fd_rc.borrow_mut();
                if fd.is_closed {
                    return Err("IOError: I/O operation on closed file".to_string());
                }
                if !backward && fd.pointer == fd.content.len() {
                    return Err("EOFError: EOF".to_string());
                }
                if backward && fd.pointer == 0 {
                    return Err("BOFError: BOF".to_string());
                }
                match fd.byte_mode {
                    ByteModeRust::Byte => {
                        if !backward {
                            let b = fd.content[fd.pointer];
                            fd.pointer += 1;
                            Ok(Value::Int(b as i64))
                        } else {
                            fd.pointer -= 1;
                            Ok(Value::Int(fd.content[fd.pointer] as i64))
                        }
                    }
                    ByteModeRust::Text => {
                        if !backward {
                            let s = std::str::from_utf8(&fd.content[fd.pointer..])
                                .map_err(|_| "IOError: invalid UTF-8 in file".to_string())?;
                            let ch = s.chars().next().unwrap(); // 空でないことは確認済み
                            let ch_len = ch.len_utf8();
                            fd.pointer += ch_len;
                            Ok(Value::Str(ch.to_string()))
                        } else {
                            let s = std::str::from_utf8(&fd.content[..fd.pointer])
                                .map_err(|_| "IOError: invalid UTF-8 in file".to_string())?;
                            let ch = s.chars().next_back().unwrap();
                            let ch_len = ch.len_utf8();
                            fd.pointer -= ch_len;
                            Ok(Value::Str(ch.to_string()))
                        }
                    }
                }
            }
            "write" | "write_line" => {
                let is_write_line = method_name == "write_line";
                let content_val = file_content_arg(evaled, "content")?;
                let mut fd = fd_rc.borrow_mut();
                if fd.is_closed {
                    return Err("IOError: I/O operation on closed file".to_string());
                }
                if fd.mode == FileOpenModeRust::Read {
                    return Err("IOError: file is opened in read-only mode".to_string());
                }
                let mut insert_bytes = value_to_bytes(content_val, &fd.byte_mode)
                    .map_err(|e| format!("TypeError: {method_name}(): {e}"))?;
                if is_write_line {
                    insert_bytes.push(b'\n');
                }
                // ポインタ位置に挿入（EOF なら追記、途中なら割り込み）
                let ptr = fd.pointer;
                let rest = fd.content[ptr..].to_vec();
                fd.content.truncate(ptr);
                fd.content.extend_from_slice(&insert_bytes);
                fd.content.extend_from_slice(&rest);
                fd.pointer = ptr + insert_bytes.len();
                Ok(Value::None)
            }
            _ => Err(format!(
                "AttributeError: FileObject has no method '{method_name}'"
            )),
        }
    }

    /// 評価済み引数でメソッドを呼び出す。`__getitem__` / `__setitem__` などの
    /// subscript ディスパッチ用。Instance と PyObject のみ対応する。
    pub(crate) fn eval_method_call_evaled(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        // Result 型のメソッド: is_OK() → bool、is_ERR() → bool
        if let Value::ResultVal { ok, .. } = &obj {
            return match method_name {
                "is_OK" => Ok(Value::Bool(*ok)),
                "is_ERR" => Ok(Value::Bool(!ok)),
                _ => Err(format!(
                    "AttributeError: Result has no method '{method_name}'"
                )),
            };
        }
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().flags() & crate::interpreter::value::INST_IMMUTABLE != 0;

                // Native method dispatch — check NATIVE_METHODS before tree-walk.
                // When a native ptr is registered we always dispatch natively (no fallback).
                if crate::interpreter::native_api::lookup_native_method_ptr(&class.name, method_name).is_some() {
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                    return crate::interpreter::native_api::try_dispatch_native_method(
                        self, obj.clone(), method_name, arg_vals,
                    ).unwrap_or_else(|| {
                        Err(format!("NativeError: dispatch failed for {}.{method_name}", class.name))
                    });
                }

                let overloads = self
                    .lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| {
                        format!(
                            "AttributeError: '{}' has no method '{method_name}'",
                            class.name
                        )
                    })?;

                let callable: Vec<Rc<FnValue>> = if inst_immutable {
                    overloads
                        .iter()
                        .filter(|f| {
                            f.params
                                .first()
                                .map(|p| p.name != "self" || !p.mutable)
                                .unwrap_or(true)
                        })
                        .cloned()
                        .collect()
                } else {
                    overloads
                };

                if callable.is_empty() {
                    return Err(format!(
                        "TypeError: cannot call mutable method '{method_name}' on immutable instance of '{}'",
                        class.name
                    ));
                }

                if callable.len() == 1 {
                    self.exec_fn_evaled(
                        callable[0].clone(),
                        &evaled,
                        Some(obj.clone()),
                        method_name,
                        None,
                    )
                } else {
                    self.dispatch_overload_evaled(callable, evaled, Some(obj.clone()), method_name, None)
                }
            }
            Value::PyObject(handle) => {
                crate::interpreter::py_interop::call_py_method(handle, method_name, &evaled)
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    // ── str メソッドディスパッチ ──────────────────────────────────────────────

}
