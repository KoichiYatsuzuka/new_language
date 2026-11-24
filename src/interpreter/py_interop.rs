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

#[path = "py_interop/gil.rs"]
mod gil;
#[path = "py_interop/convert.rs"]
mod convert;
#[path = "py_interop/arrow_objects.rs"]
mod arrow_objects;
#[path = "py_interop/collections.rs"]
mod collections;
#[path = "py_interop/modules.rs"]
mod modules;
#[path = "py_interop/errors.rs"]
mod errors;
#[path = "py_interop/calls.rs"]
mod calls;
#[path = "py_interop/ops.rs"]
mod ops;

pub(crate) use self::gil::*;
pub(crate) use self::convert::*;
pub(crate) use self::arrow_objects::*;
pub(crate) use self::collections::*;
pub(crate) use self::modules::*;
pub(crate) use self::errors::*;
pub(crate) use self::calls::*;
pub(crate) use self::ops::*;

