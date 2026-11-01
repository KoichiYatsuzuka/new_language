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
// - `with_py`: CPython へ入る唯一の入口（GIL を取り、参照の増減の待ち行列を反映する・D-5）

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use std::path::PathBuf;

use pyo3::class::basic::CompareOp;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{
    PyAnyMethods, PyByteArray, PyBytes, PyDict, PyDictMethods, PyFloat, PyFrozenSet, PyInt, PyList, PyListMethods,
    PyMemoryView, PyModule, PySet, PyTuple, PyTupleMethods,
};
use pyo3::Borrowed;

use crate::ast::BinOp;

use super::{DictData, NamespaceData, PyObjHandle, TupleData, Value};

// ---------------------------------------------------------------------------
// CPython の参照の数（pandas_import_plan.md の D-5・タスク 1-1）
// ---------------------------------------------------------------------------
//
// ⚠⚠ `PyObjHandle` の写し 1 つが CPython の参照 1 つを持つ。CPython のコードは参照の数で振る舞いを変える
//   （pandas は `sys.getrefcount(self)` で連鎖代入を見分け、numpy の `resize` は参照が多いと誤りにする）ので、
//   Arrow の束縛・欄・要素・VM のスタックの値が、CPython で同じ形を書いたときの変数・評価のスタックと同じ数を持つ。
// ⚠ 写す・捨てるのに GIL は要らない。持っていなければ増減を待ち行列（`PENDING`）に積み、CPython へ入る入口
//   （`with_py`）で反映する。CPython が数を見られるのは入口の中だけなので、CPython から見える数は正確になる。
//   （PyO3 0.22 の `Py::clone` は GIL が無いと panic する。Arrow は格納のたびに値を写すので、そのたびに GIL を取ると高い。）

thread_local! {
    /// このスレッドが `with_py` の中（GIL を持っている）か（入れ子の深さ）。
    static IN_PY: Cell<u32> = const { Cell::new(0) };
}

/// GIL を持たずに写した・捨てた `PyObjHandle` の参照の増減（オブジェクトの番地, 増やすなら真）。
/// ⚠ `async` のタスク（別のスレッド）も写すので、プロセスで 1 つ。
static PENDING: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());

/// 待ち行列がこれを超えたら GIL を取って反映する（CPython を呼ばないループで写し続けても膨らみ続けないように）。
const PENDING_LIMIT: usize = 1 << 14;

fn gil_held_here() -> bool {
    IN_PY.with(|d| d.get() > 0)
}

fn defer_refcount(ptr: NonNull<ffi::PyObject>, incref: bool) {
    let full = {
        let mut q = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        q.push((ptr.as_ptr() as usize, incref));
        q.len() > PENDING_LIMIT
    };
    if full {
        with_py(|_| ());
    }
}

/// 待ち行列を反映する（GIL の下で）。
///
/// ⚠ **増やす分を先に、減らす分を後に。** 写した値を積んだ後に元を捨てると、同じ物の +1 と -1 が並ぶ。減らす分を先に
///   反映すると、写しが残っているのに 0 になって解放されうる（写しは必ず元より先に積まれるので、先に増やせば安全）。
fn apply_pending(_py: Python<'_>) {
    let q = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    for &(p, incref) in &q {
        if incref {
            unsafe { ffi::Py_IncRef(p as *mut ffi::PyObject) }
        }
    }
    for &(p, incref) in &q {
        if !incref {
            unsafe { ffi::Py_DecRef(p as *mut ffi::PyObject) }
        }
    }
}

