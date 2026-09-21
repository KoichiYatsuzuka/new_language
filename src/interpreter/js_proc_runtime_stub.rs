// js_proc_runtime_stub.rs — 評価コアビルド（`native` feature 無効）用の Node.js ブリッジスタブ。
//
// `js_proc_runtime.rs` と**同じシグネチャ**を提供し、呼ばれたら明示エラーを返す。
// 差し替えは `interpreter.rs` の `#[cfg]` が行う（`py_interop` / `cs_dll_runtime` と同じ形）。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。** 参照元は
// `eval/calls.rs` / `exec/modules.rs` / `ffi_boundary.rs` / `value/core.rs` に散っている。

use std::path::Path;

use super::value::Value;

fn unavailable(what: &str) -> String {
    format!(
        "JsProcError: {what} is not available in the evaluation core build \
         (the Node.js bridge is excluded; rebuild with the `native` feature)"
    )
}

pub fn launch_proc(
    _node_exe: &Path,
    _bridge_script: &Path,
    _bridge_root: &Path,
) -> Result<(), String> {
    Err(unavailable("launching the Node.js bridge process"))
}

pub fn list_functions(_bridge_key: &str, module_name: &str) -> Result<Vec<String>, String> {
    Err(unavailable(&format!("listing exports of '{module_name}'")))
}

pub fn call_function(
    _bridge_key: &str,
    module_name: &str,
    fn_name: &str,
    _args: &[Value],
) -> Result<Value, String> {
    Err(unavailable(&format!("function '{module_name}.{fn_name}'")))
}
