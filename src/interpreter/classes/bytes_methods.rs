// classes/bytes_methods.rs — バイト列(bytes)メソッドのディスパッチ: eval_bytes_method（ffi_value_exchange_plan.md 3-1）。

use {
    std::rc::Rc,
    crate::interpreter::{Interpreter, Value},
};

impl Interpreter {
    /// バイト列のメソッドを評価済み引数で呼ぶ。
    ///
    /// - `decode(encoding="utf-8")` → `str`。変換できないバイト列は ValueError（置き換え文字にしない）
    pub(crate) fn eval_bytes_method(
        &mut self,
        b: Rc<[u8]>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
        match method_name {
            "decode" => {
                let enc = match vals.as_slice() {
                    [] => "utf-8".to_string(),
                    [Value::Str(e)] => e.to_string(),
                    [other] => {
                        return Err(format!(
                            "TypeError: bytes.decode() argument 1 must be str, not '{}'",
                            self.type_name(other)
                        ))
                    }
                    _ => return Err(format!("TypeError: bytes.decode() takes at most 1 argument ({} given)", vals.len())),
                };
                Ok(Value::str(crate::interpreter::encoding::decode(&b, &enc)?))
            }
            _ => Err(format!("AttributeError: 'bytes' object has no method '{method_name}'")),
        }
    }
}
