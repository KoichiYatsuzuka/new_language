// interpreter/py_interop/modules.rs — Python のモジュールの読み込み・組み込みの呼び出し・クラスの判定。

use super::*;

/// Python の組み込みの関数 `builtins.<name>(*args)` を呼ぶ（Arrow の `isinstance` / `issubclass` が CPython のクラスを受けたとき・
/// pandas_import_plan.md 3-1）。引数は Arrow → Python の変換（`ar_to_py`）、戻り値は Python → Arrow の変換。
pub fn call_py_builtin(name: &str, args: &[Value]) -> Result<Value, String> {
    with_py(|py| {
        let f = py.import_bound("builtins").and_then(|b| b.getattr(name)).map_err(python_raised)?;
        let mut objs = Vec::with_capacity(args.len());
        for a in args {
            objs.push(ar_to_py(py, a).map_err(arrow_side)?);
        }
        let r = f.call1(PyTuple::new_bound(py, objs)).map_err(python_raised)?;
        Ok(py_to_tl(py, &r))
    })
}

/// `isinstance(obj, cls)`。`cls` がクラスでなければ（`isinstance` が `TypeError`）`None`。
pub fn py_isinstance(obj: &PyObjHandle, cls: &PyObjHandle) -> Option<bool> {
    with_py(|py| {
        let cls = cls.bind(py);
        if !cls.is_instance_of::<pyo3::types::PyType>() {
            return None;
        }
        obj.bind(py).is_instance(&cls).ok()
    })
}

/// Python のオブジェクトのクラスと基底（MRO の順）の `(__module__, __name__)`（`is` / `mustbe` の判定・ffi_value_exchange_plan.md
/// 7-27）。読めない項目は飛ばす（`__module__` の無いクラスは空文字列）。
pub fn py_class_mro(h: &PyObjHandle) -> Vec<(String, String)> {
    with_py(|py| {
        let ty = h.bind(py).get_type();
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
    with_py(|py| -> PyResult<Rc<NamespaceData>> {
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
                let val = Value::PyObject(PyObjHandle::from_owned(attr.unbind()));
                members.insert(name, val);
            }
        }

        Ok(Rc::new(NamespaceData {
            name: module_name,
            members,
            // メンバーは読むたびにモジュールから引く（`NsLive::Python`）。`members` は `dir()` と呼び先の言語の判定用の写し
            live: Some(crate::interpreter::value::NsLive::Python(PyObjHandle::from_bound(module.as_any()))),
        }))
    })
    .map_err(|e| format!("ImportError: {e}"))
}
