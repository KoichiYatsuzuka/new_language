// classes/ptr_methods.rs — 外部のメモリを指す値(Ptr)のメソッドのディスパッチ: eval_ptr_method（ffi_value_exchange_plan.md 3-2・3-3）。
//
// `Ptr[void]` からの変換は `p mustbe Ptr[T]`（`ptr_ann_matches`）。

use {
    std::{cell::RefCell, rc::Rc, sync::Arc},
    crate::interpreter::{Interpreter, PtrHandle, Value},
};

impl Interpreter {
    /// `Ptr` のメソッドを評価済み引数で呼ぶ。
    ///
    /// - `release()` → `None`。所有しているメモリを今すぐ解放する（最後の参照を待たない）。借用・STATIC・
    ///   解放済み・C の側で解放済みのものは ValueError（黙って何もしない、にはしない）
    /// - `to_list()` → `list[T]`、`to_bytes()` → `bytes`（3-3）。指す先をコピーする（`PtrHandle::read_list` / `read_bytes`）
    pub(crate) fn eval_ptr_method(
        &mut self,
        p: Arc<PtrHandle>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        if matches!(method_name, "release" | "to_list" | "to_bytes") && !evaled.is_empty() {
            return Err(format!("TypeError: Ptr.{method_name}() takes no arguments ({} given)", evaled.len()));
        }
        match method_name {
            "release" => {
                p.release()?;
                Ok(Value::None)
            }
            "to_list" => Ok(Value::List(Rc::new(RefCell::new(p.read_list()?)))),
            "to_bytes" => Ok(Value::bytes(p.read_bytes()?)),
            _ => Err(format!("AttributeError: 'Ptr[{}]' object has no method '{method_name}'", p.elem_type())),
        }
    }
}
