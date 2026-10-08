/// Partial compiler: native code generation, .arc/.ars writing, and stub generation.
///
/// Submodules:
///   llvm_codegen    — LLVM IR text generator (compiled to a DLL by clang)
///   module_compiler — .arc (v0/v1) and .ars writer + runtime cache
///   stub_gen        — .ars stub text generator
pub mod llvm_codegen;
mod module_compiler;
// ⚠ Rust crate ローダ（`import[rs]`）。外部 `cargo` を起動して DLL を作るので
//   `native` 限定（評価コア切り出し #5）。参照元は `parser/imports/dispatch.rs`
//   （`editor` では型だけを読む `crate::rs_crate::load_types` を使う）と `module_compiler`。
#[cfg(feature = "native")]
pub mod rs_loader;
pub mod stub_gen;

pub use module_compiler::{
    compile, load_tlc, native_lib_ext, take_native_bytes,
    NativePayload,
};
