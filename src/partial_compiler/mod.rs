/// Partial compiler: native code generation, .arc/.ars writing, and stub generation.
///
/// Submodules:
///   llvm_codegen    — LLVM IR text generator (compiled to a DLL by clang)
///   module_compiler — .arc (v0/v1) and .ars writer + runtime cache
///   stub_gen        — .ars stub text generator
pub mod llvm_codegen;
mod module_compiler;
pub mod rs_loader;
pub mod stub_gen;

pub use module_compiler::{
    compile, load_tlc, native_lib_ext, read_tlc_source, take_native_bytes,
    NativePayload,
};
