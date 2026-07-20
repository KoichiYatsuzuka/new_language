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

use std::path::PathBuf;

// ── Internal types ────────────────────────────────────────────────────────────

/// Free function signature from a Rust crate.
pub(crate) struct RsFnSig {
    name: String,
    params: Vec<RsParam>,
    return_type: Option<String>,
    /// When `Some(TypeName)`, this is a synthesised one-shot Digest wrapper for
    /// a `pub type TypeName = ...` that implements the RustCrypto `Digest` trait.
    /// The generated wrapper calls `crate::TypeName::digest(input.as_bytes())`
    /// and returns the result as a lowercase hex `String`.
    digest_type: Option<String>,
}

/// Single parameter (name + Rust type string).
#[derive(Clone)]
pub(crate) struct RsParam {
    name: String,
    rust_type: String,
}

/// Struct definition parsed from Rust source.
pub(crate) struct RsStructSig {
    name: String,
    /// Public ABI-compatible fields.
    fields: Vec<RsField>,
    /// Methods from `impl Name { pub fn ... }`.
    methods: Vec<RsMethodSig>,
    /// Constructor params: from `pub fn new(...)` if present, else field order.
    ctor_params: Vec<RsParam>,
    /// If true, use `Name::new(...)` for construction; if false, use struct literal.
    use_new_fn: bool,
}

pub(crate) struct RsField {
    name: String,
    rust_type: String,
}

#[derive(Clone)]
pub(crate) struct RsMethodSig {
    name: String,
    params: Vec<RsParam>,
    self_mutable: bool,
    return_type: Option<String>,
    /// Set when the return type is a struct defined in the same crate.
    return_struct: Option<String>,
}

/// Where the crate source lives.
pub(crate) enum CrateSource {
    /// TODO(reserved): crates.io 依存としてのロード。`prepare_wrapper` 側の生成は
    /// 実装済みだが、`find_config` はまだ `LocalPath` しか返さないため未構築。
    #[allow(dead_code)]
    Registry { crate_name: String, version_req: String },
    LocalPath { crate_name: String, path: PathBuf },
}


mod loader;
mod parse;
mod stubs;
mod codegen;

pub(crate) use loader::*;
pub(crate) use parse::*;
pub(crate) use stubs::*;
pub(crate) use codegen::*;
