// classes.rs — クラス・インスタンス管理
// (instantiate / eval_method_call / lookup_method_in_class / lookup_class_var / freeze_instance)
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーンを辿るメソッド・クラス変数の検索を提供する。
// List / Str / Dict / Generator などの組み込み型のメソッドディスパッチもここで行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::CallArg;

use super::{ByteModeRust, FileOpenModeRust, Interpreter, Value, ClassValue, FnValue, InstanceData, GeneratorState};

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
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: str.__iter__() takes no arguments".to_string());
                    }
                    // 文字列を1文字ずつ Value::Str に変換してジェネレータにラップする
                    let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: chars,
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'str' object has no method '{method_name}'"))
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
