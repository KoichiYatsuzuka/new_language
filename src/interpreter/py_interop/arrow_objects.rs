// interpreter/py_interop/arrow_objects.rs — Arrow の関数・インスタンス・ジェネレータを写さずに Python へ渡す包み。

use super::*;
use self::arrow_error::ArrowError;
use pyo3::PyTypeInfo;

// ---------------------------------------------------------------------------
// Arrow の値を Python から使う（関数: pandas_import_plan.md の 2-5・設計は 2.7／インスタンス・クラス・ジェネレータ: 2-6・D-9）
// ---------------------------------------------------------------------------
//
// Python へ渡した Arrow の値は**写さずに共有する**（CPython と同じ。同じインスタンスを 2 回渡せば同じ包み）。中身の `Value` は
// スレッドごとの表（[`ARROW_VALUES`]）に置き、包みは番号と作ったスレッドだけを持つ（`Rc` を Python のスレッドへ持ち出さない）。
// - 関数・クラス・ジェネレータ関数 → [`ArrowFunction`]（呼べる物。クラスを呼ぶとインスタンスの包みが返る）
// - インスタンス → [`ArrowObject`] の派生（Arrow のクラスごとに 1 つ。Arrow のクラスが持つ特殊メソッドだけを持つ・[`proxy_type`]）
// - ジェネレータ → [`ArrowIterator`]
// Python から Arrow へ戻ってきた包みは元の値に戻る（[`unwrap_arrow_value`]・同じインスタンス）。
// ⚠ インスタンスの包みは**書けるか**を持つ（2.6 の ⑤）。書けるのは Arrow の側で書ける経路から渡したときだけ: 呼び出しの引数の
//   `mut` の変数・式（[`ar_to_py_arg`]）、書ける包みから読んだ物、Arrow の関数・ジェネレータが Python へ返した物。それ以外
//   （`let` の変数・Python の物への格納・演算子の相手・キャストの要素）は読むだけで、属性への代入と `mut self` のメソッドは `TypeError`。
// ⚠ 作ったスレッド以外から触られたら Python の `RuntimeError`（Arrow の値はスレッドをまたげない・2.6 の ⑥）。

thread_local! {
    /// このスレッドで Python を呼んでいるインタプリタ（包みが呼び戻す先）。
    static PY_INTERP: Cell<*mut crate::interpreter::Interpreter> = const { Cell::new(std::ptr::null_mut()) };
    /// Python へ渡した Arrow の値（番号 → 値）。
    static ARROW_VALUES: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
    /// Python の中を通っている Arrow の例外（Python の例外のオブジェクト, Arrow の文言, 元の Arrow の例外）。Arrow へ戻ったら
    /// [`python_raised`] が元の例外に戻す。
    pub(super) static ARROW_RAISED: RefCell<Vec<(PyObject, String, Option<crate::interpreter::RaisedError>)>> =
        const { RefCell::new(Vec::new()) };
    /// Arrow のクラスごとの包みの型（鍵は `ClassValue::class_id`）。
    static PROXY_TYPES: RefCell<HashMap<u32, Py<PyAny>>> = RefCell::new(HashMap::new());
    /// 包みの同一性: (インスタンスの番地, 書けるか) → 包みの弱参照。⚠ 包みが生きている間は表がインスタンスを持つので、番地は
    /// 使い回されない（死んだ弱参照の項は [`wrap_arrow_object`] が掃除する）。
    static PROXIES: RefCell<HashMap<(usize, bool), Py<PyAny>>> = RefCell::new(HashMap::new());
}

/// 包みの番号（プロセスで一意）。
static NEXT_VALUE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 作ったスレッドとは別のスレッドで捨てられた包み（作ったスレッド, 番号）。作ったスレッドで次に表を使うときに消す。
static DROPPED_ELSEWHERE: Mutex<Vec<(std::thread::ThreadId, u64)>> = Mutex::new(Vec::new());

