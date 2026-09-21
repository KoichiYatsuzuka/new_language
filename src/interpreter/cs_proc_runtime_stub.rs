// cs_proc_runtime_stub.rs — 評価コアビルド（`native` feature 無効）用の .NET 別プロセスブリッジスタブ。
//
// `cs_proc_runtime.rs` と**同じシグネチャ**を提供し、呼ばれたら明示エラーを返す。
// 差し替えは `interpreter.rs` の `#[cfg]` が行う（`py_interop` / `cs_dll_runtime` と同じ形）。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。** 参照元は
// `classes/class_methods.rs` / `classes/instantiate.rs` / `classes/method_call.rs` /
// `eval/attrs.rs` / `exec/modules.rs` に散っている。

use std::path::Path;

use super::value::Value;

fn unavailable(what: &str) -> String {
    format!(
        "CsProcError: {what} is not available in the evaluation core build \
         (the .NET out-of-process bridge is excluded; rebuild with the `native` feature)"
    )
}

pub fn launch_proc(_proc_path: &Path) -> Result<(), String> {
    Err(unavailable("launching the .NET bridge process"))
}

pub fn call_static(
    _proc_path: &Path,
    class_name: &str,
    method: &str,
    _args: &[Value],
    _ret_type: Option<&str>,
) -> Result<Value, String> {
    Err(unavailable(&format!("static method '{class_name}.{method}'")))
}

pub fn call_constructor(
    _proc_path: &Path,
    class_name: &str,
    _args: &[Value],
) -> Result<i64, String> {
    Err(unavailable(&format!("constructing '{class_name}'")))
}

pub fn call_instance(
    _proc_path: &Path,
    class_name: &str,
    _handle: i64,
    method: &str,
    _args: &[Value],
    _ret_type: Option<&str>,
) -> Result<Value, String> {
    Err(unavailable(&format!("method '{class_name}.{method}'")))
}
