// classes.rs — クラス・インスタンス管理
// (instantiate / eval_method_call / lookup_method_in_class / lookup_class_var / freeze_instance)
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーンを辿るメソッド・クラス変数の検索を提供する。
// List / Str / Dict / Generator などの組み込み型のメソッドディスパッチもここで行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::CallArg;

use super::{ByteModeRust, FileOpenModeRust, Interpreter, Value, ClassValue, FnValue, InstanceData, GeneratorState, RaisedError, RAISE_SENTINEL};
use super::str_methods::{regex_findall, regex_match, regex_search, regex_split, regex_sub, str_format};

// ---------------------------------------------------------------------------
// FileObject メソッド用ヘルパー（自由関数）
// ---------------------------------------------------------------------------

/// `(backward: bool = default)` 形式の単一引数を解析する。
fn file_bool_arg(
    evaled: &[(Option<String>, Value)],
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match evaled {
        [] => Ok(default),
        [(kw_opt, val)] => {
            if let Some(k) = kw_opt {
                if k != name {
                    return Err(format!("TypeError: unexpected keyword argument '{k}'"));
                }
            }
            match val {
                Value::Bool(b) => Ok(*b),
                _ => Err(format!("TypeError: '{name}' must be bool")),
            }
        }
        _ => Err(format!("TypeError: {name}() takes at most one argument")),
    }
}

/// `(content)` 形式の単一引数を解析して値への参照を返す。
fn file_content_arg<'a>(
    evaled: &'a [(Option<String>, Value)],
    name: &str,
) -> Result<&'a Value, String> {
    match evaled {
        [(kw_opt, val)] => {
            if let Some(k) = kw_opt {
                if k != name {
                    return Err(format!("TypeError: unexpected keyword argument '{k}'"));
                }
            }
            Ok(val)
        }
        [] => Err(format!("TypeError: missing required argument '{name}'")),
        _ => Err("TypeError: too many arguments".to_string()),
    }
}

/// `Vec<u8>` をバイトモードに応じた `Value` に変換する。
/// - Text モード: UTF-8 として `Value::Str` に変換
/// - Byte モード: バイト値のリスト `Value::List[Value::Int]` に変換
fn bytes_to_value(data: &[u8], byte_mode: &ByteModeRust) -> Value {
    match byte_mode {
        ByteModeRust::Text => Value::Str(String::from_utf8_lossy(data).into_owned()),
        ByteModeRust::Byte => Value::List(Rc::new(RefCell::new(
            data.iter().map(|&b| Value::Int(b as i64)).collect(),
        ))),
    }
}

/// `Value` をバイトモードに応じた `Vec<u8>` に変換する。
/// - Text モード: `Value::Str` → UTF-8 バイト列
/// - Byte モード: `Value::List[Value::Int]` → バイト列
fn value_to_bytes(val: &Value, byte_mode: &ByteModeRust) -> Result<Vec<u8>, String> {
    match byte_mode {
        ByteModeRust::Text => match val {
            Value::Str(s) => Ok(s.as_bytes().to_vec()),
            _ => Err("write() content must be str in text mode".to_string()),
        },
        ByteModeRust::Byte => match val {
            Value::List(items) => {
                let mut out = Vec::new();
                for item in items.borrow().iter() {
                    match item {
                        Value::Int(n) if (0..=255).contains(n) => out.push(*n as u8),
                        Value::Int(n) => return Err(format!("write() byte value {n} out of range 0-255")),
                        _ => return Err("write() content must be list[int] in byte mode".to_string()),
                    }
                }
                Ok(out)
            }
            _ => Err("write() content must be list[int] in byte mode".to_string()),
        },
    }
}

impl Interpreter {
    // --- インスタンスの凍結 ---

    /// インスタンスを不変化する: `immutable = true` にセットし、すべての `mut` フィールドを不変にする。
    ///
    /// `let` バインドされたインスタンスに適用される。以降は `mut self` メソッド呼び出しが禁止される。
    ///
    /// - `inst_rc`: 不変化するインスタンスへの共有参照
    pub(super) fn freeze_instance(inst_rc: &Rc<RefCell<InstanceData>>) {
        let mut inst = inst_rc.borrow_mut();
        inst.immutable = true;
        // すべてのフィールドを不変に変更する
        for (_, mutable) in inst.fields.values_mut() {
            *mutable = false;
        }
    }