/// CPython へ入る**唯一の入口**: GIL を取り、待ち行列を反映してから `f` を走らせる。
///
/// ⚠ CPython を呼ぶ処理は `Python::with_gil` を直接使わずにこれを通すこと。通さないと、待ち行列に残った増減を
///   反映しないまま CPython が参照の数を見る。
pub(crate) fn with_py<R>(f: impl FnOnce(Python<'_>) -> R) -> R {
    Python::with_gil(|py| {
        struct Depth;
        impl Drop for Depth {
            fn drop(&mut self) {
                IN_PY.with(|d| d.set(d.get() - 1));
            }
        }
        IN_PY.with(|d| d.set(d.get() + 1));
        let _depth = Depth;
        apply_pending(py);
        f(py)
    })
}

impl PyObjHandle {
    /// 所有している参照をそのまま受け取る。
    pub(crate) fn from_owned(obj: Py<PyAny>) -> Self {
        PyObjHandle { ptr: NonNull::new(obj.into_ptr()).expect("Py<PyAny> is never null") }
    }

    /// 新しい参照を 1 つ作って持つ（GIL の下で）。
    pub(crate) fn from_bound(obj: &Bound<'_, PyAny>) -> Self {
        Self::from_owned(obj.clone().unbind())
    }

    /// 借りる（参照の数を変えない）。CPython で同じ式を書いたときの「評価のスタックの参照」は、この値（Arrow の写し）自身。
    pub(crate) fn bind<'a, 'py>(&'a self, py: Python<'py>) -> Borrowed<'a, 'py, PyAny> {
        unsafe { Borrowed::from_ptr(py, self.ptr.as_ptr()) }
    }

    /// CPython 側に持たせる新しい参照（CPython のコンテナの要素にするときなど）。
    pub(crate) fn to_py(&self, py: Python<'_>) -> PyObject {
        self.bind(py).to_owned().unbind()
    }

    /// 自分の参照を写さずに CPython へ渡す（呼び出しの肩代わり・`call_shaped`）。
    /// ⚠ `with_py` の中で呼ぶこと（入口で待ち行列を反映済みなので、この参照は本物）。
    pub(crate) fn into_py(self, py: Python<'_>) -> PyObject {
        let p = self.ptr.as_ptr();
        std::mem::forget(self);
        unsafe { Py::from_owned_ptr(py, p) }
    }
}

impl Clone for PyObjHandle {
    fn clone(&self) -> Self {
        if gil_held_here() {
            unsafe { ffi::Py_IncRef(self.ptr.as_ptr()) }
        } else {
            defer_refcount(self.ptr, true);
        }
        PyObjHandle { ptr: self.ptr }
    }
}

impl Drop for PyObjHandle {
    fn drop(&mut self) {
        if gil_held_here() {
            unsafe { ffi::Py_DecRef(self.ptr.as_ptr()) }
        } else {
            defer_refcount(self.ptr, false);
        }
    }
}

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
        Value::PyObject(h) => Ok(h.to_py(py)),
        other => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot pass an Arrow value of type '{}' to Python",
            type_name_of(other)
        ))),
    }
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
        return Value::PyObject(PyObjHandle::from_bound(obj));
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
    // bytes → Value::Bytes（タスク 3-1）。⚠ bytearray / memoryview は PyObject のまま。
    if let Ok(b) = obj.downcast::<pyo3::types::PyBytes>() {
        use pyo3::types::PyBytesMethods;
        return Value::Bytes(b.as_bytes().into());
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
    Value::PyObject(PyObjHandle::from_bound(obj))
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
///
/// ⚠ 呼ぶ物と引数は**受け取って手放す**（写さない）。CPython のバイトコードと同じく、評価のスタックの参照（Arrow の値の写し）を
///   呼ばれた側へ引き渡すため（[`call_shaped`]・D-5）。
pub fn call_py_object(
    handle: PyObjHandle,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    call_shaped(handle, None, evaled)
}

/// Python オブジェクトのメソッドを呼び出す（受け手と引数の扱いは [`call_py_object`] と同じ）。
pub fn call_py_method(
    handle: PyObjHandle,
    method_name: &str,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    call_shaped(handle, Some(method_name), evaled)
}

// ---------------------------------------------------------------------------
// 呼び出しの肩代わり（D-5・タスク 1-1）
// ---------------------------------------------------------------------------
//
// ⚠⚠ C の API（`PyObject_Call` / `PyObject_Vectorcall`）から Python の関数を呼ぶと、呼ばれた側のフレームが引数の参照を
//   1 つ増やす。CPython のバイトコードは評価のスタックの参照をフレームへ**引き渡す**（増やさない）ので、C から呼ぶ限り
//   受け手・引数の参照の数は CPython で書いたときより 1 つ多い（実測: 変数のメソッド 3 → 4・一時的な物 2 → 3）。
//   pandas の inplace=True のメソッドは `self` の参照の数で連鎖代入を見分けるので、1 つ多いと本物の連鎖代入を見逃す。
// ⇒ 呼び出しだけを小さな Python の関数（肩代わり）に行わせる:
//       lambda __arrow_box__: __arrow_box__.pop().NAME(__arrow_box__.pop(), K=__arrow_box__.pop())
//   Arrow は自分の参照（受け手・呼ぶ物・引数）をリストに入れて手放し、肩代わりは `pop` した値を変数に入れずに呼ぶ。
//   `pop` が返した参照は評価のスタックにあり、`CALL` がそれを引き渡すので、CPython で書いたときと同じ数になる。
//   形（メソッド名・位置引数の数・キーワードの名前）ごとに 1 度だけ作って使い回す。

/// 肩代わりの関数の引数の名前（利用者の名前と重ならないように）。
const BOX: &str = "__arrow_box__";

/// 肩代わりの関数の形: (メソッド名（呼ぶ物そのものなら `None`）, 位置引数の数, キーワードの名前)。
type ShapeKey = (Option<String>, usize, Vec<String>);

static TRAMPOLINES: OnceLock<Mutex<HashMap<ShapeKey, Py<PyAny>>>> = OnceLock::new();

/// 肩代わりの関数のソースに名前として書けるか（Python の識別子で、予約語でない）。
fn is_py_identifier(s: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
        "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
    ];
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c == '_' || c.is_alphabetic())
        && cs.all(|c| c == '_' || c.is_alphanumeric())
        && !KEYWORDS.contains(&s)
        && s != BOX
}

