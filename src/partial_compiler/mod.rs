/// Partial compiler: native code generation, .arc/.ars writing, and stub generation.
///
/// Submodules:
///   llvm_codegen    — LLVM IR text generator (clang fallback path)
///   inkwell_codegen — inkwell JIT compiler (feature = "llvm", primary path)
///   module_compiler — .arc (v0/v1/v2) and .ars writer + runtime cache
///   stub_gen        — .ars stub text generator
pub mod llvm_codegen;
#[cfg(feature = "llvm")]
pub mod inkwell_codegen;
mod module_compiler;
pub mod rs_loader;
pub mod stub_gen;

pub use module_compiler::{
    compile, load_tlc, native_lib_ext, take_native_bytes,
    NativePayload,
};
