// interpreter/py_interop/collections.rs — `py_list` / `py_dict` / `py_set` の読み出しと、キャスト（`=>`）による変換・数値のバッファの読み出し。

use super::*;

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
pub(super) fn py_type_name_of(obj: &Bound<'_, PyAny>) -> String {
    let ty = obj.get_type();
    let name = ty.qualname().map(|s| s.to_string()).unwrap_or_else(|_| "object".to_string());
    let module = ty.getattr("__module__").ok().and_then(|m| m.extract::<String>().ok()).unwrap_or_default();
    if module == "builtins" || module.is_empty() { name } else { format!("{module}.{name}") }
}