/// Arrow で定義した例外と、例外にならない Arrow の誤りが Python の中を通るときの型（`arrow.ArrowError`・`Exception` の子）。
/// ⚠ PyO3 0.22 の `create_exception!` は自分の crate に無い `cfg(feature = "gil-refs")` を書くので、その警告だけ止める。
#[allow(unexpected_cfgs)]
pub(super) mod arrow_error {
    pyo3::create_exception!(arrow, ArrowError, pyo3::exceptions::PyException, "An exception raised by Arrow code.");
}

/// このスレッドで Python を呼ぶインタプリタを覚える（2-5）。Python のモジュールの import と Python を呼ぶ入口で呼ぶ。
pub fn register_interpreter(interp: *mut crate::interpreter::Interpreter) {
    PY_INTERP.with(|p| p.set(interp));
}

/// インタプリタが無くなるときに忘れる（`Interpreter` の `Drop`）。
pub fn forget_interpreter(interp: *mut crate::interpreter::Interpreter) {
    let _ = PY_INTERP.try_with(|p| {
        if p.get() == interp {
            p.set(std::ptr::null_mut())
        }
    });
}

/// Arrow の値を表に置き、番号を返す。別のスレッドで捨てられた包みの中身をここで消す。
fn store_arrow_value(val: &Value) -> u64 {
    let me = std::thread::current().id();
    let stale: Vec<u64> = {
        let mut q = DROPPED_ELSEWHERE.lock().unwrap_or_else(|e| e.into_inner());
        let (mine, others): (Vec<_>, Vec<_>) = q.drain(..).partition(|(t, _)| *t == me);
        *q = others;
        mine.into_iter().map(|(_, id)| id).collect()
    };
    // ⚠ 値は表の借用を落としてから捨てる（中の値が別の包みを捨てると、その `Drop` がまた表を借りる）
    let removed: Vec<Option<Value>> = ARROW_VALUES.with(|t| stale.iter().map(|id| t.borrow_mut().remove(id)).collect());
    drop(removed);
    let id = NEXT_VALUE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    ARROW_VALUES.with(|t| t.borrow_mut().insert(id, val.clone()));
    id
}

/// 包みが捨てられた（`Drop`）: 表から消す（別のスレッドなら、作ったスレッドで次に表を使うときに消す）。
fn release_arrow_value(id: u64, thread: std::thread::ThreadId) {
    if std::thread::current().id() == thread {
        let removed = ARROW_VALUES.try_with(|t| t.borrow_mut().remove(&id));
        drop(removed);
    } else {
        DROPPED_ELSEWHERE.lock().unwrap_or_else(|e| e.into_inner()).push((thread, id));
    }
}

/// 包みの中身（`what` は誤りの文言に出す名前）。作ったスレッド以外からは Python の `RuntimeError`。
fn arrow_value(id: u64, thread: std::thread::ThreadId, what: &str) -> PyResult<Value> {
    if std::thread::current().id() != thread {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "{what} can only be used from the thread that passed it to Python (Arrow values cannot cross threads)"
        )));
    }
    ARROW_VALUES
        .with(|t| t.borrow().get(&id).cloned())
        .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err(format!("{what} was released")))
}

/// このスレッドで走っているインタプリタ（包みが Arrow を呼び戻す先）。
///
/// ⚠ Python を呼んだ Arrow の側も `&mut Interpreter` を持ったまま待っている（ネイティブの ABI の `interp_ptr` と同じ形の
///   再入）。呼び戻しの間、外側の Arrow はこのインタプリタに触らない。
pub(super) fn running_interpreter<'a>(what: &str) -> PyResult<&'a mut crate::interpreter::Interpreter> {
    let interp = PY_INTERP.with(|p| p.get());
    if interp.is_null() {
        return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "{what} was used while no Arrow interpreter is running on this thread"
        )));
    }
    Ok(unsafe { &mut *interp })
}

