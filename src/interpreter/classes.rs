// classes.rs — クラス・インスタンス管理
// (instantiate / eval_method_call / lookup_method_in_class / lookup_class_var / freeze_instance / copy_value)
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーンを辿るメソッド・クラス変数の検索を提供する。
// List / Str / Dict / Generator などの組み込み型のメソッドディスパッチもここで行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::CallArg;

use super::str_methods::{
    regex_findall, regex_match, regex_search, regex_split, regex_sub, str_format,
};
use super::{
    ByteModeRust, ClassValue, FileOpenModeRust, FnValue, GeneratorState, InstanceData, Interpreter,
    RaisedError, Value, RAISE_SENTINEL,
};

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
                        Value::Int(n) => {
                            return Err(format!("write() byte value {n} out of range 0-255"))
                        }
                        _ => {
                            return Err("write() content must be list[int] in byte mode".to_string())
                        }
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
    /// 同一クラス・全フィールドが SWD 型（int/float または別の SWD クラス）の
    /// `Value::List` を平坦バイト配列に変換する。
    /// 変換できない場合は `None` を返す。フィールドはアルファベット順で格納される。
    pub(super) fn try_flat_freeze(items: &[Value]) -> Option<Value> {
        if items.is_empty() {
            return None;
        }
        let first_rc = match &items[0] {
            Value::Instance(rc) => rc.clone(),
            _ => return None,
        };
        let class = first_rc.borrow().class.clone();
        let class_name = class.name.clone();

        let layout = {
            let inst = first_rc.borrow();
            Self::build_flat_layout_from_instance(&inst, class.clone())?
        };

        let mut data: Vec<u8> = Vec::with_capacity(items.len() * layout.stride);
        for item in items {
            let inst_rc = match item {
                Value::Instance(rc) => rc.clone(),
                _ => return None,
            };
            let inst = inst_rc.borrow();
            if inst.class.name != class_name { return None; }
            Self::write_flat_instance(&inst.fields, &layout.fields, &mut data)?;
        }

        let len = items.len();
        Some(Value::FrozenList {
            state: Rc::new(RefCell::new(super::value::FlatListData {
                data,
                len,
                allocated_size: len,
            })),
            layout: Rc::new(layout),
        })
    }

    /// インスタンスの fields から `FlatLayout` を再帰的に構築する。
    fn build_flat_layout_from_instance(
        inst: &InstanceData,
        class: Rc<ClassValue>,
    ) -> Option<super::value::FlatLayout> {
        let mut flds: Vec<(String, super::value::FlatFieldTy)> = inst.fields
            .iter()
            .map(|(name, (val, _))| {
                let fty = Self::val_to_flat_field_ty(val)?;
                Some((name.clone(), fty))
            })
            .collect::<Option<Vec<_>>>()?;
        if flds.is_empty() { return None; }
        flds.sort_by(|a, b| a.0.cmp(&b.0));
        let stride: usize = flds.iter().map(|(_, ft)| ft.stride()).sum();
        Some(super::value::FlatLayout {
            class_name: class.name.clone(),
            fields: flds,
            stride,
            class,
        })
    }

    /// 単一の `Value` から `FlatFieldTy` を導出する（再帰的）。
    fn val_to_flat_field_ty(val: &Value) -> Option<super::value::FlatFieldTy> {
        match val {
            Value::Int(_)   => Some(super::value::FlatFieldTy::Int),
            Value::Float(_) => Some(super::value::FlatFieldTy::Float),
            Value::Instance(rc) => {
                let inst = rc.borrow();
                let sub = Self::build_flat_layout_from_instance(&inst, inst.class.clone())?;
                Some(super::value::FlatFieldTy::Struct(Rc::new(sub)))
            }
            _ => None,
        }
    }

    /// `fields` マップの値を `layout_fields` の順序に従って `data` に書き出す（再帰的）。
    fn write_flat_instance(
        fields: &std::collections::HashMap<String, (Value, bool)>,
        layout_fields: &[(String, super::value::FlatFieldTy)],
        data: &mut Vec<u8>,
    ) -> Option<()> {
        for (field_name, field_ty) in layout_fields {
            let (val, _) = fields.get(field_name)?;
            match (field_ty, val) {
                (super::value::FlatFieldTy::Int, Value::Int(n)) => {
                    data.extend_from_slice(&n.to_le_bytes());
                }
                (super::value::FlatFieldTy::Float, Value::Float(f)) => {
                    data.extend_from_slice(&f.to_le_bytes());
                }
                (super::value::FlatFieldTy::Struct(sub_layout), Value::Instance(rc)) => {
                    let inst = rc.borrow();
                    if inst.class.name != sub_layout.class_name { return None; }
                    Self::write_flat_instance(&inst.fields, &sub_layout.fields, data)?;
                }
                _ => return None,
            }
        }
        Some(())
    }

    pub(super) fn freeze_instance(inst_rc: &Rc<RefCell<InstanceData>>) {
        let mut inst = inst_rc.borrow_mut();
        inst.immutable = true;
        // すべてのフィールドを不変に変更する
        for (_, mutable) in inst.fields.values_mut() {
            *mutable = false;
        }
    }

    /// フリーズプロトコル。
    ///
    /// `freeze_fields=true` のとき: `__freeze__` フックを呼び出し、インスタンスのフィールドを不変化する。
    ///   `freeze` 文でコレクションを再帰的にフリーズする際に使用する。
    ///
    /// `freeze_fields=false` のとき: `__freeze__` フックのみ呼び出し、フィールドは不変化しない。
    ///   `let` バインドではインスタンスの Rc 参照は共有されるため、フィールドを凍結すると
    ///   他のすべての参照にも影響してしまう。`let` バインドは変数の再バインドを禁止するのみで、
    ///   オブジェクトのフィールドの可変性には影響しない。
    pub(super) fn apply_freeze_to_value(&mut self, val: &Value, freeze_fields: bool) -> Result<(), String> {
        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__freeze__") {
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(val.clone()), "__freeze__", None)?;
                } else {
                    self.dispatch_overload(overloads, &[], Some(val.clone()), None)?;
                }
            }
            if freeze_fields {
                Self::freeze_instance(inst_rc);
            }
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
    pub(super) fn instantiate(
        &mut self,
        class: Rc<ClassValue>,
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        // デフォルト値付きフィールドをインスタンスに事前設定する
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData {
            class: class.clone(),
            fields,
            immutable: false,
        }));
        let inst_val = Value::Instance(inst_rc);

        // cs-dll / cs-proc bridge dispatch: check class_vars for bridge path markers.
        let class_name = class.name.clone();
        if let Some(Value::Str(bp)) = class.class_vars.get("__cs_bridge_path__") {
            let bp_path = std::path::PathBuf::from(bp.clone());
            let evaled = self.eval_call_args(call_args)?;
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
            if let Some(bridge) = super::cs_dll_runtime::get_bridge(&bp_path) {
                let handle = super::cs_dll_runtime::call_constructor(&bridge, &class_name, &arg_vals)
                    .map_err(|e| format!("CsDll: constructor for '{class_name}' failed: {e}"))?;
                return Ok(Value::CsObject(Rc::new(super::value::CsObjectData {
                    class_name: class_name.clone(),
                    handle,
                    bridge_path: bp_path,
                    class: class.clone(),
                    is_proc: false,
                })));
            }
        }
        if let Some(Value::Str(pp)) = class.class_vars.get("__cs_proc_path__") {
            let pp_path = std::path::PathBuf::from(pp.clone());
            let evaled = self.eval_call_args(call_args)?;
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
            let handle = super::cs_proc_runtime::call_constructor(&pp_path, &class_name, &arg_vals)
                .map_err(|e| format!("CsProc: constructor for '{class_name}' failed: {e}"))?;
            return Ok(Value::CsObject(Rc::new(super::value::CsObjectData {
                class_name: class_name.clone(),
                handle,
                bridge_path: pp_path,
                class: class.clone(),
                is_proc: true,
            })));
        }

        // Native __init__ dispatch (for import[rs] structs and compiled classes).
        // Check NATIVE_METHODS before falling back to tree-walk.
        if super::native_api::lookup_native_method_ptr(&class_name, "__init__").is_some() {
            let evaled = self.eval_call_args(call_args)?;
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
            if let Some(result) = super::native_api::try_dispatch_native_method(
                self, inst_val.clone(), "__init__", arg_vals,
            ) {
                result?;
            }
            return Ok(inst_val);
        }

        // `__init__` を呼び出す（定義がない場合はスキップ）
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn(
                    init_overloads[0].clone(),
                    call_args,
                    Some(inst_val.clone()),
                    "__init__",
                    None,
                )?;
            } else {
                self.dispatch_overload(init_overloads, call_args, Some(inst_val.clone()), None)?;
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
        let inst_rc = Rc::new(RefCell::new(InstanceData {
            class: class.clone(),
            fields,
            immutable: false,
        }));
        let inst_val = Value::Instance(inst_rc);
        // Native __init__ dispatch
        let class_name = class.name.clone();
        if super::native_api::lookup_native_method_ptr(&class_name, "__init__").is_some() {
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
            if let Some(result) = super::native_api::try_dispatch_native_method(
                self, inst_val.clone(), "__init__", arg_vals,
            ) {
                result?;
            }
            return Ok(inst_val);
        }
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn_evaled(
                    init_overloads[0].clone(),
                    &evaled,
                    Some(inst_val.clone()),
                    "__init__",
                    None,
                )?;
            } else {
                self.dispatch_overload_evaled(
                    init_overloads,
                    evaled,
                    Some(inst_val.clone()),
                    "__init__",
                    None,
                )?;
            }
        }
        Ok(inst_val)
    }

    /// オブジェクトのメソッドを呼び出して結果を返す。List / Str / Instance / Dict / Generator 等の各値型へディスパッチする。
    pub(super) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
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
                                Self::write_flat_instance(&inst.fields, &layout.fields, &mut tmp)
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
                let inst_immutable = inst_rc.borrow().immutable;

                // gen_methods（`gen` キーワードで定義されたメソッド、例: `__iter__`）を優先的にチェック
                if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
                    return self.exec_generator(gen_fn, args, Some(obj.clone()));
                }

                // Native method dispatch — check NATIVE_METHODS before tree-walk.
                if super::native_api::lookup_native_method_ptr(&class.name, method_name).is_some() {
                    let evaled = self.eval_call_args(args)?;
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                    if let Some(result) = super::native_api::try_dispatch_native_method(
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
                    if let Some(bridge) = super::cs_dll_runtime::get_bridge(&bp_path) {
                        let evaled = self.eval_call_args(args)?;
                        let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                        return super::cs_dll_runtime::call_static(
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
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                    return super::cs_proc_runtime::call_static(
                        &pp_path, &class_name, method_name, &arg_vals,
                        ret_type.as_deref(),
                    ).map_err(|e| format!("CsProc: {e}"));
                }

                // クラスオブジェクトに対するメソッド呼び出し: static / class_method のみ許可
                let overloads =
                    self.lookup_method_in_class(&cls, method_name)
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
                    let mut all_evaled = vec![(None, cls_val)];
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
                        super::py_interop::call_py_object(&handle, &evaled)
                    }
                    Value::NativeFunction(fn_ref) => self.call_native_function(&fn_ref, args),
                    other => Err(format!(
                        "TypeError: '{}' object is not callable",
                        self.type_name(&other)
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
                let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                if is_proc {
                    super::cs_proc_runtime::call_instance(
                        &bp, &class_name, handle, method_name, &arg_vals,
                        ret_type.as_deref(),
                    ).map_err(|e| format!("CsProc: {e}"))
                } else {
                    match super::cs_dll_runtime::get_bridge(&bp) {
                        Some(bridge) => super::cs_dll_runtime::call_instance(
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

    /// `Signal[T]` のメソッド呼び出しを処理する。
    fn exec_signal_method(
        &mut self,
        sig_rc: std::rc::Rc<std::cell::RefCell<super::event_loop::SignalData>>,
        method_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        match method_name {
            "emit" => {
                let evaled = self.eval_call_args(args)?;
                let val = match evaled.as_slice() {
                    [(_, v)] => v.clone(),
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
                    [(_, v)] => v.clone(),
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
    fn exec_event_loop_method(
        &mut self,
        el_rc: std::rc::Rc<std::cell::RefCell<super::event_loop::EventLoopData>>,
        method_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Result<Value, String> {
        match method_name {
            "run" => {
                // EventLoop.run([timeout: float])
                let evaled = self.eval_call_args(args)?;
                let timeout_ms: Option<u64> = match evaled.as_slice() {
                    [] => None,
                    [(key, Value::Float(f))]
                        if key.is_none() || key.as_deref() == Some("timeout") =>
                    {
                        Some((f * 1000.0) as u64)
                    }
                    [(key, Value::Int(n))]
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
                    [(_, v)] => v.clone(),
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
            _ => Err(format!(
                "AttributeError: FileObject has no method '{method_name}'"
            )),
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

                // Native method dispatch — check NATIVE_METHODS before tree-walk.
                // When a native ptr is registered we always dispatch natively (no fallback).
                if super::native_api::lookup_native_method_ptr(&class.name, method_name).is_some() {
                    let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v)| v).collect();
                    return super::native_api::try_dispatch_native_method(
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
                super::py_interop::call_py_method(handle, method_name, &evaled)
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    // ── str メソッドディスパッチ ──────────────────────────────────────────────

    /// 文字列値のメソッド（`split` / `strip` / `replace` / `startswith` 等）を評価して結果を返す。
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
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}.{}() argument {} must be str, not '{}'",
                            method_name,
                            $name,
                            $idx + 1,
                            self.type_name(other)
                        ))
                    }
                    None => {
                        return Err(format!(
                            "TypeError: {}() missing argument '{}'",
                            method_name, $name
                        ))
                    }
                }
            };
        }
        macro_rules! arg_opt_str {
            ($idx:expr) => {
                match vals.get($idx) {
                    Some(Value::Str(s)) => Some(s.clone()),
                    Some(Value::None) | None => None,
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}() argument must be str or None, not '{}'",
                            method_name,
                            self.type_name(other)
                        ))
                    }
                }
            };
        }
        macro_rules! arg_int {
            ($idx:expr, $default:expr) => {
                match vals.get($idx) {
                    Some(Value::Int(n)) => *n,
                    None => $default,
                    Some(other) => {
                        return Err(format!(
                            "TypeError: {}() argument must be int, not '{}'",
                            method_name,
                            self.type_name(other)
                        ))
                    }
                }
            };
        }

        match method_name {
            "__iter__" => {
                if !vals.is_empty() {
                    return Err("TypeError: str.__iter__() takes no arguments".to_string());
                }
                let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: chars,
                    index: 0,
                }))));
            }

            // ── 大文字・小文字変換 ──────────────────────────────────────────
            "upper" => Ok(Value::Str(s.to_uppercase())),
            "lower" => Ok(Value::Str(s.to_lowercase())),
            "swapcase" => Ok(Value::Str(
                s.chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().next().unwrap_or(c)
                        } else {
                            c.to_uppercase().next().unwrap_or(c)
                        }
                    })
                    .collect(),
            )),
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
                            s.split_whitespace()
                                .map(|p| Value::Str(p.to_string()))
                                .collect()
                        } else {
                            let mut result: Vec<&str> = s
                                .splitn(maxsplit as usize + 1, |c: char| c.is_whitespace())
                                .collect();
                            result.retain(|p| !p.is_empty());
                            result.iter().map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str())
                                .map(|p| Value::Str(p.to_string()))
                                .collect()
                        } else {
                            s.splitn(maxsplit as usize + 1, sep.as_str())
                                .map(|p| Value::Str(p.to_string()))
                                .collect()
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
                            s.split_whitespace()
                                .map(|p| Value::Str(p.to_string()))
                                .collect()
                        } else {
                            let mut result: Vec<&str> = s
                                .rsplitn(maxsplit as usize + 1, |c: char| c.is_whitespace())
                                .collect();
                            result.reverse();
                            result.iter().map(|p| Value::Str(p.to_string())).collect()
                        }
                    }
                    Some(ref sep) => {
                        if maxsplit < 0 {
                            s.split(sep.as_str())
                                .map(|p| Value::Str(p.to_string()))
                                .collect()
                        } else {
                            let mut v: Vec<&str> =
                                s.rsplitn(maxsplit as usize + 1, sep.as_str()).collect();
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
                let iterable = vals
                    .first()
                    .ok_or_else(|| "TypeError: join() missing argument 'iterable'".to_string())?;
                let items = match iterable {
                    Value::List(lst) => lst.borrow().clone(),
                    Value::Tuple(t) => t.all_values().to_vec(),
                    Value::Generator(g) => g.borrow().values[g.borrow().index..].to_vec(),
                    other => {
                        return Err(format!(
                            "TypeError: join() argument must be iterable, not '{}'",
                            self.type_name(other)
                        ))
                    }
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
                let n = if sub.is_empty() {
                    slice.chars().count() + 1
                } else {
                    slice.matches(sub.as_str()).count()
                };
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
                Ok(Value::Str(
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string(),
                ))
            }
            "removesuffix" => {
                let suffix = arg_str!(0, "suffix");
                Ok(Value::Str(
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string(),
                ))
            }

            // ── 書式変換 ────────────────────────────────────────────────────
            "format" => {
                let mut pos_args: Vec<Value> = Vec::new();
                let mut kw_args: Vec<(String, Value)> = Vec::new();
                for (kw, v) in self.eval_call_args(args)? {
                    if let Some(k) = kw {
                        kw_args.push((k, v));
                    } else {
                        pos_args.push(v);
                    }
                }
                let display_fn = |v: &Value| self.display(v);
                let result = str_format(&s, &pos_args, &kw_args, &display_fn)?;
                Ok(Value::Str(result))
            }

            // ── 文字判定 ────────────────────────────────────────────────────
            "isdigit" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
            )),
            "isnumeric" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_numeric()),
            )),
            "isalpha" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
            )),
            "isalnum" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
            )),
            "isspace" => Ok(Value::Bool(
                !s.is_empty() && s.chars().all(|c| c.is_whitespace()),
            )),
            "isupper" => Ok(Value::Bool(
                !s.is_empty()
                    && s.chars().any(|c| c.is_alphabetic())
                    && s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()),
            )),
            "islower" => Ok(Value::Bool(
                !s.is_empty()
                    && s.chars().any(|c| c.is_alphabetic())
                    && s.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()),
            )),
            "isascii" => Ok(Value::Bool(s.is_ascii())),
            "isprintable" => Ok(Value::Bool(s.chars().all(|c| !c.is_control()))),

            // ── 幅揃え・ゼロ埋め ────────────────────────────────────────────
            "zfill" => {
                let width = arg_int!(0, 0).max(0) as usize;
                Ok(Value::Str(if s.len() >= width {
                    s.clone()
                } else {
                    format!("{:0>width$}", s)
                }))
            }
            "ljust" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => {
                        return Err(
                            "TypeError: ljust() fillchar must be single char str".to_string()
                        )
                    }
                };
                Ok(Value::Str(
                    format!("{:<width$}", s, width = width)
                        .replace(' ', &fill.to_string())
                        .replacen(&fill.to_string(), &fill.to_string(), width),
                ))
            }
            "rjust" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => {
                        return Err(
                            "TypeError: rjust() fillchar must be single char str".to_string()
                        )
                    }
                };
                if s.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let pad = width - s.len();
                Ok(Value::Str(format!("{}{}", fill.to_string().repeat(pad), s)))
            }
            "center" => {
                let width = arg_int!(0, 0).max(0) as usize;
                let fill = match vals.get(1) {
                    Some(Value::Str(f)) if f.chars().count() == 1 => f.chars().next().unwrap(),
                    None => ' ',
                    _ => {
                        return Err(
                            "TypeError: center() fillchar must be single char str".to_string()
                        )
                    }
                };
                if s.len() >= width {
                    return Ok(Value::Str(s.clone()));
                }
                let pad = width - s.len();
                let left = pad / 2;
                let right = pad - left;
                Ok(Value::Str(format!(
                    "{}{}{}",
                    fill.to_string().repeat(left),
                    s,
                    fill.to_string().repeat(right)
                )))
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
                let bytes: Vec<Value> =
                    s.as_bytes().iter().map(|b| Value::Int(*b as i64)).collect();
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
                    _ => Err(
                        "TypeError: ord() expected a character, but found a string of length != 1"
                            .to_string(),
                    ),
                }
            }

            // ── 正規表現メソッド ─────────────────────────────────────────────
            "match" => {
                let pattern = arg_str!(0, "pattern");
                let flags = match vals.get(1) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: match() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
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
                    Some(other) => {
                        return Err(format!(
                            "TypeError: search() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
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
                    Some(other) => {
                        return Err(format!(
                            "TypeError: findall() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                let matches = regex_findall(&s, &pattern, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    matches.into_iter().map(Value::Str).collect(),
                ))))
            }
            "sub" => {
                let pattern = arg_str!(0, "pattern");
                let repl = arg_str!(1, "repl");
                let count = arg_int!(2, 0).max(0) as usize;
                let flags = match vals.get(3) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: sub() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                Ok(Value::Str(regex_sub(&s, &pattern, &repl, count, &flags)?))
            }
            "regex_split" => {
                let pattern = arg_str!(0, "pattern");
                let maxsplit = arg_int!(1, 0).max(0) as usize;
                let flags = match vals.get(2) {
                    Some(Value::Str(f)) => f.clone(),
                    None => String::new(),
                    Some(other) => {
                        return Err(format!(
                            "TypeError: regex_split() flags must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                };
                let parts = regex_split(&s, &pattern, maxsplit, &flags)?;
                Ok(Value::List(Rc::new(RefCell::new(
                    parts.into_iter().map(Value::Str).collect(),
                ))))
            }

            _ => Err(format!(
                "AttributeError: 'str' object has no method '{method_name}'"
            )),
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
    pub(super) fn lookup_method_in_class(
        &self,
        class: &Rc<ClassValue>,
        method_name: &str,
    ) -> Option<Vec<Rc<FnValue>>> {
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

    /// インスタンス値をコピーする。
    ///
    /// 優先順位:
    /// 1. インスタンスのクラスに `__copy__` メソッドが定義されており、引数なし（`self` のみ）で
    ///    呼び出せるオーバーロードがあれば、それを呼び出す。
    /// 2. `__copy__` が存在しないか引数なしで呼び出せるオーバーロードがなければ、
    ///    `deep_copy_unfrozen` によるデフォルトのディープコピーを実行する。
    ///    `let` バインドのフリーズを解除し、新鮮な可変インスタンスとして返す。
    ///
    /// インスタンス以外の値（List / Dict 等）は `deep_copy_unfrozen` を使用する。
    ///
    /// メモリ不足などでコピーがパニックした場合は `MemoryError` を返す。
    pub(super) fn copy_value(&mut self, val: Value) -> Result<Value, String> {
        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__copy__") {
                // 引数なし（self のみ）で呼び出せるオーバーロードを選別する
                let callable: Vec<Rc<FnValue>> = overloads
                    .into_iter()
                    .filter(|f| {
                        f.params
                            .iter()
                            .filter(|p| p.name != "self")
                            .all(|p| p.default.is_some() || p.variadic)
                    })
                    .collect();
                if !callable.is_empty() {
                    return if callable.len() == 1 {
                        self.exec_fn(callable[0].clone(), &[], Some(val), "__copy__", None)
                    } else {
                        self.dispatch_overload(callable, &[], Some(val), None)
                    };
                }
            }
        }
        // デフォルト: フリーズ解除ディープコピー（パニック=メモリ不足を RuntimeError に変換）
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Interpreter::deep_copy_unfrozen(val)
        }))
        .map_err(|_| "MemoryError: insufficient memory for copy".to_string())
    }
}
