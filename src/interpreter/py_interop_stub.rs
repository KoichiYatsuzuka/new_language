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

use crate::ast::BinOp;

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
    _handle: &PyObjHandle,
    _evaled: &[(Option<String>, Value, bool)],
) -> Result<Value, String> {
    Err(unavailable("calling a Python object"))
}

pub fn call_py_method(
    _handle: &PyObjHandle,
    method_name: &str,
    _evaled: &[(Option<String>, Value, bool)],
) -> Result<Value, String> {
    Err(unavailable(&format!("Python method '{method_name}'")))
}

pub fn py_getattr(_handle: &PyObjHandle, attr: &str) -> Result<Value, String> {
    Err(unavailable(&format!("Python attribute '{attr}'")))
}

pub fn py_getitem(_handle: &PyObjHandle, _key: &Value) -> Result<Value, String> {
    Err(unavailable("Python subscript"))
}

pub fn py_setitem(_handle: &PyObjHandle, _key: &Value, _val: &Value) -> Result<(), String> {
    Err(unavailable("Python subscript assignment"))
}

pub fn py_len(_handle: &PyObjHandle) -> Result<Value, String> {
    Err(unavailable("`len()` on a Python object"))
}

pub fn py_collect_iter(_handle: &PyObjHandle) -> Result<Vec<Value>, String> {
    Err(unavailable("iterating a Python object"))
}

pub fn py_binop(_handle: &PyObjHandle, _op: &BinOp, _rhs: &Value) -> Result<Value, String> {
    Err(unavailable("a binary operator on a Python object"))
}

pub fn py_rbinop(_handle: &PyObjHandle, _op: &BinOp, _lhs: &Value) -> Result<Value, String> {
    Err(unavailable("a reflected binary operator on a Python object"))
}