/// Arrow が Python へ渡した包みなら、元の Arrow の値（同じスレッドのときだけ）。インスタンスは同じ物（同一性を保つ・2-6）。
/// ⚠ 読むだけの包みから読んだ `mut self` のメソッド（`blocked`）は戻さない（Arrow から呼べば書き換えられてしまう）。
pub(super) fn unwrap_arrow_value(obj: &Bound<'_, PyAny>) -> Option<Value> {
    let (id, thread) = if let Ok(o) = obj.downcast::<ArrowObject>() {
        let o = o.borrow();
        (o.id, o.thread)
    } else if let Ok(f) = obj.downcast::<ArrowFunction>() {
        let f = f.borrow();
        if f.blocked.is_some() {
            return None;
        }
        (f.id, f.thread)
    } else if let Ok(g) = obj.downcast::<ArrowIterator>() {
        let g = g.borrow();
        (g.id, g.thread)
    } else {
        return None;
    };
    if std::thread::current().id() != thread {
        return None;
    }
    ARROW_VALUES.with(|t| t.borrow().get(&id).cloned())
}

/// 呼び出しの引数を CPython の物にする（[`call_shaped`]）。CPython の物は自分の参照を手放して渡し（写さない・D-5）、Arrow の
/// インスタンスは `writable`（引数が `mut` の変数・式か）の包み（2-6）。
pub(super) fn ar_to_py_arg(py: Python<'_>, val: Value, writable: bool) -> PyResult<PyObject> {
    match val {
        Value::PyObject(h) => Ok(h.into_py(py)),
        ref v @ Value::Instance(_) if !is_singleton(v) => wrap_arrow_object(py, v, writable),
        other => ar_to_py(py, &other),
    }
}

/// インスタンスを写さずに `writable` の包みにする（ほかは [`ar_to_py`]）。包みから読んだ物・Arrow が Python へ返した物（2-6）。
fn to_py_shared(py: Python<'_>, val: &Value, writable: bool) -> PyResult<PyObject> {
    match val {
        Value::Instance(_) if !is_singleton(val) => wrap_arrow_object(py, val, writable),
        other => ar_to_py(py, other),
    }
}

/// CPython の同じ値に写す唯一の値（`...` / `NotImplemented`・2-3）か。包まない。
fn is_singleton(val: &Value) -> bool {
    matches!(val, Value::Instance(inst) if matches!(
        inst.borrow().class.builtin,
        crate::interpreter::BuiltinClass::Ellipsis | crate::interpreter::BuiltinClass::NotImplemented
    ))
}

/// Python から呼べる Arrow の関数・クラス・ジェネレータ関数（2-5・2-6）。中身は [`ARROW_VALUES`] にあり、作ったスレッドからだけ呼べる。
#[pyclass(name = "ArrowFunction", module = "arrow")]
pub struct ArrowFunction {
    id: u64,
    thread: std::thread::ThreadId,
    name: String,
    /// 呼べない理由（読むだけの包みから読んだ `mut self` のメソッド・2-6）。呼ぶと `TypeError`。
    blocked: Option<String>,
}

#[pymethods]
impl ArrowFunction {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        call_arrow_function(py, self, args, kwargs)
    }

    fn __repr__(&self) -> String {
        format!("<Arrow function {}>", self.name)
    }

    #[getter]
    fn __name__(&self) -> String {
        self.name.clone()
    }

    #[getter]
    fn __qualname__(&self) -> String {
        self.name.clone()
    }
}

impl Drop for ArrowFunction {
    fn drop(&mut self) {
        release_arrow_value(self.id, self.thread);
    }
}

/// Arrow の関数・クラス・ジェネレータ関数を包む（[`ArrowFunction`]）。
pub(super) fn wrap_arrow_function(py: Python<'_>, val: &Value, name: String) -> PyResult<PyObject> {
    wrap_arrow_function_with(py, val, name, None)
}

