// py_interop.rs — PyO3 を介した Python ランタイム呼び出し
//
// `import[py-int]` で読み込んだモジュールの実行時ディスパッチを担当する。
// - `load_py_int_module`: Python モジュールを PyO3 でロードし NamespaceData を構築する
// - `ar_to_py`: tl の Value を PyO3 PyObject に変換する
// - `py_to_tl`: PyO3 Bound<PyAny> を tl の Value に変換する（プリミティブ自動変換）
// - `call_py_object`: Python callable を引数付きで呼び出す
// - `call_py_method`: Python オブジェクトのメソッドを呼び出す
// - `py_getattr`: Python オブジェクトの属性を取得する
// - `py_getitem` / `py_setitem`: Python の __getitem__ / __setitem__
// - `py_len`: Python の __len__
// - `py_collect_iter`: Python iterable を Vec<Value> に一括収集する
// - `py_binop` / `py_rbinop`: Python の二項演算子（lhs=PyObject / rhs=PyObject）

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use std::path::PathBuf;

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::{
    PyAnyMethods, PyByteArray, PyBytes, PyDict, PyDictMethods, PyFloat, PyFrozenSet, PyInt, PyList, PyListMethods,
    PyMemoryView, PyModule, PySet, PyTuple, PyTupleMethods,
};

use crate::ast::BinOp;

use super::{DictData, NamespaceData, PyObjHandle, TupleData, Value};

// ---------------------------------------------------------------------------
// tl → Python 変換
// ---------------------------------------------------------------------------

/// tl の `Value` を PyO3 の `PyObject`（`Py<PyAny>`）に変換する。
pub fn ar_to_py(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
    match val {
        Value::Int(n) => Ok(n.to_object(py)),
        Value::Float(f) => Ok(f.to_object(py)),
        Value::Str(s) => Ok(s.to_object(py)),
        Value::Bool(b) => Ok(b.to_object(py)),
        Value::None => Ok(py.None()),
        Value::List(items) => {
            let py_items: Vec<PyObject> = items
                .borrow()
                .iter()
                .map(|v| ar_to_py(py, v))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new_bound(py, &py_items).into())
        }
        Value::Tuple(td) => {
            let py_items: Vec<PyObject> = td
                .values
                .iter()
                .map(|v| ar_to_py(py, v))
                .collect::<PyResult<_>>()?;
            Ok(PyTuple::new_bound(py, &py_items).into())
        }
        Value::Dict(d) => {
            let dict = PyDict::new_bound(py);
            let borrowed = d.borrow();
            for (k, v) in borrowed.all_keys().iter().zip(borrowed.all_items().iter()) {
                dict.set_item(ar_to_py(py, k)?, ar_to_py(py, v)?)?;
            }
            Ok(dict.into())
        }
        // ⚠ 以前は set を受け付けず、型名が 'unknown' の TypeError になっていた（ffi_value_exchange_plan.md 2-6）
        Value::Set(items) => {
            let py_items: Vec<PyObject> =
                items.borrow().iter().map(|v| ar_to_py(py, v)).collect::<PyResult<_>>()?;
            Ok(PySet::new_bound(py, &py_items)?.into())
        }
        Value::UInt(n) => Ok(n.to_object(py)),
        // bytes → Python の bytes（コピー・ffi_value_exchange_plan.md 5-3）。⚠ 以前は TypeError
        Value::Bytes(b) => Ok(PyBytes::new_bound(py, b).into()),
        Value::PyObject(h) => Ok(h.inner.clone_ref(py)),
        other => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot pass an Arrow value of type '{}' to Python",
            type_name_of(other)
        ))),
    }
}

