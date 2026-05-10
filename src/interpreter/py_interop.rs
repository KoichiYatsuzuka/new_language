// py_interop.rs — PyO3 を介した Python ランタイム呼び出し
//
// `import[py-int]` で読み込んだモジュールの実行時ディスパッチを担当する。
// - `load_py_int_module`: Python モジュールを PyO3 でロードし NamespaceData を構築する
// - `tl_to_py`: tl の Value を PyO3 PyObject に変換する
// - `py_to_tl`: PyO3 Bound<PyAny> を tl の Value に変換する（プリミティブ自動変換）
// - `call_py_object`: Python callable を引数付きで呼び出す
// - `call_py_method`: Python オブジェクトのメソッドを呼び出す
// - `py_getattr`: Python オブジェクトの属性を取得する

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods, PyList, PyListMethods, PyModule, PyTuple, PyTupleMethods};

use super::{DictData, NamespaceData, PyObjHandle, TupleData, Value};

// ---------------------------------------------------------------------------
// tl → Python 変換
// ---------------------------------------------------------------------------

/// tl の `Value` を PyO3 の `PyObject`（`Py<PyAny>`）に変換する。
pub fn tl_to_py(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
    match val {
        Value::Int(n) => Ok(n.to_object(py)),
        Value::Float(f) => Ok(f.to_object(py)),
        Value::Str(s) => Ok(s.to_object(py)),
        Value::Bool(b) => Ok(b.to_object(py)),
        Value::None => Ok(py.None()),
        Value::List(items) => {
            let py_items: Vec<PyObject> = items.iter()
                .map(|v| tl_to_py(py, v))
                .collect::<PyResult<_>>()?;
            Ok(PyList::new_bound(py, &py_items).into())
        }
        Value::PyObject(h) => Ok(h.inner.clone_ref(py)),
        other => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot convert tl value of type '{}' to Python",
            type_name_of(other)
        ))),
    }
}

// ---------------------------------------------------------------------------
// Python → tl 変換
// ---------------------------------------------------------------------------

/// PyO3 の `Bound<'_, PyAny>` を tl の `Value` に変換する。
pub fn py_to_tl(py: Python<'_>, obj: &Bound<'_, PyAny>) -> Value {
    // bool は int のサブクラスなので先にチェックする
    let type_name = obj.get_type().name().map(|s| s.to_string()).unwrap_or_default();
    if type_name == "bool" {
        if let Ok(b) = obj.extract::<bool>() {
            return Value::Bool(b);
        }
    }
    if let Ok(n) = obj.extract::<i64>() {
        return Value::Int(n);
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Value::Float(f);
    }
    if let Ok(s) = obj.extract::<String>() {
        return Value::Str(s);
    }
    if obj.is_none() {
        return Value::None;
    }
    // list → Value::List
    if let Ok(list) = obj.downcast::<PyList>() {
        let items: Vec<Value> = list.iter().map(|item| py_to_tl(py, &item)).collect();
        return Value::List(items);
    }
    // tuple → Value::Tuple
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let mut values = Vec::new();
        let mut types = Vec::new();
        for item in tup.iter() {
            let v = py_to_tl(py, &item);
            types.push(type_name_of(&v).to_string());
            values.push(v);
        }
        return Value::Tuple(Rc::new(TupleData::new(values, types)));
    }
    // dict → Value::Dict（Any 型）
    if let Ok(d) = obj.downcast::<PyDict>() {
        let mut dict = DictData::new("Any".to_string(), "Any".to_string());
        for (k, v) in d.iter() {
            dict.set(py_to_tl(py, &k), py_to_tl(py, &v));
        }
        return Value::Dict(Rc::new(RefCell::new(dict)));
    }
    // その他 → PyObject（opaque ラップ）
    Value::PyObject(Rc::new(PyObjHandle { inner: obj.clone().unbind() }))
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
            if name.starts_with('_') { continue; }

            if let Ok(attr) = module.getattr(name.as_str()) {
                let val = Value::PyObject(Rc::new(PyObjHandle { inner: attr.unbind() }));
                members.insert(name, val);
            }
        }

        Ok(Rc::new(NamespaceData { name: module_name, members }))
    })
    .map_err(|e| format!("ImportError: {e}"))
}

// ---------------------------------------------------------------------------
// 呼び出しヘルパー
// ---------------------------------------------------------------------------

/// Python callable を引数付きで呼び出す。
pub fn call_py_object(
    handle: &PyObjHandle,
    evaled: &[(Option<String>, Value)],
) -> Result<Value, String> {
    Python::with_gil(|py| -> PyResult<Value> {
        let callable = handle.inner.bind(py);
        let (pos_args, kwargs_dict) = build_py_args(py, evaled)?;
        let result = callable.call(pos_args, Some(&kwargs_dict))?;
        Ok(py_to_tl(py, &result))
    })
    .map_err(|e| format!("RuntimeError: Python call failed: {e}"))
}

/// Python オブジェクトのメソッドを呼び出す。
pub fn call_py_method(
    handle: &PyObjHandle,
    method_name: &str,
    evaled: &[(Option<String>, Value)],
) -> Result<Value, String> {
    Python::with_gil(|py| -> PyResult<Value> {
        let obj = handle.inner.bind(py);
        let method = obj.getattr(method_name)?;
        let (pos_args, kwargs_dict) = build_py_args(py, evaled)?;
        let result = method.call(pos_args, Some(&kwargs_dict))?;
        Ok(py_to_tl(py, &result))
    })
    .map_err(|e| format!("AttributeError: Python method call '{method_name}' failed: {e}"))
}

/// Python オブジェクトの属性を取得して tl の Value として返す。
pub fn py_getattr(handle: &PyObjHandle, attr: &str) -> Result<Value, String> {
    Python::with_gil(|py| -> PyResult<Value> {
        let obj = handle.inner.bind(py);
        let attr_val = obj.getattr(attr)?;
        Ok(py_to_tl(py, &attr_val))
    })
    .map_err(|e| format!("AttributeError: Python attribute '{attr}' not found: {e}"))
}

// ---------------------------------------------------------------------------
// 内部ヘルパー
// ---------------------------------------------------------------------------

/// 評価済み引数リストから PyTuple（位置引数）と PyDict（キーワード引数）を構築する。
fn build_py_args<'py>(
    py: Python<'py>,
    evaled: &[(Option<String>, Value)],
) -> PyResult<(Bound<'py, PyTuple>, Bound<'py, PyDict>)> {
    let mut positional: Vec<PyObject> = Vec::new();
    let kwargs = PyDict::new_bound(py);
    for (name, val) in evaled {
        let py_val = tl_to_py(py, val)?;
        match name {
            None => positional.push(py_val),
            Some(k) => { kwargs.set_item(k, py_val)?; }
        }
    }
    let args_tuple = PyTuple::new_bound(py, positional);
    Ok((args_tuple, kwargs))
}

/// Value のランタイム型名を返す（エラーメッセージ用）。
fn type_name_of(val: &Value) -> &'static str {
    match val {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bool(_) => "bool",
        Value::None => "NoneType",
        Value::List(_) => "list",
        Value::Dict(_) => "dict",
        Value::Tuple(_) => "tuple",
        Value::PyObject(_) => "object",
        _ => "unknown",
    }
}