fn wrap_arrow_function_with(py: Python<'_>, val: &Value, name: String, blocked: Option<String>) -> PyResult<PyObject> {
    let id = store_arrow_value(val);
    let thread = std::thread::current().id();
    Ok(Py::new(py, ArrowFunction { id, thread, name, blocked })?.into_any())
}

/// Python が [`ArrowFunction`] を呼んだ: このスレッドのインタプリタで関数を呼ぶ（再入・GIL は持ったまま）。引数は
/// Python → Arrow（`py_to_tl`）、戻り値は Arrow → Python（インスタンスは書ける包み・[`to_py_shared`]）。本体の Arrow の例外は
/// Python の例外（[`arrow_error_to_py`]）。
fn call_arrow_function(
    py: Python<'_>,
    f: &ArrowFunction,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyObject> {
    if let Some(reason) = &f.blocked {
        return Err(pyo3::exceptions::PyTypeError::new_err(reason.clone()));
    }
    let what = format!("the Arrow function '{}'", f.name);
    let callee = arrow_value(f.id, f.thread, &what)?;
    let interp = running_interpreter(&what)?;
    let mut evaled: Vec<(Option<String>, Value, bool)> = args.iter().map(|a| (None, py_to_tl(py, &a), false)).collect();
    if let Some(kw) = kwargs {
        for (k, v) in kw.iter() {
            evaled.push((Some(k.extract::<String>()?), py_to_tl(py, &v), false));
        }
    }
    match interp.call_value_evaled(callee, evaled, &f.name, None, 0) {
        Ok(v) => to_py_shared(py, &v, true),
        Err(e) => {
            // 呼んだ側の枠は Arrow の呼び出しの位置が無い（Python が呼んだ）ので、呼んだ Python の枠（ファイル・行・関数）にする
            if e == crate::interpreter::RAISE_SENTINEL {
                if let Some(frame) = interp.current_exception.as_mut().and_then(|r| r.frames.last_mut()) {
                    if frame.file.is_empty() {
                        if let Some((file, line, name)) = python_caller_frame(py) {
                            *frame = crate::interpreter::value::StackFrame { file, line, col: 0, fn_name: name, context: String::new() };
                        }
                    }
                }
            }
            Err(arrow_error_to_py(py, interp, e))
        }
    }
}

/// Arrow のクラスのメソッドのうち、包みの型（[`proxy_type`]）に写す特殊メソッド。Python は特殊メソッドを型から引くので、
/// Arrow のクラスが持つものだけを持つ派生を作る（持たない `__iter__` を持つと pandas などが「list のような物」と見なす）。
/// ⚠ 比較（`__eq__` / `__lt__` …）・`__hash__`・`__str__` / `__repr__` は [`ArrowObject`] 自身が持ち、Arrow の意味で答える。
const SPECIAL_METHODS: &[&str] = &[
    "__len__", "__iter__", "__next__", "__getitem__", "__setitem__", "__delitem__", "__contains__", "__call__", "__bool__",
    "__neg__", "__pos__", "__abs__", "__invert__", "__index__", "__int__", "__float__", "__add__", "__radd__", "__sub__",
    "__rsub__", "__mul__", "__rmul__", "__truediv__", "__rtruediv__", "__floordiv__", "__rfloordiv__", "__mod__", "__rmod__",
    "__pow__", "__rpow__", "__and__", "__rand__", "__or__", "__ror__", "__xor__", "__rxor__", "__lshift__", "__rlshift__",
    "__rshift__", "__rrshift__", "__enter__", "__exit__",
];

/// 包みの型を作る Python の関数（Arrow のクラスごとに 1 度だけ呼ぶ）。特殊メソッドは [`ArrowObject::_arrow_special`] へ回す。
/// ⚠ `__slots__ = ()` で `__dict__` を持たせない（属性の読み書きはすべて [`ArrowObject`] の `__getattr__` / `__setattr__`）。
const PROXY_FACTORY: &str = r#"
def make(base, name, specials):
    def forward(m):
        def method(self, *args, **kwargs):
            return self._arrow_special(m, args, kwargs)
        method.__name__ = m
        method.__qualname__ = name + "." + m
        return method
    ns = {"__slots__": (), "__module__": "arrow", "__qualname__": name}
    for m in specials:
        ns[m] = forward(m)
    return type(name, (base,), ns)
"#;

static PROXY_FACTORY_FN: OnceLock<Py<PyAny>> = OnceLock::new();

/// Arrow のクラス（`class_id`）の包みの型。初めてなら [`PROXY_FACTORY`] で作って覚える。
fn proxy_type<'py>(py: Python<'py>, class_id: u32, name: &str, specials: &[&str]) -> PyResult<Bound<'py, PyAny>> {
    if let Some(t) = PROXY_TYPES.with(|m| m.borrow().get(&class_id).map(|t| t.clone_ref(py))) {
        return Ok(t.into_bound(py));
    }
    let make = match PROXY_FACTORY_FN.get() {
        Some(f) => f.bind(py).clone(),
        None => {
            let module = PyModule::from_code_bound(py, PROXY_FACTORY, "arrow_proxy.py", "arrow_proxy")?;
            let f = module.getattr("make")?;
            let _ = PROXY_FACTORY_FN.set(f.clone().unbind());
            f
        }
    };
    let ty = make.call1((ArrowObject::type_object_bound(py), name, PyTuple::new_bound(py, specials)))?;
    PROXY_TYPES.with(|m| m.borrow_mut().insert(class_id, ty.clone().unbind()));
    Ok(ty)
}

