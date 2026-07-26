// eval/calls.rs — 呼び出し評価: 関数呼び出し・キャスト・型コンストラクタ呼び出し・値呼び出し・AsyncManager 生成。


use {
    std::cell::RefCell, std::rc::Rc, std::sync::Arc,
    crate::ast::{CallArg, Expr},
    crate::token::Span,
    crate::interpreter::{
        Interpreter, NativeFnRef, SliceValue, Value, Var,
    },
};
use super::*;

impl Interpreter {
    /// 関数呼び出し式 `func(args)` を評価する。
    /// テンプレート instantiate・メソッド呼び出し・組み込み関数・ユーザー定義関数・クラスコンストラクタ・
    /// ジェネレータ・ネイティブ関数・型コンストラクタなど、呼び出し先の種別に応じて適切なパスへ分岐する。
    pub(crate) fn eval_call(
        &mut self,
        func: &Expr,
        args: &[CallArg],
        call_span: &Span,
        cache: &crate::ast::NativeCallCache,
    ) -> Result<Value, String> {
        // ── インラインキャッシュ命中: AST に焼き込まれた typed ネイティブ関数 ──
        // スコープ検索・Value マッチ・組み込みチェックをすべて跳ばして直接ディスパッチ。
        // （充填条件: 不変バインディング + typed ABI あり — 下の NativeFunction アーム参照）
        let cached = cache.0.borrow().clone();
        if let Some(any_arc) = cached {
            if let Some(fn_ref) = any_arc.downcast_ref::<NativeFnRef>() {
                return self.dispatch_native_typed_exprs(fn_ref, &any_arc, args);
            }
        }

        if let Expr::TemplateInstantiate { base, type_args } = func {
            let tmpl_val = self.eval(base)?;
            return self.instantiate_template(tmpl_val, type_args, args);
        }
        if let Expr::Attr { object, attr, .. } = func {
            let obj_val = self.eval(object)?;
            return self.eval_method_call(obj_val, attr, args, Some(cache));
        }
        // ── R4: Arrow 関数呼び先キャッシュ命中（Ident のみ） ──
        // 不変グローバル関数と初回解決済みなら、builtin 判定・名前引き・name.clone を跳ばして
        // `scopes[0]` の slot から直接ディスパッチする。
        if let Expr::Ident(name) = func {
            if let Some(idx) = cache.1.get(self.slot_epoch) {
                let cached_fn = match self.scopes[0].slot(idx) {
                    Some(Var::Immutable(Value::Function(f))) => Some(f.clone()),
                    _ => None,
                };
                if let Some(f) = cached_fn {
                    #[cfg(debug_assertions)]
                    {
                        // キャッシュした呼び先が、名前引き解決と一致することを検証する。
                        let live = self.get_val(name);
                        debug_assert!(
                            matches!(&live, Some(Value::Function(lf)) if Rc::ptr_eq(lf, &f)),
                            "R4 callee cache mismatch for '{name}'"
                        );
                    }
                    return self.exec_fn(f, args, None, name, Some(call_span.clone()));
                }
                // 想定外（束縛が変わった等）は通常経路へ委譲する。
            }
        }

        if let Expr::Ident(name) = func {
            if let Some(result) = self.eval_builtin_ident_call(name, args) {
                return result;
            }
        }
        let call_name: &str = match func {
            Expr::Ident(n) => n,
            _ => "<anonymous>",
        };
        let callee = self.eval(func)?;
        match callee {
            Value::Function(fn_val) => {
                // R4: 不変グローバル関数への Ident 呼び出しなら global slot を焼き込む。
                if let Expr::Ident(name) = func {
                    if !self.scopes[self.frame_floor..].iter().any(|s| s.contains_key(name)) {
                        if let Some(idx) = self.scopes[0].slot_of(name) {
                            if matches!(self.scopes[0].slot(idx), Some(Var::Immutable(_))) {
                                cache.1.fill(self.slot_epoch, idx as u32);
                            }
                        }
                    }
                }
                self.exec_fn(fn_val, args, None, call_name, Some(call_span.clone()))
            }
            Value::OverloadedFn(candidates) => {
                let evaled_args = self.eval_call_args(args)?;
                self.dispatch_overload_evaled(candidates, evaled_args, None, call_name, Some(call_span.clone()))
            }
            Value::Class(cls) => self.instantiate(cls, args),
            Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
            Value::TemplateFn(_) | Value::TemplateClass(_) | Value::TemplateGenFn(_) => Err(
                "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
            ),
            Value::PyObject(handle) => {
                let evaled_args = self.eval_call_args(args)?;
                crate::interpreter::py_interop::call_py_object(&handle, &evaled_args)
            }
            Value::Instance(_) => {
                self.eval_method_call(callee, "__call__", args, None)
            }
            Value::NativeFunction(fn_ref) => {
                // ── キャッシュ充填（AST への焼き込み） ──
                // 条件: Ident 呼び出し + 不変バインディング + typed ABI あり +
                //       全引数が位置引数 + 引数数一致（≤16）。
                // 不変バインディングは再代入・再宣言とも禁止のため無効化は不要。
                if fn_ref.typed_sig.is_some()
                    && fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed) != 0
                    && fn_ref.n_params <= 16
                    && args.len() == fn_ref.n_params
                    && args.iter().all(|a| matches!(a, CallArg::Positional(_)))
                {
                    if let Expr::Ident(name) = func {
                        let immutable_binding =
                            self.get_var(name).map(|v| !v.is_mutable()).unwrap_or(false);
                        if immutable_binding {
                            *cache.0.borrow_mut() = Some(
                                fn_ref.clone()
                                    as Arc<dyn std::any::Any + Send + Sync>,
                            );
                        }
                    }
                }
                self.call_native_function(&fn_ref, args)
            }
            Value::JsProcFn(data) => {
                let evaled_args = self.eval_call_args(args)?;
                let vals: Vec<Value> = evaled_args.into_iter().map(|(_, v, _)| v).collect();
                crate::interpreter::js_proc_runtime::call_function(&data.bridge_key, &data.module_name, &data.fn_name, &vals)
            }
            Value::Type(type_name) => {
                self.eval_type_constructor_call(&type_name, args)
            }
            Value::Protocol(proto_name) => {
                Err(format!("TypeError: protocol '{proto_name}' cannot be instantiated"))
            }
            other => Err(format!("TypeError: '{}' object is not callable", self.type_name(&other))),
        }
    }

    /// キャスト式 `obj => TypeName` を評価する。
    ///
    /// 動作:
    /// 1. ターゲット型が `new_type` クラスの場合 → コンストラクタ呼び出し `TypeName(inner_val)`
    ///    (obj 自身が new_type インスタンスのときは先に `.value` を取り出してからラップする)
    /// 2. obj が new_type インスタンスかつターゲット型がそのベース型の場合 → `.value` を返す
    /// 3. オブジェクトがインスタンスで `__cast__[TypeName]` メソッドを持つ場合 → そのメソッドを呼び出す
    /// 4. それ以外 → TypeError
    pub(crate) fn eval_cast(&mut self, object: &crate::ast::Expr, type_name: &str) -> Result<Value, String> {
        let obj = self.eval(object)?;

        // new_type インスタンスなら内部値を先に取り出しておく
        let inner_val = if let Value::Instance(ref inst_rc) = obj {
            let b = inst_rc.borrow();
            if b.class.new_type_base.is_some() {
                b.class.field_index.get("value").and_then(|&idx| b.field_value(idx))
            } else {
                None
            }
        } else {
            None
        };

        // --- new_type へのダウンキャスト: TypeName(obj) と等価 ---
        // obj 自身が new_type インスタンスの場合は内部値を渡してネストを防ぐ
        if let Some(target_val) = self.get_val(type_name) {
            if let Value::Class(ref cls) = target_val {
                if cls.new_type_base.is_some() {
                    let cls_rc = cls.clone();
                    let arg = inner_val.unwrap_or(obj);
                    return self.instantiate_evaled(cls_rc, vec![(None, arg, true)]);
                }
            }
        }

        // --- list ⇒ fixed_list: flat conversion ---
        let target_is_fixed = type_name == "fixed_list"
            || type_name.starts_with("fixed_list[");
        let target_is_list = type_name == "list"
            || type_name.starts_with("list[");
        if target_is_fixed {
            return match obj {
                Value::FrozenList { .. } => Ok(obj),  // already a fixed_list
                Value::List(ref rc) => {
                    let items = rc.borrow().clone();
                    Self::try_flat_freeze(&items).ok_or_else(|| {
                        "CastError: cannot cast list to fixed_list: \
                         elements must be homogeneous class instances \
                         with only int/float fields".to_string()
                    })
                }
                _ => Err(format!(
                    "CastError: cannot cast '{}' to 'fixed_list'",
                    self.type_name(&obj)
                )),
            };
        }
        if target_is_list {
            if let Value::FrozenList { ref state, ref layout } = obj {
                let st = state.borrow();
                let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                return Ok(Value::List(Rc::new(RefCell::new(items))));
            }
        }

        // --- インスタンスの __cast__[TypeName] メソッド呼び出し ---
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();

                // new_type インスタンスをそのベース型にキャスト: .value を返す
                if let Some(ref base) = class.new_type_base {
                    if base == type_name {
                        let b = inst_rc.borrow();
                        let val = b.class.field_index.get("value").and_then(|&idx| b.field_value(idx));
                        return val.ok_or_else(|| {
                            format!("TypeError: '{}' has no 'value' field", class.name)
                        });
                    }
                }

                let method_key = format!("__cast__[{}]", type_name);
                let overloads = self
                    .lookup_method_in_class(&class, &method_key)
                    .ok_or_else(|| {
                        format!(
                        "TypeError: '{}' is not castable to '{}' (no __cast__[{}] method defined)",
                        class.name, type_name, type_name
                    )
                    })?;
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(obj), "__cast__", None)
                } else {
                    self.dispatch_overload(overloads, &[], Some(obj), None)
                }
            }
            other => Err(format!(
                "TypeError: cast operator '=>' requires an instance or new_type target, \
                 got '{}' cast to '{}'",
                self.type_name(other),
                type_name
            )),
        }
    }

    /// 型コンストラクタ呼び出し（`list(...)`, `dict(...)`, `str(...)` 等）を評価する。
    pub(crate) fn eval_type_constructor_call(
        &mut self,
        type_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        if type_name == "AsyncManager" {
            return self.make_async_manager(args);
        }
        // Signal[T]() — Arrow ネイティブのイベントソースを生成する。
        // テンプレート引数 T はランタイムでは無視する。
        if type_name == "Signal" {
            return Ok(Value::Signal(std::rc::Rc::new(std::cell::RefCell::new(
                crate::interpreter::event_loop::SignalData::new(),
            ))));
        }
        let evaled = self.eval_call_args(args)?;
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
        self.call_type_by_name_evaled(type_name, vals)
    }

    /// Dispatch an already-evaluated argument list to a built-in type constructor.
    /// Called by `eval_type_constructor_call` (from AST) and `call_value_with_args`
    /// (from native callbacks that hold a `Value::Type`).
    pub(crate) fn call_type_by_name_evaled(
        &mut self,
        type_name: &str,
        vals: Vec<Value>,
    ) -> Result<Value, String> {
        match type_name {
            "str" => {
                let has_instance_str = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__str__")
                } else {
                    false
                };
                if has_instance_str {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__str__", vec![])
                        .map(|r| match r {
                            Value::Str(s) => Value::Str(s),
                            other => Value::Str(self.display(&other)),
                        });
                }
                match vals.as_slice() {
                    [] => Ok(Value::Str(String::new())),
                    [v] => Ok(Value::Str(self.display(v))),
                    _ => Err("TypeError: str() takes at most 1 argument".to_string()),
                }
            },
            "int" => match vals.as_slice() {
                [] => Ok(Value::Int(0)),
                [Value::Int(n)] => Ok(Value::Int(*n)),
                [Value::Float(f)] => Ok(Value::Int(*f as i64)),
                [Value::Bool(b)] => Ok(Value::Int(if *b { 1 } else { 0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| format!("ValueError: invalid literal for int(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: int() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: int() takes at most 1 argument".to_string()),
            },
            "float" => match vals.as_slice() {
                [] => Ok(Value::Float(0.0)),
                [Value::Float(f)] => Ok(Value::Float(*f)),
                [Value::Int(n)] => Ok(Value::Float(*n as f64)),
                [Value::Bool(b)] => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| format!("ValueError: invalid literal for float(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: float() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: float() takes at most 1 argument".to_string()),
            },
            "complex" => match vals.as_slice() {
                [] => Ok(Value::Complex(0.0, 0.0)),
                [Value::Complex(re, im)] => Ok(Value::Complex(*re, *im)),
                [Value::Float(f)] => Ok(Value::Complex(*f, 0.0)),
                [Value::Int(n)] => Ok(Value::Complex(*n as f64, 0.0)),
                [Value::Complex(re, _), Value::Float(im2)] => Ok(Value::Complex(*re, *im2)),
                [Value::Float(re), Value::Float(im)] => Ok(Value::Complex(*re, *im)),
                [Value::Int(re), Value::Int(im)] => Ok(Value::Complex(*re as f64, *im as f64)),
                [Value::Int(re), Value::Float(im)] => Ok(Value::Complex(*re as f64, *im)),
                [Value::Float(re), Value::Int(im)] => Ok(Value::Complex(*re, *im as f64)),
                [other] => Err(format!(
                    "TypeError: complex() argument must be a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: complex() takes at most 2 arguments".to_string()),
            },
            "bool" => match vals.as_slice() {
                [] => Ok(Value::Bool(false)),
                [Value::Bool(b)] => Ok(Value::Bool(*b)),
                [Value::Int(n)] => Ok(Value::Bool(*n != 0)),
                [Value::Float(f)] => Ok(Value::Bool(*f != 0.0)),
                [Value::Str(s)] => Ok(Value::Bool(!s.is_empty())),
                [Value::None] => Ok(Value::Bool(false)),
                [Value::List(lst)] => Ok(Value::Bool(!lst.borrow().is_empty())),
                [Value::Set(s)] => Ok(Value::Bool(!s.borrow().is_empty())),
                [_] => Ok(Value::Bool(true)),
                _ => Err("TypeError: bool() takes at most 1 argument".to_string()),
            },
            "list" => match vals {
                ref v if v.is_empty() => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => match vals.into_iter().next().unwrap() {
                    Value::List(lst) => Ok(Value::List(lst)),
                    Value::FrozenList { state, layout } => {
                        let st = state.borrow();
                        let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    Value::Set(s) => Ok(Value::List(Rc::new(RefCell::new(s.borrow().clone())))),
                    Value::Str(s) => {
                        let chars = s.chars().map(|c| Value::Str(c.to_string())).collect();
                        Ok(Value::List(Rc::new(RefCell::new(chars))))
                    }
                    other => Err(format!(
                        "TypeError: '{}' object is not iterable",
                        self.type_name(&other)
                    )),
                },
                _ => Err("TypeError: list() takes at most 1 argument".to_string()),
            },
            "set" => match vals {
                ref v if v.is_empty() => Ok(Value::Set(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => {
                    let arg = vals.into_iter().next().unwrap();
                    let items: Vec<Value> = match arg {
                        Value::Set(s) => s.borrow().clone(),
                        Value::List(lst) => lst.borrow().clone(),
                        Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                        Value::Tuple(t) => t.all_values().to_vec(),
                        other => {
                            return Err(format!(
                                "TypeError: '{}' object is not iterable",
                                self.type_name(&other)
                            ))
                        }
                    };
                    let mut result: Vec<Value> = Vec::new();
                    for v in items {
                        set_insert(&mut result, v, self);
                    }
                    Ok(Value::Set(Rc::new(RefCell::new(result))))
                }
                _ => Err("TypeError: set() takes at most 1 argument".to_string()),
            },
            "slice" => {
                let check_index = |v: Value, label: &str| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        Value::Instance(ref inst) if inst.borrow().class.name == "Index" => {
                            Ok(Some(v))
                        }
                        other => Err(format!(
                            "TypeError: slice {label} must be int, Index, or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                let check_step = |v: Value| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        other => Err(format!(
                            "TypeError: slice step must be int or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                match vals.len() {
                    2 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        Ok(Value::Slice(Rc::new(SliceValue {
                            begin,
                            end,
                            step: None,
                        })))
                    }
                    3 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        let step = check_step(it.next().unwrap())?;
                        Ok(Value::Slice(Rc::new(SliceValue { begin, end, step })))
                    }
                    _ => Err("TypeError: slice() takes 2 or 3 arguments".to_string()),
                }
            }
            "uint" => match vals.as_slice() {
                [] => Ok(Value::UInt(0)),
                [Value::UInt(n)] => Ok(Value::UInt(*n)),
                [Value::Int(n)] => Ok(Value::UInt(*n as u64)),
                [Value::Bool(b)] => Ok(Value::UInt(if *b { 1 } else { 0 })),
                [other] => Err(format!(
                    "TypeError: uint() argument must be an integer, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: uint() takes at most 1 argument".to_string()),
            },
            "id" => {
                if vals.len() != 1 {
                    return Err("TypeError: id() takes exactly one argument".to_string());
                }
                let val = vals.into_iter().next().unwrap();
                let raw: u64 = match &val {
                    Value::Instance(rc) => Rc::as_ptr(rc) as u64,
                    Value::List(rc) => Rc::as_ptr(rc) as u64,
                    Value::Dict(rc) => Rc::as_ptr(rc) as u64,
                    Value::Function(rc) => Rc::as_ptr(rc) as u64,
                    Value::OverloadedFn(v) => v.as_ptr() as u64,
                    Value::Generator(rc) => Rc::as_ptr(rc) as u64,
                    Value::GeneratorFn(rc) => Rc::as_ptr(rc) as u64,
                    Value::Tuple(rc) => Rc::as_ptr(rc) as u64,
                    Value::Int(n) => *n as u64,
                    Value::UInt(n) => *n,
                    Value::Float(f) => f.to_bits(),
                    Value::Bool(b) => *b as u64,
                    Value::Str(s) => {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        s.hash(&mut h);
                        h.finish()
                    }
                    Value::None => 0u64,
                    _ => 0u64,
                };
                let pointer_cls = match self.get_val("pointer") {
                    Some(Value::Class(cls)) => cls,
                    _ => return Err("RuntimeError: 'pointer' type is not defined".to_string()),
                };
                let mut inst = crate::interpreter::InstanceData::new_empty(pointer_cls, 0);
                inst.store_field(0, Value::UInt(raw), true);
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
            "len" => {
                let has_instance_len = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__len__", vec![])
                        .and_then(|r| match r {
                            Value::Int(n) => Ok(Value::Int(n)),
                            other => Err(format!(
                                "TypeError: __len__ must return int, not '{}'",
                                self.type_name(&other)
                            )),
                        });
                }
                match vals.as_slice() {
                    [Value::List(lst)] => Ok(Value::Int(lst.borrow().len() as i64)),
                    [Value::FrozenList { state, .. }] => Ok(Value::Int(state.borrow().len as i64)),
                    [Value::Str(s)] => Ok(Value::Int(s.len() as i64)),
                    [Value::Dict(d)] => Ok(Value::Int(d.borrow().len() as i64)),
                    [Value::Set(s)] => Ok(Value::Int(s.borrow().len() as i64)),
                    [Value::Tuple(t)] => Ok(Value::Int(t.len() as i64)),
                    [other] => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(other)
                    )),
                    _ => Err("TypeError: len() takes exactly 1 argument".to_string()),
                }
            },
            // Result コンストラクタ: Ok(value) / Err(error)
            "Ok" => match vals.as_slice() {
                [v] => Ok(Value::ResultVal { ok: true, inner: Box::new(v.clone()) }),
                _ => Err("TypeError: Ok() takes exactly 1 argument".to_string()),
            },
            "Err" => match vals.as_slice() {
                [v] => Ok(Value::ResultVal { ok: false, inner: Box::new(v.clone()) }),
                _ => Err("TypeError: Err() takes exactly 1 argument".to_string()),
            },
            other => Err(format!("TypeError: '{}' object is not callable", other)),
        }
    }

    // --- ネイティブ関数呼び出し ---

    /// 任意の呼び出し可能な `Value` を評価済み引数リストで呼び出す。
    /// ネイティブコールバック `ar_call_fn` から呼ばれる。
    pub(crate) fn call_value_with_args(
        &mut self,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        // ネイティブ呼び出しは引数を保守的に mutable 扱い（従来動作）。
        let evaled: Vec<(Option<String>, Value, bool)> =
            args.into_iter().map(|v| (None, v, true)).collect();
        self.call_value_evaled(callee, evaled, "<fn>", None)
    }

    /// 評価済み引数（`is_mutable` フラグ込み）で任意の呼び出し可能値をディスパッチする。
    /// VM の `Call` op（正しい `is_mutable` フラグをコンパイル時に算出）と `call_value_with_args`
    /// の共通実装。
    pub(crate) fn call_value_evaled(
        &mut self,
        callee: Value,
        evaled: Vec<(Option<String>, Value, bool)>,
        fn_name: &str,
        call_span: Option<Span>,
    ) -> Result<Value, String> {
        match callee {
            Value::Function(fn_val) => {
                self.exec_fn_evaled(fn_val, &evaled, None, fn_name, call_span)
            }
            Value::OverloadedFn(candidates) => {
                self.dispatch_overload_evaled(candidates, evaled, None, fn_name, call_span)
            }
            Value::Class(cls) => self.instantiate_evaled(cls, evaled),
            Value::NativeFunction(fn_ref) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                self.dispatch_native_evaled(&fn_ref, vals)
            }
            Value::Type(type_name) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                self.call_type_by_name_evaled(&type_name, vals)
            }
            Value::Instance(_) => self.eval_method_call_evaled(callee, "__call__", evaled),
            Value::PyObject(ref handle) => {
                crate::interpreter::py_interop::call_py_object(handle, &evaled)
            }
            other => Err(format!(
                "TypeError: '{}' object is not callable",
                self.type_name(&other)
            )),
        }
    }

    /// VM 用: グローバルスコープ（`scopes[0]`）から名前の値を引く。呼び先解決に使う。
    /// 呼び出し元スコープを跨がず、トップレベル関数の自由名＝グローバルという規則に一致する。
    pub(crate) fn vm_get_global(&self, name: &str) -> Option<Value> {
        self.scopes[0].get(name).map(|v| v.get_value())
    }

    /// AsyncManager(num_thread=N [, raise_immediately=bool]) コンストラクタ
    pub(crate) fn make_async_manager(&mut self, args: &[crate::ast::CallArg]) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;

        let mut num_thread: usize = 1;
        let mut raise_immediately: bool = false;

        match evaled.as_slice() {
            [] => {}
            _ => {
                for (kw, val, _) in &evaled {
                    match kw.as_deref() {
                        Some("num_thread") | None => {
                            match val {
                                Value::Int(n) if *n > 0 => num_thread = *n as usize,
                                Value::UInt(n) => num_thread = *n as usize,
                                other => return Err(format!(
                                    "TypeError: AsyncManager num_thread must be a positive int, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some("raise_immediately") => {
                            match val {
                                Value::Bool(b) => raise_immediately = *b,
                                other => return Err(format!(
                                    "TypeError: AsyncManager raise_immediately must be bool, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some(k) => return Err(format!(
                            "TypeError: AsyncManager() got unexpected keyword argument '{k}'"
                        )),
                    }
                }
            }
        }

        let mgr = crate::interpreter::async_mgr::AsyncManagerData::new(num_thread, raise_immediately);
        Ok(Value::AsyncManager(Rc::new(RefCell::new(mgr))))
    }
}