fn trampoline<'py>(py: Python<'py>, key: &ShapeKey) -> PyResult<Bound<'py, PyAny>> {
    let cache = TRAMPOLINES.get_or_init(Default::default);
    if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
        return Ok(f.bind(py).clone());
    }
    let (method, n_pos, kw_names) = key;
    let pop = format!("{BOX}.pop()");
    let mut args: Vec<String> = (0..*n_pos).map(|_| pop.clone()).collect();
    args.extend(kw_names.iter().map(|k| format!("{k}={pop}")));
    let callee = match method {
        Some(m) => format!("{pop}.{m}"),
        None => pop.clone(),
    };
    let src = format!("lambda {BOX}: {callee}({})", args.join(", "));
    let f = py.eval_bound(&src, Some(&PyDict::new_bound(py)), None)?;
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), f.clone().unbind());
    Ok(f)
}

/// Arrow の値を、写さずに（CPython の物なら自分の参照を手放して）CPython の物にする。
fn ar_to_py_owned(py: Python<'_>, val: Value) -> PyResult<PyObject> {
    match val {
        Value::PyObject(h) => Ok(h.into_py(py)),
        other => ar_to_py(py, &other),
    }
}

/// `head`（呼ぶ物、または `method` の受け手）を呼ぶ。引数の参照も受け取って手放す。
fn call_shaped(
    head: PyObjHandle,
    method: Option<&str>,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let mut pos: Vec<Value> = Vec::new();
        let mut kw: Vec<(String, Value)> = Vec::new();
        for (name, val, _) in evaled {
            match name {
                None => pos.push(val),
                Some(k) => kw.push((k, val)),
            }
        }
        let kw_names: Vec<String> = kw.iter().map(|(k, _)| k.clone()).collect();
        let unique = kw_names.iter().enumerate().all(|(i, k)| !kw_names[..i].contains(k));
        let shapeable = method.map_or(true, is_py_identifier) && kw_names.iter().all(|k| is_py_identifier(k)) && unique;
        if !shapeable {
            // ⚠ ソースに書けない名前（`getattr` でしか引けないメソッド名・予約語のキーワード）だけは、肩代わりを使わずに呼ぶ。
            //   このときだけ、受け手・引数の参照は CPython で書いたときより 1 つ多い。
            return call_unshaped(py, head, method, pos, kw);
        }
        let f = trampoline(py, &(method.map(str::to_string), pos.len(), kw_names)).map_err(arrow_side)?;
        let mut owned: Vec<PyObject> = Vec::with_capacity(1 + pos.len() + kw.len());
        owned.push(head.into_py(py));
        for v in pos {
            owned.push(ar_to_py_owned(py, v).map_err(arrow_side)?);
        }
        for (_, v) in kw {
            owned.push(ar_to_py_owned(py, v).map_err(arrow_side)?);
        }
        // 肩代わりは後ろから `pop` するので、[頭, 位置引数.., キーワード..] の逆順に並べる。
        // ⚠ 要素は `PyList_SET_ITEM` で**引き渡す**（`PyList::new_bound` は写すので、作る間だけ参照が 1 つ多くなる）。
        owned.reverse();
        let list = unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyList_New(owned.len() as ffi::Py_ssize_t)) }
            .map_err(arrow_side)?;
        for (i, o) in owned.into_iter().enumerate() {
            unsafe { ffi::PyList_SET_ITEM(list.as_ptr(), i as ffi::Py_ssize_t, o.into_ptr()) };
        }
        let result = f.call1((list,)).map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// 肩代わりを使わない呼び出し（[`call_shaped`] のソースに名前を書けないときだけ）。
fn call_unshaped(
    py: Python<'_>,
    head: PyObjHandle,
    method: Option<&str>,
    pos: Vec<Value>,
    kw: Vec<(String, Value)>,
) -> Result<Value, String> {
    let callable = match method {
        Some(m) => head.bind(py).getattr(m).map_err(python_raised)?,
        None => head.bind(py).to_owned(),
    };
    let mut positional: Vec<PyObject> = Vec::with_capacity(pos.len());
    for v in pos {
        positional.push(ar_to_py_owned(py, v).map_err(arrow_side)?);
    }
    let kwargs = PyDict::new_bound(py);
    for (k, v) in kw {
        kwargs.set_item(k, ar_to_py_owned(py, v).map_err(arrow_side)?).map_err(arrow_side)?;
    }
    let result = callable.call(PyTuple::new_bound(py, positional), Some(&kwargs)).map_err(python_raised)?;
    Ok(py_to_tl(py, &result))
}

