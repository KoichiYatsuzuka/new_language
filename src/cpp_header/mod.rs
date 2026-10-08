// cpp_header/mod.rs — C/C++ のヘッダから**型の情報**を読む部分（`import[cpp-dll]` / `import[cpp-lib]` の型の出所）。
//
// 実行時に中継 DLL をビルドして読み込む `interpreter/cpp_bridge`（`native` 限定）から切り出した。
// ヘッダ・設定ファイルを読むだけで、ファイルへのアクセスは `crate::import_fs` を通すので、
// VS Code 拡張（wasm）でも CLI と同じ型の情報が得られる（editor_import_resolution_plan.md 2-2）。

#![allow(unused_imports)] // 切り出す前の `cpp_bridge` と同じ（サブモジュールが共有の `use` を持つ）

pub mod types;
mod header_parser;
mod typedef_loader;
mod config;

pub use types::{CType, CStructDef, CFnSig};
pub use header_parser::{parse_header_full, parse_header, collect_included_headers};
pub use typedef_loader::load_system_typedefs;
pub use config::{CppBuildConfig, load_cpp_config};
