/// Partial compiler: native code generation, .tlc/.tls writing, and stub generation.
///
/// Submodules:
///   codegen         — Rust source generator (tl fn → i64 ABI)
///   module_compiler — .tlc (v0/v1) and .tls writer + runtime DLL cache
///   stub_gen        — .tls stub text generator
mod codegen;
mod module_compiler;
mod stub_gen;

pub use module_compiler::{compile, load_tlc, native_lib_ext, take_native_bytes};