/// Python のオブジェクトのクラスと基底（MRO の順）の `(__module__, __name__)`（`is` / `mustbe` の判定・ffi_value_exchange_plan.md
/// 7-27）。読めない項目は飛ばす（`__module__` の無いクラスは空文字列）。
pub fn py_class_mro(h: &PyObjHandle) -> Vec<(String, String)> {
    Python::with_gil(|py| {
        let ty = h.inner.bind(py).get_type();
        let Ok(mro) = ty.getattr("__mro__") else { return Vec::new() };
        let Ok(items) = mro.iter() else { return Vec::new() };
        items
            .filter_map(|c| c.ok())
            .filter_map(|c| {
                let name = c.getattr("__name__").ok()?.extract::<String>().ok()?;
                let module = c.getattr("__module__").ok().and_then(|m| m.extract::<String>().ok()).unwrap_or_default();
                Some((module, name))
            })
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Python → tl 変換
// ---------------------------------------------------------------------------

/// PyO3 の `Bound<'_, PyAny>` を tl の `Value` に変換する。
pub fn py_to_tl(py: Python<'_>, obj: &Bound<'_, PyAny>) -> Value {
    // bool は int のサブクラスなので先にチェックする
    let type_name = obj
        .get_type()
        .name()
        .map(|s| s.to_string())
        .unwrap_or_default();
    if type_name == "bool" {
        if let Ok(b) = obj.extract::<bool>() {
            return Value::Bool(b);
        }
    }
    if let Ok(n) = obj.extract::<i64>() {
        return Value::Int(n);
    }
    // ⚠ 2-6: Python の int で i64 に入らないものは、下の f64 への変換に落ちて**黙って float**（精度を失う）になっていた。
    //   Python のオブジェクトのまま渡す（値を失わず、演算は Python 側で行える）。
    if obj.is_instance_of::<PyInt>() {
        return Value::PyObject(Arc::new(PyObjHandle { inner: obj.clone().unbind() }));
    }
    // ⚠ 2-6: float だけを float にする。以前は `__float__` を持つもの（`Decimal` / `Fraction` など）も黙って float に
    //   変換していた（`Decimal("0.1")` の精度が失われる）。それらは Python のオブジェクトのまま渡す。
    if obj.is_instance_of::<PyFloat>() {
        if let Ok(f) = obj.extract::<f64>() {
            return Value::Float(f);
        }
    }
    if let Ok(s) = obj.extract::<String>() {
        return Value::str(s);
    }
    // bytes / bytearray → Arrow の bytes（コピー・5-3・D-9）。⚠ 以前は Python のオブジェクトのままだった
    if let Ok(b) = obj.downcast::<PyBytes>() {
        return Value::bytes(b.as_bytes().to_vec());
    }
    if let Ok(b) = obj.downcast::<PyByteArray>() {
        return Value::bytes(b.to_vec());
    }
    // memoryview は 1 バイトの要素（`B` / `b` / `c`）のものだけ bytes にする。int32 などの view をバイト列にすると
    // 別の表現に黙って変わるので、Python のオブジェクトのまま渡す
    if obj.is_instance_of::<PyMemoryView>() {
        let byte_view = obj.getattr("format").and_then(|f| f.extract::<String>()).is_ok_and(|f| matches!(f.as_str(), "B" | "b" | "c"));
        if byte_view {
            if let Ok(b) = obj.call_method0("tobytes").and_then(|b| b.downcast_into::<PyBytes>().map_err(PyErr::from)) {
                return Value::bytes(b.as_bytes().to_vec());
            }
        }
    }
    if obj.is_none() {
        return Value::None;
    }
    // list → Value::List
    if let Ok(list) = obj.downcast::<PyList>() {
        let items: Vec<Value> = list.iter().map(|item| py_to_tl(py, &item)).collect();
        return Value::List(Rc::new(RefCell::new(items)));
    }
    // tuple → Value::Tuple
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let mut values = Vec::new();
        let mut types = Vec::new();
        for item in tup.iter() {
            let v = py_to_tl(py, &item);
            types.push(type_name_of(&v));
            values.push(v);
        }
        return Value::Tuple(Rc::new(TupleData::new(values, types)));
    }
    // dict → Value::Dict（Any 型）
    if let Ok(d) = obj.downcast::<PyDict>() {
        let mut dict = DictData::new("Any".to_string(), "Any".to_string());
        let mut all_keys_ok = true;
        for (k, v) in d.iter() {
            // ⚠ Arrow がキーにできない型（tuple / 非整数 float / None など）を含む dict は
            // **要素を黙って捨てない**（B1-a。以前はここで無言の欠落が起きていた）。
            // 1 つでも弾かれたら dict への変換自体をやめ、下の `PyObject` フォールバックへ
            // 落として**丸ごと**渡す（情報を落とさず、Python 側の索引で扱える）。
            if crate::interpreter::Interpreter::dict_set_pure(&mut dict, py_to_tl(py, &k), py_to_tl(py, &v)).is_err() {
                all_keys_ok = false;
                break;
            }
        }
        if all_keys_ok {
            return Value::Dict(Rc::new(RefCell::new(dict)));
        }
    }
    // set / frozenset → Value::Set（2-6。以前は Python のオブジェクトのままだった）。
    // ⚠ Arrow の set に入れられない要素（list / dict / PyObject など）を含むなら、dict と同じく要素を捨てずに
    //   丸ごと Python のオブジェクトのまま渡す。
    if obj.is_instance_of::<PySet>() || obj.is_instance_of::<PyFrozenSet>() {
        if let Ok(iter) = obj.iter() {
            let mut items: Vec<Value> = Vec::new();
            let mut ok = true;
            for item in iter {
                let Ok(item) = item else { ok = false; break };
                let v = py_to_tl(py, &item);
                if DictData::reject_key(&v).is_some() || matches!(v, Value::PyObject(_)) {
                    ok = false;
                    break;
                }
                match items.iter().try_fold(false, |found, e| {
                    crate::interpreter::Interpreter::values_eq_pure(e, &v).map(|eq| found || eq)
                }) {
                    Ok(false) => items.push(v),
                    Ok(true) => {}
                    Err(_) => { ok = false; break; }
                }
            }
            if ok {
                return Value::Set(Rc::new(RefCell::new(items)));
            }
        }
    }
    // その他 → PyObject（opaque ラップ）
    Value::PyObject(Arc::new(PyObjHandle {
        inner: obj.clone().unbind(),
    }))
}

// ---------------------------------------------------------------------------
// モジュールロード
// ---------------------------------------------------------------------------

/// Python モジュールを PyO3 でロードして `NamespaceData` を構築する。
///
/// - `module_path`: モジュール名パーツ（例: `["numpy"]`, `["os", "path"]`）
/// - `extra_search_dirs`: Python の `sys.path` 先頭に追加するディレクトリ
pub fn load_py_int_module(
    module_path: &[String],
    extra_search_dirs: &[PathBuf],
) -> Result<Rc<NamespaceData>, String> {
    let module_name = module_path.join(".");
    Python::with_gil(|py| -> PyResult<Rc<NamespaceData>> {
        // extra_search_dirs を sys.path の先頭に追加する
        let sys = py.import_bound("sys")?;
        let sys_path = sys.getattr("path")?;
        for dir in extra_search_dirs.iter().rev() {
            let dir_str = dir.to_string_lossy();
            sys_path.call_method1("insert", (0i32, dir_str.as_ref()))?;
        }

        let module = PyModule::import_bound(py, module_name.as_str())?;
        let mut members: HashMap<String, Value> = HashMap::new();

        let dir = module.dir()?;
        for name_obj in dir.iter() {
            let name: String = match name_obj.extract() {
                Ok(s) => s,
                Err(_) => continue,
            };
            if name.starts_with('_') {
                continue;
            }

            if let Ok(attr) = module.getattr(name.as_str()) {
                let val = Value::PyObject(Arc::new(PyObjHandle {
                    inner: attr.unbind(),
                }));
                members.insert(name, val);
            }
        }

        Ok(Rc::new(NamespaceData {
            name: module_name,
            members,
            live: None,
        }))
    })
    .map_err(|e| format!("ImportError: {e}"))
}

// ---------------------------------------------------------------------------
// 呼び出しヘルパー
// ---------------------------------------------------------------------------

/// Arrow の値を Python へ渡せなかった（`ar_to_py`・ffi_value_exchange_plan.md 7-24）。Arrow 側の型の誤りなので
/// `TypeError`（`PyErr` の表記 `TypeError: cannot pass an Arrow value of type 'Point' to Python` のまま）。
/// ⚠ Python が投げた例外（[`python_raised`]）と分ける。以前はどちらも呼び出しの種類の例外（`RuntimeError: Python call
///   failed: TypeError: …`）に包まれていた
fn arrow_side(e: PyErr) -> String {
    e.to_string()
}

/// Python が投げた例外（7-24・利用者の決定）。C# / JS と同じく `ForeignError` で、メッセージは Python の例外の種類と
/// メッセージ（`ValueError: bad`）。⚠ 以前は呼び出しの種類で決まる例外（関数は `RuntimeError`・メソッドは `AttributeError`・
/// 添字は `IndexError`）になり、Python の例外の種類が失われていた（`ValueError` を投げるメソッドが `AttributeError`）
fn python_raised(e: PyErr) -> String {
    format!("ForeignError: {e}")
}

/// Python callable を引数付きで呼び出す。
pub fn call_py_object(
    handle: &PyObjHandle,
    evaled: &[(Option<String>, Value, bool)],
) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let callable = handle.inner.bind(py);
        let (pos_args, kwargs_dict) = build_py_args(py, evaled).map_err(arrow_side)?;
        let result = callable.call(pos_args, Some(&kwargs_dict)).map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// Python オブジェクトのメソッドを呼び出す。
pub fn call_py_method(
    handle: &PyObjHandle,
    method_name: &str,
    evaled: &[(Option<String>, Value, bool)],
) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let method = obj.getattr(method_name).map_err(python_raised)?;
        let (pos_args, kwargs_dict) = build_py_args(py, evaled).map_err(arrow_side)?;
        let result = method.call(pos_args, Some(&kwargs_dict)).map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// Python オブジェクトの属性を取得して tl の Value として返す。
pub fn py_getattr(handle: &PyObjHandle, attr: &str) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let attr_val = obj.getattr(attr).map_err(python_raised)?;
        Ok(py_to_tl(py, &attr_val))
    })
}

