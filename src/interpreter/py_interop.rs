// py_interop.rs — PyO3 を介した Python ランタイム呼び出し
//
// `import[py]` で読み込んだモジュールの実行時ディスパッチを担当する。
// - `load_py_int_module`: Python モジュールを PyO3 でロードし NamespaceData を構築する
// - `ar_to_py`: tl の Value を PyO3 PyObject に変換する
// - `py_to_tl`: PyO3 Bound<PyAny> を tl の Value に変換する（プリミティブ自動変換）
// - `call_py_object`: Python callable を引数付きで呼び出す
// - `call_py_method`: Python オブジェクトのメソッドを呼び出す
// - `py_getattr`: Python オブジェクトの属性を取得する
// - `py_getitem` / `py_setitem`: Python の __getitem__ / __setitem__
// - `py_len`: Python の __len__
// - `py_get_iter` / `py_iter_next`: `for` の反復子と、そこから 1 つ取り出す（2-4）
// - `py_truthy` / `py_unary`: 真偽と単項演算子（2-2）
// - `py_binop` / `py_rbinop`: Python の二項演算子（lhs=PyObject / rhs=PyObject）
// - `with_py`: CPython へ入る唯一の入口（GIL を取り、参照の増減の待ち行列を反映する・D-5）

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use std::path::PathBuf;

use pyo3::class::basic::CompareOp;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{
    PyAnyMethods, PyByteArray, PyBytes, PyDict, PyDictMethods, PyFloat, PyFrozenSet, PyInt, PyList, PyListMethods,
    PyMemoryView, PyModule, PySet, PyTuple, PyTupleMethods,
};
use pyo3::types::PyString;
use pyo3::Borrowed;

use crate::ast::{BinOp, UnaryOp};

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
    /// [`PENDING`] と入れ替える空の `Vec`（積むたびに確保し直さないように・[`apply_pending`]）。
    static PENDING_SPARE: Cell<Vec<(usize, bool)>> = const { Cell::new(Vec::new()) };
    /// [`call_shaped`] の引数を並べる空の `Vec`（呼び出しのたびに確保し直さないように）。⚠ 入れ子の呼び出しは空の `Vec` を受け取る。
    static ARG_SPARE: Cell<Vec<PyObject>> = const { Cell::new(Vec::new()) };
}

/// GIL を持たずに写した・捨てた `PyObjHandle` の参照の増減（オブジェクトの番地, 増やすなら真）。
/// ⚠ `async` のタスク（別のスレッド）も写すので、プロセスで 1 つ。
static PENDING: Mutex<Vec<(usize, bool)>> = Mutex::new(Vec::new());

/// [`PENDING`] に積まれた分があるか。⚠ `with_py` は越境のたびに通るので、空なら錠を取らない（pandas_import_plan.md の 4-2）。
/// 別のスレッドが積んだばかりの分を見逃しても、次の入口で反映される（積んだスレッド自身の入口では必ず見える）。
static PENDING_ANY: AtomicBool = AtomicBool::new(false);

/// 待ち行列がこれを超えたら GIL を取って反映する（CPython を呼ばないループで写し続けても膨らみ続けないように）。
const PENDING_LIMIT: usize = 1 << 14;

fn gil_held_here() -> bool {
    IN_PY.with(|d| d.get() > 0)
}

fn defer_refcount(ptr: NonNull<ffi::PyObject>, incref: bool) {
    let full = {
        let mut q = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        q.push((ptr.as_ptr() as usize, incref));
        PENDING_ANY.store(true, Ordering::Release);
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
    if !PENDING_ANY.load(Ordering::Acquire) {
        return;
    }
    let mut q = PENDING_SPARE.with(Cell::take);
    {
        // ⚠ 錠は入れ替えるだけで手放す。`Py_DecRef` は Python の終了処理を走らせるので、錠を持ったままだと入れ子の入口で止まる
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::swap(&mut *pending, &mut q);
        PENDING_ANY.store(false, Ordering::Release);
    }
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
    q.clear();
    PENDING_SPARE.with(|s| s.set(q));
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
        STDIO_SETUP.call_once(|| line_buffer_stdio(py));
        apply_pending(py);
        f(py)
    })
}

/// CPython の標準出力の設定を 1 度だけ行ったか（[`line_buffer_stdio`]）。
static STDIO_SETUP: std::sync::Once = std::sync::Once::new();

/// CPython の `sys.stdout` / `sys.stderr` を行ごとに書き出す設定にする（初めて CPython へ入るとき・pandas_import_plan.md 3-1）。
///
/// ⚠⚠ 埋め込みの CPython の `sys.stdout` は、端末でなければ（ファイル・パイプへ出すと）ブロック単位で溜める。Arrow の `print`
///   （Rust の標準出力・行ごと）と出す順が入れ替わり、終わるときに CPython を後始末しないので、溜めた分は**黙って消えた**
///   （Python のコードの `print` は `flush=True` を付けないと出なかった）。Arrow と同じく改行ごとに書き出す。
/// ⚠ 改行で終わらない残り（`print(x, end="")`）は、プログラムの終わりに [`flush_stdio`] が書き出す。
fn line_buffer_stdio(py: Python<'_>) {
    const CODE: &str = "import sys\n\
for _s in (sys.stdout, sys.stderr):\n    \
if _s is not None and hasattr(_s, 'reconfigure'):\n        \
_s.reconfigure(line_buffering=True)\n";
    // ⚠ 設定できなくても（`sys.stdout` を差し替えた環境など）実行は続ける。出す順が CPython の既定のままになるだけ。
    let _ = py.run_bound(CODE, Some(&PyDict::new_bound(py)), None);
}

/// CPython の `sys.stdout` / `sys.stderr` に溜まった分を書き出す（プログラムの終わり・[`line_buffer_stdio`]）。
/// CPython をまだ使っていなければ何もしない（ここで CPython を起動しない）。
pub fn flush_stdio() {
    if !STDIO_SETUP.is_completed() {
        return;
    }
    with_py(|py| {
        let _ = py.run_bound(
            "import sys\nfor _s in (sys.stdout, sys.stderr):\n    if _s is not None:\n        _s.flush()\n",
            Some(&PyDict::new_bound(py)),
            None,
        );
    });
}

