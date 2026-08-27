// ops/display.rs — 値の表示・repr 生成: display / display_repr / repr_val。

use {
    std::rc::Rc,
    crate::interpreter::{Interpreter, Value},
};
use super::*;

impl Interpreter {
    /// 値を `print()` 出力用の文字列に変換する。
    /// 文字列値はクォートなしでそのまま返す（`display_repr` との違い）。
    ///
    /// - `val`: 表示する値
    ///
    /// 戻り値: 人間が読みやすい表示文字列
    pub(crate) fn display(&self, val: &Value) -> String {
        match val {
            Value::Int(n) => n.to_string(),
            Value::UInt(n) => n.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Value::Complex(re, im) => {
                let fmt_f = |f: f64| -> String {
                    let f = if f == 0.0 { 0.0 } else { f }; // normalize -0.0
                    if f.fract() == 0.0 && f.abs() < 1e15 {
                        format!("{f:.1}")
                    } else {
                        f.to_string()
                    }
                };
                let re_n = if *re == 0.0 { 0.0 } else { *re };
                let im_n = if *im == 0.0 { 0.0 } else { *im };
                if im_n >= 0.0 {
                    format!("({}+{}j)", fmt_f(re_n), fmt_f(im_n))
                } else {
                    format!("({}-{}j)", fmt_f(re_n), fmt_f(im_n.abs()))
                }
            }
            Value::Str(s) => s.to_string(),
            Value::Bool(b) => if *b { "True" } else { "False" }.to_string(),
            Value::None => "None".to_string(),
            Value::Undefined => "Undefined".to_string(),
            Value::List(items) => {
                let parts: Vec<String> = items
                    .borrow()
                    .iter()
                    .map(|v| self.display_repr(v))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Function(fn_rc) => {
                let addr = Rc::as_ptr(fn_rc) as usize;
                let sig = format_fn_params(&fn_rc.params);
                format!("<function '{}'({}) at 0x{:x}>", fn_rc.name, sig, addr)
            }
            Value::OverloadedFn(fns) => {
                let first = &fns[0];
                let addr = Rc::as_ptr(first) as usize;
                format!(
                    "<function '{}' ({} overloads) at 0x{:x}>",
                    first.name,
                    fns.len(),
                    addr
                )
            }
            Value::Class(c) => format!("<class '{}'>", c.name),
            Value::Instance(i) => {
                let class_name = i.borrow().class.name.clone();
                let addr = Rc::as_ptr(i) as usize;
                format!("<{} object at 0x{:x}>", class_name, addr)
            }
            Value::Type(name) => format!("<class '{name}'>"),
            Value::Trait(name) => format!("<trait '{name}'>"),
            Value::Protocol(name) => format!("<protocol '{name}'>"),
            Value::TemplateFn(t) => format!("<template function '{}'>", t.name),
            Value::TemplateClass(t) => format!("<template class '{}'>", t.name),
            Value::GeneratorFn(gf) => format!("<generator function '{}'>", gf.name),
            Value::TemplateGenFn(t) => format!("<template generator function '{}'>", t.name),
            Value::Generator(s) => {
                let state = s.borrow();
                let addr = Rc::as_ptr(s) as usize;
                let yield_type = if state.values.is_empty() {
                    "Any".to_string()
                } else {
                    self.type_name(&state.values[0]).to_string()
                };
                format!("<generator object[{}] at 0x{:x}>", yield_type, addr)
            }
            Value::Dict(d) => {
                let d = d.borrow();
                if d.is_empty() {
                    "{}".to_string()
                } else {
                    let keys = d.all_keys();
                    let vals = d.all_items();
                    let parts: Vec<String> = keys
                        .iter()
                        .zip(vals.iter())
                        .map(|(k, v)| format!("{}: {}", self.display_repr(k), self.display_repr(v)))
                        .collect();
                    format!("{{{}}}", parts.join(", "))
                }
            }
            Value::Tuple(t) => {
                let vals = t.all_values();
                if vals.len() == 1 {
                    format!("({},)", self.display_repr(&vals[0]))
                } else {
                    let parts: Vec<String> = vals.iter().map(|v| self.display_repr(v)).collect();
                    format!("({})", parts.join(", "))
                }
            }
            Value::Set(s) => {
                let s = s.borrow();
                if s.is_empty() {
                    "set()".to_string()
                } else {
                    let parts: Vec<String> = s.iter().map(|v| self.display_repr(v)).collect();
                    format!("{{{}}}", parts.join(", "))
                }
            }
            Value::Namespace(ns) => format!("<module '{}'>", ns.name),
            Value::FileObject(fd_rc) => {
                let fd = fd_rc.borrow();
                if fd.is_closed {
                    format!("<FileObject '{}' (closed)>", fd.path)
                } else {
                    format!("<FileObject '{}' pos={}>", fd.path, fd.pointer)
                }
            }
            Value::PyObject(h) => pyo3::Python::with_gil(|py| {
                use pyo3::types::PyAnyMethods;
                h.inner
                    .bind(py)
                    .repr()
                    .and_then(|r| r.extract::<String>())
                    .unwrap_or_else(|_| "<PyObject>".to_string())
            }),
            Value::NativeFunction(r) => format!("<native function '{}'>", r.fn_name),
            Value::Slice(s) => {
                let b = s
                    .begin
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                let e = s
                    .end
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                let st = s
                    .step
                    .as_ref()
                    .map(|v| self.display(v))
                    .unwrap_or_else(|| "None".to_string());
                format!("slice({b}, {e}, {st})")
            }
            Value::AsyncManager(rc) => {
                let mgr = rc.borrow();
                format!(
                    "<AsyncManager num_thread={} tasks={}>",
                    mgr.num_thread,
                    mgr.progress.len()
                )
            }
            Value::AsyncStatusVal(s) => s.display_str().to_string(),
            Value::Signal(sig_rc) => {
                let sig = sig_rc.borrow();
                let addr = std::rc::Rc::as_ptr(sig_rc) as usize;
                format!("<Signal handlers={} at 0x{:x}>", sig.handlers.len(), addr)
            }
            Value::EventLoop(_) => "<EventLoop>".to_string(),
            Value::CsObject(o) => format!("<CsObject '{}' handle={}>", o.class_name, o.handle),
            Value::JsProcFn(data) => {
                format!("<js function '{}.{}'>", data.module_name, data.fn_name)
            }
            Value::ResultVal { ok, inner } => {
                if *ok {
                    format!("Ok({})", self.display(inner))
                } else {
                    format!("Err({})", self.display(inner))
                }
            }
            Value::FrozenList { state, layout } => {
                let st = state.borrow();
                let parts: Vec<String> = (0..st.len)
                    .map(|i| self.display_repr(&layout.reconstruct_item(&st.data, i)))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
        }
    }

    /// 値をコレクション内要素の表示用文字列に変換する。
    /// 文字列値はシングルクォートで囲み、リストは各要素を再帰的に repr 表示する。
    /// `display` との違い: 文字列値が `'...'` 形式で出力される点。
    ///
    /// - `val`: 表示する値
    ///
    /// 戻り値: repr 形式の表示文字列
    pub(crate) fn display_repr(&self, val: &Value) -> String {
        match val {
            Value::Str(s) => format!("'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items
                    .borrow()
                    .iter()
                    .map(|v| self.display_repr(v))
                    .collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Dict(_) | Value::Tuple(_) | Value::Slice(_) => self.display(val),
            _ => self.display(val),
        }
    }

    /// `repr(val)` の実装。ユーザー定義 `__repr__` メソッドを呼び出し、
    /// 定義されていない場合はデフォルトの repr 文字列を返す。
    /// コレクション内のインスタンスに対しても再帰的に `__repr__` を呼び出す。
    ///
    /// 戻り値: `Ok(String)` — repr 文字列。`Err(message)` — `__repr__` 内でエラーが発生した場合。
    pub(crate) fn repr_val(&mut self, val: &Value) -> Result<String, String> {
        match val {
            // コレクション: 各要素に repr_val を再帰適用
            Value::List(items) => {
                let items_clone: Vec<Value> = items.borrow().clone();
                let parts: Result<Vec<String>, _> =
                    items_clone.iter().map(|v| self.repr_val(v)).collect();
                Ok(format!("[{}]", parts?.join(", ")))
            }
            Value::Dict(d) => {
                let (keys, vals) = {
                    let db = d.borrow();
                    (db.all_keys(), db.all_items())
                };
                if keys.is_empty() {
                    return Ok("{}".to_string());
                }
                let mut parts = Vec::new();
                for (k, v) in keys.iter().zip(vals.iter()) {
                    let kr = self.repr_val(k)?;
                    let vr = self.repr_val(v)?;
                    parts.push(format!("{kr}: {vr}"));
                }
                Ok(format!("{{{}}}", parts.join(", ")))
            }
            Value::Tuple(t) => {
                let vals = t.all_values().to_vec();
                if vals.len() == 1 {
                    let r = self.repr_val(&vals[0])?;
                    Ok(format!("({r},)"))
                } else {
                    let parts: Result<Vec<String>, _> =
                        vals.iter().map(|v| self.repr_val(v)).collect();
                    Ok(format!("({})", parts?.join(", ")))
                }
            }
            Value::Set(s) => {
                let items_clone: Vec<Value> = s.borrow().clone();
                if items_clone.is_empty() {
                    return Ok("set()".to_string());
                }
                let parts: Result<Vec<String>, _> =
                    items_clone.iter().map(|v| self.repr_val(v)).collect();
                Ok(format!("{{{}}}", parts?.join(", ")))
            }
            // インスタンス: new_type_base または __repr__ を優先して使用
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();

                // new_type でプリミティブを基底とする場合: ClassName(repr_of_value)
                if let Some(ref base) = class.new_type_base {
                    if matches!(base.as_str(), "int" | "float" | "str" | "bool" | "uint") {
                        let inner_val = {
                            let b = inst_rc.borrow();
                            b.class.field_index.get("value").and_then(|&idx| b.field_value(idx))
                        };
                        if let Some(v) = inner_val {
                            let inner = self.repr_val(&v)?;
                            return Ok(format!("{}({})", class.name, inner));
                        }
                    }
                }

                // ユーザー定義 __repr__ を呼び出す
                if class.methods.contains_key("__repr__") {
                    let result = self.eval_method_call_evaled(val.clone(), "__repr__", vec![])?;
                    return match result {
                        Value::Str(s) => Ok(s.to_string()),
                        other => Ok(self.display(&other)),
                    };
                }

                // デフォルト: <ClassName object at 0xADDR>
                Ok(self.display(val))
            }
            // その他の型はデフォルト表示
            _ => Ok(self.display_repr(val)),
        }
    }

}
