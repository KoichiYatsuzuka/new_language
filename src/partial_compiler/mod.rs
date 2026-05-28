/// Partial compiler: native code generation, .hvc/.hvs writing, and stub generation.
///
/// Submodules:
///   codegen         — Rust source generator (tl fn → i64 ABI)
///   module_compiler — .hvc (v0/v1) and .hvs writer + runtime DLL cache
///   stub_gen        — .hvs stub text generator
mod codegen;
mod module_compiler;
pub mod rs_loader;
mod stub_gen;

pub use module_compiler::{compile, load_tlc, native_lib_ext, take_native_bytes};
