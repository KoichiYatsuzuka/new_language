// py_interop_stub.rs — 評価コアビルド（`native` feature 無効）用の Python 相互運用スタブ。
//
// `py_interop.rs` と**同じシグネチャ**を提供し、呼ばれたら明示エラーを返す。
// 差し替えは `interpreter.rs` の `#[cfg]` が行う。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。** `Value::PyObject` の
// match アームは `eval/subscript.rs` / `exec/control_flow.rs` / `ops/operators.rs` など
// 10 箇所以上に散っており、そこへ `#[cfg]` を配ると網羅 `match` の 2 段強制
// （`language-dev-principles` §2）を崩す方向に働く。**面（シグネチャ）は残し、実装だけ落とす。**
//
// ⚠ 評価コアでは `PyObjHandle` が `Infallible` を持つ構築不能型なので、
// ここへ到達する `Value::PyObject` は**そもそも作れない**。到達したらバグの合図。

use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::{BinOp, UnaryOp};

use super::value::{NamespaceData, PyObjHandle, Value};

/// 評価コアで Python 相互運用に到達したときの共通エラー。
fn unavailable(what: &str) -> String {
    format!(
        "PyInteropError: {what} is not available in the evaluation core build \
         (Python interop is excluded; rebuild with the `native` feature)"
    )
}

pub fn load_py_int_module(
    _module_path: &[String],
    _extra_search_dirs: &[PathBuf],
) -> Result<Rc<NamespaceData>, String> {
    Err(unavailable("`import[py-int]`"))
}

pub fn call_py_object(
    _handle: PyObjHandle,
    _evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    Err(unavailable("calling a Python object"))
}

pub fn call_py_method(
    _handle: PyObjHandle,
    method_name: &str,
    _evaled: Vec<(Option<String>, Value, bool)>,
) -> Result<Value, String> {
    Err(unavailable(&format!("Python method '{method_name}'")))
}

/// Python が投げた例外の控え（`py_interop` と同じ形・評価コアでは作らない）。
#[allow(dead_code)]
pub enum PyRaised {
    Builtin(Vec<Value>),
    Object(PyObjHandle),
    Arrow(crate::interpreter::RaisedError),
}

/// 評価コアには Python が無いので、覚えるインタプリタも無い（`py_interop` と同じ形・2-5）。
pub fn register_interpreter(_interp: *mut crate::interpreter::Interpreter) {}

pub fn forget_interpreter(_interp: *mut crate::interpreter::Interpreter) {}

pub fn take_python_exception(_msg: &str) -> Option<PyRaised> {
    None
}

pub fn take_arrow_exception(_msg: &str) -> Option<crate::interpreter::RaisedError> {
    None
}

pub fn py_exception_is(_h: &PyObjHandle, _type_name: &str) -> bool {
    false
}

pub fn py_type_name(_h: &PyObjHandle) -> String {
    "<Python object>".to_string()
}

pub fn py_exception_line(_h: &PyObjHandle) -> String {
    "<Python exception>".to_string()
}

pub fn py_getattr(_handle: &PyObjHandle, attr: &str) -> Result<Value, String> {
    Err(unavailable(&format!("Python attribute '{attr}'")))
}

pub fn py_getitem(_handle: &PyObjHandle, _key: &Value) -> Result<Value, String> {
    Err(unavailable("Python subscript"))
}

pub fn py_setattr(_handle: &PyObjHandle, attr: &str, _val: &Value) -> Result<(), String> {
    Err(unavailable(&format!("Python attribute assignment '{attr}'")))
}

pub fn py_setitem(_handle: &PyObjHandle, _key: &Value, _val: &Value) -> Result<(), String> {
    Err(unavailable("Python subscript assignment"))
}

/// 評価コアのビルドには Python のオブジェクトが無い（`py_interop::py_int_text` の写し）。
pub fn py_int_text(_handle: &PyObjHandle) -> Option<String> {
    None
}

pub fn py_len(_handle: &PyObjHandle) -> Result<Value, String> {
    Err(unavailable("`len()` on a Python object"))
}

pub fn py_truthy(_handle: &PyObjHandle) -> Result<bool, String> {
    Err(unavailable("the truth value of a Python object"))
}

pub fn py_unary(_handle: &PyObjHandle, _op: &UnaryOp) -> Result<Value, String> {
    Err(unavailable("a unary operator on a Python object"))
}

/// 評価コアビルドでは Python のオブジェクトはできないので呼ばれない（呼ばれたら「呼べない」と答える）。
pub fn py_is_callable(_handle: &PyObjHandle) -> bool {
    false
}

pub fn py_get_iter(_handle: &PyObjHandle) -> Result<PyObjHandle, String> {
    Err(unavailable("iterating a Python object"))
}

pub fn py_iter_next(_handle: &PyObjHandle) -> Result<Option<Value>, String> {
    Err(unavailable("iterating a Python object"))
}

pub fn py_binop(_handle: &PyObjHandle, _op: &BinOp, _rhs: &Value) -> Result<Value, String> {
    Err(unavailable("a binary operator on a Python object"))
}

pub fn py_rbinop(_handle: &PyObjHandle, _op: &BinOp, _lhs: &Value) -> Result<Value, String> {
    Err(unavailable("a reflected binary operator on a Python object"))
}

/// 評価コアでは CPython の物を作れないので、どれも CPython の list / dict / set ではない。
pub fn py_collection_kind(_h: &PyObjHandle) -> Option<&'static str> {
    None
}

pub fn cast_to_python(_val: &Value, _target: &crate::type_check::InferredType) -> Result<Value, String> {
    Err(unavailable("`=> py_list` / `py_dict` / `py_set`"))
}

pub fn cast_from_python(_h: &PyObjHandle, _target: &crate::type_check::InferredType) -> Result<Value, String> {
    Err(unavailable("casting a Python collection"))
}

pub fn py_collection_items(_h: &PyObjHandle) -> Option<Vec<Value>> {
    None
}

pub fn py_dict_pairs(_h: &PyObjHandle) -> Option<Vec<(Value, Value)>> {
    None
}

/// 評価コアでは GIL が無いので、そのまま走らせる。
pub(crate) fn without_gil<R>(f: impl FnOnce() -> R) -> R {
    f()
}
