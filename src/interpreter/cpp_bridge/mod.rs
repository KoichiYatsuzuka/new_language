// cpp_bridge — C++ DLL / static-lib bridge for import[cpp-dll] and import[cpp-lib].
#![allow(unused_imports)] // re-exports intentionally expose the full API surface
//
// Syntax: import[cpp-lib] Dir.Name with stub as alias
//         import[cpp-dll] Dir.Name with stub as alias
//
// Sub-modules by role:
//   types          — CType / CStructDef の**実行時に依存する**メソッド（定義は `crate::cpp_header::types`）
//   codegen        — gen_dll_wrapper (generate Rust wrapper source)
//   compiler       — compile_wrapper / compile_tl_dll / gen_cpp_shim_source / MSVC shim
//
// ⚠ ヘッダから型の情報を読む部分（types の定義・header_parser・typedef_loader・config）は
//   `crate::cpp_header` へ切り出した（拡張の wasm でも使うため・editor_import_resolution_plan.md 2-2）。
//   ここでは同じ名前で引けるよう再エクスポートする。

mod types;
mod codegen;
mod compiler;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use crate::cpp_header::{CType, CStructDef, CFnSig};
pub use crate::cpp_header::{parse_header_full, parse_header, collect_included_headers};
pub use crate::cpp_header::load_system_typedefs;
pub use codegen::gen_dll_wrapper;
pub use crate::cpp_header::{CppBuildConfig, load_cpp_config};
pub use compiler::{
    compile_wrapper, compile_tl_dll,
    find_msvc_vcvarsall, gen_cpp_shim_source, MsvcPaths,
};
