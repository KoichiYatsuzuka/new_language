// interpreter/py_interop/calls.rs — Python の callable の呼び出し（肩代わりの関数・引数の組み立て）。

use super::*;

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
    pub(super) fn into_py(self, py: Python<'_>) -> PyObject {
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
