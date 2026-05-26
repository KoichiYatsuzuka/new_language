// types.rs — C type model, struct definitions, and function signatures
// shared across the cpp_bridge sub-modules.

use super::super::native_api::{TL_FALSE, TL_NONE, TL_TRUE};

// ── C type model ─────────────────────────────────────────────────────────────

/// A C type that can cross the tl ↔ native boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    /// `int`, `short`, `char`, `int32_t`, `uint32_t` → tl `int`
    Int,
    /// `long`, `long long`, `int64_t`, `uint64_t`, `size_t` → tl `int`
    Long,
    /// `float` → tl `float`
    Float,
    /// `double` → tl `float`
    Double,
    /// `bool` → tl `bool`
    Bool,
    /// `void` (return type only) → tl `None`
    Void,
    /// `void*` → tl `int` (opaque pointer stored as raw integer)
    VoidPtr,
    /// `T*` or `const T*` pointer parameter.
    /// `mutable = true` → output param: value written back after the call.
    /// `mutable = false` → input param: read-only, no write-back.
    Ptr { inner: Box<CType>, mutable: bool },
    /// `char*` / `const char*` — tl `str` ↔ null-terminated C string.
    CharPtr,
    /// `const StructName*` or `StructName*` — opaque struct pointer.
    /// Marshaled as `void*` across the ABI; the shim casts to the real type at the call site.
    OpaqueStructPtr { type_name: String, mutable: bool },
    /// Struct/union passed by value (e.g. `VECTOR`, `MATRIX`).
    /// Parameters: shim receives `void*` and dereferences with `*(TypeName*)ptr`.
    /// Return type: shim writes into a per-function static buffer and returns its address.
    /// Maps to tl `int` (opaque handle = pointer to the struct data).
    ByValueStruct { type_name: String },
    /// Function pointer parameter (e.g. `void (*callback)(int)`).
    /// Passed as an opaque `void*`; maps to tl `function`.
    FnPtr,
}

impl CType {
    /// C type string used in the generated C++ shim source.
    pub fn c_type_str(&self) -> String {
        match self {
            CType::Int => "int".to_string(),
            CType::Long => "long long".to_string(),
            CType::Float => "float".to_string(),
            CType::Double => "double".to_string(),
            CType::Bool => "int".to_string(),
            CType::Void => "void".to_string(),
            CType::VoidPtr => "void*".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("{}*", inner.c_type_str())
                } else {
                    format!("const {}*", inner.c_type_str())
                }
            }
            CType::CharPtr => "const char*".to_string(),
            // Emit void* for opaque struct pointers; the call-site cast is handled in gen_cpp_shim_source
            CType::OpaqueStructPtr { .. } => "void*".to_string(),
            // By-value structs and fn ptrs: shim receives void* and handles the real type
            CType::ByValueStruct { .. } | CType::FnPtr => "void*".to_string(),
        }
    }

    /// Rust type used in the `extern "C"` declaration inside the generated wrapper.
    pub(crate) fn rust_extern_type(&self) -> String {
        match self {
            CType::Int => "i32".to_string(),
            CType::Long => "i64".to_string(),
            CType::Float => "f32".to_string(),
            CType::Double => "f64".to_string(),
            CType::Bool => "i32".to_string(),
            CType::Void => "()".to_string(),
            CType::VoidPtr => "*mut i8".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("*mut {}", inner.rust_extern_type())
                } else {
                    format!("*const {}", inner.rust_extern_type())
                }
            }
            CType::CharPtr => "*const u8".to_string(),
            CType::OpaqueStructPtr { .. } => "*mut i8".to_string(),
            CType::ByValueStruct { .. } | CType::FnPtr => "*mut i8".to_string(),
        }
    }

    /// Rust expression that converts a tl handle (`i64`) to this C type (non-pointer only).
    /// For pointer types, use `gen_ptr_init` instead.
    pub(crate) fn from_handle(&self, handle: &str) -> String {
        match self {
            CType::Int => format!("((*CB).to_int)({handle}) as i32"),
            CType::Long => format!("((*CB).to_int)({handle})"),
            CType::Float => format!("((*CB).to_float)({handle}) as f32"),
            CType::Double => format!("((*CB).to_float)({handle})"),
            CType::Bool => format!("if {handle} == {TL_TRUE}i64 {{ 1i32 }} else {{ 0i32 }}"),
            CType::Void => "()".to_string(),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } => format!("{handle} as *mut i8"),
            CType::CharPtr => format!("((*CB).to_cstr)({handle})"),
            CType::ByValueStruct { .. } | CType::FnPtr => format!("{handle} as *mut i8"),
            CType::Ptr { .. } => panic!("use gen_ptr_init for pointer parameters"),
        }
    }

    /// Rust expression that wraps a C return value into a tl handle.
    pub(crate) fn to_handle(&self, val: &str) -> String {
        match self {
            CType::Int => format!("((*CB).make_int)({val} as i64)"),
            CType::Long => format!("((*CB).make_int)({val})"),
            CType::Float => format!("((*CB).make_float)({val} as f64)"),
            CType::Double => format!("((*CB).make_float)({val})"),
            CType::Bool => format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}"),
            CType::Void => format!("{TL_NONE}i64"),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } => format!("{val} as i64"),
            // ByValueStruct: val is *mut i8 pointing at the shim's static buffer
            CType::ByValueStruct { .. } | CType::FnPtr => format!("{val} as i64"),
            CType::CharPtr | CType::Ptr { .. } => format!("{TL_NONE}i64"), // pointers as return are opaque
        }
    }
}

// ── Struct definition ─────────────────────────────────────────────────────────

/// A C struct/union definition extracted from a `typedef struct { … } Name;` form.
/// Only structs whose every field resolves to a primitive `CType` are emitted.
#[derive(Debug, Clone)]
pub struct CStructDef {
    /// The typedef alias name (e.g. `"VECTOR"`).
    pub name: String,
    /// Fields in declaration order: `(field_name, CType)`.
    pub fields: Vec<(String, CType)>,
}

// ── Function signature ───────────────────────────────────────────────────────

/// A C function signature extracted from a `.h` file.
#[derive(Debug, Clone)]
pub struct CFnSig {
    pub name: String,
    pub params: Vec<(String, CType)>,
    pub ret: CType,
    /// C++ namespace this function lives in, if any (e.g. `"DxLib"`).
    pub namespace: Option<String>,
    /// Index of the first optional parameter (those with DEFAULTPARAM / C++ default args).
    /// Callers may omit tail parameters from index `n_required` onwards; omitted args
    /// are padded with 0 (the tl None handle, which marshals to a NULL pointer or 0).
    pub n_required: usize,
}
