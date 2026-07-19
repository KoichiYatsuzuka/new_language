// classes/method_call.rs — メソッド呼び出し評価の中核: eval_method_call。

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
    /// オブジェクトのメソッドを呼び出して結果を返す。List / Str / Instance / Dict / Generator 等の各値型へディスパッチする。
    pub(crate) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        // Result 型のメソッド: is_OK() → bool、is_ERR() → bool
        if let Value::ResultVal { ok, .. } = &obj {
            if !args.is_empty() {
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
                        if !args.is_empty() {
                            return Err("TypeError: list.__iter__() takes no arguments".to_string());
                        }
                        return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values: items.borrow().clone(),
                            index: 0,
                        }))));
                    }
                    "append" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: list.append() takes exactly 1 argument".to_string());
                        }
                        let item = evaled.into_iter().next().unwrap().1;
                        items.borrow_mut().push(item);
                        return Ok(Value::None);
                    }
                    "pop" => {
                        if !args.is_empty() {
                            return Err("TypeError: list.pop() takes no arguments".to_string());
                        }
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
                        if !args.is_empty() {
                            return Err("TypeError: fixed_list.__iter__() takes no arguments".to_string());
                        }
                        let st = state.borrow();
                        let values = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values,
                            index: 0,
                        }))))
                    }
                    "__contains__" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: fixed_list.__contains__() takes exactly 1 argument".to_string());
                        }
                        let needle = &evaled[0].1;
                        let st = state.borrow();
                        let found = (0..st.len)
                            .map(|i| layout.reconstruct_item(&st.data, i))
                            .any(|v| self.values_eq(&v, needle));
                        Ok(Value::Bool(found))
                    }
                    "allocated_size" => {
                        if !args.is_empty() {
                            return Err("TypeError: fixed_list.allocated_size() takes no arguments".to_string());
                        }
                        Ok(Value::Int(state.borrow().allocated_size as i64))
                    }
                    "append" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: fixed_list.append() takes exactly 1 argument".to_string());
                        }
                        let item = evaled.into_iter().next().unwrap().1;
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
            Value::Str(s) => self.eval_str_method(s.clone(), method_name, args),
            Value::Complex(re, im) => {
                let re = *re;
                let im = *im;
                match method_name {
                    "real" => {
                        if !args.is_empty() {
                            return Err("TypeError: complex.real() takes no arguments".to_string());
                        }
                        Ok(Value::Float(re))
                    }
                    "imag" => {
                        if !args.is_empty() {
                            return Err("TypeError: complex.imag() takes no arguments".to_string());
                        }
                        Ok(Value::Float(im))
                    }
                    "angle" => {
                        if !args.is_empty() {
                            return Err("TypeError: complex.angle() takes no arguments".to_string());
                        }
                        Ok(Value::Float(im.atan2(re)))
                    }
                    _ => Err(format!(
                        "AttributeError: 'complex' object has no method '{method_name}'"
                    )),
                }
            }
            Value::Instance(inst_rc) => {
                // 組み込み copy() メソッド: __copy__ を優先し、なければ deepcopy
                if method_name == "copy" {
                    if !args.is_empty() {
                        return Err(format!(
                            "TypeError: {}.copy() takes no arguments",
                            inst_rc.borrow().class.name
                        ));
                    }
                    return self.copy_value(obj.clone());
                }

                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().flags() & crate::interpreter::value::INST_IMMUTABLE != 0;

                // gen_methods（`gen` キーワードで定義されたメソッド、例: `__iter__`）を優先的にチェック
                if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
                    return self.exec_generator(gen_fn, args, Some(obj.clone()));
                }

                // Native method dispatch — check NATIVE_METHODS before tree-walk.
                if crate::interpreter::native_api::lookup_native_method_ptr(&class.name, method_name).is_some() {
                    let evaled = self.eval_call_args(args)?;
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                    if let Some(result) = crate::interpreter::native_api::try_dispatch_native_method(
                        self, obj.clone(), method_name, arg_vals,
                    ) {
                        return result;
                    }
                }

                let overloads = self
                    .lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| {
                        format!(
                            "AttributeError: '{}' has no method '{method_name}'",
                            class.name
                        )
                    })?;

                // static / class_method はインスタンスからは呼び出せない
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

                // 不変インスタンスは `mut self` を要求するオーバーロードを除外する
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
                    self.exec_fn(callable[0].clone(), args, Some(obj.clone()), method_name, None)
                } else {
                    self.dispatch_overload(callable, args, Some(obj.clone()), None)
                }
            }
            Value::Class(cls) => {
                // cs-dll static method dispatch
                if let Some(Value::Str(bp)) = cls.class_vars.get("__cs_bridge_path__") {
                    let bp_path = std::path::PathBuf::from(bp.clone());
                    let class_name = cls.name.clone();
                    let ret_type: Option<String> = cls
                        .methods
                        .get(method_name)
                        .and_then(|overloads| overloads.first())
                        .and_then(|f| f.return_type.clone());
                    if let Some(bridge) = crate::interpreter::cs_dll_runtime::get_bridge(&bp_path) {
                        let evaled = self.eval_call_args(args)?;
                        let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                        return crate::interpreter::cs_dll_runtime::call_static(
                            &bridge, &class_name, method_name, &arg_vals,
                            ret_type.as_deref(),
                        ).map_err(|e| format!("CsDll: {e}"));
                    }
                }
                // cs-proc static method dispatch
                if let Some(Value::Str(pp)) = cls.class_vars.get("__cs_proc_path__") {
                    let pp_path = std::path::PathBuf::from(pp.clone());
                    let class_name = cls.name.clone();
                    let ret_type: Option<String> = cls
                        .methods
                        .get(method_name)
                        .and_then(|overloads| overloads.first())
                        .and_then(|f| f.return_type.clone());
                    let evaled = self.eval_call_args(args)?;
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
                        self.exec_fn(overloads[0].clone(), args, None, method_name, None)
                    } else {
                        self.dispatch_overload(overloads, args, None, None)
                    };
                }

                if cls.class_method_names.contains(method_name) {
                    let cls_val = Value::Class(cls.clone());
                    let evaled = self.eval_call_args(args)?;
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
                        if !args.is_empty() {
                            return Err(format!(
                                "TypeError: dict.{method_name}() takes no arguments"
                            ));
                        }
                        Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_keys()))))
                    }
                    // `d.item()` / `d.values()` — 値のリストを返す
                    "item" | "values" => {
                        if !args.is_empty() {
                            return Err(format!(
                                "TypeError: dict.{method_name}() takes no arguments"
                            ));
                        }
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
                        if !args.is_empty() {
                            return Err("TypeError: set.__iter__() takes no arguments".to_string());
                        }
                        let items = s.borrow().clone();
                        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                            values: items,
                            index: 0,
                        }))))
                    }
                    "add" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: set.add() takes exactly 1 argument".to_string());
                        }
                        let item = evaled.into_iter().next().unwrap().1;
                        let mut s_mut = s.borrow_mut();
                        if !s_mut.iter().any(|v| self.values_eq(v, &item)) {
                            s_mut.push(item);
                        }
                        Ok(Value::None)
                    }
                    "discard" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.discard() takes exactly 1 argument".to_string()
                            );
                        }
                        let item = &evaled[0].1;
                        let mut s_mut = s.borrow_mut();
                        if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, item)) {
                            s_mut.remove(pos);
                        }
                        Ok(Value::None)
                    }
                    "remove" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.remove() takes exactly 1 argument".to_string()
                            );
                        }
                        let item = &evaled[0].1;
                        let mut s_mut = s.borrow_mut();
                        if let Some(pos) = s_mut.iter().position(|v| self.values_eq(v, item)) {
                            s_mut.remove(pos);
                            Ok(Value::None)
                        } else {
                            Err(format!("KeyError: {} is not in set", self.display(item)))
                        }
                    }
                    "pop" => {
                        if !args.is_empty() {
                            return Err("TypeError: set.pop() takes no arguments".to_string());
                        }
                        let mut s_mut = s.borrow_mut();
                        if s_mut.is_empty() {
                            Err("KeyError: pop from an empty set".to_string())
                        } else {
                            Ok(s_mut.remove(0))
                        }
                    }
                    "clear" => {
                        if !args.is_empty() {
                            return Err("TypeError: set.clear() takes no arguments".to_string());
                        }
                        s.borrow_mut().clear();
                        Ok(Value::None)
                    }
                    "copy" => {
                        if !args.is_empty() {
                            return Err("TypeError: set.copy() takes no arguments".to_string());
                        }
                        Ok(Value::Set(Rc::new(RefCell::new(s.borrow().clone()))))
                    }
                    "union" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.union() takes exactly 1 argument".to_string()
                            );
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!(
                                "TypeError: set.union() argument must be a set or list, not '{}'",
                                self.type_name(other)
                            )),
                        };
                        let mut result = s.borrow().clone();
                        for v in other_items {
                            if !result.iter().any(|x| self.values_eq(x, &v)) {
                                result.push(v);
                            }
                        }
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "intersection" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: set.intersection() takes exactly 1 argument"
                                .to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.intersection() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result: Vec<Value> = s
                            .borrow()
                            .iter()
                            .filter(|v| other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned()
                            .collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "difference" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.difference() takes exactly 1 argument".to_string()
                            );
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.difference() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result: Vec<Value> = s
                            .borrow()
                            .iter()
                            .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned()
                            .collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "symmetric_difference" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.symmetric_difference() takes exactly 1 argument"
                                    .to_string(),
                            );
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.symmetric_difference() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
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
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.issubset() takes exactly 1 argument".to_string()
                            );
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.issubset() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result = s
                            .borrow()
                            .iter()
                            .all(|v| other_items.iter().any(|x| self.values_eq(x, v)));
                        Ok(Value::Bool(result))
                    }
                    "issuperset" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err(
                                "TypeError: set.issuperset() takes exactly 1 argument".to_string()
                            );
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.issuperset() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
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
                if !args.is_empty() {
                    return Err("TypeError: Generator.next() takes no arguments".to_string());
                }
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
                match member {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None, method_name, None),
                    Value::OverloadedFn(candidates) => {
                        let evaled = self.eval_call_args(args)?;
                        self.dispatch_overload_evaled(candidates, evaled, None, method_name, None)
                    }
                    Value::Class(cls) => self.instantiate(cls, args),
                    Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
                    Value::PyObject(handle) => {
                        let evaled = self.eval_call_args(args)?;
                        crate::interpreter::py_interop::call_py_object(&handle, &evaled)
                    }
                    Value::NativeFunction(fn_ref) => self.call_native_function(&fn_ref, args),
                    Value::JsProcFn { bridge_key, module_name, fn_name } => {
                        let evaled_args = self.eval_call_args(args)?;
                        let vals: Vec<Value> = evaled_args.into_iter().map(|(_, v, _)| v).collect();
                        crate::interpreter::js_proc_runtime::call_function(&bridge_key, &module_name, &fn_name, &vals)
                    }
                    other => Err(format!(
                        "TypeError: '{}' object is not callable",
                        self.type_name(&other)
                    )),
                }
            }
            Value::PyObject(handle) => {
                // Python オブジェクトのメソッドを PyO3 経由で呼び出す
                let evaled = self.eval_call_args(args)?;
                crate::interpreter::py_interop::call_py_method(handle, method_name, &evaled)
            }
            Value::FileObject(fd_rc) => {
                let fd_rc = fd_rc.clone();
                let evaled = self.eval_call_args(args)?;
                self.exec_file_method(fd_rc, method_name, &evaled)
            }
            Value::AsyncManager(mgr_rc) => {
                match method_name {
                    "all_done" => {
                        if !args.is_empty() {
                            return Err(
                                "TypeError: AsyncManager.all_done() takes no arguments".to_string()
                            );
                        }
                        let all = mgr_rc.borrow().all_done();
                        Ok(Value::Bool(all))
                    }
                    "wait_for_finish" => {
                        // wait_for_finish(await_interval_msec = 100)
                        let evaled = self.eval_call_args(args)?;
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
                                exception: Value::Str(e),
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
                self.exec_signal_method(sig_rc.clone(), method_name, args)
            }
            Value::EventLoop(el_rc) => {
                self.exec_event_loop_method(el_rc.clone(), method_name, args)
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
                let evaled = self.eval_call_args(args)?;
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

    // ---------------------------------------------------------------------------
    // Signal メソッド
    // ---------------------------------------------------------------------------

}