/// このスレッドが GIL を持っていれば、手放して `f` を走らせる（`async` のタスクを待つ間・pandas_import_plan.md の 2-8）。
///
/// ⚠⚠ 待つ側が GIL を持ったままだと、Python の物を使うタスクが GIL を取れずに止まる（Python が呼んだ Arrow の関数の中で
///   `mng.wait_for_finish()` すると、タスクの `c.bump()` を待ち続けた・実測）。待つ間だけ CPython の `PyEval_SaveThread` で手放す。
/// ⚠ 手放している間はこのスレッドも GIL を持たないので、写し・捨てた参照の増減は待ち行列へ回す（`IN_PY` を 0 にする）。
pub(crate) fn without_gil<R>(f: impl FnOnce() -> R) -> R {
    let depth = IN_PY.with(|d| d.get());
    if depth == 0 {
        return f();
    }
    struct Restore(*mut ffi::PyThreadState, u32);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe { ffi::PyEval_RestoreThread(self.0) };
            IN_PY.with(|d| d.set(self.1));
        }
    }
    IN_PY.with(|d| d.set(0));
    let _restore = Restore(unsafe { ffi::PyEval_SaveThread() }, depth);
    f()
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
        // ⚠⚠ Arrow の list / dict / set は**キャストせずには渡さない**（pandas_import_plan.md の D-12・2-11）。写して渡すと Python の
        //   書き換えが戻らず、包んで渡すと pandas などが壊れる（6.8）。`=> py_list[T]` で CPython の物にする（`cast_to_python`）。
        //   以前はここで深く写していた
        Value::List(_) | Value::FrozenList { .. } => Err(arrow_collection_to_python("list")),
        Value::Dict(_) => Err(arrow_collection_to_python("dict")),
        Value::Set(_) => Err(arrow_collection_to_python("set")),
        Value::Tuple(td) => {
            let py_items: Vec<PyObject> = td
                .values
                .iter()
                .map(|v| ar_to_py(py, v))
                .collect::<PyResult<_>>()?;
            Ok(PyTuple::new_bound(py, &py_items).into())
        }
        Value::UInt(n) => Ok(n.to_object(py)),
        // bytes → Python の bytes（コピー・ffi_value_exchange_plan.md 5-3）。⚠ 以前は TypeError
        Value::Bytes(b) => Ok(PyBytes::new_bound(py, b).into()),
        Value::PyObject(h) => Ok(h.to_py(py)),
        // スライス → Python の `slice`（境界は何でも・省略は None・pandas_import_plan.md 2-3）。`a[1:, 0]` の添字は
        // スライスを持つタプルなので、上のタプルの腕からここへ来る。
        Value::Slice(s) => {
            let part = |v: &Option<Value>| v.as_ref().map_or_else(|| Ok(py.None()), |v| ar_to_py(py, v));
            let (begin, end, step) = (part(&s.begin)?, part(&s.end)?, part(&s.step)?);
            let raw = unsafe { ffi::PySlice_New(begin.as_ptr(), end.as_ptr(), step.as_ptr()) };
            Ok(unsafe { Bound::from_owned_ptr_or_err(py, raw) }?.unbind())
        }
        // 唯一の値（`...` / `Ellipsis`・`NotImplemented`）は CPython の同じ値（`a[..., 0]`・2-3）
        Value::Instance(inst)
            if matches!(
                inst.borrow().class.builtin,
                crate::interpreter::BuiltinClass::Ellipsis | crate::interpreter::BuiltinClass::NotImplemented
            ) =>
        {
            Ok(if inst.borrow().class.builtin == crate::interpreter::BuiltinClass::Ellipsis {
                py.Ellipsis()
            } else {
                py.NotImplemented()
            })
        }
        // Arrow の関数（`fn`・クロージャ・束縛メソッド・多重定義・ネイティブの関数）は Python から呼べる包み（2-5・2.7）
        Value::Function(f) => wrap_arrow_function(py, val, f.name.clone()),
        Value::OverloadedFn(c) => wrap_arrow_function(py, val, c.first().map(|f| f.name.clone()).unwrap_or_default()),
        Value::NativeFunction(r) => wrap_arrow_function(py, val, r.fn_name.clone()),
        // インスタンスは写さずに包む（読むだけ・書けるのは引数の `mut` の変数などから渡したとき・2-6）。クラス・ジェネレータ関数は
        // 呼べる包み、ジェネレータは Python の反復子の包み
        Value::Instance(_) => wrap_arrow_object(py, val, false),
        Value::Class(c) => wrap_arrow_function(py, val, c.name.clone()),
        Value::GeneratorFn(g) => wrap_arrow_function(py, val, g.name.clone()),
        Value::Generator(_) => wrap_arrow_iterator(py, val),
        other => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "cannot pass an Arrow value of type '{}' to Python",
            type_name_of(other)
        ))),
    }
}

/// Arrow のコレクションをキャストせずに Python へ渡そうとした（[`ar_to_py`]・D-12・2-11）。
fn arrow_collection_to_python(kind: &str) -> PyErr {
    pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot pass an Arrow {kind} to Python as is; convert it with `=> py_{kind}[..]` (Python works on a Python collection; changes to a copy would not come back)"
    ))
}

