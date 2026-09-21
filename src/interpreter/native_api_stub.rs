// native_api_stub.rs — 評価コアビルド（`native` feature 無効）用のネイティブ callback ABI スタブ。
//
// `native_api/mod.rs` のうち**コア経路から参照される 3 シンボルだけ**を同じシグネチャで提供する。
// 差し替えは `interpreter.rs` の `#[cfg]` が行う（`py_interop` / `cs_dll_runtime` と同じ形）。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。** 参照元は
// `classes/instantiate.rs`（ネイティブ `__init__` の dispatch）・`classes/method_call.rs`
// （ネイティブメソッドの dispatch とオーバーロード判定）・`value/native.rs`（typed ABI 呼び出し）で、
// いずれも**分岐の条件式**に埋まっている。`#[cfg]` を配ると条件が読めなくなる。
//
// ⚠ ここで返す値は「ネイティブメソッドは 1 つも登録されていない」と等価。評価コアでは
// `import[cpp-*]` / `import[rs]` 自体が落ちているので、登録される経路が存在しない。

use super::value::Value;
use super::Interpreter;

/// `native_api::ErrSlot` と同じレイアウト。typed ABI 呼び出しの失敗を受け取る枠。
/// 評価コアでは呼び出し自体が起きないので、既定値のまま使われることしかない。
#[repr(C)]
pub struct ErrSlot {
    pub type_ptr: *const u8,
    pub type_len: u64,
    pub msg_ptr: *const u8,
    pub msg_len: u64,
}

impl Default for ErrSlot {
    fn default() -> Self {
        Self {
            type_ptr: std::ptr::null(),
            type_len: 0,
            msg_ptr: std::ptr::null(),
            msg_len: 0,
        }
    }
}

impl ErrSlot {
    /// 本体と同じ面。評価コアでは typed ABI 呼び出しが起きないので到達しない。
    pub fn to_error_string(&self) -> String {
        "NativeCallError: typed ABI call is not available in the evaluation core build"
            .to_string()
    }
}

/// 評価コアにはネイティブメソッドが 1 つも登録されないので、常に `None`。
pub fn lookup_native_method_ptr(_class_name: &str, _method_name: &str) -> Option<usize> {
    None
}

/// 同上。`None` は「ネイティブ側では扱わない」を意味し、呼び出し側は
/// 通常の（Arrow 側の）メソッド解決へ進む。
pub fn try_dispatch_native_method(
    _interp: &mut Interpreter,
    _obj: Value,
    _method_name: &str,
    _arg_vals: Vec<Value>,
) -> Option<Result<Value, String>> {
    None
}
