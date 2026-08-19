// classes/method_call.rs — メソッド呼び出し評価の中核: eval_method_call。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::interpreter::{
        FnValue, GeneratorState,
        Interpreter, RaisedError, Value, RAISE_SENTINEL,
    },
};

impl Interpreter {
    /// 評価済み引数で **インスタンスメソッド** を呼び出す（VM の `CallMethod` 用）。
    /// `eval_method_call` の `Value::Instance` アームと同一のディスパッチ
    /// （copy / gen / native / static・class 判定 / 不変性フィルタ / オーバーロード）を、
    /// 評価済み引数（`is_mutable` フラグ込み）で行う。呼び出し側は obj が Instance であることを
    /// 型注釈で保証してから使う（型検査が健全性を担保）。
    pub(crate) fn call_instance_method_evaled(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
        cache: Option<&crate::ast::AttrCache>,
        call_span: Option<crate::token::Span>,
    ) -> Result<Value, String> {
        let inst_rc = match &obj {
            Value::Instance(rc) => rc.clone(),
            _ => {
                return Err(format!(
                    "TypeError: '{}' object has no method '{method_name}'",
                    self.type_name(&obj)
                ))
            }
        };
        if method_name == "copy" {
            if !evaled.is_empty() {
                return Err(format!(
                    "TypeError: {}.copy() takes no arguments",
                    inst_rc.borrow().class.name
                ));
            }
            return self.copy_value(obj);
        }

        // method IC 命中: plain 非 mut-self 単一メソッドを直接ディスパッチ（eval_method_call と同一）。
        if let Some(c) = cache {
            let class_id = inst_rc.borrow().class.class_id;
            if c.get(class_id).is_some() {
                let class = inst_rc.borrow().class.clone();
                if let Some(overloads) = class.methods.get(method_name) {
                    if overloads.len() == 1 {
                        let f = overloads[0].clone();
                        return self.exec_fn_evaled(f, &evaled, Some(obj), method_name, call_span);
                    }
                }
            }
        }

        let class = inst_rc.borrow().class.clone();
        let inst_immutable =
            inst_rc.borrow().flags() & crate::interpreter::value::INST_IMMUTABLE != 0;

        if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
            return self.exec_generator_evaled(gen_fn, evaled, Some(obj));
        }
        if crate::interpreter::native_api::lookup_native_method_ptr(&class.name, method_name)
            .is_some()
        {
            let arg_vals: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
            if let Some(result) = crate::interpreter::native_api::try_dispatch_native_method(
                self,
                obj.clone(),
                method_name,
                arg_vals,
            ) {
                return result;
            }
        }
        let overloads = self.lookup_method_in_class(&class, method_name).ok_or_else(|| {
            format!("AttributeError: '{}' has no method '{method_name}'", class.name)
        })?;
        let n_overloads = overloads.len();
        if class.static_method_names.contains(method_name) {
            return Err(format!(
                "AttributeError: static method '{}' must be called on the class, not an instance; use '{}.{}(...)'",
                method_name, class.name, method_name
            ));
        }
        if class.class_method_names.contains(method_name) {
            return Err(format!(
                "AttributeError: class method '{}' must be called on the class, not an instance; use '{}.{}(...)'",
                method_name, class.name, method_name
            ));
        }
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
            // method IC 充填（eval_method_call と同一条件: 単一 overload・非 mut-self・native なし）。
            if let Some(c) = cache {
                let self_is_mut = callable[0]
                    .params
                    .first()
                    .map(|p| p.name == "self" && p.mutable)
                    .unwrap_or(false);
                if n_overloads == 1
                    && !self_is_mut
                    && crate::interpreter::native_api::lookup_native_method_ptr(
                        &class.name,
                        method_name,
                    )
                    .is_none()
                {
                    c.fill(class.class_id, 0, 0);
                }
            }
            self.exec_fn_evaled(callable[0].clone(), &evaled, Some(obj), method_name, call_span)
        } else {
            self.dispatch_overload_evaled(callable, evaled, Some(obj), method_name, call_span)
        }
    }

    /// オブジェクトのメソッドを呼び出して結果を返す。List / Str / Instance / Dict / Generator 等の各値型へディスパッチする。
    ///
    /// `cache` が `Some` の場合、インスタンスメソッド解決を method IC（`cache.2`）で高速化する。
    /// 内部呼び出し（for ループの `next`/`__iter__` 等）は `None` を渡す。
    pub(crate) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
        cache: Option<&crate::ast::NativeCallCache>,
    ) -> Result<Value, String> {
        // #55: AST 式を取るツリーウォーク入口の通過を数える（既定ビルドでは消える）。
        crate::interpreter::tw_stats::record_site(3);
        // #27-b: **引数を評価してから評価済み版へ委譲するだけ**にした。
        // 実装は `eval_method_call_full` に 1 つだけ置く（VM も同じものを通る）。
        //
        // ⚠ 評価順が 1 点だけ変わる: 以前は `expect_no_args`（旧 CallArg 版・削除済み）が**引数を評価せずに**
        // arity エラーを返していた（15 箇所）。今は先に評価する。正常系（0 引数）では
        // 差が無く、差が出るのは「no-arg メソッドに副作用つき引数を渡す」= どのみち
        // エラーになるコードだけ。`eval_one_arg`（同・削除済み）は元から全評価後に検査していたので不変。
        let evaled = self.eval_call_args(args)?;
        self.eval_method_call_full(obj, method_name, evaled, cache)
    }

    /// メソッド呼び出しの**唯一の実装**（#27-b）。評価済み引数を受ける。
    ///
    /// ツリーウォーク（`eval_method_call`）も VM（`Op::CallMethod` → `call_instance_method_evaled`
    /// の非 Instance フォールバック）も、レシーバ種別のディスパッチはすべてここを通る。
    /// **`*_evaled` 版とずれた実装を作らない**（#22 系列で実バグを 4 回出した形）。
    pub(crate) fn eval_method_call_full(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
        cache: Option<&crate::ast::NativeCallCache>,
    ) -> Result<Value, String> {
        // Result 型のメソッド: is_OK() → bool、is_ERR() → bool
        if let Value::ResultVal { ok, .. } = &obj {
            if !evaled.is_empty() {
                return Err(format!("TypeError: Result.{method_name}() takes no arguments"));
            }
            return match method_name {
                "is_OK" => Ok(Value::Bool(*ok)),
                "is_ERR" => Ok(Value::Bool(!ok)),
                _ => Err(format!(
                    "AttributeError: '{}' object has no method '{method_name}'",
                    self.type_name(&obj)
                )),
            };
        }
        match &obj {
            Value::List(items) => {
                match method_name {
                    "__iter__" => {
                        Self::expect_no_args_evaled(&evaled, "list", "__iter__")?;
                        return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values: items.borrow().clone(),
                            index: 0,
                        }))));
                    }
                    "append" => {
                        let item = Self::one_arg_evaled(evaled, "list", "append")?;
                        items.borrow_mut().push(item);
                        return Ok(Value::None);
                    }
                    "pop" => {
                        Self::expect_no_args_evaled(&evaled, "list", "pop")?;
                        let mut v = items.borrow_mut();
                        if v.is_empty() {
                            return Err("IndexError: pop from empty list".to_string());
                        }
                        return Ok(v.pop().unwrap());
                    }
                    _ => {}
                }
                Err(format!(
                    "AttributeError: 'list' object has no method '{method_name}'"
                ))
            }
            Value::FrozenList { ref state, ref layout } => {
                match method_name {
                    "__iter__" => {
                        Self::expect_no_args_evaled(&evaled, "fixed_list", "__iter__")?;
                        let st = state.borrow();
                        let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values,
                            index: 0,
                        }))))
                    }
                    "__contains__" => {
                        let needle = Self::one_arg_evaled(evaled, "fixed_list", "__contains__")?;
                        let st = state.borrow();
                        let found = (0..st.len)
                            .map(|i| layout.reconstruct_item(&st.data, i))
                            .any(|v| self.values_eq(&v, &needle));
                        Ok(Value::Bool(found))
                    }
                    "allocated_size" => {
                        Self::expect_no_args_evaled(&evaled, "fixed_list", "allocated_size")?;
                        Ok(Value::Int(state.borrow().allocated_size as i64))
                    }
                    "append" => {
                        let item = Self::one_arg_evaled(evaled, "fixed_list", "append")?;
                        match item {
                            Value::Instance(inst_rc) => {
                                let inst = inst_rc.borrow();
                                if inst.class.name != layout.class_name {
                                    return Err(format!(
                                        "TypeError: fixed_list.append(): expected instance of '{}', got '{}'",
                                        layout.class_name, inst.class.name
                                    ));
                                }
                                let mut st = state.borrow_mut();
                                // Grow capacity when full (double, minimum 1)
                                if st.len >= st.allocated_size {
                                    let new_cap = (st.allocated_size * 2).max(1);
                                    st.data.resize(new_cap * layout.stride, 0);
                                    st.allocated_size = new_cap;
                                }
                                // Write each field recursively (alphabetical order)
                                let base_offset = st.len * layout.stride;
                                let mut tmp = Vec::with_capacity(layout.stride);
                                Self::write_flat_instance(&inst, &layout.fields, &mut tmp)
                                    .ok_or_else(|| format!(
                                        "TypeError: fixed_list.append(): field type mismatch for class '{}'",
                                        layout.class_name
                                    ))?;
                                st.data[base_offset..base_offset + layout.stride]
                                    .copy_from_slice(&tmp);
                                st.len += 1;
                                Ok(Value::None)
                            }
                            other => Err(format!(
                                "TypeError: fixed_list.append(): expected class instance, got '{}'",
                                self.type_name(&other)
                            )),
                        }
                    }
                    _ => Err(format!(
                        "AttributeError: 'fixed_list' object has no method '{method_name}'"
                    )),
                }
            }
            Value::Str(s) => self.eval_str_method(s.clone(), method_name, evaled),
            Value::Complex(re, im) => {
                let re = *re;
                let im = *im;
                match method_name {
                    "real" => {
                        Self::expect_no_args_evaled(&evaled, "complex", "real")?;
                        Ok(Value::Float(re))
                    }
                    "imag" => {
                        Self::expect_no_args_evaled(&evaled, "complex", "imag")?;
                        Ok(Value::Float(im))
                    }
                    "angle" => {
                        Self::expect_no_args_evaled(&evaled, "complex", "angle")?;
                        Ok(Value::Float(im.atan2(re)))
                    }
                    _ => Err(format!(
                        "AttributeError: 'complex' object has no method '{method_name}'"
                    )),
                }
            }
            // #27-b: Instance のディスパッチは `call_instance_method_evaled` に 1 本化した。
            // 以前はこのアームが同じ判断（copy／method IC／gen／native／static・class 判定／
            // 不変性フィルタ／オーバーロード）を**二重に実装**しており、VM 経路と食い違う余地があった。
            Value::Instance(_) => {
                self.call_instance_method_evaled(obj, method_name, evaled, cache.map(|c| &c.2), None)
            }
            Value::Class(cls) => {
                // cs-dll static method dispatch
                if let Some(Value::Str(bp)) = cls.class_vars.get("__cs_bridge_path__") {
                    let bp_path = std::path::PathBuf::from(&**bp);
                    let class_name = cls.name.clone();
                    let ret_type: Option<String> = cls
                        .methods
                        .get(method_name)
                        .and_then(|overloads| overloads.first())
                        .and_then(|f| f.return_type.clone());
                    if let Some(bridge) = crate::interpreter::cs_dll_runtime::get_bridge(&bp_path) {
                        let evaled = evaled;
                        let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                        return crate::interpreter::cs_dll_runtime::call_static(
                            &bridge, &class_name, method_name, &arg_vals,
                            ret_type.as_deref(),
                        ).map_err(|e| format!("CsDll: {e}"));
                    }
                }
                // cs-proc static method dispatch
                if let Some(Value::Str(pp)) = cls.class_vars.get("__cs_proc_path__") {
                    let pp_path = std::path::PathBuf::from(&**pp);
                    let class_name = cls.name.clone();
                    let ret_type: Option<String> = cls
                        .methods
                        .get(method_name)
                        .and_then(|overloads| overloads.first())
                        .and_then(|f| f.return_type.clone());
                    let evaled = evaled;
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                    return crate::interpreter::cs_proc_runtime::call_static(
                        &pp_path, &class_name, method_name, &arg_vals,
                        ret_type.as_deref(),
                    ).map_err(|e| format!("CsProc: {e}"));
                }

                // クラスオブジェクトに対するメソッド呼び出し: static / class_method のみ許可
                let overloads =
                    self.lookup_method_in_class(cls, method_name)
                        .ok_or_else(|| {
                            format!(
                                "AttributeError: class '{}' has no method '{method_name}'",
                                cls.name
                            )
                        })?;

                if cls.static_method_names.contains(method_name) {
                    return if overloads.len() == 1 {
                        self.exec_fn_evaled(overloads[0].clone(), &evaled, None, method_name, None)
                    } else {
                        self.dispatch_overload_evaled(overloads, evaled, None, method_name, None)
                    };
                }

                if cls.class_method_names.contains(method_name) {
                    let cls_val = Value::Class(cls.clone());
                    let evaled = evaled;
                    let mut all_evaled: Vec<(Option<String>, Value, bool)> = vec![(None, cls_val, true)];
                    all_evaled.extend(evaled);
                    return if overloads.len() == 1 {
                        self.exec_fn_evaled(overloads[0].clone(), &all_evaled, None, method_name, None)
                    } else {
                        self.dispatch_overload_evaled(overloads, all_evaled, None, method_name, None)
                    };
                }

                Err(format!(
                    "TypeError: cannot call instance method '{method_name}' on class '{}' directly; use an instance",
                    cls.name
                ))
            }
            Value::Dict(d) => {
                match method_name {
                    // `d.key()` / `d.keys()` — キーのリストを返す
                    "key" | "keys" => {
                        Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                        Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_keys()))))
                    }
                    // `d.item()` / `d.values()` — 値のリストを返す
                    "item" | "values" => {
                        Self::expect_no_args_evaled(&evaled, "dict", method_name)?;
                        Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_items()))))
                    }
                    _ => Err(format!(
                        "AttributeError: 'dict' object has no method '{method_name}'"
                    )),
                }
            }
            Value::Set(s) => {
                match method_name {
                    "__iter__" => {
                        Self::expect_no_args_evaled(&evaled, "set", "__iter__")?;
                        let items = s.borrow().clone();
                        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values: items,
                            index: 0,
                        }))))
                    }
                    "add" => {
                        let item = Self::one_arg_evaled(evaled, "set", "add")?;
                        let mut s_mut = s.borrow_mut();
                        if !s_mut.iter().any(|v| self.values_eq(v, &item)) {
                            s_mut.push(item);
                        }
                        Ok(Value::None)
                    }
                    "discard" => {
                        let item = Self::one_arg_evaled(evaled, "set", "discard")?;
                        let mut s_mut = s.borrow_mut();
                        if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, &item)) {
                            s_mut.remove(pos);
                        }
                        Ok(Value::None)
                    }
                    "remove" => {
                        let item = Self::one_arg_evaled(evaled, "set", "remove")?;
                        let mut s_mut = s.borrow_mut();
                        if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, &item)) {
                            s_mut.remove(pos);
                            Ok(Value::None)
                        } else {
                            Err(format!("KeyError: {} is not in set", self.display(&item)))
                        }
                    }
                    "pop" => {
                        Self::expect_no_args_evaled(&evaled, "set", "pop")?;
                        let mut s_mut = s.borrow_mut();
                        if s_mut.is_empty() {
                            Err("KeyError: pop from an empty set".to_string())
                        } else {
                            Ok(s_mut.remove(0))
                        }
                    }
                    "clear" => {
                        Self::expect_no_args_evaled(&evaled, "set", "clear")?;
                        s.borrow_mut().clear();
                        Ok(Value::None)
                    }
                    "copy" => {
                        Self::expect_no_args_evaled(&evaled, "set", "copy")?;
                        Ok(Value::Set(Rc::new(RefCell::new(s.borrow().clone()))))
                    }
                    "union" => {
                        let other = Self::one_arg_evaled(evaled, "set", "union")?;
                        let other_items = self.set_other_items(&other, "union")?;
                        let mut result = s.borrow().clone();
                        for v in other_items {
                            if !result.iter().any(|x| self.values_eq(x, &v)) {
                                result.push(v);
                            }
                        }
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "intersection" => {
                        let other = Self::one_arg_evaled(evaled, "set", "intersection")?;
                        let other_items = self.set_other_items(&other, "intersection")?;
                        let result: Vec<Value> = s
                            .borrow()
                            .iter()
                            .filter(|v| other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned()
                            .collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "difference" => {
                        let other = Self::one_arg_evaled(evaled, "set", "difference")?;
                        let other_items = self.set_other_items(&other, "difference")?;
                        let result: Vec<Value> = s
                            .borrow()
                            .iter()
                            .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned()
                            .collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "symmetric_difference" => {
                        let other = Self::one_arg_evaled(evaled, "set", "symmetric_difference")?;
                        let other_items = self.set_other_items(&other, "symmetric_difference")?;
                        let s_ref = s.borrow();
                        let mut result: Vec<Value> = s_ref
                            .iter()
                            .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned()
                            .collect();
                        for v in &other_items {
                            if !s_ref.iter().any(|x| self.values_eq(x, v)) {
                                result.push(v.clone());
                            }
                        }
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "issubset" => {
                        let other = Self::one_arg_evaled(evaled, "set", "issubset")?;
                        let other_items = self.set_other_items(&other, "issubset")?;
                        let result = s
                            .borrow()
                            .iter()
                            .all(|v| other_items.iter().any(|x| self.values_eq(x, v)));
                        Ok(Value::Bool(result))
                    }
                    "issuperset" => {
                        let other = Self::one_arg_evaled(evaled, "set", "issuperset")?;
                        let other_items = self.set_other_items(&other, "issuperset")?;
                        let s_ref = s.borrow();
                        let result = other_items
                            .iter()
                            .all(|v| s_ref.iter().any(|x| self.values_eq(x, v)));
                        Ok(Value::Bool(result))
                    }
                    _ => Err(format!(
                        "AttributeError: 'set' object has no method '{method_name}'"
                    )),
                }
            }
            Value::Generator(state) => {
                if method_name != "next" {
                    return Err(format!(
                        "AttributeError: Generator object has no method '{method_name}'"
                    ));
                }
                Self::expect_no_args_evaled(&evaled, "Generator", "next")?;
                let mut s = state.borrow_mut();
                if s.index < s.values.len() {
                    // 次の yield 値を返してインデックスを進める
                    let val = s.values[s.index].clone();
                    s.index += 1;
                    Ok(val)
                } else {
                    // ジェネレータが枯渇した: for ループはこのエラーでループを終了する
                    Err("EndOfIteration: generator is exhausted".to_string())
                }
            }
            Value::Namespace(ns) => {
                // モジュール名前空間の場合: メンバを取り出して関数として呼び出す
                let member = ns.members.get(method_name).cloned().ok_or_else(|| {
                    format!(
                        "AttributeError: module '{}' has no attribute '{method_name}'",
                        ns.name
                    )
                })?;
                // ── C 軸（実行方式）へ委譲する（#22-c）──
                // ここは 22-a 時点で `eval_call` / `call_value_evaled` と並ぶ **3 つ目の**
                // 実行方式ディスパッチだった。アーム集合が微妙にずれており
                // （`Type`/`Instance` が無い一方 `Namespace` だけ有る）、
                // その種のずれが実バグになった（#22-a の `JsProcFn` 欠落）。
                //
                // ⚠ `NativeFunction` だけは委譲できない。`mut` ポインタの write-back に
                // **引数の式**が要るため（評価済みの値からは書き戻し先の変数が判らない）。
                // これは `eval_call` 側に残した例外と同じ理由で、引数メタデータを
                // 運べるようにする改修（B 軸）が済めば解消できる。
                //
                // FFI 境界検査の `node_id` に 0 を渡すのは意図的。`mod.func()` の戻り値検査は
                // 呼び出し元の `eval_call`（`Expr::Attr` 分岐）が行うので、ここで検査すると二重になる。
                match member {
                    Value::NativeFunction(fn_ref) => self.dispatch_native_evaled(&fn_ref, evaled.into_iter().map(|(_, v, _)| v).collect()),
                    other => {
                        let evaled = evaled;
                        self.call_value_evaled(other, evaled, method_name, None, 0)
                    }
                }
            }
            Value::PyObject(handle) => {
                // Python オブジェクトのメソッドを PyO3 経由で呼び出す
                let evaled = evaled;
                crate::interpreter::py_interop::call_py_method(handle, method_name, &evaled)
            }
            Value::FileObject(fd_rc) => {
                let fd_rc = fd_rc.clone();
                let evaled = evaled;
                self.exec_file_method(fd_rc, method_name, &evaled)
            }
            Value::AsyncManager(mgr_rc) => {
                match method_name {
                    "all_done" => {
                        Self::expect_no_args_evaled(&evaled, "AsyncManager", "all_done")?;
                        let all = mgr_rc.borrow().all_done();
                        Ok(Value::Bool(all))
                    }
                    "wait_for_finish" => {
                        // wait_for_finish(await_interval_msec = 100)
                        let evaled = evaled;
                        let interval_ms: u64 = match evaled.as_slice() {
                            [] => 100,
                            [(key, Value::Int(n), _)] if key.is_none() || key.as_deref() == Some("await_interval_msec") => (*n).max(1) as u64,
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
                                    std::thread::sleep(std::time::Duration::from_millis(
                                        interval_ms,
                                    ));
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
            Value::Signal(sig_rc) => {
                self.exec_signal_method(sig_rc.clone(), method_name, evaled)
            }
            Value::EventLoop(el_rc) => {
                self.exec_event_loop_method(el_rc.clone(), method_name, evaled)
            }
            Value::CsObject(obj_data) => {
                let class_name = obj_data.class_name.clone();
                let handle = obj_data.handle;
                let bp = obj_data.bridge_path.clone();
                let is_proc = obj_data.is_proc;
                let class = obj_data.class.clone();
                let ret_type: Option<String> = class
                    .methods
                    .get(method_name)
                    .and_then(|overloads| overloads.first())
                    .and_then(|f| f.return_type.clone());
                let evaled = evaled;
                let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                if is_proc {
                    crate::interpreter::cs_proc_runtime::call_instance(
                        &bp, &class_name, handle, method_name, &arg_vals,
                        ret_type.as_deref(),
                    ).map_err(|e| format!("CsProc: {e}"))
                } else {
                    match crate::interpreter::cs_dll_runtime::get_bridge(&bp) {
                        Some(bridge) => crate::interpreter::cs_dll_runtime::call_instance(
                            &bridge, &class_name, handle, method_name, &arg_vals,
                            ret_type.as_deref(),
                        ).map_err(|e| format!("CsDll: {e}")),
                        None => Err(format!("CsDll: bridge DLL not loaded for '{class_name}'")),
                    }
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    // ── メソッド引数検証ヘルパ ──────────────────────────────────────────────

    /// 引数が 0 個であることを検証する（評価済み版・#27-b）。
    ///
    /// ⚠ 旧 CallArg 版は**引数を評価せずに** arity エラーを返していた。今は先に評価される。
    /// エラーメッセージは同一なので、差が出るのは「no-arg メソッドに副作用つき引数を渡したとき、
    /// その副作用が起きるか」だけ（どのみちエラーになるコード）。
    fn expect_no_args_evaled(
        evaled: &[(Option<String>, Value, bool)],
        type_name: &str,
        method: &str,
    ) -> Result<(), String> {
        if evaled.is_empty() {
            Ok(())
        } else {
            Err(format!("TypeError: {type_name}.{method}() takes no arguments"))
        }
    }

    /// ちょうど 1 個であることを検証してその値を返す（評価済み版・#27-b）。
    /// 旧 CallArg 版（`eval_one_arg`・削除済み）も**元から全評価後に検査**していたので意味論は完全に同じ。
    fn one_arg_evaled(
        evaled: Vec<(Option<String>, Value, bool)>,
        type_name: &str,
        method: &str,
    ) -> Result<Value, String> {
        if evaled.len() != 1 {
            return Err(format!("TypeError: {type_name}.{method}() takes exactly 1 argument"));
        }
        Ok(evaled.into_iter().next().unwrap().1)
    }

    /// set 演算の引数（`set` または `list`）を `Vec<Value>` に変換する。
    fn set_other_items(&self, other: &Value, method: &str) -> Result<Vec<Value>, String> {
        match other {
            Value::Set(o) => Ok(o.borrow().clone()),
            Value::List(l) => Ok(l.borrow().clone()),
            _ => Err(format!(
                "TypeError: set.{method}() argument must be a set or list, not '{}'",
                self.type_name(other)
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // Signal メソッド
    // ---------------------------------------------------------------------------

}
