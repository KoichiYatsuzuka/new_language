// interpreter/py_interop/ops.rs — Python の物の操作: 属性・添字・反復・演算子・比較。

use super::*;

/// 添字の鍵・代入する値（Arrow の値）を CPython の物にする。CPython の物は**借りるだけ**（CPython の `x[k] = v` も、
/// 評価のスタックの参照のほかに参照を増やさない・D-5）。
enum PyOperand<'a, 'py> {
    Borrowed(Borrowed<'a, 'py, PyAny>),
    Owned(Bound<'py, PyAny>),
}

impl PyOperand<'_, '_> {
    pub(super) fn as_ptr(&self) -> *mut ffi::PyObject {
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
pub(super) fn py_type_has_len(obj: &Bound<'_, PyAny>) -> bool {
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
