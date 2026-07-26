// eval/builtins.rs — 組み込み関数呼び出しの評価: eval_builtin_ident_call / eval_builtin_open。


use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::interpreter::{
        ByteModeRust, FileData, FileOpenModeRust, GeneratorState,
        Interpreter, TupleData, Value,
    },
};
use super::*;

impl Interpreter {
    /// 組み込み関数名を受け取り、該当する組み込みを実行して結果を返す。
    /// 未知の名前には `None` を返してユーザー定義関数の探索にフォールスルーする。
    /// 評価済み引数で「純粋・共通」な組み込みを呼ぶ（VM の `CallBuiltin` op 用）。
    /// `eval_builtin_ident_call` の対応アームと**同一意味論**（引数は VM がスタックで評価済み）。
    /// ここで扱わない名前は `None`（コンパイラは扱う名前だけ `CallBuiltin` を発行する）。
    pub(crate) fn eval_builtin_evaled(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Option<Result<Value, String>> {
        match name {
            "print" => {
                let mut parts: Vec<String> = Vec::with_capacity(args.len());
                for v in &args {
                    match self.display_str(v) {
                        Ok(s) => parts.push(s),
                        Err(e) => return Some(Err(e)),
                    }
                }
                println!("{}", parts.join(" "));
                Some(Ok(Value::None))
            }
            "range" => Some(match args.as_slice() {
                [Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                    (0..*stop).map(Value::Int).collect(),
                )))),
                [Value::Int(start), Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                    (*start..*stop).map(Value::Int).collect(),
                )))),
                [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                    let mut items = Vec::new();
                    let mut i = *start;
                    if *step > 0 {
                        while i < *stop {
                            items.push(Value::Int(i));
                            i += step;
                        }
                    } else if *step < 0 {
                        while i > *stop {
                            items.push(Value::Int(i));
                            i += step;
                        }
                    }
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
                _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
            }),
            "len" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: len() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                let has_instance_len = if let Value::Instance(inst_rc) = &val {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    return Some(self.eval_method_call_evaled(val, "__len__", vec![]).and_then(
                        |r| match r {
                            Value::Int(n) => Ok(Value::Int(n)),
                            other => Err(format!(
                                "TypeError: __len__ must return int, not '{}'",
                                self.type_name(&other)
                            )),
                        },
                    ));
                }
                Some(match &val {
                    Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
                    Value::FrozenList { ref state, .. } => Ok(Value::Int(state.borrow().len as i64)),
                    Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                    Value::Dict(d) => Ok(Value::Int(d.borrow().all_keys().len() as i64)),
                    Value::Set(s) => Ok(Value::Int(s.borrow().len() as i64)),
                    Value::Tuple(t) => Ok(Value::Int(t.len() as i64)),
                    Value::PyObject(handle) => crate::interpreter::py_interop::py_len(handle),
                    _ => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(&val)
                    )),
                })
            }
            "next" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: next() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(match val {
                    v @ Value::Generator(_) => self.eval_method_call_evaled(v, "next", vec![]),
                    v @ Value::Instance(_) => self.eval_method_call_evaled(v, "__next__", vec![]),
                    other => Err(format!(
                        "TypeError: '{}' object is not an iterator",
                        self.type_name(&other)
                    )),
                })
            }
            "repr" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: repr() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(self.repr_val(&val).map(Value::Str))
            }
            "id" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: id() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(self.call_type_by_name_evaled("id", vec![val]))
            }
            "enumerate" => {
                // VM は位置引数のみ渡す（`start=` キーワードは compile_call_args が bail）。
                // ツリーウォークの位置引数 1 個・start=0 の経路と一致。
                if args.len() != 1 {
                    return Some(Err(format!(
                        "TypeError: enumerate() expected 1 positional argument, got {}",
                        args.len()
                    )));
                }
                let iterable = args.into_iter().next().unwrap();
                Some(self.enumerate_core(iterable, 0))
            }
            "zip" => Some(self.zip_core(args)),
            "getenv" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: getenv() takes 1 or 2 arguments (name[, default])".to_string(),
                    ));
                }
                let mut it = args.into_iter();
                let name = match it.next().unwrap() {
                    Value::Str(s) => s,
                    other => {
                        return Some(Err(format!(
                            "TypeError: getenv() name must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                };
                let default = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Some(Err(format!(
                            "TypeError: getenv() default must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    None => String::new(),
                };
                Some(Ok(Value::Str(std::env::var(&name).unwrap_or(default))))
            }
            _ => None,
        }
    }

    /// enumerate のコア: 評価済みの反復対象と開始値からタプル列（`(index, value)`）の
    /// Generator を作る。CallArg 版（`eval_builtin_ident_call`）と評価済み版（VM の
    /// `eval_builtin_evaled`）で共有し、意味論の分岐を防ぐ。
    pub(crate) fn enumerate_core(&mut self, iterable: Value, start: i64) -> Result<Value, String> {
        let items = self.collect_iterable(iterable)?;
        let tuples: Vec<Value> = items
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let idx = start + i as i64;
                let type_str = self.type_name(&v).to_string();
                Value::Tuple(Rc::new(TupleData::new(
                    vec![Value::Int(idx), v],
                    vec!["int".to_string(), type_str],
                )))
            })
            .collect();
        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
            values: tuples,
            index: 0,
        }))))
    }

    /// zip のコア: 評価済みの反復対象群から、最短長ぶんのタプル列の Generator を作る。
    /// CallArg 版と評価済み版で共有する。
    pub(crate) fn zip_core(&mut self, iters_vals: Vec<Value>) -> Result<Value, String> {
        let mut iters: Vec<Vec<Value>> = Vec::new();
        for v in iters_vals {
            iters.push(self.collect_iterable(v)?);
        }
        if iters.is_empty() {
            return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                values: vec![],
                index: 0,
            }))));
        }
        let min_len = iters.iter().map(|it| it.len()).min().unwrap_or(0);
        let tuples: Vec<Value> = (0..min_len)
            .map(|i| {
                let vals: Vec<Value> = iters.iter().map(|it| it[i].clone()).collect();
                let types: Vec<String> =
                    vals.iter().map(|v| self.type_name(v).to_string()).collect();
                Value::Tuple(Rc::new(TupleData::new(vals, types)))
            })
            .collect();
        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
            values: tuples,
            index: 0,
        }))))
    }

    pub(crate) fn eval_builtin_ident_call(
        &mut self,
        name: &str,
        args: &[CallArg],
    ) -> Option<Result<Value, String>> {
        match name {
            "print" => {
                let mut parts: Vec<String> = Vec::new();
                for a in args {
                    let v = match self.eval(a.expr()) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    let s = match self.display_str(&v) {
                        Ok(s) => s,
                        Err(e) => return Some(Err(e)),
                    };
                    parts.push(s);
                }
                println!("{}", parts.join(" "));
                Some(Ok(Value::None))
            }
            "next" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: next() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match val {
                    v @ Value::Generator(_) => self.eval_method_call_evaled(v, "next", vec![]),
                    v @ Value::Instance(_) => self.eval_method_call_evaled(v, "__next__", vec![]),
                    other => Err(format!(
                        "TypeError: '{}' object is not an iterator",
                        self.type_name(&other)
                    )),
                })
            }
            "repr" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: repr() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.repr_val(&val).map(Value::Str))
            }
            "range" => {
                let evaled: Result<Vec<_>, _> = args.iter().map(|a| self.eval(a.expr())).collect();
                let evaled = match evaled {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match evaled.as_slice() {
                    [Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                        (0..*stop).map(Value::Int).collect(),
                    )))),
                    [Value::Int(start), Value::Int(stop)] => Ok(Value::List(Rc::new(
                        RefCell::new((*start..*stop).map(Value::Int).collect()),
                    ))),
                    [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                        let mut items = Vec::new();
                        let mut i = *start;
                        if *step > 0 {
                            while i < *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        } else if *step < 0 {
                            while i > *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        }
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
                })
            }
            "len" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: len() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                // Instance の __len__ を優先チェック（borrow を落としてから呼び出す）
                let has_instance_len = if let Value::Instance(inst_rc) = &val {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    return Some(
                        self.eval_method_call_evaled(val, "__len__", vec![])
                            .and_then(|r| match r {
                                Value::Int(n) => Ok(Value::Int(n)),
                                other => Err(format!(
                                    "TypeError: __len__ must return int, not '{}'",
                                    self.type_name(&other)
                                )),
                            }),
                    );
                }
                Some(match &val {
                    Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
                    Value::FrozenList { ref state, .. } => Ok(Value::Int(state.borrow().len as i64)),
                    Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                    Value::Dict(d) => Ok(Value::Int(d.borrow().all_keys().len() as i64)),
                    Value::Set(s) => Ok(Value::Int(s.borrow().len() as i64)),
                    Value::Tuple(t) => Ok(Value::Int(t.len() as i64)),
                    Value::PyObject(handle) => crate::interpreter::py_interop::py_len(handle),
                    _ => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(&val)
                    )),
                })
            }
            // ── mutable flat-list built-ins ───────────────────────────────────
            // create_flat_int_list(size, val) → fixed_list[Cell]
            // Allocates a flat byte buffer directly — no Cell instance allocation.
            // Requires 'Cell' class to be in scope (via `from ant_render import Cell`).
            "create_flat_int_list" => {
                if args.len() != 2 {
                    return Some(Err("TypeError: create_flat_int_list() takes exactly 2 arguments".to_string()));
                }
                let size_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let init_v = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let size = match &size_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: create_flat_int_list: size must be int".to_string())) };
                let init = match &init_v { Value::Int(n) => *n, _ => return Some(Err("TypeError: create_flat_int_list: val must be int".to_string())) };
                let cell_class = match self.get_val("Cell") {
                    Some(Value::Class(c)) => c,
                    _ => return Some(Err("NameError: create_flat_int_list requires 'Cell' class in scope".to_string())),
                };
                let init_bytes = init.to_le_bytes();
                let mut raw = vec![0u8; size * 8];
                for chunk in raw.chunks_exact_mut(8) { chunk.copy_from_slice(&init_bytes); }
                let flat_data = crate::interpreter::value::FlatListData { data: raw, len: size, allocated_size: size };
                let layout = crate::interpreter::value::FlatLayout {
                    class_name: "Cell".to_string(),
                    fields: vec![("v".to_string(), crate::interpreter::value::FlatFieldTy::Int)],
                    stride: 8,
                    class: cell_class,
                };
                Some(Ok(Value::FrozenList { state: Rc::new(RefCell::new(flat_data)), layout: Rc::new(layout) }))
            }
            // flat_get_int(grid, idx) → int
            "flat_get_int" => {
                if args.len() != 2 {
                    return Some(Err("TypeError: flat_get_int() takes exactly 2 arguments".to_string()));
                }
                let grid_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx_v  = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx = match &idx_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: flat_get_int: idx must be int".to_string())) };
                match &grid_v {
                    Value::FrozenList { state, .. } => {
                        let s = state.borrow();
                        if idx >= s.len { return Some(Err(format!("IndexError: flat_get_int index {idx} out of range (len {})", s.len))); }
                        let off = idx * 8;
                        let bytes: [u8; 8] = s.data[off..off + 8].try_into().unwrap();
                        Some(Ok(Value::Int(i64::from_le_bytes(bytes))))
                    }
                    _ => Some(Err(format!("TypeError: flat_get_int expects fixed_list, got {}", self.type_name(&grid_v)))),
                }
            }
            // flat_set_int(grid, idx, val) → None  — writes directly into the flat buffer
            "flat_set_int" => {
                if args.len() != 3 {
                    return Some(Err("TypeError: flat_set_int() takes exactly 3 arguments".to_string()));
                }
                let grid_v = match self.eval(args[0].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx_v  = match self.eval(args[1].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let val_v  = match self.eval(args[2].expr()) { Ok(v) => v, Err(e) => return Some(Err(e)) };
                let idx = match &idx_v { Value::Int(n) => *n as usize, _ => return Some(Err("TypeError: flat_set_int: idx must be int".to_string())) };
                let val = match &val_v { Value::Int(n) => *n, _ => return Some(Err("TypeError: flat_set_int: val must be int".to_string())) };
                match &grid_v {
                    Value::FrozenList { state, .. } => {
                        let mut s = state.borrow_mut();
                        if idx >= s.len { return Some(Err(format!("IndexError: flat_set_int index {idx} out of range"))); }
                        let off = idx * 8;
                        s.data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                        Some(Ok(Value::None))
                    }
                    _ => Some(Err(format!("TypeError: flat_set_int expects fixed_list, got {}", self.type_name(&grid_v)))),
                }
            }
            // ─────────────────────────────────────────────────────────────────
            "id" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: id() takes exactly one argument".to_string()));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.call_type_by_name_evaled("id", vec![val]))
            }
            "open" => Some(self.eval_builtin_open(args)),
            "close" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: close() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match val {
                    Value::FileObject(fd_rc) => {
                        fd_rc.borrow_mut().close();
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: close() argument must be FileObject, not '{}'",
                        self.type_name(&other)
                    )),
                })
            }
            "enumerate" => {
                let mut positional: Vec<Value> = Vec::new();
                let mut start_val: Option<Value> = None;
                for arg in args {
                    match arg {
                        CallArg::Positional(e) => match self.eval(e) {
                            Ok(v) => positional.push(v),
                            Err(e) => return Some(Err(e)),
                        },
                        CallArg::Keyword { name, value } if name == "start" => {
                            match self.eval(value) {
                                Ok(v) => start_val = Some(v),
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        CallArg::Keyword { name, .. } => {
                            return Some(Err(format!(
                                "TypeError: enumerate() got unexpected keyword argument '{name}'"
                            )));
                        }
                        CallArg::Variadic(_) => {
                            return Some(Err(
                                "TypeError: enumerate() does not support variadic arguments".to_string()
                            ));
                        }
                    }
                }
                if positional.len() != 1 {
                    return Some(Err(format!(
                        "TypeError: enumerate() expected 1 positional argument, got {}",
                        positional.len()
                    )));
                }
                let start = match start_val {
                    Some(Value::Int(n)) => n,
                    Some(other) => {
                        return Some(Err(format!(
                            "TypeError: enumerate() 'start' must be int, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    None => 0i64,
                };
                let iterable = positional.into_iter().next().unwrap();
                Some(self.enumerate_core(iterable, start))
            }
            "zip" => {
                for arg in args.iter() {
                    if matches!(arg, CallArg::Keyword { .. }) {
                        return Some(Err(
                            "TypeError: zip() takes no keyword arguments".to_string()
                        ));
                    }
                }
                let mut iters_vals: Vec<Value> = Vec::new();
                for arg in args.iter() {
                    match self.eval(arg.expr()) {
                        Ok(v) => iters_vals.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Some(self.zip_core(iters_vals))
            }
            "getenv" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: getenv() takes 1 or 2 arguments (name[, default])".to_string(),
                    ));
                }
                let name = match self.eval(args[0].expr()) {
                    Ok(Value::Str(s)) => s,
                    Ok(other) => {
                        return Some(Err(format!(
                            "TypeError: getenv() name must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    Err(e) => return Some(Err(e)),
                };
                let default = if args.len() > 1 {
                    match self.eval(args[1].expr()) {
                        Ok(Value::Str(s)) => s,
                        Ok(other) => {
                            return Some(Err(format!(
                                "TypeError: getenv() default must be str, not '{}'",
                                self.type_name(&other)
                            )))
                        }
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    String::new()
                };
                Some(Ok(Value::Str(std::env::var(&name).unwrap_or(default))))
            }
            "parse_ar" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: parse_ar() takes 1 or 2 arguments (source[, path])".to_string(),
                    ));
                }
                let source = match self.eval(args[0].expr()) {
                    Ok(Value::Str(s)) => s,
                    Ok(other) => {
                        return Some(Err(format!(
                            "TypeError: parse_ar() source must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    Err(e) => return Some(Err(e)),
                };
                let path = if args.len() > 1 {
                    match self.eval(args[1].expr()) {
                        Ok(Value::Str(s)) => s,
                        Ok(other) => {
                            return Some(Err(format!(
                                "TypeError: parse_ar() path must be str, not '{}'",
                                self.type_name(&other)
                            )))
                        }
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    String::new()
                };
                let source = source.strip_prefix('\u{FEFF}').map(str::to_string).unwrap_or(source);
                let tokens = crate::lexer::Lexer::new(&source, &*path).tokenize();
                let source_dir = std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.to_path_buf());
                let stmts = match crate::parser::Parser::new(tokens, source_dir).parse_program() {
                    Ok(s) => s,
                    Err(e) => return Some(Err(format!("ParseError in parse_ar: {e}"))),
                };
                Some(Ok(crate::interpreter::ast_value::stmts_to_value(&stmts)))
            }
            _ => None,
        }
    }

    /// 組み込み関数 `open()` を実行してファイルオブジェクト (`Value::FileObject`) を返す。
    /// 引数 `file_path`, `open_mode`, `start_point`, `byte_recognizing`, `encoding`, `exclusion` を解析し、
    /// 対応する `std::fs::OpenOptions` を構築してファイルを開く。
    pub(crate) fn eval_builtin_open(&mut self, args: &[CallArg]) -> Result<Value, String> {
        use std::collections::HashMap as HMap;
        use std::fs::OpenOptions;
        use std::io::Read as IoRead;
        let evaled = self.eval_call_args(args)?;
        let mut kw: HMap<String, Value> = HMap::new();
        let mut pos: Vec<Value> = Vec::new();
        for (k, v, _) in evaled {
            match k {
                Some(n) => {
                    kw.insert(n, v);
                }
                None => pos.push(v),
            }
        }
        let file_path = extract_path_str(
            get_arg(&pos, &kw, 0, "file_path")
                .ok_or("TypeError: open() missing required argument 'file_path'")?,
        )?;
        let open_mode_int = extract_enum_int(
            get_arg(&pos, &kw, 1, "open_mode")
                .ok_or("TypeError: open() missing required argument 'open_mode'")?,
            "enum_item_FileOpenMode",
        )?;
        let start_point_int: i64 = get_arg(&pos, &kw, 2, "start_point")
            .map(|v| extract_enum_int(v, "enum_item_StartPoint"))
            .transpose()?
            .unwrap_or(0);
        let byte_mode_int: i64 = get_arg(&pos, &kw, 3, "byte_recognizing")
            .map(|v| extract_enum_int(v, "enum_item_ByteRecognizingMode"))
            .transpose()?
            .unwrap_or(1);
        let enc_int: i64 = get_arg(&pos, &kw, 4, "encoding")
            .map(|v| extract_enum_int(v, "enum_item_Encoding"))
            .transpose()?
            .unwrap_or(1);
        if enc_int == 3 {
            return Err("NotImplementedError: Shift-JIS encoding is not yet supported".to_string());
        }
        let _exclusion: bool = get_arg(&pos, &kw, 5, "exclusion")
            .map(|v| match v {
                Value::Bool(b) => Ok(*b),
                _ => Err("TypeError: open() 'exclusion' must be bool".to_string()),
            })
            .transpose()?
            .unwrap_or(true);

        let mode = match open_mode_int {
            0 => FileOpenModeRust::Write,
            1 => FileOpenModeRust::Rewrite,
            2 => FileOpenModeRust::Read,
            3 => FileOpenModeRust::MakeAndWrite,
            n => return Err(format!("TypeError: invalid FileOpenMode value {n}")),
        };
        let byte_mode = if byte_mode_int == 0 {
            ByteModeRust::Byte
        } else {
            ByteModeRust::Text
        };

        let std_path = std::path::Path::new(&file_path);
        if mode == FileOpenModeRust::MakeAndWrite && std_path.exists() {
            return Err(format!(
                "RuntimeError: open() make_and_write: file '{}' already exists",
                file_path
            ));
        }

        let (file, content) = match mode {
            FileOpenModeRust::Read => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                (f, c)
            }
            FileOpenModeRust::Write => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| format!("IOError: cannot read '{}': {e}", file_path))?;
                (f, c)
            }
            FileOpenModeRust::Rewrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot open '{}': {e}", file_path))?;
                (f, Vec::new())
            }
            FileOpenModeRust::MakeAndWrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(std_path)
                    .map_err(|e| format!("IOError: cannot create '{}': {e}", file_path))?;
                (f, Vec::new())
            }
        };

        let (content, bom_skip) = if enc_int == 2 && content.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (content[3..].to_vec(), 3usize)
        } else {
            (content, 0usize)
        };
        let _ = bom_skip;
        let pointer = if start_point_int == 1 {
            content.len()
        } else {
            0
        };

        let fd = FileData {
            path: file_path,
            mode,
            byte_mode,
            content,
            pointer,
            is_closed: false,
            file_handle: Some(file),
        };
        Ok(Value::FileObject(Rc::new(RefCell::new(fd))))
    }

}
