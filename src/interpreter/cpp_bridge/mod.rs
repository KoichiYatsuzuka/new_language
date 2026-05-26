// cpp_bridge — C++ DLL / static-lib bridge for import[cpp-dll] and import[cpp-lib].
#![allow(unused_imports)] // re-exports intentionally expose the full API surface
//
// Syntax: import[cpp-lib] Dir.Name with stub as alias
//         import[cpp-dll] Dir.Name with stub as alias
//
// Sub-modules by role:
//   types          — CType enum, CStructDef, CFnSig
//   header_parser  — parse_header_full / parse_header / collect_included_headers
//   typedef_loader — load_system_typedefs (typedef alias resolution)
//   codegen        — gen_dll_wrapper (generate Rust wrapper source)
//   config         — CppBuildConfig / load_cpp_config
//   compiler       — compile_wrapper / compile_tl_dll / gen_cpp_shim_source / MSVC shim

mod types;
mod header_parser;
mod typedef_loader;
mod codegen;
mod config;
mod compiler;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use types::{CType, CStructDef, CFnSig};
pub use header_parser::{parse_header_full, parse_header, collect_included_headers};
pub use typedef_loader::load_system_typedefs;
pub use codegen::gen_dll_wrapper;
pub use config::{CppBuildConfig, load_cpp_config};
pub use compiler::{
    compile_wrapper, compile_tl_dll,
    find_msvc_vcvarsall, gen_cpp_shim_source, MsvcPaths,
};
