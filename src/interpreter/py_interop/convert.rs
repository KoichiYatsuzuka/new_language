// interpreter/py_interop/convert.rs — Arrow の値と Python の物の変換（`ar_to_py` / `py_to_tl`）。

use super::*;

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
pub(super) fn exception_arg_to_py(py: Python<'_>, val: &Value) -> PyResult<PyObject> {
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
// 内部ヘルパー
// ---------------------------------------------------------------------------

/// Value のランタイム型名を返す（エラーメッセージ・タプルの要素型名用・[`element_type_name`]）。
pub(super) fn type_name_of(val: &Value) -> String {
    crate::interpreter::ops::typecheck::element_type_name(val)
}
