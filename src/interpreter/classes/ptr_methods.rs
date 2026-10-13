// classes/ptr_methods.rs — 外部のメモリを指す値(Ptr)のメソッドのディスパッチ: eval_ptr_method（ffi_value_exchange_plan.md 3-2）。
//
// ⚠ 展開（`p.to_list()` / `p.to_bytes()`）と `Ptr[void]` からの変換は 3-3（型検査と一緒に入れる）。

use {
    std::sync::Arc,
    crate::interpreter::{Interpreter, PtrHandle, Value},
};

impl Interpreter {
    /// `Ptr` のメソッドを評価済み引数で呼ぶ。
    ///
    /// - `release()` → `None`。所有しているメモリを今すぐ解放する（最後の参照を待たない）。借用・STATIC・
    ///   解放済み・C の側で解放済みのものは ValueError（黙って何もしない、にはしない）
    pub(crate) fn eval_ptr_method(
        &mut self,
        p: Arc<PtrHandle>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        match method_name {
            "release" => {
                if !evaled.is_empty() {
                    return Err(format!("TypeError: Ptr.release() takes no arguments ({} given)", evaled.len()));
                }
                p.release()?;
                Ok(Value::None)
            }
            _ => Err(format!("AttributeError: 'Ptr[{}]' object has no method '{method_name}'", p.elem_type())),
        }
    }
}