/// 添字の鍵・代入する値（Arrow の値）を CPython の物にする。CPython の物は**借りるだけ**（CPython の `x[k] = v` も、
/// 評価のスタックの参照のほかに参照を増やさない・D-5）。
enum PyOperand<'a, 'py> {
    Borrowed(Borrowed<'a, 'py, PyAny>),
    Owned(Bound<'py, PyAny>),
}

impl PyOperand<'_, '_> {
    fn as_ptr(&self) -> *mut ffi::PyObject {
        match self {
            PyOperand::Borrowed(b) => b.as_ptr(),
            PyOperand::Owned(o) => o.as_ptr(),
        }
    }
}

fn py_operand<'a, 'py>(py: Python<'py>, val: &'a Value) -> PyResult<PyOperand<'a, 'py>> {
    match val {
        Value::PyObject(h) => Ok(PyOperand::Borrowed(h.bind(py))),
        other => Ok(PyOperand::Owned(ar_to_py(py, other)?.into_bound(py))),
    }
}

/// Python オブジェクトの属性を取得して tl の Value として返す。
pub fn py_getattr(handle: &PyObjHandle, attr: &str) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
        let attr_val = obj.getattr(attr).map_err(python_raised)?;
        Ok(py_to_tl(py, &attr_val))
    })
}

/// `obj[key]`: Python の `__getitem__` を呼び出す。
///
/// ⚠ 受け手と鍵は借りるだけ（`PyObject_GetItem` を直接呼ぶ・[`py_operand`]）。PyO3 の get_item（`PyAnyMethods`）は鍵を写す。
pub fn py_getitem(handle: &PyObjHandle, key: &Value) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
        let py_key = py_operand(py, key).map_err(arrow_side)?;
        let result = unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyObject_GetItem(obj.as_ptr(), py_key.as_ptr())) }
            .map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// `obj[key] = val`: Python の `__setitem__` を呼び出す。
///
/// ⚠ 受け手・鍵・値は借りるだけ（`PyObject_SetItem` を直接呼ぶ）。CPython の `obj[key] = val` と同じ参照の数で
///   `__setitem__` が走る（pandas はここで `self` の参照の数から連鎖代入を見分ける・D-5）。
pub fn py_setitem(handle: &PyObjHandle, key: &Value, val: &Value) -> Result<(), String> {
    with_py(|py| -> Result<(), String> {
        let obj = handle.bind(py);
        let py_key = py_operand(py, key).map_err(arrow_side)?;
        let py_val = py_operand(py, val).map_err(arrow_side)?;
        if unsafe { ffi::PyObject_SetItem(obj.as_ptr(), py_key.as_ptr(), py_val.as_ptr()) } == -1 {
            return Err(python_raised(PyErr::fetch(py)));
        }
        Ok(())
    })
}

/// Python の int（`bool` を除く）なら、その表記（`str(obj)`）。i64 に入らない int は Python のオブジェクトのまま届くので、
/// 境界検査が OverflowError の文面に使う（`ffi_boundary`・7-9）。
pub fn py_int_text(handle: &PyObjHandle) -> Option<String> {
    with_py(|py| {
        let obj = handle.bind(py);
        if obj.is_instance_of::<PyInt>() && !obj.is_instance_of::<pyo3::types::PyBool>() {
            obj.str().ok().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// `len(obj)`: Python の `__len__` を呼び出す。
pub fn py_len(handle: &PyObjHandle) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
        let n = obj.len().map_err(python_raised)?;
        Ok(Value::Int(n as i64))
    })
}

/// `callable(obj)`: Python の `callable()` と同じ判定（python_builtins_plan.md のタスク 2-4）。
pub fn py_is_callable(handle: &PyObjHandle) -> bool {
    with_py(|py| handle.bind(py).is_callable())
}

/// Python iterable を走査して要素を `Vec<Value>` に収集する。for ループ用。
pub fn py_collect_iter(handle: &PyObjHandle) -> Result<Vec<Value>, String> {
    with_py(|py| -> Result<Vec<Value>, String> {
        let obj = handle.bind(py);
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
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
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
                    .call_method1("__contains__", (obj.to_owned().unbind(),))
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
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
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

/// Value のランタイム型名を返す（エラーメッセージ・タプルの要素型名用・[`element_type_name`]）。
fn type_name_of(val: &Value) -> String {
    crate::interpreter::ops::typecheck::element_type_name(val)
}