/// Arrow のインスタンスを包む（[`ArrowObject`] の派生）。同じインスタンスを同じ書けるかで渡したら同じ包み（CPython の `is`）。
pub(super) fn wrap_arrow_object(py: Python<'_>, val: &Value, writable: bool) -> PyResult<PyObject> {
    use pyo3::pyclass_init::PyObjectInit;
    use pyo3::types::{PyWeakrefMethods, PyWeakrefReference};
    let Value::Instance(inst) = val else {
        return ar_to_py(py, val);
    };
    let key = (Rc::as_ptr(inst) as usize, writable);
    let known = PROXIES.with(|m| m.borrow().get(&key).map(|w| w.clone_ref(py)));
    if let Some(weak) = known {
        if let Ok(weak) = weak.bind(py).downcast::<PyWeakrefReference>() {
            let obj = weak.get_object();
            if !obj.is_none() {
                return Ok(obj.unbind());
            }
        }
    }
    let (class_id, class_name, specials) = {
        let i = inst.borrow();
        let specials: Vec<&str> = SPECIAL_METHODS.iter().copied().filter(|m| i.class.methods.contains_key(*m)).collect();
        (i.class.class_id, i.class.name.clone(), specials)
    };
    let ty = proxy_type(py, class_id, &class_name, &specials)?;
    let id = store_arrow_value(val);
    let init = ArrowObject { id, thread: std::thread::current().id(), class_name, writable };
    let raw = unsafe { PyClassInitializer::from(init).into_new_object(py, ty.as_ptr() as *mut ffi::PyTypeObject) }?;
    let obj: PyObject = unsafe { Py::from_owned_ptr(py, raw) };
    let weak = PyWeakrefReference::new_bound(obj.bind(py))?;
    PROXIES.with(|m| {
        let mut m = m.borrow_mut();
        // 死んだ包みの項を掃除する（表が大きくなったときだけ）
        if m.len() >= 64 {
            m.retain(|_, w| {
                w.bind(py).downcast::<PyWeakrefReference>().is_ok_and(|w| !w.get_object().is_none())
            });
        }
        m.insert(key, weak.into_any().unbind());
    });
    Ok(obj)
}

