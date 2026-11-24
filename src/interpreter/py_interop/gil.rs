// interpreter/py_interop/gil.rs — CPython へ入る入口（`with_py`）と、CPython の物の参照の増減。

use super::*;

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
    pub(super) static ARG_SPARE: Cell<Vec<PyObject>> = const { Cell::new(Vec::new()) };
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
