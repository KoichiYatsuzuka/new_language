// eval/native_stub.rs — 評価コアビルド（`native` feature 無効）用のネイティブ dispatch スタブ。
//
// `eval/native.rs` と**同じシグネチャ**を提供し、呼ばれたら明示エラーを返す。
// 差し替えは `eval/mod.rs` の `#[cfg]` が行う。
//
// ⚠ **呼び出し側を `#[cfg]` で刻まないためにこの形にしている。** メソッドごと消すと
// `classes/method_call.rs` / `classes/instantiate.rs` / `eval/attrs.rs` などの呼び出し側に
// `#[cfg]` を配ることになり、網羅 `match` の 2 段強制（`language-dev-principles` §2）を
// 崩す方向に働く。**面（シグネチャ）は残し、実装だけ落とす。**
//
// ⚠ ここへ到達するのは「評価コアビルドで `import[cpp-*]` / `import[cs-*]` 由来の値を
// 呼んだ」ときだけ。評価コアではそもそもそれらを import できない（`exec/modules.rs` の
// FFI 経路が落ちている）ので、通常は到達しない。到達したら**バグの合図**として扱う。

use std::sync::Arc;

use crate::ast::CallArg;

use super::super::value::{NativeFnRef, Value};
use super::super::Interpreter;

/// 評価コアでネイティブ呼び出しに到達したときの共通エラー。
fn unavailable(fn_name: &str) -> String {
    format!(
        "NativeCallError: native function '{fn_name}' is not available in the evaluation core \
         build (FFI is excluded; rebuild with the `native` feature)"
    )
}

impl Interpreter {
    pub(crate) fn call_native_function(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        _args: &[CallArg],
    ) -> Result<Value, String> {
        Err(unavailable(&fn_ref.fn_name))
    }

    pub(crate) fn dispatch_native_typed_exprs(
        &mut self,
        fn_ref: &NativeFnRef,
        _any_arc: &Arc<dyn std::any::Any + Send + Sync>,
        _args: &[CallArg],
    ) -> Result<Value, String> {
        Err(unavailable(&fn_ref.fn_name))
    }

    pub(crate) fn dispatch_native_evaled(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        _vals: Vec<Value>,
    ) -> Result<Value, String> {
        Err(unavailable(&fn_ref.fn_name))
    }

    pub(crate) fn dispatch_native_evaled_wb(
        &mut self,
        fn_ref: &Arc<NativeFnRef>,
        _vals: Vec<Value>,
        _wb_mask: u32,
        _wb_out: &mut Vec<(u8, Value)>,
    ) -> Result<Value, String> {
        Err(unavailable(&fn_ref.fn_name))
    }
}