/// `obj` のクラスのメソッド `name` が `mut self` か（読むだけの包みからは呼べない）。
fn method_mutates_self(obj: &Value, name: &str) -> bool {
    let Value::Instance(inst) = obj else { return false };
    inst.borrow()
        .class
        .methods
        .get(name)
        .and_then(|fs| fs.first())
        .is_some_and(|f| f.params.first().is_some_and(|p| p.name == "self" && p.mutable))
}

/// Python から使う Arrow のインスタンス（2-6・D-9）。Arrow のクラスごとの派生（[`proxy_type`]）のインスタンスとして作る。
/// 属性・メソッドは Arrow のインスタンスを読み、`str` / `repr` / 比較 / `hash` は Arrow の意味（`__str__` / `__eq__` … を持てばそれ）。
#[pyclass(name = "ArrowObject", module = "arrow", subclass, weakref)]
pub struct ArrowObject {
    id: u64,
    thread: std::thread::ThreadId,
    class_name: String,
    /// 書けるか（属性への代入・`mut self` のメソッド）。上の節の doc。
    writable: bool,
}

impl ArrowObject {
    /// 中身のインスタンスと、呼び戻すインタプリタ。
    pub(super) fn target<'a>(&self) -> PyResult<(Value, &'a mut crate::interpreter::Interpreter)> {
        let what = format!("the Arrow object '{}'", self.class_name);
        let v = arrow_value(self.id, self.thread, &what)?;
        Ok((v, running_interpreter(&what)?))
    }

    /// 読むだけの包みから `mut self` のメソッド `method` を呼ぼうとしたときの誤りの文言（呼べるなら `None`）。
    fn blocked(&self, obj: &Value, method: &str) -> Option<String> {
        (!self.writable && method_mutates_self(obj, method)).then(|| {
            format!(
                "cannot call the `mut self` method '{method}' of the Arrow object '{}' from Python: it was passed \
                 read-only (pass a `mut` variable to let Python change it)",
                self.class_name
            )
        })
    }
}

#[pymethods]
impl ArrowObject {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<PyObject> {
        let (obj, interp) = self.target()?;
        let blocked = self.blocked(&obj, name);
        let v = interp.get_attr_val(obj, name, None).map_err(|e| arrow_error_to_py(py, interp, e))?;
        match (&v, blocked) {
            (Value::Function(f), Some(reason)) => wrap_arrow_function_with(py, &v, f.name.clone(), Some(reason)),
            _ => to_py_shared(py, &v, self.writable),
        }
    }

