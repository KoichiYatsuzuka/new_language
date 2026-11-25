// interpreter/py_interop/errors.rs — Python の例外と Arrow の例外の橋渡し。

use super::*;

// ---------------------------------------------------------------------------
// 呼び出しヘルパー
// ---------------------------------------------------------------------------

/// Arrow の値を Python へ渡せなかった（`ar_to_py`・ffi_value_exchange_plan.md 7-24）。Arrow 側の型の誤りなので
/// `TypeError`（`PyErr` の表記 `TypeError: cannot pass an Arrow value of type 'Point' to Python` のまま）。
/// ⚠ Python が投げた例外（[`python_raised`]）と分ける。以前はどちらも呼び出しの種類の例外（`RuntimeError: Python call
///   failed: TypeError: …`）に包まれていた
pub(super) fn arrow_side(e: PyErr) -> String {
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
pub(super) fn python_raised(e: PyErr) -> String {
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
