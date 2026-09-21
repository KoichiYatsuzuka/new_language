// cs_dll_runtime_stub.rs — 評価コアビルド（`native` feature 無効）用の .NET ブリッジスタブ。
//
// `cs_dll_runtime.rs` と**同じシグネチャ**を提供し、呼ばれたら明示エラーを返す。
// 差し替えは `interpreter.rs` の `#[cfg]` が行う（`py_interop` / `eval/native` と同じ形）。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。**
// `Value::CsObject` の match アームが `classes/instantiate.rs` / `classes/method_call.rs` /
// `classes/class_methods.rs` / `eval/attrs.rs` に散っており、そこへ `#[cfg]` を配ると
// 網羅 `match` の 2 段強制（`language-dev-principles` §2）を崩す方向に働く。

use std::path::Path;
use std::sync::Arc;

use super::value::Value;

/// 評価コアの `BridgeLib`。構築不能（`load_bridge` が必ず `Err` を返すため到達しない）。
pub struct BridgeLib {
    _never: std::convert::Infallible,
}

/// 評価コアで .NET ブリッジに到達したときの共通エラー。
fn unavailable(what: &str) -> String {
    format!(
        "CsInteropError: {what} is not available in the evaluation core build \
         (.NET interop is excluded; rebuild with the `native` feature)"
    )
}

pub fn load_bridge(_path: &Path) -> Result<Arc<BridgeLib>, String> {
    Err(unavailable("loading a .NET bridge"))
}

pub fn get_bridge(_path: &Path) -> Option<Arc<BridgeLib>> {
    None
}

pub fn call_constructor(
    _bridge: &BridgeLib,
    class_name: &str,
    _args: &[Value],
) -> Result<i64, String> {
    Err(unavailable(&format!("constructing '{class_name}'")))
}

pub fn call_static(
    _bridge: &BridgeLib,
    class_name: &str,
    method: &str,
    _args: &[Value],
    _ret_type: Option<&str>,
) -> Result<Value, String> {
    Err(unavailable(&format!("static method '{class_name}.{method}'")))
}

pub fn call_instance(
    _bridge: &BridgeLib,
    class_name: &str,
    _handle: i64,
    method: &str,
    _args: &[Value],
    _ret_type: Option<&str>,
) -> Result<Value, String> {
    Err(unavailable(&format!("method '{class_name}.{method}'")))
}