    fn __setattr__(&self, py: Python<'_>, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if !self.writable {
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "cannot set '{name}' of the Arrow object '{}' from Python: it was passed read-only \
                 (pass a `mut` variable to let Python change it)",
                self.class_name
            )));
        }
        let (obj, interp) = self.target()?;
        let v = py_to_tl(py, value);
        interp.attr_assign_ic(obj, name, v, None).map_err(|e| arrow_error_to_py(py, interp, e))
    }

    fn __delattr__(&self, name: &str) -> PyResult<()> {
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot delete '{name}' of the Arrow object '{}': the members of an Arrow class are fixed by its body",
            self.class_name
        )))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let (obj, interp) = self.target()?;
        interp.repr_val(&obj).map_err(|e| arrow_error_to_py(py, interp, e))
    }

    fn __str__(&self, py: Python<'_>) -> PyResult<String> {
        let (obj, interp) = self.target()?;
        interp.display_str(&obj).map_err(|e| arrow_error_to_py(py, interp, e))
    }

    fn __richcmp__(&self, py: Python<'_>, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<PyObject> {
        let (obj, interp) = self.target()?;
        let rhs = py_to_tl(py, other);
        let op = match op {
            CompareOp::Lt => BinOp::Lt,
            CompareOp::Le => BinOp::LtEq,
            CompareOp::Eq => BinOp::Eq,
            CompareOp::Ne => BinOp::NotEq,
            CompareOp::Gt => BinOp::Gt,
            CompareOp::Ge => BinOp::GtEq,
        };
        let r = interp.apply_binop_dyn(&op, obj, rhs).map_err(|e| arrow_error_to_py(py, interp, e))?;
        ar_to_py(py, &r)
    }

    fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
        let (obj, interp) = self.target()?;
        interp.hash_value(&obj).map(|h| h as isize).map_err(|e| arrow_error_to_py(py, interp, e))
    }

    fn __dir__(&self) -> PyResult<Vec<String>> {
        let (obj, _) = self.target()?;
        let Value::Instance(inst) = obj else { return Ok(Vec::new()) };
        let inst = inst.borrow();
        let mut names: Vec<String> = inst.class.field_index.keys().chain(inst.class.methods.keys()).cloned().collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// Arrow のクラスが持つ特殊メソッド `name`（`__len__` / `__getitem__` / `__add__` …）を呼ぶ。包みの型の派生が回してくる
    /// （[`PROXY_FACTORY`]）。戻り値のインスタンスは、この包みと同じく書ける・読むだけ。
    #[pyo3(signature = (name, args, kwargs = None))]
    fn _arrow_special(
        &self,
        py: Python<'_>,
        name: &str,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        let (obj, interp) = self.target()?;
        if let Some(reason) = self.blocked(&obj, name) {
            return Err(pyo3::exceptions::PyTypeError::new_err(reason));
        }
        let mut evaled: Vec<(Option<String>, Value, bool)> = args.iter().map(|a| (None, py_to_tl(py, &a), true)).collect();
        if let Some(kw) = kwargs {
            for (k, v) in kw.iter() {
                evaled.push((Some(k.extract::<String>()?), py_to_tl(py, &v), true));
            }
        }
        match interp.call_instance_method_evaled(obj, name, evaled, None, None, self.writable) {
            Ok(v) => to_py_shared(py, &v, self.writable),
            Err(e) => Err(arrow_error_to_py(py, interp, e)),
        }
    }
}

impl Drop for ArrowObject {
    fn drop(&mut self) {
        release_arrow_value(self.id, self.thread);
    }
}

/// Python から回す Arrow のジェネレータ（2-6）。`for x in gen`・`next(gen)`・`list(gen)` で 1 つずつ取り出す。
#[pyclass(name = "ArrowIterator", module = "arrow")]
pub struct ArrowIterator {
    id: u64,
    thread: std::thread::ThreadId,
}

#[pymethods]
impl ArrowIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        let what = "the Arrow generator";
        let Value::Generator(state) = arrow_value(self.id, self.thread, what)? else {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(format!("{what} is not a generator")));
        };
        let interp = running_interpreter(what)?;
        match interp.gen_next(&state) {
            Ok(Some(v)) => Ok(Some(to_py_shared(py, &v, true)?)),
            Ok(None) => Ok(None),
            Err(e) => Err(arrow_error_to_py(py, interp, e)),
        }
    }
}

impl Drop for ArrowIterator {
    fn drop(&mut self) {
        release_arrow_value(self.id, self.thread);
    }
}

/// Arrow のジェネレータを包む（[`ArrowIterator`]）。
pub(super) fn wrap_arrow_iterator(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
    let id = store_arrow_value(val);
    Ok(Py::new(py, ArrowIterator { id, thread: std::thread::current().id() })?.into_any())
}

/// いま走っている Python の枠（Arrow の関数を呼んだ Python の関数）の (ファイル, 行, 関数名)。トレースバック用。
fn python_caller_frame(py: Python<'_>) -> Option<(String, usize, String)> {
    let frame = py.import_bound("sys").ok()?.call_method1("_getframe", (0,)).ok()?;
    let code = frame.getattr("f_code").ok()?;
    Some((
        code.getattr("co_filename").ok()?.extract().ok()?,
        frame.getattr("f_lineno").ok()?.extract().ok()?,
        code.getattr("co_name").ok()?.extract().ok()?,
    ))
}