/// 例外の `args` を CPython の物にする（[`stash_arrow_raised`]）。Arrow の list などは**写す**。
/// ⚠ ここだけ写すのは、例外の `args` が投げたときの値の記録で、Python が書き換えて Arrow へ戻す物ではないから（Arrow へ戻った
///   例外は元の Arrow の例外に戻る・2-5）。ほかの場所で写すのはキャスト（`=> py_list[T]`）だけ（D-12）。
fn exception_arg_to_py(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
    let items = |vs: &[Value]| vs.iter().map(|v| exception_arg_to_py(py, v)).collect::<PyResult<Vec<_>>>();
    match val {
        Value::List(xs) => Ok(PyList::new_bound(py, items(&xs.borrow())?).into_any().unbind()),
        Value::Set(xs) => Ok(PySet::new_bound(py, &items(&xs.borrow())?)?.into_any().unbind()),
        Value::Tuple(td) => Ok(PyTuple::new_bound(py, items(td.all_values())?).into_any().unbind()),
        Value::Dict(d) => {
            let dict = PyDict::new_bound(py);
            for (k, v) in d.borrow().all_pairs() {
                dict.set_item(exception_arg_to_py(py, &k)?, exception_arg_to_py(py, &v)?)?;
            }
            Ok(dict.into_any().unbind())
        }
        other => ar_to_py(py, other),
    }
}

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
    static ARROW_RAISED: RefCell<Vec<(PyObject, String, Option<crate::interpreter::RaisedError>)>> =
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
mod arrow_error {
    pyo3::create_exception!(arrow, ArrowError, pyo3::exceptions::PyException, "An exception raised by Arrow code.");
}
use arrow_error::ArrowError;
use pyo3::PyTypeInfo;

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
fn running_interpreter<'a>(what: &str) -> PyResult<&'a mut crate::interpreter::Interpreter> {
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
fn unwrap_arrow_value(obj: &Bound<'_, PyAny>) -> Option<Value> {
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
fn ar_to_py_arg(py: Python<'_>, val: Value, writable: bool) -> PyResult<PyObject> {
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
fn wrap_arrow_function(py: Python<'_>, val: &Value, name: String) -> PyResult<PyObject> {
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
fn wrap_arrow_object(py: Python<'_>, val: &Value, writable: bool) -> PyResult<PyObject> {
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
    fn target<'a>(&self) -> PyResult<(Value, &'a mut crate::interpreter::Interpreter)> {
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
fn wrap_arrow_iterator(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
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
// Python → tl 変換
// ---------------------------------------------------------------------------

/// 組み込みの型**そのもの**（派生は含まない）の `int` / `float` / `str` / `bool` / `None` を写す（[`py_to_tl`] の先頭）。
/// どれでもなければ `None`。
///
/// ⚠ 要素を 1 つずつ写す所（`=> list[T]`・`for x in pl`）の大半はこれらで、[`py_to_tl`] の下の道は 1 つにつき型の名前の文字列を
///   作り、float / str は整数の取り出しを失敗させて例外の物まで作っていた（pandas_import_plan.md の 4-1）。型のポインタを比べるだけ
///   なら Python のコードは走らないので、下の道と結果は変わらない（派生は `__index__` などを上書きできるので下の道を通す）。
#[inline]
fn py_to_tl_exact(obj: &Bound<'_, PyAny>) -> Option<Value> {
    use std::ptr::addr_of_mut;
    let p = obj.as_ptr();
    // SAFETY: `obj` は GIL の下で生きている物。組み込みの型の静的な型オブジェクトと比べて、中身を読むだけ。
    unsafe {
        let ty = ffi::Py_TYPE(p);
        if ty == addr_of_mut!(ffi::PyLong_Type) {
            let mut overflow = 0;
            let n = ffi::PyLong_AsLongLongAndOverflow(p, &mut overflow);
            // ⚠ i64 に入らない int は Python のオブジェクトのまま（下の道の 2-6 と同じ）
            return Some(if overflow == 0 { Value::Int(n) } else { Value::PyObject(PyObjHandle::from_bound(obj)) });
        }
        if ty == addr_of_mut!(ffi::PyFloat_Type) {
            return Some(Value::Float(ffi::PyFloat_AS_DOUBLE(p)));
        }
        if ty == addr_of_mut!(ffi::PyUnicode_Type) {
            // ⚠ UTF-8 にできない str（孤立サロゲート）は `None` で下の道へ（Python のオブジェクトのまま）
            return obj.downcast::<PyString>().ok()?.to_str().ok().map(Value::str);
        }
        if ty == addr_of_mut!(ffi::PyBool_Type) {
            return Some(Value::Bool(p == ffi::Py_True()));
        }
        if p == ffi::Py_None() {
            return Some(Value::None);
        }
        // tuple → Value::Tuple（不変なので写しても区別がつかない）
        if ty == addr_of_mut!(ffi::PyTuple_Type) {
            let tup = obj.downcast::<PyTuple>().ok()?;
            let values: Vec<Value> = tup.iter().map(|item| py_to_tl(obj.py(), &item)).collect();
            return Some(Value::Tuple(Rc::new(TupleData::new(values))));
        }
    }
    None
}

/// `obj` の型の `__name__` が `name` か（`__name__` を読まずに `tp_name` の最後の部分を比べる。`__name__` を読むと Python の文字列を作る）。
fn type_name_is(obj: &Bound<'_, PyAny>, name: &str) -> bool {
    // SAFETY: `tp_name` は型オブジェクトが持つ NUL 終端の文字列
    let full = unsafe { std::ffi::CStr::from_ptr((*ffi::Py_TYPE(obj.as_ptr())).tp_name) };
    full.to_bytes().rsplit(|&c| c == b'.').next() == Some(name.as_bytes())
}

/// PyO3 の `Bound<'_, PyAny>` を tl の `Value` に変換する。
pub fn py_to_tl(py: Python<'_>, obj: &Bound<'_, PyAny>) -> Value {
    if let Some(v) = py_to_tl_exact(obj) {
        return v;
    }
    // Arrow が渡した包みは元の Arrow の値（同じインスタンス・2-5 / 2-6）。⚠ 先に見る（`__index__` を持つ Arrow のクラスの包みを
    //   下の int の取り出しが int にしてしまう）
    if let Some(v) = unwrap_arrow_value(obj) {
        return v;
    }
    // 唯一の値（`...`・`NotImplemented`）は Arrow の同じ値（`ar_to_py` の逆・pandas_import_plan.md 3-1）。
    // ⚠ 以前は CPython の物のまま届いたので、`pe.pick() === Ellipsis` が偽で、`let` でも受けられなかった。
    for (single, kind) in [
        (py.Ellipsis(), crate::interpreter::BuiltinClass::Ellipsis),
        (py.NotImplemented(), crate::interpreter::BuiltinClass::NotImplemented),
    ] {
        if obj.is(&single) {
            if let Ok(interp) = running_interpreter("Ellipsis") {
                return interp.builtin_singleton(kind);
            }
        }
    }
    // bool は int のサブクラスなので先にチェックする（numpy の `bool` もここ）
    if type_name_is(obj, "bool") {
        if let Ok(b) = obj.extract::<bool>() {
            return Value::Bool(b);
        }
    }
    // ⚠ `__index__` が無い物は整数の取り出しが必ず失敗する。先に見て、失敗のたびに例外の物を作らない（4-1）。
    // ⚠ 長さを持つ物（容器・numpy の `ndarray`。0 次元の配列は `__index__` が成功する）は整数にしない（Python の物のまま）。
    //   型スタブも容器はクラスの型で読む（`py_numeric_class`・pandas_import_plan.md の 6-11）。以前は 0 次元の配列や
    //   `__index__` を持つ容器が黙って int になり、型スタブの型（クラス）と食い違った
    if unsafe { ffi::PyIndex_Check(obj.as_ptr()) } != 0 && !py_type_has_len(obj) {
        if let Ok(n) = obj.extract::<i64>() {
            return Value::Int(n);
        }
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
    if obj.is_instance_of::<PyString>() {
        if let Ok(s) = obj.extract::<String>() {
            return Value::str(s);
        }
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
    // ⚠⚠ list / dict / set は**写さずに CPython の物のまま**（`py_list` / `py_dict` / `py_set`・pandas_import_plan.md の D-8・D-12・
    //   2-7）。Python の書き換えが Arrow から見え、Arrow の書き換えも Python から見える（CPython と同じ）。Arrow の list などにするには
    //   `=> list[T]`。以前はここで Arrow の list / dict / set に写していた（Python の書き換えが戻らない・同じ物でなくなる）。
    if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyDict>() || obj.is_instance_of::<PySet>() {
        return Value::PyObject(PyObjHandle::from_bound(obj));
    }
    // tuple → Value::Tuple（不変なので写しても区別がつかない）
    if let Ok(tup) = obj.downcast::<PyTuple>() {
        let values: Vec<Value> = tup.iter().map(|item| py_to_tl(py, &item)).collect();
        return Value::Tuple(Rc::new(TupleData::new(values)));
    }
    // frozenset → Value::Set（不変なので写す・2-6）。
    // ⚠ Arrow の set に入れられない要素（list / dict / PyObject など）を含むなら、要素を捨てずに丸ごと Python のオブジェクトの
    //   まま渡す。
    if obj.is_instance_of::<PyFrozenSet>() {
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
// `py_list` / `py_dict` / `py_set`（pandas_import_plan.md の D-12・2-7）
// ---------------------------------------------------------------------------
//
// 中身が CPython の list / dict / set の組み込みの型。Arrow の list / dict / set とはキャスト `=>` で写す。写すのは
// **キャスト先の型の深さまで**（`[[1]] => py_list[py_list[int]]` は中も CPython の list・`pl => list[py_list[int]]` は外側だけ）で、
// 写しながら要素を宣言の型と照合する（違えば `TypeError`）。`py_*` から `py_*` へは写さずに照合だけする（同じ物を返す）。

/// CPython の list / dict / set（派生も・`isinstance`）なら `"list"` / `"dict"` / `"set"`。
pub fn py_collection_kind(h: &PyObjHandle) -> Option<&'static str> {
    with_py(|py| collection_kind(&h.bind(py)))
}

fn collection_kind(o: &Bound<'_, PyAny>) -> Option<&'static str> {
    if o.is_instance_of::<PyList>() {
        Some("list")
    } else if o.is_instance_of::<PyDict>() {
        Some("dict")
    } else if o.is_instance_of::<PySet>() {
        Some("set")
    } else {
        None
    }
}

/// CPython の list / set の要素（Arrow の値に変換したもの）。list / set でなければ `None`（境界の照合・`ffi_boundary`）。
pub fn py_collection_items(h: &PyObjHandle) -> Option<Vec<Value>> {
    with_py(|py| {
        let o = h.bind(py);
        if !matches!(collection_kind(&o), Some("list" | "set")) {
            return None;
        }
        Some(o.iter().ok()?.filter_map(|x| x.ok()).map(|x| py_to_tl(py, &x)).collect())
    })
}

/// CPython の dict の鍵と値（Arrow の値に変換したもの）。dict でなければ `None`。
pub fn py_dict_pairs(h: &PyObjHandle) -> Option<Vec<(Value, Value)>> {
    with_py(|py| {
        let o = h.bind(py);
        let d = o.downcast::<PyDict>().ok()?;
        Some(d.iter().map(|(k, v)| (py_to_tl(py, &k), py_to_tl(py, &v))).collect())
    })
}

/// `val => target`（`target` は `py_list[T]` / `py_dict[K, V]` / `py_set[T]`）。Arrow のコレクションは写し、CPython の物は
/// 照合して同じ物を返す。
pub fn cast_to_python(val: &Value, target: &crate::type_check::InferredType) -> Result<Value, String> {
    with_py(|py| {
        let obj = to_python_as(py, val, target).map_err(|e| cast_error(target, e))?;
        Ok(Value::PyObject(PyObjHandle::from_owned(obj)))
    })
}

/// `pl => target`（`pl` は CPython の list / dict / set・`target` は Arrow の `list[T]` / `dict[K, V]` / `set[T]`）。
pub fn cast_from_python(h: &PyObjHandle, target: &crate::type_check::InferredType) -> Result<Value, String> {
    with_py(|py| from_python_as(py, &h.bind(py), target).map_err(|e| cast_error(target, e)))
}

/// キャストの誤り。⚠ `int` に入らない整数は `OverflowError` のまま（ほかの境界と同じ・ffi_value_exchange_plan.md の 7-9）。
fn cast_error(target: &crate::type_check::InferredType, e: String) -> String {
    if e.starts_with("OverflowError:") {
        e
    } else {
        format!("TypeError: cannot cast to '{target}': {e}")
    }
}

/// 照合に通らなかった値の説明（`element of type 'str' where 'int' was expected`）。
fn element_mismatch(got: &str, want: &crate::type_check::InferredType) -> String {
    format!("found an element of type '{got}' where '{want}' was expected")
}

/// Arrow の値 `val` を、型 `ty` の CPython の物にする（`=> py_list[..]` の要素ごとの処理）。
fn to_python_as(py: Python<'_>, val: &Value, ty: &crate::type_check::InferredType) -> Result<PyObject, String> {
    use crate::type_check::InferredType as T;
    let items_of = |v: &Value| -> Option<Vec<Value>> {
        match v {
            Value::List(items) => Some(items.borrow().clone()),
            Value::FrozenList { state, layout } => {
                let st = state.borrow();
                Some((0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect())
            }
            _ => None,
        }
    };
    match (ty, val) {
        // CPython の物は照合だけして同じ物（`py_list[Any]` → `py_list[int]`）
        (T::PyList(_) | T::PyDict(..) | T::PySet(_), Value::PyObject(h)) => {
            let obj = h.bind(py);
            check_python_as(py, &obj, ty)?;
            Ok(obj.to_owned().unbind())
        }
        (T::PyList(e), v) => match items_of(v) {
            Some(items) => {
                let elems: Vec<PyObject> = items.iter().map(|x| to_python_as(py, x, e)).collect::<Result<_, _>>()?;
                Ok(PyList::new_bound(py, &elems).into_any().unbind())
            }
            None => Err(element_mismatch(&type_name_of(v), ty)),
        },
        (T::PySet(e), Value::Set(items)) => {
            let elems: Vec<PyObject> =
                items.borrow().iter().map(|x| to_python_as(py, x, e)).collect::<Result<_, _>>()?;
            Ok(PySet::new_bound(py, &elems).map_err(|e| e.to_string())?.into_any().unbind())
        }
        (T::PyDict(kt, vt), Value::Dict(d)) => {
            let dict = PyDict::new_bound(py);
            for (k, v) in d.borrow().all_pairs() {
                dict.set_item(to_python_as(py, &k, kt)?, to_python_as(py, &v, vt)?).map_err(|e| e.to_string())?;
            }
            Ok(dict.into_any().unbind())
        }
        (T::Tuple(ts), Value::Tuple(td)) if td.all_values().len() == ts.len() => {
            let elems: Vec<PyObject> =
                td.all_values().iter().zip(ts).map(|(x, t)| to_python_as(py, x, t)).collect::<Result<_, _>>()?;
            Ok(PyTuple::new_bound(py, &elems).into_any().unbind())
        }
        (T::Union(ts), v) => ts
            .iter()
            .find_map(|t| to_python_as(py, v, t).ok())
            .ok_or_else(|| element_mismatch(&type_name_of(v), ty)),
        // ⚠ Arrow のコレクションは、キャスト先の型が CPython のコレクションを指すときだけ写す（D-12）。`py_list[Any]` の要素の
        //   Arrow の list を黙って写さない（どの深さまで CPython の物にするかが型に書かれていない）
        (_, Value::List(_) | Value::FrozenList { .. } | Value::Dict(_) | Value::Set(_)) => Err(format!(
            "{} (an Arrow {} can only become a py_list / py_dict / py_set; write it in the element type)",
            element_mismatch(&type_name_of(val), ty),
            type_name_of(val)
        )),
        (_, v) => {
            match crate::interpreter::ffi_boundary::check_common(v, ty) {
                crate::interpreter::ffi_boundary::Verdict::Mismatch { actual } => return Err(element_mismatch(&actual, ty)),
                _ => {}
            }
            ar_to_py(py, v).map_err(|e| e.to_string())
        }
    }
}

/// CPython の物 `obj` を型 `ty` の Arrow の値にする（`=> list[..]` の要素ごとの処理）。`ty` が `py_*` なら照合だけして同じ物。
fn from_python_as(py: Python<'_>, obj: &Bound<'_, PyAny>, ty: &crate::type_check::InferredType) -> Result<Value, String> {
    use crate::type_check::InferredType as T;
    let kind_err = || element_mismatch(&py_type_name_of(obj), ty);
    match ty {
        T::PyList(_) | T::PyDict(..) | T::PySet(_) => {
            check_python_as(py, obj, ty)?;
            Ok(Value::PyObject(PyObjHandle::from_bound(obj)))
        }
        T::ListOf(e) => {
            let Ok(list) = obj.downcast::<PyList>() else {
                // 数値の配列（numpy の配列など）は `a.tolist() => list[T]` と同じ意味（pandas_import_plan.md の 4-3）。⚠ 読めない形
                // （float16・逆のバイト順・`uint64` のあふれ・バッファを持たない pandas の Series）は本当に `tolist()` を呼ぶので、
                // 値と誤りは同じになる
                if let Some(v) = read_numeric_buffer(py, obj, ty) {
                    return Ok(v);
                }
                if !obj.hasattr("tolist").unwrap_or(false) {
                    return Err(kind_err());
                }
                let listed = obj.call_method0("tolist").map_err(|e| e.to_string())?;
                return if listed.is_instance_of::<PyList>() { from_python_as(py, &listed, ty) } else { Err(kind_err()) };
            };
            let mut items: Vec<Value> = Vec::with_capacity(list.len());
            for x in list.iter() {
                items.push(from_python_as(py, &x, e)?);
            }
            Ok(Value::List(Rc::new(RefCell::new(items))))
        }
        T::SetOf(e) => {
            if !obj.is_instance_of::<PySet>() {
                return Err(kind_err());
            }
            let mut items: Vec<Value> = Vec::new();
            for x in obj.iter().map_err(|e| e.to_string())? {
                let v = from_python_as(py, &x.map_err(|e| e.to_string())?, e)?;
                if DictData::reject_key(&v).is_some() {
                    return Err(format!("an element of type '{}' cannot be in an Arrow set", type_name_of(&v)));
                }
                items.push(v);
            }
            Ok(Value::Set(Rc::new(RefCell::new(items))))
        }
        T::DictOf(kt, vt) => {
            let d = obj.downcast::<PyDict>().map_err(|_| kind_err())?;
            let mut dict = DictData::new(kt.to_string(), vt.to_string());
            for (k, v) in d.iter() {
                let (k, v) = (from_python_as(py, &k, kt)?, from_python_as(py, &v, vt)?);
                crate::interpreter::Interpreter::dict_set_pure(&mut dict, k, v)?;
            }
            Ok(Value::Dict(Rc::new(RefCell::new(dict))))
        }
        T::Tuple(ts) => {
            let tup = obj.downcast::<PyTuple>().map_err(|_| kind_err())?;
            if tup.len() != ts.len() {
                return Err(kind_err());
            }
            let values: Vec<Value> = tup.iter().zip(ts).map(|(x, t)| from_python_as(py, &x, t)).collect::<Result<_, _>>()?;
            Ok(Value::Tuple(Rc::new(TupleData::new(values))))
        }
        T::Union(ts) => ts.iter().find_map(|t| from_python_as(py, obj, t).ok()).ok_or_else(kind_err),
        _ => {
            use crate::interpreter::ffi_boundary::Verdict;
            let v = py_to_tl(py, obj);
            match crate::interpreter::ffi_boundary::python_verdict(&v, ty) {
                Verdict::Ok | Verdict::Unverifiable => Ok(v),
                Verdict::Coerce(c) => Ok(c),
                Verdict::Mismatch { actual } => Err(element_mismatch(&actual, ty)),
                // ⚠ 以前は `_ =>` で通していたので、`pl => list[int]` の結果に Python の物（i64 に入らない int）が混ざった
                Verdict::Overflow { value } => Err(format!(
                    "OverflowError: an element read from Python is {value}, which does not fit in `int` (64-bit; declared `{ty}`)"
                )),
            }
        }
    }
}

/// 数値のバッファの要素の種類（[`read_numeric_buffer`]）。
#[derive(Clone, Copy)]
enum BufferElem {
    F64,
    F32,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl BufferElem {
    /// `struct` の書式 1 文字と要素の大きさから。このマシンのバイト順の数・真偽値だけ。ほか（float16 の `e`・複素数・文字列）は `None`。
    /// ⚠ `l` の大きさは環境で違う（numpy の `int32` は Windows で `l`）ので、大きさで決める。
    fn of(format: &[u8], itemsize: ffi::Py_ssize_t) -> Option<Self> {
        let code = match format {
            [c] | [b'@' | b'=', c] => *c,
            [b'<', c] if cfg!(target_endian = "little") => *c,
            [b'>', c] if cfg!(target_endian = "big") => *c,
            _ => return None,
        };
        Some(match (code, itemsize) {
            (b'd', 8) => Self::F64,
            (b'f', 4) => Self::F32,
            (b'?', 1) => Self::Bool,
            (b'b', 1) => Self::I8,
            (b'B', 1) => Self::U8,
            (b'h', 2) => Self::I16,
            (b'H', 2) => Self::U16,
            (b'i' | b'l' | b'q' | b'n', 4) => Self::I32,
            (b'i' | b'l' | b'q' | b'n', 8) => Self::I64,
            (b'I' | b'L' | b'Q' | b'N', 4) => Self::U32,
            (b'I' | b'L' | b'Q' | b'N', 8) => Self::U64,
            _ => return None,
        })
    }

    /// この要素を、宣言の要素の型 `target` の値として読めるか（`tolist()` の要素を照合したときに通る組だけ）。
    /// `float` には整数も入る（PEP 484・`python_verdict`）。⚠ `bool` は `int` にも `float` にも入らない（照合と同じ）。
    fn fits(self, target: &crate::type_check::InferredType) -> bool {
        use crate::type_check::InferredType as T;
        let is_float = matches!(self, Self::F64 | Self::F32);
        let is_bool = matches!(self, Self::Bool);
        match target {
            T::Any => true,
            T::Float => !is_bool,
            T::Int => !is_float && !is_bool,
            T::Bool => is_bool,
            _ => false,
        }
    }
}

/// 数値のバッファを読むのに使う値の作り方。
#[derive(Clone, Copy)]
struct BufferRead {
    elem: BufferElem,
    /// 整数を `float` にする（宣言の要素が `float` のとき）。
    int_as_float: bool,
}

/// `dim` 次元目から下を Arrow のリストの要素にする（`start` はこの次元の先頭）。`i64` に入らない `uint64` があれば `None`。
///
/// # Safety
/// `view` は C の `PyObject_GetBuffer` で取った形（`shape` / `strides` が `ndim` 個）で、`start` はその範囲の中を指すこと。
unsafe fn read_buffer_dim(view: &ffi::Py_buffer, dim: usize, start: *const u8, how: BufferRead) -> Option<Vec<Value>> {
    let n = *view.shape.add(dim);
    let stride = *view.strides.add(dim);
    let mut out = Vec::with_capacity(n as usize);
    if dim + 1 < view.ndim as usize {
        for i in 0..n {
            let row = read_buffer_dim(view, dim + 1, start.offset(i * stride), how)?;
            out.push(Value::List(Rc::new(RefCell::new(row))));
        }
        return Some(out);
    }
    let int = |x: i64| if how.int_as_float { Value::Float(x as f64) } else { Value::Int(x) };
    macro_rules! each {
        ($t:ty, $make:expr) => {
            for i in 0..n {
                // ⚠ `read_unaligned`: 構造化配列の欄などは要素の大きさに揃っていない。⚠ `stride` は負にもなる（`a[::-1]`）
                let x = start.offset(i * stride).cast::<$t>().read_unaligned();
                out.push($make(x));
            }
        };
    }
    match how.elem {
        BufferElem::F64 => each!(f64, Value::Float),
        BufferElem::F32 => each!(f32, |x: f32| Value::Float(x as f64)),
        BufferElem::Bool => each!(u8, |x: u8| Value::Bool(x != 0)),
        BufferElem::I8 => each!(i8, |x: i8| int(x as i64)),
        BufferElem::I16 => each!(i16, |x: i16| int(x as i64)),
        BufferElem::I32 => each!(i32, |x: i32| int(x as i64)),
        BufferElem::I64 => each!(i64, int),
        BufferElem::U8 => each!(u8, |x: u8| int(x as i64)),
        BufferElem::U16 => each!(u16, |x: u16| int(x as i64)),
        BufferElem::U32 => each!(u32, |x: u32| int(x as i64)),
        BufferElem::U64 => {
            for i in 0..n {
                let x = start.offset(i * stride).cast::<u64>().read_unaligned();
                out.push(int(i64::try_from(x).ok()?));
            }
        }
    }
    Some(out)
}

/// `obj` の数値のバッファ（numpy の配列など）を、型 `ty`（`list[float]`・`list[list[int]]` など）の Arrow のリストにする。CPython の物を
/// 1 つも作らずに読む（pandas_import_plan.md の 4-3）。読めない形なら `None`（呼び出し側が `tolist()` の道へ回す）。
///
/// ⚠ `tolist()` を照合したときに通る組だけを読む（[`BufferElem::fits`]）。通らない組はここで誤りにせず `None` を返し、同じ誤りを `tolist()` の
///   道に出させる。⚠ 次元の数は `ty` のリストの深さと同じでなければ読まない。
fn read_numeric_buffer(py: Python<'_>, obj: &Bound<'_, PyAny>, ty: &crate::type_check::InferredType) -> Option<Value> {
    use crate::type_check::InferredType as T;
    /// ⚠ `Py_buffer` は動かせない（中のポインタが自分の欄を指すことがある）ので、その場に置いて指す。
    struct Release(*mut ffi::Py_buffer);
    impl Drop for Release {
        fn drop(&mut self) {
            // SAFETY: `PyObject_GetBuffer` が成功した物を、GIL の下で 1 度だけ手放す
            unsafe { ffi::PyBuffer_Release(self.0) }
        }
    }
    let mut leaf = ty;
    let mut depth = 0;
    while let T::ListOf(e) = leaf {
        leaf = e;
        depth += 1;
    }
    let mut view = std::mem::MaybeUninit::<ffi::Py_buffer>::uninit();
    // SAFETY: GIL の下で、`view` は成功したときだけ初期化されて読まれる。読む間は Python のコードが走らないので、バッファは変わらない
    unsafe {
        if ffi::PyObject_CheckBuffer(obj.as_ptr()) == 0 {
            return None;
        }
        if ffi::PyObject_GetBuffer(obj.as_ptr(), view.as_mut_ptr(), ffi::PyBUF_FORMAT | ffi::PyBUF_STRIDES) != 0 {
            let _ = PyErr::take(py);
            return None;
        }
        let _release = Release(view.as_mut_ptr());
        let v = &*view.as_ptr();
        if v.format.is_null() || v.shape.is_null() || v.strides.is_null() || v.ndim < 1 || v.ndim as usize != depth {
            return None;
        }
        let elem = BufferElem::of(std::ffi::CStr::from_ptr(v.format).to_bytes(), v.itemsize)?;
        if !elem.fits(leaf) {
            return None;
        }
        let how = BufferRead { elem, int_as_float: *leaf == T::Float };
        let items = read_buffer_dim(v, 0, v.buf as *const u8, how)?;
        Some(Value::List(Rc::new(RefCell::new(items))))
    }
}

/// CPython の物 `obj` が型 `ty`（`py_list[T]` など）に合うか、要素まで照合する（写さない）。
fn check_python_as(py: Python<'_>, obj: &Bound<'_, PyAny>, ty: &crate::type_check::InferredType) -> Result<(), String> {
    use crate::type_check::InferredType as T;
    let want = match ty {
        T::PyList(_) => "list",
        T::PyDict(..) => "dict",
        T::PySet(_) => "set",
        _ => return from_python_as(py, obj, ty).map(|_| ()),
    };
    if collection_kind(obj) != Some(want) {
        return Err(element_mismatch(&py_type_name_of(obj), ty));
    }
    match ty {
        T::PyList(e) | T::PySet(e) if **e != T::Unresolved => {
            for x in obj.iter().map_err(|e| e.to_string())? {
                check_python_as(py, &x.map_err(|e| e.to_string())?, e)?;
            }
            Ok(())
        }
        T::PyDict(kt, vt) if **kt != T::Unresolved => {
            for (k, v) in obj.downcast::<PyDict>().map_err(|e| e.to_string())?.iter() {
                check_python_as(py, &k, kt)?;
                check_python_as(py, &v, vt)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// CPython の物の型の名前（組み込みの型はモジュールを付けない）。
fn py_type_name_of(obj: &Bound<'_, PyAny>) -> String {
    let ty = obj.get_type();
    let name = ty.qualname().map(|s| s.to_string()).unwrap_or_else(|_| "object".to_string());
    let module = ty.getattr("__module__").ok().and_then(|m| m.extract::<String>().ok()).unwrap_or_default();
    if module == "builtins" || module.is_empty() { name } else { format!("{module}.{name}") }
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

/// Python が投げた例外（pandas_import_plan.md の 2-1）。文言は CPython のトレースバックの最後の行と同じ
/// （`ValueError: bad`・Python で定義したクラスは `pandas.errors.ParserError: bad`）。
///
/// 例外のオブジェクトは [`RAISED`] に控え、`try` が捕まえるときに [`take_python_exception`] が Arrow の例外の値にする:
/// - 組み込みの例外（`__module__` が `builtins` で、Arrow が同じ名前の例外を持つ）は**同じ名前の Arrow の例外**。`args` は
///   Python の `args` を Arrow の値にしたもの（`KeyError` のキー・`OSError` の `errno` … も CPython と同じ）
/// - Python で定義した例外（と Arrow の持たない組み込みの例外）は **CPython のオブジェクトのまま**。`except m.Err:` は
///   CPython の `isinstance` で、`except ValueError:` / `except Exception:` は CPython の組み込みのクラスとの `isinstance`
///   で捕まる（`Interpreter::vm_exc_matches`）。属性（`e.args`・利用者の欄）は CPython から読む
///
/// ⚠ 以前（7-24）は C# / JS と同じく `ForeignError: ValueError: bad` で、`except ValueError:` で捕まらなかった。
fn python_raised(e: PyErr) -> String {
    with_py(|py| {
        let value = e.value_bound(py).clone();
        // Arrow の関数（2-5）が投げた例外が Python の中を通って戻ってきた: 元の Arrow の例外（文言も元のまま）
        let back = ARROW_RAISED.with(|r| {
            let mut r = r.borrow_mut();
            let pos = r.iter().position(|(o, _, _)| o.as_ptr() == value.as_ptr());
            pos.map(|i| r.remove(i))
        });
        if let Some((_, text, raised)) = back {
            if let Some(raised) = raised {
                RAISED.with(|r| *r.borrow_mut() = Some((text.clone(), PyRaised::Arrow(raised))));
            }
            return text;
        }
        let text = exception_line(&value);
        let ty = value.get_type();
        let name = ty.qualname().map(|s| s.to_string()).unwrap_or_default();
        let module = ty.getattr("__module__").ok().and_then(|m| m.extract::<String>().ok()).unwrap_or_default();
        let builtin = module == "builtins"
            && crate::interpreter::built_in_types::builtin_exception_of(&name) == Some(name.as_str());
        let raised = if builtin {
            let args = value
                .getattr("args")
                .ok()
                .and_then(|a| a.iter().ok().map(|it| it.filter_map(|x| x.ok()).map(|x| py_to_tl(py, &x)).collect()))
                .unwrap_or_default();
            PyRaised::Builtin(args)
        } else {
            PyRaised::Object(PyObjHandle::from_bound(&value))
        };
        RAISED.with(|r| *r.borrow_mut() = Some((text.clone(), raised)));
        text
    })
}

/// 例外のオブジェクトの、CPython のトレースバックの最後の行（組み込みのクラスはモジュールを付けない・`str(e)` が空なら名前だけ）。
fn exception_line(value: &Bound<'_, PyAny>) -> String {
    let ty = value.get_type();
    let name = ty.qualname().map(|s| s.to_string()).unwrap_or_else(|_| "Exception".to_string());
    let module = ty.getattr("__module__").ok().and_then(|m| m.extract::<String>().ok()).unwrap_or_default();
    let message = value.str().map(|s| s.to_string()).unwrap_or_default();
    let shown = if module == "builtins" || module.is_empty() { name } else { format!("{module}.{name}") };
    if message.is_empty() { shown } else { format!("{shown}: {message}") }
}

/// Python が投げた例外を Arrow の例外の値にするための控え（[`python_raised`]）。
pub enum PyRaised {
    /// 組み込みの例外: 同じ名前の Arrow の例外にする。中身は `args`（Arrow の値）
    Builtin(Vec<Value>),
    /// Python で定義した例外: CPython のオブジェクトのまま
    Object(PyObjHandle),
    /// Arrow の関数（2-5）が投げ、Python の中を通って戻ってきた Arrow の例外: 元の値のまま
    Arrow(crate::interpreter::RaisedError),
}

thread_local! {
    /// 最後に Python が投げた例外（文言, 控え）。⚠ 文言が一致するときだけ取り出す（別の誤りの控えを拾わない・
    /// `Interpreter::error_args` と同じ）。
    static RAISED: RefCell<Option<(String, PyRaised)>> = const { RefCell::new(None) };
}

/// 文言 `msg` の誤りが、Arrow の関数（2-5）が投げて Python の中を通って戻ってきた Arrow の例外なら、元の例外を取り出す。
/// 最上位の文で捕まえられなかったときの報告に使う（`Interpreter::uncaught_error_report`）。
pub fn take_arrow_exception(msg: &str) -> Option<crate::interpreter::RaisedError> {
    RAISED.with(|r| {
        let mut r = r.borrow_mut();
        match r.as_ref() {
            Some((text, PyRaised::Arrow(_))) if text == msg => match r.take() {
                Some((_, PyRaised::Arrow(raised))) => Some(raised),
                _ => None,
            },
            _ => None,
        }
    })
}

/// 文言 `msg` の誤りが Python の投げた例外なら、その控え（[`python_raised`]）を取り出す。
/// `Interpreter::make_internal_raised_error` が `try` で捕まえるときに呼ぶ。
pub fn take_python_exception(msg: &str) -> Option<PyRaised> {
    RAISED.with(|r| {
        let mut r = r.borrow_mut();
        match r.as_ref() {
            Some((text, _)) if text == msg => r.take().map(|(_, raised)| raised),
            _ => None,
        }
    })
}

/// CPython の例外のオブジェクト `h` が、組み込みの例外のクラス `type_name`（`ValueError` / `Exception` …）の
/// インスタンスか（CPython の `isinstance`・`except ValueError:` で Python で定義した例外を捕まえる・2-1）。
/// `type_name` が CPython の組み込みの例外でなければ偽（Arrow で定義したクラスは CPython の物と関係が無い）。
pub fn py_exception_is(h: &PyObjHandle, type_name: &str) -> bool {
    with_py(|py| {
        let Ok(builtins) = py.import_bound("builtins") else { return false };
        let Ok(cls) = builtins.getattr(type_name) else { return false };
        let Ok(cls) = cls.downcast_into::<pyo3::types::PyType>() else { return false };
        cls.is_subclass_of::<pyo3::exceptions::PyBaseException>().unwrap_or(false)
            && h.bind(py).is_instance(&cls).unwrap_or(false)
    })
}

/// CPython の物の型の名前（`frames.Frame`・組み込みの型はモジュールを付けない・誤りの文言用・D-11）。
pub fn py_type_name(h: &PyObjHandle) -> String {
    with_py(|py| py_type_name_of(&h.bind(py)))
}

/// CPython の例外のオブジェクトの、トレースバックの最後の行（`pkg.MyErr: bad`・[`python_raised`] と同じ形）。
pub fn py_exception_line(h: &PyObjHandle) -> String {
    with_py(|py| exception_line(&h.bind(py).to_owned()))
}

/// Python callable を引数付きで呼び出す。
///
/// ⚠ 呼ぶ物と引数は**受け取って手放す**（写さない）。CPython のバイトコードと同じく、評価のスタックの参照（Arrow の値の写し）を
///   呼ばれた側へ引き渡すため（[`call_shaped`]・D-5）。
pub fn call_py_object(
    handle: PyObjHandle,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    call_shaped(CallHead::Owned(handle), None, evaled)
}

/// Python オブジェクトのメソッドを呼び出す（受け手と引数の扱いは [`call_py_object`] と同じ）。
pub fn call_py_method(
    handle: PyObjHandle,
    method_name: &str,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    call_shaped(CallHead::Owned(handle), Some(method_name), evaled)
}

/// Python のモジュールの属性を読んで呼ぶ（`m.f(..)`）。受け手のモジュールは借りる（呼び出しの間だけ参照を 1 つ持つ）。
pub fn call_py_module_fn(
    module: &PyObjHandle,
    method_name: &str,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    call_shaped(CallHead::Borrowed(module), Some(method_name), evaled)
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

/// 位置引数だけの肩代わり: メソッド名（呼ぶ物そのものなら `""`）→ 位置引数の数 → 肩代わり。
/// ⚠ 呼び出しのたびに引くので、[`ShapeKey`] を作らずに `&str` で引ける形にしてある。ここにあれば名前はソースに書けると確かめ済み。
type PositionalShapes = HashMap<String, Vec<Option<Py<PyAny>>>>;
static POSITIONAL_TRAMPOLINES: OnceLock<Mutex<PositionalShapes>> = OnceLock::new();

fn positional_trampoline<'py>(py: Python<'py>, method: Option<&str>, n_pos: usize) -> Option<Bound<'py, PyAny>> {
    let cache = POSITIONAL_TRAMPOLINES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.get(method.unwrap_or(""))?.get(n_pos)?.as_ref().map(|f| f.bind(py).clone())
}

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
    if kw_names.is_empty() {
        let mut positional = POSITIONAL_TRAMPOLINES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        let by_count = positional.entry(method.clone().unwrap_or_default()).or_default();
        if by_count.len() <= *n_pos {
            by_count.resize_with(*n_pos + 1, || None);
        }
        by_count[*n_pos] = Some(f.clone().unbind());
    }
    Ok(f)
}

/// 呼ぶ物・受け手の持ち方（[`call_shaped`]）。
enum CallHead<'a> {
    /// 自分の参照を手放して渡す（Arrow の値の写しの受け手・D-5）。
    Owned(PyObjHandle),
    /// 借りている物（モジュール）。呼び出しの間だけ参照を 1 つ持つ。
    /// ⚠ 写してから渡すと、GIL の外で写すので待ち行列を通る（呼び出しのたびに錠を取る・pandas_import_plan.md の 4-2）。
    Borrowed(&'a PyObjHandle),
}

impl CallHead<'_> {
    fn into_py(self, py: Python<'_>) -> PyObject {
        match self {
            CallHead::Owned(h) => h.into_py(py),
            CallHead::Borrowed(h) => h.to_py(py),
        }
    }
}

/// `head`（呼ぶ物、または `method` の受け手）を呼ぶ。引数の参照も受け取って手放す。
fn call_shaped(
    head: CallHead<'_>,
    method: Option<&str>,
    evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        // 位置引数だけで、同じ形を呼んだことがあれば、名前の検査も形の鍵も要らない（呼び出しのたびに通る・4-2）
        if evaled.iter().all(|(name, ..)| name.is_none()) {
            if let Some(f) = positional_trampoline(py, method, evaled.len()) {
                let n = evaled.len();
                return call_trampoline(py, f, head, evaled.into_iter().map(|(_, v, writable)| (v, writable)), n);
            }
        }
        // 3 つ目は引数が `mut` の変数・式か（Arrow のインスタンスを書ける包みにするか・2-6）
        let mut pos: Vec<(Value, bool)> = Vec::new();
        let mut kw: Vec<(String, Value, bool)> = Vec::new();
        for (name, val, writable) in evaled {
            match name {
                None => pos.push((val, writable)),
                Some(k) => kw.push((k, val, writable)),
            }
        }
        let kw_names: Vec<String> = kw.iter().map(|(k, _, _)| k.clone()).collect();
        let unique = kw_names.iter().enumerate().all(|(i, k)| !kw_names[..i].contains(k));
        let shapeable = method.map_or(true, is_py_identifier) && kw_names.iter().all(|k| is_py_identifier(k)) && unique;
        if !shapeable {
            // ⚠ ソースに書けない名前（`getattr` でしか引けないメソッド名・予約語のキーワード）だけは、肩代わりを使わずに呼ぶ。
            //   このときだけ、受け手・引数の参照は CPython で書いたときより 1 つ多い。
            return call_unshaped(py, head.into_py(py), method, pos, kw);
        }
        let f = trampoline(py, &(method.map(str::to_string), pos.len(), kw_names)).map_err(arrow_side)?;
        let n = pos.len() + kw.len();
        call_trampoline(py, f, head, pos.into_iter().chain(kw.into_iter().map(|(_, v, writable)| (v, writable))), n)
    })
}

/// 肩代わり `f` を呼ぶ。`args` は位置引数・キーワード引数の順の `n` 個の（値, 書ける包みにするか）。
fn call_trampoline<'py>(
    py: Python<'py>,
    f: Bound<'py, PyAny>,
    head: CallHead<'_>,
    args: impl Iterator<Item = (Value, bool)>,
    n: usize,
) -> Result<Value, String> {
    // ⚠ 全部を CPython の物にしてから中身の入ったリストを作る（`PyList_New` の空の欄は、Python のコードが走る前に埋める約束）
    let mut owned = ARG_SPARE.with(Cell::take);
    owned.reserve(n + 1);
    owned.push(head.into_py(py));
    for (v, writable) in args {
        owned.push(ar_to_py_arg(py, v, writable).map_err(arrow_side)?);
    }
    let list = unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyList_New(owned.len() as ffi::Py_ssize_t)) }
        .map_err(arrow_side)?;
    // 肩代わりは後ろから `pop` するので、[頭, 位置引数.., キーワード..] の逆順に並べる。
    // ⚠ 要素は `PyList_SET_ITEM` で**引き渡す**（`PyList::new_bound` は写すので、作る間だけ参照が 1 つ多くなる）。
    let last = owned.len() - 1;
    for (i, o) in owned.drain(..).enumerate() {
        unsafe { ffi::PyList_SET_ITEM(list.as_ptr(), (last - i) as ffi::Py_ssize_t, o.into_ptr()) };
    }
    ARG_SPARE.with(|s| s.set(owned));
    // ⚠ 引数 1 つの呼び出しは組を作らない（`PyObject_CallOneArg`）
    let result = unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyObject_CallOneArg(f.as_ptr(), list.as_ptr())) }
        .map_err(python_raised)?;
    Ok(py_to_tl(py, &result))
}

/// 肩代わりを使わない呼び出し（[`call_shaped`] のソースに名前を書けないときだけ）。
fn call_unshaped(
    py: Python<'_>,
    head: PyObject,
    method: Option<&str>,
    pos: Vec<(Value, bool)>,
    kw: Vec<(String, Value, bool)>,
) -> Result<Value, String> {
    let callable = match method {
        Some(m) => head.bind(py).getattr(m).map_err(python_raised)?,
        None => head.into_bound(py),
    };
    let mut positional: Vec<PyObject> = Vec::with_capacity(pos.len());
    for (v, writable) in pos {
        positional.push(ar_to_py_arg(py, v, writable).map_err(arrow_side)?);
    }
    let kwargs = PyDict::new_bound(py);
    for (k, v, writable) in kw {
        kwargs.set_item(k, ar_to_py_arg(py, v, writable).map_err(arrow_side)?).map_err(arrow_side)?;
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

/// `obj.attr = val`: CPython の `setattr`（`PyObject_SetAttr`・pandas_import_plan.md の 2-9）。モジュールの大域への代入
/// （`gm.count = 5`）もここ（Python ではモジュールの大域は外から書き換えられる）。
///
/// ⚠ 受け手と値は借りるだけ（[`py_setitem`] と同じ。PyO3 の `setattr` は値を写す）。CPython の `o.x = v` と同じ参照の数で
///   `__setattr__` / データディスクリプタの `__set__`（pandas の `df.columns = ..`）が走る（D-5）。
pub fn py_setattr(handle: &PyObjHandle, attr: &str, val: &Value) -> Result<(), String> {
    with_py(|py| -> Result<(), String> {
        let obj = handle.bind(py);
        let name = pyo3::types::PyString::new_bound(py, attr);
        let py_val = py_operand(py, val).map_err(arrow_side)?;
        if unsafe { ffi::PyObject_SetAttr(obj.as_ptr(), name.as_ptr(), py_val.as_ptr()) } == -1 {
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

/// `if x:` / `while` / `not x` / `and` / `or` / `bool(x)` の真偽: CPython の `PyObject_IsTrue`（`__bool__`、無ければ
/// `__len__`、どちらも無ければ真・pandas_import_plan.md 2-2）。`__bool__` が投げた例外（numpy の配列の「真偽が曖昧」・
/// `__bool__` が bool 以外を返したときの `TypeError` など）は Python の例外として届く（2-1）。
pub fn py_truthy(handle: &PyObjHandle) -> Result<bool, String> {
    with_py(|py| match unsafe { ffi::PyObject_IsTrue(handle.bind(py).as_ptr()) } {
        -1 => Err(python_raised(PyErr::fetch(py))),
        r => Ok(r != 0),
    })
}

/// 単項演算子: `-x` は `PyNumber_Negative`、`~x` は `PyNumber_Invert`（`__neg__` / `__invert__`・numpy の配列や pandas の
/// 列にも効く）、`not x` は [`py_truthy`] の否定（2-2）。
pub fn py_unary(handle: &PyObjHandle, op: &UnaryOp) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
        let raw = match op {
            UnaryOp::Neg => unsafe { ffi::PyNumber_Negative(obj.as_ptr()) },
            UnaryOp::BitNot => unsafe { ffi::PyNumber_Invert(obj.as_ptr()) },
            UnaryOp::Not => return Ok(Value::Bool(!py_truthy(handle)?)),
        };
        let result = unsafe { Bound::from_owned_ptr_or_err(py, raw) }.map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// `callable(obj)`: Python の `callable()` と同じ判定（python_builtins_plan.md のタスク 2-4）。
pub fn py_is_callable(handle: &PyObjHandle) -> bool {
    with_py(|py| handle.bind(py).is_callable())
}

/// `for x in obj` の反復子: CPython の `iter(obj)`（`PyObject_GetIter`・pandas_import_plan.md 2-4）。回せない物は
/// CPython の `TypeError`（`'X' object is not iterable`）。
pub fn py_get_iter(handle: &PyObjHandle) -> Result<PyObjHandle, String> {
    with_py(|py| -> Result<PyObjHandle, String> {
        let it = handle.bind(py).iter().map_err(python_raised)?;
        Ok(PyObjHandle::from_owned(it.into_any().unbind()))
    })
}

/// 反復子から 1 つ取り出す（`PyIter_Next`・2-4）。尽きたら `None`。`__next__` が投げた例外は Python の例外（2-1）。
/// ⚠ 1 つごとに CPython へ入る（GIL を取る）。まとめないのは、取り出しの間に Arrow の本体が走るから（CPython の `for` と
///   同じ順で副作用が起きる）。
pub fn py_iter_next(handle: &PyObjHandle) -> Result<Option<Value>, String> {
    with_py(|py| -> Result<Option<Value>, String> {
        let raw = unsafe { ffi::PyIter_Next(handle.bind(py).as_ptr()) };
        if raw.is_null() {
            return match PyErr::take(py) {
                Some(e) => Err(python_raised(e)),
                None => Ok(None),
            };
        }
        let item = unsafe { Bound::from_owned_ptr(py, raw) };
        Ok(Some(py_to_tl(py, &item)))
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
            BinOp::Eq => obj.rich_compare(py_rhs, CompareOp::Eq),
            // `===`（Python の `is`）は同一性で、`__eq__` を呼ばない（D-4・5-1）。⚠ `apply_binop` が先に
            // `values_ref_eq` で受ける（Arrow の値を Python へ写す前に）ので、ここは直接の呼び手のため。
            BinOp::RefEq => {
                return Ok(Value::Bool(matches!(rhs, Value::PyObject(r) if r.identity() == handle.identity())))
            }
            BinOp::NotEq => obj.rich_compare(py_rhs, CompareOp::Ne),
            BinOp::Lt => obj.rich_compare(py_rhs, CompareOp::Lt),
            BinOp::Gt => obj.rich_compare(py_rhs, CompareOp::Gt),
            BinOp::LtEq => obj.rich_compare(py_rhs, CompareOp::Le),
            BinOp::GtEq => obj.rich_compare(py_rhs, CompareOp::Ge),
            BinOp::And | BinOp::Or => return Err(NO_LOGICAL_BINOP.to_string()),
            BinOp::In | BinOp::NotIn => {
                // obj (LHS item) in py_rhs (RHS container)。CPython の `in`（`PySequence_Contains`: `__contains__`、
                // 無ければ反復）。
                let b = py_rhs.bind(py).contains(obj).map_err(python_raised)?;
                return Ok(Value::Bool(if matches!(op, BinOp::In) { b } else { !b }));
            }
        }
        .map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// rhs が `PyObject` のときの二項演算。lhs を Python へ写し、**左を受け手にして** CPython の演算子の手順に任せる
/// （`PyNumber_Add` / `PyObject_RichCompare`: 左の `__add__` → `NotImplemented` なら右の `__radd__` → どちらも
/// `NotImplemented` なら `TypeError`）。
/// ⚠ 以前は右の反射メソッド（`__radd__`）を直接呼んだので、`NotImplemented` が値として出てきて、反射メソッドの
///   無い物は `AttributeError` になった（CPython はどちらも `TypeError: unsupported operand type(s)`）。比較は左右を
///   入れ替えて右の `__gt__` などを先に呼んでいた（known_bugs.md の #8 の ②）。
pub fn py_rbinop(handle: &PyObjHandle, op: &BinOp, lhs: &Value) -> Result<Value, String> {
    with_py(|py| -> Result<Value, String> {
        let obj = handle.bind(py);
        let py_lhs = ar_to_py(py, lhs).map_err(arrow_side)?;
        let l = py_lhs.bind(py);
        let result = match op {
            BinOp::Add => l.add(obj),
            BinOp::Sub => l.sub(obj),
            BinOp::Mul => l.mul(obj),
            BinOp::Div => l.div(obj),
            BinOp::FloorDiv => l.floor_div(obj),
            BinOp::Mod => l.rem(obj),
            BinOp::Pow => l.pow(obj, py.None()),
            BinOp::LShift => l.lshift(obj),
            BinOp::RShift => l.rshift(obj),
            BinOp::BitAnd => l.bitand(obj),
            BinOp::BitOr => l.bitor(obj),
            BinOp::BitXor => l.bitxor(obj),
            BinOp::Eq => l.rich_compare(obj, CompareOp::Eq),
            // `===` は同一性（`py_binop` と同じ・D-4・5-1）
            BinOp::RefEq => {
                return Ok(Value::Bool(matches!(lhs, Value::PyObject(l) if l.identity() == handle.identity())))
            }
            BinOp::NotEq => l.rich_compare(obj, CompareOp::Ne),
            BinOp::Lt => l.rich_compare(obj, CompareOp::Lt),
            BinOp::Gt => l.rich_compare(obj, CompareOp::Gt),
            BinOp::LtEq => l.rich_compare(obj, CompareOp::Le),
            BinOp::GtEq => l.rich_compare(obj, CompareOp::Ge),
            BinOp::And | BinOp::Or => return Err(NO_LOGICAL_BINOP.to_string()),
            BinOp::In | BinOp::NotIn => {
                // py_lhs (item) in obj/handle (RHS container)。CPython の `in`（`py_binop` と同じ）。
                let b = obj.contains(l).map_err(python_raised)?;
                return Ok(Value::Bool(if matches!(op, BinOp::In) { b } else { !b }));
            }
        }
        .map_err(python_raised)?;
        Ok(py_to_tl(py, &result))
    })
}

/// CPython の物が名前 `attr` の属性を持つか（protocol の型テスト `x is P`・pandas_import_plan.md の 6-6）。CPython 3.12 の
/// `isinstance(x, Protocol)` と同じく `inspect.getattr_static` で引くので、利用者のコード（`__getattr__` / 記述子）を走らせない。
/// ⚠ `getattr_static` が例外を投げるのは CPython の内部の失敗だけ（属性が無ければ番兵を返す）。そのときは持たないとする
///   （型テストは真偽を返すだけで例外を伝えられない）。
pub fn py_has_static_attr(handle: &PyObjHandle, attr: &str) -> bool {
    with_py(|py| {
        let lookup = || -> PyResult<bool> {
            let getattr_static = py.import_bound("inspect")?.getattr("getattr_static")?;
            let missing = py.import_bound("builtins")?.getattr("object")?.call0()?;
            let found = getattr_static.call1((handle.bind(py), attr, &missing))?;
            Ok(!found.is(&missing))
        };
        lookup().unwrap_or(false)
    })
}

/// 型が長さ（`__len__`・`sq_length` / `mp_length`）を持つか（`py_to_tl` の `__index__` の写し方・6-11）。型の欄を見るだけで
/// 呼び出しはしない。
fn py_type_has_len(obj: &Bound<'_, PyAny>) -> bool {
    unsafe {
        let tp = ffi::Py_TYPE(obj.as_ptr());
        let seq = (*tp).tp_as_sequence;
        let map = (*tp).tp_as_mapping;
        (!seq.is_null() && (*seq).sq_length.is_some()) || (!map.is_null() && (*map).mp_length.is_some())
    }
}

/// CPython の `hash(obj)`（辞書のキー・集合の要素・known_bugs.md の #17）。ハッシュにできない物（`py_list` など）は CPython と同じ
/// `TypeError: unhashable type: ..`。
pub fn py_hash(handle: &PyObjHandle) -> Result<i64, String> {
    with_py(|py| handle.bind(py).hash().map(|n| n as i64).map_err(python_raised))
}

/// 片方が CPython の物の `a == b` の真偽（CPython の `PyObject_RichCompareBool`: 同じ物なら真、ほかは `__eq__` の
/// 答えの真偽）。容器の要素との比較（`pyobj in xs`・`xs.index(pyobj)`）が使う（`Interpreter::values_eq_expr`・
/// known_bugs.md の #8）。Python へ渡せない Arrow の値（容器）は `a == b` と同じ `TypeError`。
pub fn py_eq_bool(a: &Value, b: &Value) -> Result<bool, String> {
    with_py(|py| -> Result<bool, String> {
        let l = ar_to_py(py, a).map_err(arrow_side)?;
        let r = ar_to_py(py, b).map_err(arrow_side)?;
        let res = unsafe { ffi::PyObject_RichCompareBool(l.as_ptr(), r.as_ptr(), ffi::Py_EQ) };
        if res < 0 {
            return Err(python_raised(PyErr::fetch(py)));
        }
        Ok(res == 1)
    })
}

// ---------------------------------------------------------------------------
// 内部ヘルパー
// ---------------------------------------------------------------------------

/// Value のランタイム型名を返す（エラーメッセージ・タプルの要素型名用・[`element_type_name`]）。
fn type_name_of(val: &Value) -> String {
    crate::interpreter::ops::typecheck::element_type_name(val)
}
