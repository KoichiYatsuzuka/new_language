/// Native module loader for `import[rs]` — reads crate source directly from the
/// cargo registry (or a local path), auto-discovers compatible `pub fn` and
/// `pub struct` + `impl` blocks, and generates call-through ABI wrappers.
///
/// # Config (`ar_config.json` — `rust.crates_path`)
///
/// ```json
/// {
///   "rust": {
///     "crates_path": "/path/to/cargo/registry/src/index.crates.io-..."
///   }
/// }
/// ```
///
/// # Compatibility rules
///
/// **Free functions** — wrapped when ALL of the following hold:
/// - No generic type parameters
/// - Every parameter and return type is a Arrow primitive: `i*`, `u*`,
///   `f32`, `f64`, `bool`, `String`, `&str`
///
/// **Structs** — wrapped when ALL of the following hold:
/// - No generic type parameters on the struct
/// - All `pub` fields have ABI-compatible types
/// - Constructor: either `pub fn new(...) -> Self` exists, or all fields are `pub`
///
/// **Struct methods** — wrapped when ALL of the following hold:
/// - `&self` or `&mut self` receiver (no generic params)
/// - All parameter and return types are ABI-compatible


// ── Types / source scanning / stubs ─────────────────────────────────────────
//
// シグネチャの型・ソースの走査（`scan_all_sigs`）・型スタブ（`make_stubs`）・`find_config` は
// `crate::rs_crate` へ切り出した（拡張の wasm でも使うため・editor_import_resolution_plan.md 2-2）。
// ここでは同じ名前で引けるよう再エクスポートする。

pub(crate) use crate::rs_crate::*;

mod loader;
mod codegen;

pub(crate) use loader::*;
pub(crate) use codegen::*;