    /// 値に対してフリーズプロトコルを適用する。
    ///
    /// インスタンスの場合: `__freeze__` メソッドが定義されていれば呼び出し、その後 `freeze_instance` を実行する。
    /// その他の型: 現時点では何もしない（将来の拡張用）。
    pub(super) fn apply_freeze_to_value(&mut self, val: &Value) -> Result<(), String> {
        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__")?;
                } else {
                    self.dispatch_overload(overloads, &[], Some(val.clone()))?;
                }
            }
            Self::freeze_instance(inst_rc);
        }
        Ok(())
    }

    // --- クラスのインスタンス化 ---

    /// クラスを引数付きでインスタンス化して `Value::Instance` を返す。
    ///
    /// 処理フロー:
    /// 1. `field_defaults` からデフォルトフィールドを初期化
    /// 2. `InstanceData` を構築して `Rc<RefCell>` に包む
    /// 3. `__init__` メソッドを呼び出す（オーバーロードがある場合は `dispatch_overload`）
    ///
    /// - `class`: インスタンス化するクラス定義
    /// - `call_args`: コンストラクタ引数リスト（AST の `CallArg`）
    ///
    /// 戻り値: `Ok(Value::Instance)` — 初期化済みインスタンス。`Err` — コンストラクタ実行エラー
    pub(super) fn instantiate(&mut self, class: Rc<ClassValue>, call_args: &[CallArg]) -> Result<Value, String> {
        // デフォルト値付きフィールドをインスタンスに事前設定する
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields, immutable: false }));
        let inst_val = Value::Instance(inst_rc);

        // `__init__` を呼び出す（定義がない場合はスキップ）
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn(init_overloads[0].clone(), call_args, Some(inst_val.clone()), "__init__")?;
            } else {
                self.dispatch_overload(init_overloads, call_args, Some(inst_val.clone()))?;
            }
        }

        Ok(inst_val)
    }

    /// オブジェクトのメソッドを呼び出して結果を返す。
    ///
    /// 各値型に対してディスパッチを行う:
    /// - `List`: `__iter__()` のみ対応（組み込みジェネレータを返す）
    /// - `Str`: `__iter__()` のみ対応（文字ごとのジェネレータを返す）
    /// - `Instance`: `gen_methods`（ジェネレータメソッド）を優先し、次に通常メソッドを検索
    ///   - 不変インスタンスは `mut self` メソッドを呼べない
    ///   - オーバーロードがある場合は `dispatch_overload` で解決する
    /// - `Dict`: `key()` / `item()` のみ対応
    /// - `Generator`: `next()` のみ対応（枯渇時は `EndOfIteration` エラー）
    ///
    /// - `obj`: メソッドを呼び出す対象の値
    /// - `method_name`: 呼び出すメソッド名
    /// - `args`: 呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Value)` — メソッドの返り値。`Err(message)` — AttributeError 等
    /// 評価済み引数リストでクラスをインスタンス化する（デコレータ適用などに使用）。
    pub(super) fn instantiate_evaled(
        &mut self,
        class: Rc<ClassValue>,
        evaled: Vec<(Option<String>, Value)>,
    ) -> Result<Value, String> {
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields, immutable: false }));
        let inst_val = Value::Instance(inst_rc);
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn_evaled(init_overloads[0].clone(), &evaled, Some(inst_val.clone()), "__init__")?;
            } else {
                self.dispatch_overload_evaled(init_overloads, evaled, Some(inst_val.clone()), "__init__")?;
            }
        }
        Ok(inst_val)
    }

    pub(super) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        match &obj {
            Value::List(items) => {
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: list.__iter__() takes no arguments".to_string());
                    }
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: items.borrow().clone(),
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'list' object has no method '{method_name}'"))
            }
            Value::Str(s) => {
                self.eval_str_method(s.clone(), method_name, args)
            }
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().immutable;

                // gen_methods（`gen` キーワードで定義されたメソッド、例: `__iter__`）を優先的にチェック
                if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
                    return self.exec_generator(gen_fn, args, Some(obj.clone()));
                }

                let overloads = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;

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
                    overloads.iter().filter(|f| {
                        f.params.first().map(|p| p.name != "self" || !p.mutable).unwrap_or(true)
                    }).cloned().collect()
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
                    self.exec_fn(callable[0].clone(), args, Some(obj.clone()), method_name)
                } else {
                    self.dispatch_overload(callable, args, Some(obj.clone()))
                }
            }
            Value::Class(cls) => {
                // クラスオブジェクトに対するメソッド呼び出し: static / class_method のみ許可
                let overloads = self.lookup_method_in_class(&cls, method_name)
                    .ok_or_else(|| format!("AttributeError: class '{}' has no method '{method_name}'", cls.name))?;

                if cls.static_method_names.contains(method_name) {
                    return if overloads.len() == 1 {
                        self.exec_fn(overloads[0].clone(), args, None, method_name)
                    } else {
                        self.dispatch_overload(overloads, args, None)
                    };
                }

                if cls.class_method_names.contains(method_name) {
                    let cls_val = Value::Class(cls.clone());
                    let evaled = self.eval_call_args(args)?;
                    let mut all_evaled = vec![(None, cls_val)];
                    all_evaled.extend(evaled);
                    return if overloads.len() == 1 {
                        self.exec_fn_evaled(overloads[0].clone(), &all_evaled, None, method_name)
                    } else {
                        self.dispatch_overload_evaled(overloads, all_evaled, None, method_name)
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
                            return Err(format!("TypeError: dict.{method_name}() takes no arguments"));
                        }
                        Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_keys()))))
                    }
                    // `d.item()` / `d.values()` — 値のリストを返す
                    "item" | "values" => {
                        if !args.is_empty() {
                            return Err(format!("TypeError: dict.{method_name}() takes no arguments"));
                        }
                        Ok(Value::List(Rc::new(RefCell::new(d.borrow().all_items()))))
                    }
                    _ => Err(format!("AttributeError: 'dict' object has no method '{method_name}'")),
                }
            }
            Value::Set(s) => {
                match method_name {
                    "__iter__" => {
                        if !args.is_empty() {
                            return Err("TypeError: set.__iter__() takes no arguments".to_string());
                        }
                        let items = s.borrow().clone();
                        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState { values: items, index: 0 }))))
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
                            return Err("TypeError: set.discard() takes exactly 1 argument".to_string());
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
                            return Err("TypeError: set.remove() takes exactly 1 argument".to_string());
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
                            return Err("TypeError: set.union() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.union() argument must be a set or list, not '{}'", self.type_name(other))),
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
                            return Err("TypeError: set.intersection() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.intersection() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result: Vec<Value> = s.borrow().iter()
                            .filter(|v| other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned().collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "difference" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: set.difference() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.difference() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result: Vec<Value> = s.borrow().iter()
                            .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned().collect();
                        Ok(Value::Set(Rc::new(RefCell::new(result))))
                    }
                    "symmetric_difference" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: set.symmetric_difference() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.symmetric_difference() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let s_ref = s.borrow();
                        let mut result: Vec<Value> = s_ref.iter()
                            .filter(|v| !other_items.iter().any(|x| self.values_eq(x, v)))
                            .cloned().collect();
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
                            return Err("TypeError: set.issubset() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.issubset() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let result = s.borrow().iter()
                            .all(|v| other_items.iter().any(|x| self.values_eq(x, v)));
                        Ok(Value::Bool(result))
                    }
                    "issuperset" => {
                        let evaled = self.eval_call_args(args)?;
                        if evaled.len() != 1 {
                            return Err("TypeError: set.issuperset() takes exactly 1 argument".to_string());
                        }
                        let other = &evaled[0].1;
                        let other_items: Vec<Value> = match other {
                            Value::Set(o) => o.borrow().clone(),
                            Value::List(l) => l.borrow().clone(),
                            _ => return Err(format!("TypeError: set.issuperset() argument must be a set or list, not '{}'", self.type_name(other))),
                        };
                        let s_ref = s.borrow();
                        let result = other_items.iter()
                            .all(|v| s_ref.iter().any(|x| self.values_eq(x, v)));
                        Ok(Value::Bool(result))
                    }
                    _ => Err(format!("AttributeError: 'set' object has no method '{method_name}'")),
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
                let member = ns.members.get(method_name)
                    .cloned()
                    .ok_or_else(|| format!(
                        "AttributeError: module '{}' has no attribute '{method_name}'",
                        ns.name
                    ))?;
                match member {
                    Value::Function(fn_val) => self.exec_fn(fn_val, args, None, method_name),
                    Value::OverloadedFn(candidates) => {
                        let evaled = self.eval_call_args(args)?;
                        self.dispatch_overload_evaled(candidates, evaled, None, method_name)
                    }
                    Value::Class(cls) => self.instantiate(cls, args),
                    Value::GeneratorFn(gen_fn) => self.exec_generator(gen_fn, args, None),
                    Value::PyObject(handle) => {
                        let evaled = self.eval_call_args(args)?;
                        super::py_interop::call_py_object(&handle, &evaled)
                    }
                    Value::NativeFunction(fn_ref) => {
                        self.call_native_function(&fn_ref, args)
                    }
                    other => Err(format!(
                        "TypeError: '{}' object is not callable", self.type_name(&other)
                    )),
                }
            }
            Value::PyObject(handle) => {
                // Python オブジェクトのメソッドを PyO3 経由で呼び出す
                let evaled = self.eval_call_args(args)?;
                super::py_interop::call_py_method(&handle, method_name, &evaled)
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
                            return Err("TypeError: AsyncManager.all_done() takes no arguments".to_string());
                        }
                        let all = mgr_rc.borrow().all_done();
                        Ok(Value::Bool(all))
                    }
                    "wait_for_finish" => {
                        // wait_for_finish(await_interval_msec = 100)
                        let evaled = self.eval_call_args(args)?;
                        let interval_ms: u64 = match evaled.as_slice() {
                            [] => 100,
                            [(key, Value::Int(n))] if key.is_none() || key.as_deref() == Some("await_interval_msec") => (*n).max(1) as u64,
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

                            if done { break; }

                            if abort_triggered {
                                // Cancel remaining pending tasks then wait for running ones
                                mgr_rc.borrow_mut().cancel_pending();
                                // Keep polling until all running threads finish
                                loop {
                                    {
                                        let mut mgr = mgr_rc.borrow_mut();
                                        mgr.poll_completed();
                                        if mgr.all_done() { break; }
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
                            if mgr.raise_immediately { mgr.first_error() } else { None }
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
                    _ => Err(format!("AttributeError: 'AsyncManager' has no method '{method_name}'")),
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    // ---------------------------------------------------------------------------
    // FileObject メソッド
    // ---------------------------------------------------------------------------

    fn exec_file_method(
        &mut self,
        fd_rc: Rc<RefCell<super::FileData>>,
        method_name: &str,
        evaled: &[(Option<String>, Value)],
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
                    let skip_end = if p > 0 && fd.content[p - 1] == b'\n' { p - 1 } else { p };
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
                            let ch = s.chars().rev().next().unwrap();
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
            _ => Err(format!("AttributeError: FileObject has no method '{method_name}'")),
        }
    }

    /// 評価済み引数でメソッドを呼び出す。`__getitem__` / `__setitem__` などの
    /// subscript ディスパッチ用。Instance と PyObject のみ対応する。
    pub(super) fn eval_method_call_evaled(
        &mut self,
        obj: Value,
        method_name: &str,
        evaled: Vec<(Option<String>, Value)>,
    ) -> Result<Value, String> {
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().immutable;

                let overloads = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;

                let callable: Vec<Rc<FnValue>> = if inst_immutable {
                    overloads.iter().filter(|f| {
                        f.params.first().map(|p| p.name != "self" || !p.mutable).unwrap_or(true)
                    }).cloned().collect()
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
                    self.exec_fn_evaled(callable[0].clone(), &evaled, Some(obj.clone()), method_name)
                } else {
                    self.dispatch_overload_evaled(callable, evaled, Some(obj.clone()), method_name)
                }
            }
            Value::PyObject(handle) => {
                super::py_interop::call_py_method(handle, method_name, &evaled)
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    // ── str メソッドディスパッチ ──────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    pub(super) fn eval_str_method(
        &mut self,
        s: String,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();

        // Helper: extract str from first positional arg
        macro_rules! arg_str {
            ($idx:expr, $name:literal) => {
                match vals.get($idx) {
                    Some(Value::Str(s)) => s.clone(),
                    Some(other) => return Err(format!(
                        "TypeError: {}.{}() argument {} must be str, not '{}'",
                        method_name, $name, $idx + 1,
                        self.type_name(other)
                    )),
                    None => return Err(format!("TypeError: {}() missing argument '{}'", method_name, $name)),
                }
            };
        }
        macro_rules! arg_opt_str {
            ($idx:expr) => {
                match vals.get($idx) {
                    Some(Value::Str(s)) => Some(s.clone()),
                    Some(Value::None) | None => None,
                    Some(other) => return Err(format!(
                        "TypeError: {}() argument must be str or None, not '{}'", method_name,
                        self.type_name(other)
                    )),
                }
            };
        }
        macro_rules! arg_int {
            ($idx:expr, $default:expr) => {
                match vals.get($idx) {
                    Some(Value::Int(n)) => *n,
                    None => $default,
                    Some(other) => return Err(format!(
                        "TypeError: {}() argument must be int, not '{}'", method_name,
                        self.type_name(other)
                    )),
                }
            };
        }

        match method_name {
            "__iter__" => {
                if !vals.is_empty() {
                    return Err("TypeError: str.__iter__() takes no arguments".to_string());
                }
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState { values: chars, index: 0 }))));
            }

            // ── 大文字・小文字変換 ──────────────────────────────────────────
            "upper"      => Ok(Value::Str(s.to_uppercase())),
            "lower"      => Ok(Value::Str(s.to_lowercase())),
            "swapcase"   => Ok(Value::Str(s.chars().map(|c| {
                if c.is_uppercase() { c.to_lowercase().next().unwrap_or(c) }
                else { c.to_uppercase().next().unwrap_or(c) }
            }).collect())),
            "capitalize" => Ok(Value::Str({
                let mut cs = s.chars();
                match cs.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase(),
                }
            })),
            "title" => Ok(Value::Str({
                let mut result = String::new();
                let mut capitalize_next = true;
                for c in s.chars() {
                    if c.is_whitespace() || !c.is_alphanumeric() {
                        capitalize_next = true;
                        result.push(c);
                    } else if capitalize_next {
                        result.extend(c.to_uppercase());
                        capitalize_next = false;
                    } else {
                        result.extend(c.to_lowercase());
                    }
                }
                result
            })),

            // ── 空白除去 ────────────────────────────────────────────────────
            "strip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::Str(match chars_arg {
                    None => s.trim().to_string(),
                    Some(ref ch) => s.trim_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }
            "lstrip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::Str(match chars_arg {
                    None => s.trim_start().to_string(),
                    Some(ref ch) => s.trim_start_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }
            "rstrip" => {
                let chars_arg = arg_opt_str!(0);
                Ok(Value::Str(match chars_arg {
                    None => s.trim_end().to_string(),
                    Some(ref ch) => s.trim_end_matches(|c: char| ch.contains(c)).to_string(),
                }))
            }

            // ── 分割 ────────────────────────────────────────────────────────
            "split" => {
                let sep = arg_opt_str!(0);
                let maxsplit = arg_int!(1, -1);
                let parts: Vec<Value> = match sep {
                    None => {
                        if maxsplit < 0 {
                            s.split_whitespace().map(|p| Value::Str(p.to_string())).collect()
                        } else {
                            let mut result: Vec<&str> = s.splitn(maxsplit as usize + 1, |c: char| c.is_whitespace()).collect();
                            result.retain(|p| !p.is_empty());
                            result.iter().map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str()).map(|p| Value::Str(p.to_string())).collect()
                        } else {
                            s.splitn(maxsplit as usize + 1, sep.as_str())
                                .map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }
            "rsplit" => {
                let sep = arg_opt_str!(0);
                let maxsplit = arg_int!(1, -1);
                let parts: Vec<Value> = match sep {
                    None => {
                        if maxsplit < 0 {
                            s.split_whitespace().map(|p| Value::Str(p.to_string())).collect()
                        } else {
                            let mut result: Vec<&str> = s.rsplitn(maxsplit as usize + 1, |c: char| c.is_whitespace()).collect();
                            result.reverse();
                            result.iter().map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str()).map(|p| Value::Str(p.to_string())).collect()
                        } else {
                            let mut v: Vec<&str> = s.rsplitn(maxsplit as usize + 1, sep.as_str()).collect();
                            v.reverse();
                            v.iter().map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                };
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }
            "splitlines" => {
                let parts: Vec<Value> = s.lines().map(|p| Value::Str(p.to_string())).collect();
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }

            // ── 結合 ────────────────────────────────────────────────────────
            "join" => {
                let iterable = vals.first()
                    .ok_or_else(|| "TypeError: join() missing argument 'iterable'".to_string())?;
                let items = match iterable {
                    Value::List(lst) => lst.borrow().clone(),
                    Value::Tuple(t) => t.all_values().to_vec(),
                    Value::Generator(g) => g.borrow().values[g.borrow().index..].to_vec(),
                    other => return Err(format!(
                        "TypeError: join() argument must be iterable, not '{}'", self.type_name(other)
                    )),
                };
                let parts: Vec<String> = items.iter().map(|v| self.display(v)).collect();
                Ok(Value::Str(parts.join(&s)))
            }

            // ── 検索 ────────────────────────────────────────────────────────
            "find" => {
                let sub = arg_str!(0, "sub");
                let start = arg_int!(1, 0).max(0) as usize;
                let end = arg_int!(2, s.len() as i64).max(0) as usize;
                let slice = &s[start.min(s.len())..end.min(s.len())];
                Ok(Value::Int(match slice.find(sub.as_str()) {
                    Some(i) => (i + start) as i64,
                    None => -1,
                }))
            }
            "rfind" => {
                let sub = arg_str!(0, "sub");
                let start = arg_int!(1, 0).max(0) as usize;
                let end = arg_int!(2, s.len() as i64).max(0) as usize;
                let slice = &s[start.min(s.len())..end.min(s.len())];
                Ok(Value::Int(match slice.rfind(sub.as_str()) {
                    Some(i) => (i + start) as i64,
                    None => -1,
                }))
            }
            "index" => {
                let sub = arg_str!(0, "sub");
                let start = arg_int!(1, 0).max(0) as usize;
                let end = arg_int!(2, s.len() as i64).max(0) as usize;
                let slice = &s[start.min(s.len())..end.min(s.len())];
                match slice.find(sub.as_str()) {
                    Some(i) => Ok(Value::Int((i + start) as i64)),
                    None => Err(format!("ValueError: substring '{}' not found", sub)),
                }
            }
            "rindex" => {
                let sub = arg_str!(0, "sub");
                let start = arg_int!(1, 0).max(0) as usize;
                let end = arg_int!(2, s.len() as i64).max(0) as usize;
                let slice = &s[start.min(s.len())..end.min(s.len())];
                match slice.rfind(sub.as_str()) {
                    Some(i) => Ok(Value::Int((i + start) as i64)),
                    None => Err(format!("ValueError: substring '{}' not found", sub)),
                }
            }
            "count" => {
                let sub = arg_str!(0, "sub");
                let start = arg_int!(1, 0).max(0) as usize;
                let end = arg_int!(2, s.len() as i64).max(0) as usize;
                let slice = &s[start.min(s.len())..end.min(s.len())];
                let n = if sub.is_empty() { slice.chars().count() + 1 }
                        else { slice.matches(sub.as_str()).count() };
                Ok(Value::Int(n as i64))
            }
            "contains" => {
                let sub = arg_str!(0, "sub");
                Ok(Value::Bool(s.contains(sub.as_str())))
            }
            "startswith" => {
                let prefix = arg_str!(0, "prefix");
                Ok(Value::Bool(s.starts_with(prefix.as_str())))
            }
            "endswith" => {
                let suffix = arg_str!(0, "suffix");
                Ok(Value::Bool(s.ends_with(suffix.as_str())))
            }

            // ── 置換 ────────────────────────────────────────────────────────
            "replace" => {
                let old = arg_str!(0, "old");
                let new = arg_str!(1, "new");
                let count = arg_int!(2, -1);
                Ok(Value::Str(if count < 0 {
                    s.replace(old.as_str(), new.as_str())
                } else {
                    s.replacen(old.as_str(), new.as_str(), count as usize)
                }))
            }
            "removeprefix" => {
                let prefix = arg_str!(0, "prefix");
                Ok(Value::Str(s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string()))
            }
            "removesuffix" => {
                let suffix = arg_str!(0, "suffix");
                Ok(Value::Str(s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string()))
            }

            // ── 書式変換 ────────────────────────────────────────────────────
            "format" => {
                let mut pos_args: Vec<Value> = Vec::new();
                let mut kw_args: Vec<(String, Value)> = Vec::new();
                for (kw, v) in self.eval_call_args(args)? {
                    if let Some(k) = kw { kw_args.push((k, v)); }
                    else { pos_args.push(v); }
                }
                let display_fn = |v: &Value| self.display(v);
                let result = str_format(&s, &pos_args, &kw_args, &display_fn)?;
                Ok(Value::Str(result))
            }

            // ── 文字判定 ────────────────────────────────────────────────────
            "isdigit"  => Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))),
            "isnumeric"=> Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_numeric()))),
            "isalpha"  => Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic()))),
            "isalnum"  => Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphanumeric()))),
            "isspace"  => Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_whitespace()))),
            "isupper"  => Ok(Value::Bool(!s.is_empty() && s.chars().any(|c| c.is_alphabetic()) && s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()))),
            "islower"  => Ok(Value::Bool(!s.is_empty() && s.chars().any(|c| c.is_alphabetic()) && s.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()))),
            "isascii"  => Ok(Value::Bool(s.is_ascii())),
            "isprintable" => Ok(Value::Bool(s.chars().all(|c| !c.is_control()))),

            // ── 幅揃え・ゼロ埋め ────────────────────────────────────────────
            "zfill" => {
                let width = arg_int!(0, 0).max(0) as usize;
                Ok(Value::Str(if s.len() >= width { s.clone() }
                    else { format!("{:0>width$}", s) }))
            }
            "ljust" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => return Err("TypeError: ljust() fillchar must be single char str".to_string()),
                };
                Ok(Value::Str(format!("{:<width$}", s, width = width).replace(' ', &fill.to_string()).replacen(&fill.to_string(), &fill.to_string(), width)))
            }
            "rjust" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => return Err("TypeError: rjust() fillchar must be single char str".to_string()),
                };
                if s.len() >= width { return Ok(Value::Str(s.clone())); }
                let pad = width - s.len();
                Ok(Value::Str(format!("{}{}", fill.to_string().repeat(pad), s)))
            }
            "center" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => return Err("TypeError: center() fillchar must be single char str".to_string()),
                };
                if s.len() >= width { return Ok(Value::Str(s.clone())); }
                let pad = width - s.len();
                let left = pad / 2;
                let right = pad - left;
                Ok(Value::Str(format!("{}{}{}", fill.to_string().repeat(left), s, fill.to_string().repeat(right))))
            }

            // ── 分割（区切りを含む） ─────────────────────────────────────────
            "partition" => {
                let sep = arg_str!(0, "sep");
                let (a, b, c) = match s.find(sep.as_str()) {
                    Some(i) => (&s[..i], sep.as_str(), &s[i + sep.len()..]),
                    None => (s.as_str(), "", ""),
                };
                Ok(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vec![
                        Value::Str(a.to_string()),
                        Value::Str(b.to_string()),
                        Value::Str(c.to_string()),
                    ],
                    vec!["str".to_string(), "str".to_string(), "str".to_string()],
                ))))
            }
            "rpartition" => {
                let sep = arg_str!(0, "sep");
                let (a, b, c) = match s.rfind(sep.as_str()) {
                    Some(i) => (&s[..i], sep.as_str(), &s[i + sep.len()..]),
                    None => ("", "", s.as_str()),
                };
                Ok(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(
                    vec![
                        Value::Str(a.to_string()),
                        Value::Str(b.to_string()),
                        Value::Str(c.to_string()),
                    ],
                    vec!["str".to_string(), "str".to_string(), "str".to_string()],
                ))))
            }

            // ── その他変換 ───────────────────────────────────────────────────
            "expandtabs" => {
                let tabsize = arg_int!(0, 8).max(0) as usize;
                Ok(Value::Str(s.replace('\t', &" ".repeat(tabsize))))
            }
            "encode" => {
                // 簡易実装: UTF-8 バイト列を int のリストで返す
                let bytes: Vec<Value> = s.as_bytes().iter().map(|b| Value::Int(*b as i64)).collect();
                Ok(Value::List(Rc::new(RefCell::new(bytes))))
            }
            "chars" => {
                // 文字リストを返す
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                Ok(Value::List(Rc::new(RefCell::new(chars))))
            }
            "ord" => {
                // 1 文字の文字列を ord 値 (int) に変換
                let mut cs = s.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) => Ok(Value::Int(c as i64)),
                    _ => Err("TypeError: ord() expected a character, but found a string of length != 1".to_string()),
                }
            }

            // ── 正規表現メソッド ─────────────────────────────────────────────
            "match" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => return Err(format!("TypeError: match() flags must be str, not '{}'", self.type_name(other))),
                };
                match regex_match(&s, &pattern, &flags)? {
                    Some(m) => Ok(Value::Str(m)),
                    None => Ok(Value::None),
                }
            }
            "search" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => return Err(format!("TypeError: search() flags must be str, not '{}'", self.type_name(other))),
                };
                match regex_search(&s, &pattern, &flags)? {
                    Some(m) => Ok(Value::Str(m)),
                    None => Ok(Value::None),
                }
            }
            "findall" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => return Err(format!("TypeError: findall() flags must be str, not '{}'", self.type_name(other))),
                };
                let matches = regex_findall(&s, &pattern, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    matches.into_iter().map(Value::Str).collect()
                ))))
            }
            "sub" => {
                let pattern = arg_str!(0, "pattern");
                let repl = arg_str!(1, "repl");
                let count = arg_int!(2, 0).max(0) as usize;
                let flags = match vals.get(3) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => return Err(format!("TypeError: sub() flags must be str, not '{}'", self.type_name(other))),
                };
                Ok(Value::Str(regex_sub(&s, &pattern, &repl, count, &flags)?))
            }
            "regex_split" => {
                let pattern = arg_str!(0, "pattern");
                let maxsplit = arg_int!(1, 0).max(0) as usize;
                let flags = match vals.get(2) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => return Err(format!("TypeError: regex_split() flags must be str, not '{}'", self.type_name(other))),
                };
                let parts = regex_split(&s, &pattern, maxsplit, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    parts.into_iter().map(Value::Str).collect()
                ))))
            }

            _ => Err(format!("AttributeError: 'str' object has no method '{method_name}'")),
        }
    }

    /// メソッドをクラスから検索する。クラス本体の `methods` マップのみを参照する。
    ///
    /// 注意: クラス間継承は無効化されており、trait ベースの継承のみパース時にサポートされる。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `method_name`: 検索するメソッド名
    ///
    /// 戻り値: `Some(Vec<Rc<FnValue>>)` — オーバーロード候補リスト。`None` — 見つからない
    pub(super) fn lookup_method_in_class(&self, class: &Rc<ClassValue>, method_name: &str) -> Option<Vec<Rc<FnValue>>> {
        if let Some(overloads) = class.methods.get(method_name) {
            return Some(overloads.clone());
        }
        // クラス間継承は無効。trait ベースの継承はパース時にのみサポートされる。
        None
    }

    /// `const` クラス変数をクラスの `class_vars` から検索する。
    ///
    /// 現在はクラス変数の継承（基底クラスへの遡及検索）は未実装。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `name`: クラス変数名
    ///
    /// 戻り値: `Some(Value)` — クラス変数の値。`None` — 見つからない
    pub(super) fn lookup_class_var(class: &Rc<ClassValue>, name: &str) -> Option<Value> {
        class.class_vars.get(name).cloned()
        // 注: 基底クラスへの遡及検索にはスコープへのアクセスが必要なため、現在は未実装
    }
}