/// Arrow の例外（誤りの文言）を Python の例外にする（2-5）。
/// - Python の物の例外（Python から Arrow を通って戻ってきた）は同じ物を投げ直す
/// - Arrow の組み込みの例外は同じ名前の Python の例外（`args` も渡す）、Arrow で定義した例外と例外にならない誤りは
///   `arrow.ArrowError`（文言は Arrow のトレースバックの最後の行）
/// どちらも [`ARROW_RAISED`] に控え、Arrow へ戻ったら元の例外に戻す。
fn arrow_error_to_py(py: Python<'_>, interp: &mut crate::interpreter::Interpreter, err: String) -> PyErr {
    let raised = if err == crate::interpreter::RAISE_SENTINEL {
        interp.take_current_exception()
    } else {
        interp.make_internal_raised_error(&err)
    };
    let Some(raised) = raised else {
        return stash_arrow_raised(py, ArrowError::type_object_bound(py).as_any(), vec![Value::str(err.as_str())], err, None);
    };
    let (class_name, builtin, args) = match &raised.exception {
        Value::PyObject(h) => return PyErr::from_value_bound(h.bind(py).to_owned()),
        Value::Instance(inst) => {
            let inst = inst.borrow();
            let name = inst.class.name.clone();
            let builtin = crate::interpreter::Interpreter::in_exception_hierarchy(&inst.class)
                && crate::interpreter::built_in_types::builtin_exception_of(&name) == Some(name.as_str());
            let args = crate::interpreter::Interpreter::exc_args_of(&inst);
            (name, builtin, args)
        }
        other => (type_name_of(other).to_string(), false, None),
    };
    let message = interp.display_str(&raised.exception).unwrap_or_default();
    let text = if message.is_empty() { class_name.clone() } else { format!("{class_name}: {message}") };
    let py_builtin = if builtin {
        py.import_bound("builtins").ok().and_then(|b| b.getattr(class_name.as_str()).ok())
    } else {
        None
    };
    match py_builtin {
        Some(ty) => {
            let args = match args {
                Some(Value::Tuple(t)) => t.all_values().to_vec(),
                _ if message.is_empty() => Vec::new(),
                _ => vec![Value::str(message.as_str())],
            };
            stash_arrow_raised(py, &ty, args, text, Some(raised))
        }
        None => stash_arrow_raised(py, ArrowError::type_object_bound(py).as_any(), vec![Value::str(text.as_str())], text, Some(raised)),
    }
}

/// 例外の型 `ty` を `args` で作り、Arrow へ戻ったときのために控える（[`ARROW_RAISED`]）。Python がもう持っていない控え
/// （参照が控えの 1 つだけ）はここで捨てる。
fn stash_arrow_raised(
    py: Python<'_>,
    ty: &Bound<'_, PyAny>,
    args: Vec<Value>,
    text: String,
    raised: Option<crate::interpreter::RaisedError>,
) -> PyErr {
    let py_args: Vec<PyObject> =
        args.iter().map(|a| exception_arg_to_py(py, a).unwrap_or_else(|_| py.None())).collect();
    let exc = match ty.call1(PyTuple::new_bound(py, &py_args)) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let dropped: Vec<_> = ARROW_RAISED.with(|r| {
        let mut r = r.borrow_mut();
        let (keep, gone): (Vec<_>, Vec<_>) =
            r.drain(..).partition(|(o, _, _)| unsafe { ffi::Py_REFCNT(o.as_ptr()) } > 1);
        *r = keep;
        r.push((exc.clone().unbind(), text, raised));
        gone
    });
    drop(dropped);
    PyErr::from_value_bound(exc)
}