/// `obj[key]`: Python の `__getitem__` を呼び出す。
pub fn py_getitem(handle: &PyObjHandle, key: &Value) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let py_key = ar_to_py(py, key).map_err(arrow_side)?;
        let result = obj.get_item(py_key).map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// `obj[key] = val`: Python の `__setitem__` を呼び出す。
pub fn py_setitem(handle: &PyObjHandle, key: &Value, val: &Value) -> Result<(), String> {
    Python::with_gil(|py| -> Result<(), String> {
        let obj = handle.inner.bind(py);
        let py_key = ar_to_py(py, key).map_err(arrow_side)?;
        let py_val = ar_to_py(py, val).map_err(arrow_side)?;
        obj.set_item(py_key, py_val).map_err(python_raised)
    })
}

/// Python の int（`bool` を除く）なら、その表記（`str(obj)`）。i64 に入らない int は Python のオブジェクトのまま届くので、
/// 境界検査が OverflowError の文面に使う（`ffi_boundary`・7-9）。
pub fn py_int_text(handle: &PyObjHandle) -> Option<String> {
    Python::with_gil(|py| {
        let obj = handle.inner.bind(py);
        if obj.is_instance_of::<PyInt>() && !obj.is_instance_of::<pyo3::types::PyBool>() {
            obj.str().ok().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// `len(obj)`: Python の `__len__` を呼び出す。
pub fn py_len(handle: &PyObjHandle) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let n = obj.len().map_err(python_raised)?;
        Ok(Value::Int(n as i64))
    })
}

/// Python iterable を走査して要素を `Vec<Value>` に収集する。for ループ用。
pub fn py_collect_iter(handle: &PyObjHandle) -> Result<Vec<Value>, String> {
    Python::with_gil(|py| -> Result<Vec<Value>, String> {
        let obj = handle.inner.bind(py);
        let iter = obj.iter().map_err(python_raised)?;
        let mut items = Vec::new();
        for item in iter {
            items.push(py_to_tl(py, &item.map_err(python_raised)?));
        }
        Ok(items)
    })
}

/// `and` / `or` は Python の演算子メソッドに写せない（Arrow 側の誤り）。
const NO_LOGICAL_BINOP: &str = "TypeError: cannot apply 'and'/'or' to Python objects via binop";

/// lhs が `PyObject` のときの二項演算。`op` を Python の演算子メソッドにマップする。
pub fn py_binop(handle: &PyObjHandle, op: &BinOp, rhs: &Value) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let py_rhs = ar_to_py(py, rhs).map_err(arrow_side)?;
        let result = match op {
            BinOp::Add => obj.add(py_rhs),
            BinOp::Sub => obj.sub(py_rhs),
            BinOp::Mul => obj.mul(py_rhs),
            BinOp::Div => obj.div(py_rhs),
            BinOp::FloorDiv => obj.floor_div(py_rhs),
            BinOp::Mod => obj.rem(py_rhs),
            BinOp::Pow => obj.pow(py_rhs, py.None()),
            BinOp::LShift => obj.lshift(py_rhs),
            BinOp::RShift => obj.rshift(py_rhs),
            BinOp::BitAnd => obj.bitand(py_rhs),
            BinOp::BitOr => obj.bitor(py_rhs),
            BinOp::BitXor => obj.bitxor(py_rhs),
            BinOp::Eq | BinOp::RefEq => obj.rich_compare(py_rhs, CompareOp::Eq),
            BinOp::NotEq => obj.rich_compare(py_rhs, CompareOp::Ne),
            BinOp::Lt => obj.rich_compare(py_rhs, CompareOp::Lt),
            BinOp::Gt => obj.rich_compare(py_rhs, CompareOp::Gt),
            BinOp::LtEq => obj.rich_compare(py_rhs, CompareOp::Le),
            BinOp::GtEq => obj.rich_compare(py_rhs, CompareOp::Ge),
            BinOp::And | BinOp::Or => return Err(NO_LOGICAL_BINOP.to_string()),
            BinOp::In | BinOp::NotIn => {
                // obj (LHS item) in py_rhs (RHS container)
                let container = py_rhs.bind(py);
                let b: bool = container
                    .call_method1("__contains__", (obj.clone().unbind(),))
                    .and_then(|c| c.extract())
                    .map_err(python_raised)?;
                return Ok(Value::Bool(if matches!(op, BinOp::In) { b } else { !b }));
            }
        }
        .map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// rhs が `PyObject` のときの二項演算。比較は左右を入れ替え、算術は反射メソッドを呼ぶ。
pub fn py_rbinop(handle: &PyObjHandle, op: &BinOp, lhs: &Value) -> Result<Value, String> {
    Python::with_gil(|py| -> Result<Value, String> {
        let obj = handle.inner.bind(py);
        let py_lhs = ar_to_py(py, lhs).map_err(arrow_side)?;
        let result = match op {
            // 算術: 反射メソッドを直接呼び出す
            BinOp::Add => obj.call_method1("__radd__", (py_lhs,)),
            BinOp::Sub => obj.call_method1("__rsub__", (py_lhs,)),
            BinOp::Mul => obj.call_method1("__rmul__", (py_lhs,)),
            BinOp::Div => obj.call_method1("__rtruediv__", (py_lhs,)),
            BinOp::FloorDiv => obj.call_method1("__rfloordiv__", (py_lhs,)),
            BinOp::Mod => obj.call_method1("__rmod__", (py_lhs,)),
            BinOp::Pow => obj.call_method1("__rpow__", (py_lhs,)),
            BinOp::LShift => obj.call_method1("__rlshift__", (py_lhs,)),
            BinOp::RShift => obj.call_method1("__rrshift__", (py_lhs,)),
            BinOp::BitAnd => obj.call_method1("__rand__", (py_lhs,)),
            BinOp::BitOr => obj.call_method1("__ror__", (py_lhs,)),
            BinOp::BitXor => obj.call_method1("__rxor__", (py_lhs,)),
            // 比較: 左右を入れ替えて対応する演算子を使う
            BinOp::Eq | BinOp::RefEq => obj.rich_compare(py_lhs, CompareOp::Eq),
            BinOp::NotEq => obj.rich_compare(py_lhs, CompareOp::Ne),
            BinOp::Lt => obj.rich_compare(py_lhs, CompareOp::Gt), // lhs < rhs → rhs > lhs
            BinOp::Gt => obj.rich_compare(py_lhs, CompareOp::Lt), // lhs > rhs → rhs < lhs
            BinOp::LtEq => obj.rich_compare(py_lhs, CompareOp::Ge),
            BinOp::GtEq => obj.rich_compare(py_lhs, CompareOp::Le),
            BinOp::And | BinOp::Or => return Err(NO_LOGICAL_BINOP.to_string()),
            BinOp::In | BinOp::NotIn => {
                // py_lhs (item) in obj/handle (RHS container)
                let b: bool = obj
                    .call_method1("__contains__", (py_lhs.clone_ref(py),))
                    .and_then(|c| c.extract())
                    .map_err(python_raised)?;
                return Ok(Value::Bool(if matches!(op, BinOp::In) { b } else { !b }));
            }
        }
        .map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

// ---------------------------------------------------------------------------
// 内部ヘルパー
// ---------------------------------------------------------------------------

/// 評価済み引数リストから PyTuple（位置引数）と PyDict（キーワード引数）を構築する。
fn build_py_args<'py>(
    py: Python<'py>,
    evaled: &[(Option<String>, Value, bool)],
) -> PyResult<(Bound<'py, PyTuple>, Bound<'py, PyDict>)> {
    let mut positional: Vec<PyObject> = Vec::new();
    let kwargs = PyDict::new_bound(py);
    for (name, val, _) in evaled {
        let py_val = ar_to_py(py, val)?;
        match name {
            None => positional.push(py_val),
            Some(k) => {
                kwargs.set_item(k, py_val)?;
            }
        }
    }
    let args_tuple = PyTuple::new_bound(py, positional);
    Ok((args_tuple, kwargs))
}

/// Value のランタイム型名を返す（エラーメッセージ・タプルの要素型名用・[`element_type_name`]）。
fn type_name_of(val: &Value) -> String {
    crate::interpreter::ops::typecheck::element_type_name(val)
}
