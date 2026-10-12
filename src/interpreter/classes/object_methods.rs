// classes/object_methods.rs — 特殊オブジェクトのメソッド実行: Signal / EventLoop / File のメソッド、および evaled 版メソッド呼び出し。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        ByteModeRust, FileOpenModeRust,
        Interpreter, Value,
    },
};
use super::*;

impl Interpreter {
    /// `Signal[T]` のメソッド呼び出しを処理する。
    pub(crate) fn exec_signal_method(
        &mut self,
        sig_rc: std::rc::Rc<std::cell::RefCell<crate::interpreter::event_loop::SignalData>>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "emit" => {
                let val = match evaled.as_slice() {
                    [(_, v, _)] => v.clone(),
                    [] => Value::None,
                    _ => return Err("TypeError: Signal.emit() takes exactly 1 argument".to_string()),
                };
                // 全ハンドラを取得（is_once のものはリストから除去される）。
                let handlers = sig_rc.borrow_mut().collect_handlers_for_emit();
                let el_rc = self.events.data.clone();
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
                let val = match evaled.as_slice() {
                    [(_, v, _)] => v.clone(),
                    [] => Value::None,
                    _ => return Err("TypeError: Signal.emit_async() takes exactly 1 argument".to_string()),
                };
                // EventLoop のキューに積むだけ。実際の呼び出しは EventLoop.run() が行う。
                let el_rc = self.events.data.clone();
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
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "run" => {
                // EventLoop.run([timeout: float])
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

    /// `FileObject` のメソッド（Arrow の `read` / `read_line` / `read_letter` / `write` / `write_line` と、
    /// Python の `readline` / `readlines` / `writelines` / `close` / `flush` / `tell`）を実行する。
    ///
    /// Python の名前はどのファイルにも足した（Arrow の名前とぶつからない）。同じ名前で意味の違う `read` / `write` は、
    /// Python の形で開いたファイル（`FileData::py_mode`）だけ CPython の意味にする（python_builtins_plan.md のタスク 5-4）。
    pub(crate) fn exec_file_method(
        &mut self,
        fd_rc: Rc<RefCell<crate::interpreter::FileData>>,
        method_name: &str,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Result<Value, String> {
        let py_form = fd_rc.borrow().py_mode.is_some();
        match method_name {
            "readline" | "readlines" | "writelines" | "close" | "flush" | "tell" => {
                return self.exec_file_py_method(fd_rc, method_name, evaled)
            }
            "read" | "write" if py_form => return self.exec_file_py_method(fd_rc, method_name, evaled),
            _ => {}
        }
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
                bytes_to_value(&data, &fd.byte_mode)
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
                    bytes_to_value(&data, &fd.byte_mode)
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
                    bytes_to_value(&data, &fd.byte_mode)
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
                            Ok(Value::str(ch.to_string()))
                        } else {
                            let s = std::str::from_utf8(&fd.content[..fd.pointer])
                                .map_err(|_| "IOError: invalid UTF-8 in file".to_string())?;
                            let ch = s.chars().next_back().unwrap();
                            let ch_len = ch.len_utf8();
                            fd.pointer -= ch_len;
                            Ok(Value::str(ch.to_string()))
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

    /// Python のファイルのメソッド（python_builtins_plan.md のタスク 5-4）。文言は CPython 3.12 と同じ。
    ///
    /// - `read(size=-1)` / `readline(size=-1)` / `readlines(hint=-1)`: 終わりでは `""`（誤りにしない）。`size` は
    ///   テキストなら文字の数、バイト列ならバイトの数。行は `\n` まで（`\n` を含む）
    /// - `write(s)`: 位置へ書いて、書いた数（文字・バイト）を返す。`writelines(it)`: 順に書く（改行は足さない）
    /// - `close()`（何度呼んでもよい）/ `flush()`（閉じずにディスクへ書き戻す）/ `tell()`（位置・バイト）
    /// - 返す値: テキストは `str`。バイト列は Python の形なら `bytes`、Arrow の形なら従来どおり `list[int]`
    ///
    /// ⚠ 閉じたファイルは CPython と同じ `ValueError: I/O operation on closed file.`（Arrow の形は従来の文言）。
    /// ⚠ 読めない／書けないモード（`"w"` で読む・`"r"` で書く）は `OSError: not readable` / `not writable`
    ///   （CPython は `io.UnsupportedOperation`。`OSError` と `ValueError` の両方を継承するが、Arrow の例外の
    ///   表は単一継承なので `OSError` にした。`except OSError` で捕まえられるのは同じ）。
    /// ⚠ `seek` は無い（`"r+"` も受けない。Arrow の `write` は位置へ挿入するので、位置を動かせると上書きと違う）。
    fn exec_file_py_method(
        &mut self,
        fd_rc: Rc<RefCell<crate::interpreter::FileData>>,
        method_name: &str,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Result<Value, String> {
        let py_form = fd_rc.borrow().py_mode.is_some();
        let closed_err = || {
            if py_form {
                "ValueError: I/O operation on closed file.".to_string()
            } else {
                "IOError: I/O operation on closed file".to_string()
            }
        };
        let check_open = |fd: &crate::interpreter::FileData| if fd.is_closed { Err(closed_err()) } else { Ok(()) };
        match method_name {
            "close" => {
                file_no_args(evaled, "close")?;
                fd_rc.borrow_mut().close();
                Ok(Value::None)
            }
            "flush" => {
                file_no_args(evaled, "flush")?;
                let mut fd = fd_rc.borrow_mut();
                check_open(&fd)?;
                fd.flush_to_disk();
                Ok(Value::None)
            }
            "tell" => {
                file_no_args(evaled, "tell")?;
                let fd = fd_rc.borrow();
                check_open(&fd)?;
                Ok(Value::Int(fd.pointer as i64))
            }
            "read" | "readline" => {
                let size = file_size_arg(evaled, method_name, "size")?;
                let mut fd = fd_rc.borrow_mut();
                check_open(&fd)?;
                if !fd.readable() {
                    return Err("OSError: not readable".to_string());
                }
                let end = file_read_end(&fd, size, method_name == "readline");
                let start = fd.pointer;
                fd.pointer = end;
                file_bytes_value(&fd.content[start..end], &fd)
            }
            "readlines" => {
                let hint = file_size_arg(evaled, "readlines", "hint")?;
                let mut fd = fd_rc.borrow_mut();
                check_open(&fd)?;
                if !fd.readable() {
                    return Err("OSError: not readable".to_string());
                }
                let mut lines = Vec::new();
                let mut total = 0usize;
                while fd.pointer < fd.content.len() {
                    let start = fd.pointer;
                    let end = file_read_end(&fd, None, true);
                    fd.pointer = end;
                    let line = file_bytes_value(&fd.content[start..end], &fd)?;
                    total += match &line {
                        Value::Str(s) => s.chars().count(),
                        _ => end - start,
                    };
                    lines.push(line);
                    // CPython と同じく、合計が `hint` に達したらその行で止める（`hint` が 0 以下なら全部）。
                    if hint.is_some_and(|h| h > 0 && total >= h) {
                        break;
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(lines))))
            }
            "write" => {
                let content = file_content_arg(evaled, "s")?.clone();
                let mut fd = fd_rc.borrow_mut();
                check_open(&fd)?;
                if !fd.writable() {
                    return Err("OSError: not writable".to_string());
                }
                let (bytes, count) = file_encode(&content, &fd)?;
                file_insert(&mut fd, &bytes);
                Ok(Value::Int(count as i64))
            }
            "writelines" => {
                let lines = file_content_arg(evaled, "lines")?.clone();
                {
                    let fd = fd_rc.borrow();
                    check_open(&fd)?;
                    if !fd.writable() {
                        return Err("OSError: not writable".to_string());
                    }
                }
                // ⚠ 借用を落としてから回す（要素を取り出す途中で利用者のコードが走りうる）。
                for line in self.drain_iterable(lines)? {
                    let mut fd = fd_rc.borrow_mut();
                    let (bytes, _) = file_encode(&line, &fd)?;
                    file_insert(&mut fd, &bytes);
                }
                Ok(Value::None)
            }
            other => Err(format!("AttributeError: FileObject has no method '{other}'")),
        }
    }

    /// 評価済み引数でメソッドを呼び出す。`__getitem__` / `__setitem__` などの
    /// 評価済み引数でメソッドを呼び出す（#27-b で**統一実装への委譲**になった）。
    ///
    /// 以前はここに `Instance`/`PyObject` だけの独自ディスパッチがあり、CallArg 版
    /// （16 レシーバ）と**ずれていた**。VM はこちらしか呼べないため
    /// 「`--vm=off` では動くが `--vm=auto` で落ちる」が繰り返し起きた（#22-a/-b/-c・#10-b′）。
    /// ⚠ `--vm` は #33 で削除済み（以下は当時の記録）。今は VM 一本なので
    /// **食い違いの検出は `compare_python_impl.ps1`（参照実装との突き合わせ）が担う**。
    /// 実装は `eval_method_call_full` 1 箇所だけにする。
    pub(crate) fn eval_method_call_evaled(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        self.eval_method_call_full(obj, method_name, evaled, None)
    }

    // ── str メソッドディスパッチ ──────────────────────────────────────────────

}
