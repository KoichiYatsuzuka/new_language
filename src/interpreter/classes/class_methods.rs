// classes/class_methods.rs — **クラスオブジェクト自身**に対するメソッド呼び出しのディスパッチ。
//
// #63 で `eval_method_call_full` の `Value::Class` アーム（71 行）を切り出したもの。
// インスタンスに対する呼び出しは `method_call.rs` の `call_instance_method_evaled`（別物）。
//
// ⚠ ここが受けるのは `static` / `class_method` だけ。インスタンスメソッドを
// クラス経由で呼ぶのは**エラー**（末尾の `TypeError`）。
// ⚠ C# 相互運用（`import[cs-dll]` / `import[cs-proc]`）の **static メソッドもここを通る**。
// 判定はクラス変数 `__cs_bridge_path__` / `__cs_proc_path__` の有無で、
// これを焼き込むのは `Interpreter::exec_import`（#58 の `inject_class_var`）。

use {
    std::rc::Rc,
    crate::interpreter::{ClassValue, Interpreter, Value},
};

impl Interpreter {
    /// クラスオブジェクトのメソッドを評価済み引数で呼ぶ（#63 で切り出し）。
    /// 呼び出し元は `eval_method_call_full` のみ。
    pub(crate) fn eval_class_method(
        &mut self,
        cls: Rc<ClassValue>,
        method_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        // cs-dll static method dispatch
        if let Some(Value::Str(bp)) = cls.class_vars.get("__cs_bridge_path__") {
            let bp_path = std::path::PathBuf::from(&**bp);
            let class_name = cls.name.clone();
            let ret_type: Option<String> = cls
                .methods
                .get(method_name)
                .and_then(|overloads| overloads.first())
                .and_then(|f| f.return_type.clone());
            if let Some(bridge) = crate::interpreter::cs_dll_runtime::get_bridge(&bp_path) {
                let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                return crate::interpreter::cs_dll_runtime::call_static(
                    &bridge,
                    &class_name,
                    method_name,
                    &arg_vals,
                    ret_type.as_deref(),
                )
                .map_err(|e| format!("CsDll: {e}"));
            }
        }
        // cs-proc static method dispatch
        if let Some(Value::Str(pp)) = cls.class_vars.get("__cs_proc_path__") {
            let pp_path = std::path::PathBuf::from(&**pp);
            let class_name = cls.name.clone();
            let ret_type: Option<String> = cls
                .methods
                .get(method_name)
                .and_then(|overloads| overloads.first())
                .and_then(|f| f.return_type.clone());
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
            return crate::interpreter::cs_proc_runtime::call_static(
                &pp_path,
                &class_name,
                method_name,
                &arg_vals,
                ret_type.as_deref(),
            )
            .map_err(|e| format!("CsProc: {e}"));
        }

        // クラスオブジェクトに対するメソッド呼び出し: static / class_method のみ許可
        let overloads = self
            .lookup_method_in_class(&cls, method_name)
            .ok_or_else(|| {
                format!(
                    "AttributeError: class '{}' has no method '{method_name}'",
                    cls.name
                )
            })?;

        if cls.static_method_names.contains(method_name) {
            return if overloads.len() == 1 {
                self.exec_fn_evaled(overloads[0].clone(), &evaled, None, method_name, None)
            } else {
                self.dispatch_overload_evaled(overloads, evaled, None, method_name, None)
            };
        }

        if cls.class_method_names.contains(method_name) {
            // ⚠ `class_method` は**第 1 引数にクラス自身**を渡す（`self` の代わり）。
            let cls_val = Value::Class(cls.clone());
            let mut all_evaled: Vec<(Option<String>, Value, bool)> = vec![(None, cls_val, true)];
            all_evaled.extend(evaled);
            return if overloads.len() == 1 {
                self.exec_fn_evaled(overloads[0].clone(), &all_evaled, None, method_name, None)
            } else {
                self.dispatch_overload_evaled(overloads, all_evaled, None, method_name, None)
            };
        }

        Err(format!(
            "TypeError: cannot call instance method '{method_name}' on class '{}' directly; use an instance",
            cls.name
        ))
    }
}
