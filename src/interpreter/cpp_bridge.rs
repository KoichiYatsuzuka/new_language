// cpp_bridge.rs — C++ DLL / static-lib bridge for import[cpp-dll] and import[cpp-lib].
//
// Syntax: import[cpp-lib] Dir.Name with stub as alias
//         import[cpp-dll] Dir.Name with stub as alias
//
// Flow for import[cpp-dll] Dir.Name with stub:
//   1. parse_header() reads the stub .h file and extracts extern "C" function signatures.
//   2. gen_dll_wrapper() emits a standalone Rust source file that:
//        - Dynamically loads the original DLL via OS APIs (LoadLibraryA / dlopen).
//        - Exports one `{name}_tl(argc, argv) -> i64` wrapper per function,
//          marshaling between tl handles and native C types.
//   3. compile_wrapper() compiles the source with `rustc --crate-type cdylib`.
//   4. The resulting DLL is loaded by the interpreter via the existing NativeFnRef ABI.
//
// Flow for import[cpp-lib] Dir.Name with stub:
//   Same as above except the wrapper uses `#[link(name="...", kind="static")]`
//   instead of dynamic loading, and the parent directory is passed to rustc as -L.
//
// Supported C types: int / long / float / double / bool / void / void* / const char*
// Not yet supported: struct / union / enum types, function pointers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::native_api::{TL_FALSE, TL_NONE, TL_TRUE};

// ── Implementation constants ─────────────────────────────────────────────────
const TMP_RS_NAME: &str = "_tl_cpp_bridge.rs";
const TMP_DLL_STEM: &str = "_tl_cpp_bridge";
const RUSTC_EDITION: &str = "2021";
const RUSTC_OPT_LEVEL: &str = "2";
const DEFAULTPARAM_MACRO: &str = "DEFAULTPARAM";
const CONFIG_FILE_NAME: &str = "tl_config.json";
const TL_DLL_PREFIX: &str = "tl_";
const TL_SYMS_EXT: &str = "syms";
const TL_SHIM_SUFFIX: &str = "_shim";
const MAX_COMPILE_PASSES: usize = 5;
// RTLD_LAZY = 1 on all POSIX platforms; this value is embedded as literal `1`
// in the PLATFORM_LOADER string below (it cannot be referenced via a Rust const
// inside a string literal without converting the const to a runtime function).
#[allow(dead_code)]
const RTLD_LAZY: i32 = 1;

const DEFAULT_SYSTEM_LIBS: &[&str] = &[
    "winmm.lib",
    "imm32.lib",
    "ws2_32.lib",
    "dxguid.lib",
    "d3d9.lib",
    "d3d11.lib",
    "dxgi.lib",
    "dinput8.lib",
    "d3dcompiler.lib",
];
// Ordered by preference: more specific (versioned) first.
const DEFAULT_LIB_PATTERNS: &[&str] = &["_vs2015_x64_md.lib", "_x64.lib"];
const DEFAULT_TARGET_ARCH: &str = "amd64";

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
    fn rust_extern_type(&self) -> String {
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
    fn from_handle(&self, handle: &str) -> String {
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
    fn to_handle(&self, val: &str) -> String {
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

// ── Header parser ────────────────────────────────────────────────────────────

/// Parse C/C++ function declarations from a header file.
///
/// `custom` maps C type names to tl primitive types
/// (`"int"` / `"long"` / `"float"` / `"double"` / `"bool"` / `"void"`).
/// Checked before `typedefs`.
///
/// `typedefs` is a pre-resolved alias map built by `load_system_typedefs`.
/// It maps C type aliases to their underlying C++ primitive strings so that
/// platform-specific names (e.g. `DWORD`, `HWND`) are resolved without
/// hardcoding.  Pass `&HashMap::new()` when no system headers are loaded.
///
/// Recognises:
/// - `extern "C" { ... }` blocks — plain C linkage, recurses with same namespace
/// - `namespace X { ... }` blocks — recurses with namespace = X
///
/// Functions with unresolvable types are silently skipped.
/// Parse C/C++ function declarations and struct definitions from a header file.
/// Returns `(functions, structs)`.  Struct definitions are only emitted when
/// every field type is resolvable to a primitive `CType`.
pub fn parse_header_full(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> (Vec<CFnSig>, Vec<CStructDef>) {
    let stripped = strip_comments(content);
    let mut decls: Vec<(String, Option<String>)> = Vec::new();
    scan_scope(&stripped, None, &mut decls);

    let mut sigs = Vec::new();
    for (decl, ns) in &decls {
        if let Ok(sig) = parse_fn_decl_ns(decl, ns.clone(), custom, typedefs) {
            sigs.push(sig);
        }
    }

    let structs = parse_struct_bodies(&stripped, custom, typedefs);
    (sigs, structs)
}

pub fn parse_header(
    content: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CFnSig> {
    parse_header_full(content, custom, typedefs).0
}

/// Scan a header's raw text for local `#include "filename.h"` directives and
/// return paths that exist relative to `header_dir`.
///
/// Used when `precompile_macros` is non-empty: the main header may conditionally
/// include other headers (e.g. `#ifdef WINDOWS_DESKTOP_OS … #include "DxFunctionWin.h"`)
/// whose function declarations we also need to parse.
pub fn collect_included_headers(raw_content: &str, header_dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for line in raw_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#include") {
            continue;
        }
        let after = trimmed["#include".len()..].trim_start();
        // Only quoted includes (local headers), not angle-bracket system headers
        if !after.starts_with('"') {
            continue;
        }
        let inner = &after[1..];
        if let Some(end) = inner.find('"') {
            let fname = &inner[..end];
            // Only simple filenames (no path separators) resolved relative to header_dir
            let candidate = header_dir.join(fname);
            if candidate.exists() && !result.contains(&candidate) {
                result.push(candidate);
            }
        }
    }
    result
}

// ── Typedef alias loading ────────────────────────────────────────────────────

/// Load `typedef` aliases from the given system/SDK header files and resolve
/// them transitively to C++ primitive type strings.
///
/// The returned map covers:
/// - Plain `typedef`: `typedef unsigned long DWORD` → `"DWORD"` → `"unsigned long"`
/// - Multi-alias: `typedef unsigned long DWORD, *LPDWORD` → both aliases
/// - Struct typedef: `typedef struct tag { … } A, *B` → `A → tag`, `B → tag*`
/// - `DECLARE_HANDLE(X)` macro → `"X"` → `"void*"` (opaque pointer)
///
/// `macros` is the list of predefined macros (from `precompile_macros`) used to
/// resolve `#ifdef`/`#elif defined(…)` branches (e.g. `_WIN64` → `ULONG_PTR = unsigned __int64`).
///
/// Each listed header and its angle-bracket sub-headers are followed recursively.
/// Transitive aliases are fully resolved (e.g. `DWORD_PTR` → `ULONG_PTR` → `unsigned __int64`).
pub fn load_system_typedefs(header_paths: &[String], macros: &[String]) -> HashMap<String, String> {
    let mut raw: HashMap<String, String> = HashMap::new();
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for p in header_paths {
        collect_typedefs_from_file(Path::new(p), &mut raw, &mut visited, macros);
    }
    resolve_typedef_map(&mut raw);
    raw
}

fn collect_typedefs_from_file(
    path: &Path,
    out: &mut HashMap<String, String>,
    visited: &mut std::collections::HashSet<PathBuf>,
    macros: &[String],
) {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let header_dir = path.parent().unwrap_or(Path::new("."));
    for inc in collect_typedef_includes(&content, header_dir) {
        collect_typedefs_from_file(&inc, out, visited, macros);
    }
    // Use macro-aware preprocessor so #ifdef _WIN64 blocks are resolved correctly.
    let stripped = strip_and_preprocess(&content, macros);
    collect_declare_handles(&stripped, out);
    collect_typedefs_from_text(&stripped, out);
}

/// Collect headers to follow for typedef loading.
/// Follows both `#include "file.h"` (quoted) and `#include <file.h>` (angle-bracket).
/// Angle-bracket names are resolved relative to `header_dir`; if not found there,
/// sibling directories (one level up from `header_dir`) are also checked.
fn collect_typedef_includes(raw_content: &str, header_dir: &Path) -> Vec<PathBuf> {
    // Sibling dirs: e.g. `um/` and `shared/` both live under the SDK version root.
    let parent = header_dir.parent();
    let mut result = Vec::new();
    for line in raw_content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#include") {
            continue;
        }
        let after = trimmed["#include".len()..].trim_start();
        let (fname, require_exists) = if after.starts_with('"') {
            let inner = &after[1..];
            (inner.find('"').map(|e| inner[..e].to_string()), false)
        } else if after.starts_with('<') {
            let inner = &after[1..];
            // Only plain filenames — skip paths with slashes (e.g. <sys/types.h>)
            (inner.find('>').map(|e| inner[..e].to_string()), true)
        } else {
            (None, false)
        };
        let fname = match fname {
            Some(f) if !f.is_empty() => f,
            _ => continue,
        };
        // Try header_dir first, then sibling directories
        let mut found: Option<PathBuf> = None;
        let candidate = header_dir.join(&fname);
        if candidate.exists() {
            found = Some(candidate);
        } else if require_exists {
            if let Some(p) = parent {
                if let Ok(entries) = std::fs::read_dir(p) {
                    for entry in entries.flatten() {
                        let c = entry.path().join(&fname);
                        if c.exists() {
                            found = Some(c);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(p) = found {
            if !result.contains(&p) {
                result.push(p);
            }
        }
    }
    result
}

/// Detect `DECLARE_HANDLE(Name)` macro calls in stripped source and register
/// each `Name` as an opaque pointer (`void*`).
fn collect_declare_handles(stripped: &str, out: &mut HashMap<String, String>) {
    let mut rest = stripped;
    while let Some(pos) = rest.find("DECLARE_HANDLE") {
        rest = &rest[pos + "DECLARE_HANDLE".len()..];
        let after = rest.trim_start();
        if after.starts_with('(') {
            let inner = &after[1..];
            if let Some(close) = inner.find(')') {
                let name = inner[..close].trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    out.entry(name.to_string()).or_insert_with(|| "void*".to_string());
                }
            }
        }
    }
}

/// Scan stripped source for `typedef <type_expr> <Alias>;` statements and
/// populate `out` with `Alias → type_expr` entries (first definition wins).
fn collect_typedefs_from_text(stripped: &str, out: &mut HashMap<String, String>) {
    let mut i = 0;
    let mut seg_start = 0;

    while i < stripped.len() {
        let at_boundary = i == 0 || {
            let prev = stripped[..i].chars().last().unwrap_or(' ');
            !prev.is_alphanumeric() && prev != '_'
        };

        if at_boundary {
            // Skip `extern "C" { ... }` — recurse inside to catch typedefs there too
            if stripped[i..].starts_with("extern") {
                let after_kw = stripped[i + 6..].trim_start();
                if after_kw.starts_with('"') {
                    if let Some(qclose) = after_kw[1..].find('"') {
                        let rest = after_kw[2 + qclose..].trim_start();
                        if rest.starts_with('{') {
                            if let Some(brace_end) = find_matching_brace(rest) {
                                collect_typedefs_from_text(&rest[1..brace_end], out);
                                let consumed = (stripped.len() - rest.len()) - i + brace_end + 1;
                                i += consumed;
                                seg_start = i;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Skip `{ ... }` blocks (struct/union/enum bodies, namespace bodies, etc.)
        if stripped[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&stripped[i..]) {
                // If the segment so far starts with `typedef`, keep seg_start so the full
                // `typedef struct Tag { … } Alias;` is captured when we hit the `;`.
                let seg_before_brace = stripped[seg_start..i].trim_start();
                i += brace_end + 1;
                if !seg_before_brace.starts_with("typedef") {
                    seg_start = i;
                }
                continue;
            }
        }

        if stripped[i..].starts_with(';') {
            let segment = stripped[seg_start..i].trim();
            for (alias, type_expr) in parse_typedef_segments(segment) {
                out.entry(alias).or_insert(type_expr);
            }
            i += 1;
            seg_start = i;
            continue;
        }

        i += 1;
    }
}

/// Returns true if `seg` is a function-pointer typedef.
/// Catches `(*name)`, `(__cdecl* name)`, `(CALLBACK* name)`, `(NTAPI* name)`, etc.
/// by looking for any `*` that appears inside a `(…)` group.
fn typedef_contains_fn_ptr(seg: &str) -> bool {
    let mut depth = 0i32;
    for c in seg.chars() {
        match c {
            '(' => depth += 1,
            ')' => { if depth > 0 { depth -= 1; } }
            '*' if depth > 0 => return true,
            _ => {}
        }
    }
    false
}

/// Parse a `typedef` segment and return all `(alias, type_expr)` pairs it declares.
///
/// Handles all three equivalent forms:
///  - `typedef struct Tag { … } A, *B;`  →  `A → Tag`, `B → Tag*`
///  - `typedef unsigned long DWORD, *LPDWORD;`  →  `DWORD → unsigned long`, `LPDWORD → unsigned long*`
///  - `typedef void *PVOID;`  →  `PVOID → void*`
///
/// Function-pointer typedefs (any `*` inside `(…)`) are skipped.
fn parse_typedef_segments(seg: &str) -> Vec<(String, String)> {
    let words: Vec<&str> = seg.split_whitespace().collect();
    if words.first().copied() != Some("typedef") || words.len() < 3 {
        return vec![];
    }
    if typedef_contains_fn_ptr(seg) {
        return vec![];
    }

    // ── Struct/union/enum typedef with body ──────────────────────────────────
    if seg.contains('{') {
        let brace_start = seg.find('{').unwrap();
        let brace_end = match find_matching_brace(&seg[brace_start..]) {
            Some(p) => brace_start + p,
            None => return vec![],
        };
        let aliases_str = seg[brace_end + 1..].trim();
        if aliases_str.is_empty() {
            return vec![];
        }
        // Tag name = first identifier after `struct`/`union`/`enum` keyword, before `{`
        let before_brace_words: Vec<&str> = seg[..brace_start].split_whitespace().collect();
        let tag = before_brace_words
            .iter()
            .skip(1) // skip "typedef"
            .skip_while(|&&w| w == "struct" || w == "union" || w == "enum")
            .find(|&&w| w.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_'))
            .copied();
        let base_type = match tag {
            Some(t) => t.to_string(),
            None => return vec![],
        };
        return parse_alias_list(aliases_str, &base_type);
    }

    // ── Standard typedef: `typedef <type> alias1[, alias2, …]` ──────────────
    let after_typedef = seg.trim().strip_prefix("typedef").unwrap().trim_start();
    let parts: Vec<&str> = after_typedef.split(',').collect();
    if parts.is_empty() {
        return vec![];
    }

    // First comma-group: `<type tokens> first_alias`
    let first_words: Vec<&str> = parts[0].split_whitespace().collect();
    if first_words.len() < 2 {
        return vec![];
    }
    let alias_raw = *first_words.last().unwrap();
    let (alias, alias_stars) = extract_alias_token(alias_raw);
    if alias.is_empty() {
        return vec![];
    }
    // Base type = words before the alias, not counting their pointer decoration
    let type_words: Vec<&str> = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w != "*")
        .copied()
        .collect();
    let standalone_before = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w == "*")
        .count();
    let base_type = type_words.join(" ");
    let first_stars = alias_stars + standalone_before;

    let mut results = Vec::new();
    let mut type_expr = base_type.clone();
    if first_stars > 0 {
        type_expr.push_str(&"*".repeat(first_stars));
    }
    results.push((alias.to_string(), type_expr));

    // Remaining comma-groups: each is just an alias with optional leading `*`
    for part in &parts[1..] {
        let part_words: Vec<&str> = part.split_whitespace().collect();
        let extra = part_words.iter().filter(|&&w| w == "*").count();
        let alias_raw = match part_words.iter().find(|&&w| w != "*") {
            Some(w) => w,
            None => continue,
        };
        let (alias, alias_stars) = extract_alias_token(alias_raw);
        if alias.is_empty() {
            continue;
        }
        let stars = alias_stars + extra;
        let mut type_expr = base_type.clone();
        if stars > 0 {
            type_expr.push_str(&"*".repeat(stars));
        }
        results.push((alias.to_string(), type_expr));
    }
    results
}

/// Extract `(identifier, pointer_count)` from a raw alias token like `"*PVOID"` or `"LPDWORD"`.
/// Returns `("", 0)` if the token is not a valid identifier.
fn extract_alias_token(raw: &str) -> (&str, usize) {
    let leading = raw.chars().take_while(|&c| c == '*').count();
    let without_leading = &raw[leading..];
    let alias = without_leading.trim_end_matches('*');
    let trailing = without_leading.len() - alias.len();
    if alias.is_empty() || !alias.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_') {
        return ("", 0);
    }
    (alias, leading + trailing)
}

/// Parse a comma-separated alias list (the `A, *B, C` part after `}` in a struct typedef)
/// against a given base type.
fn parse_alias_list(aliases_str: &str, base_type: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for part in aliases_str.split(',') {
        let part_words: Vec<&str> = part.split_whitespace().collect();
        let extra = part_words.iter().filter(|&&w| w == "*").count();
        let alias_raw = match part_words.iter().find(|&&w| w != "*") {
            Some(w) => w,
            None => continue,
        };
        let (alias, alias_stars) = extract_alias_token(alias_raw);
        if alias.is_empty() {
            continue;
        }
        let stars = alias_stars + extra;
        let mut type_expr = base_type.to_string();
        if stars > 0 {
            type_expr.push_str(&"*".repeat(stars));
        }
        results.push((alias.to_string(), type_expr));
    }
    results
}

// ── Struct body parsing ───────────────────────────────────────────────────────

/// Scan stripped source for `typedef struct/union Tag { … } Alias;` definitions
/// and return a `CStructDef` for each alias (non-pointer only) whose every field
/// resolves to a primitive `CType`.
fn parse_struct_bodies(
    stripped: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<CStructDef> {
    let mut result = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;

    while i < stripped.len() {
        if stripped[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&stripped[i..]) {
                let seg_before = stripped[seg_start..i].trim_start();
                let is_struct_typedef = seg_before.starts_with("typedef")
                    && (seg_before.contains(" struct ") || seg_before.contains(" union "));

                // `class Name { … }` or `struct Name { … }` (not typedef)
                let class_name = if !is_struct_typedef {
                    let w: Vec<&str> = seg_before.split_whitespace().collect();
                    if matches!(w.first().copied(), Some("class") | Some("struct"))
                        && w.len() >= 2
                        && w[1].chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        Some(w[1].to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };

                if is_struct_typedef {
                    let body = &stripped[i + 1..i + brace_end];
                    let rest = &stripped[i + brace_end + 1..];
                    if let Some(semi_pos) = rest.find(';') {
                        let aliases_str = rest[..semi_pos].trim();
                        if !aliases_str.is_empty() {
                            let fields = parse_struct_field_decls(body, custom, typedefs);
                            if !fields.is_empty() {
                                for (alias, ptr_suffix) in parse_alias_list(aliases_str, "") {
                                    if !ptr_suffix.contains('*') {
                                        result.push(CStructDef {
                                            name: alias,
                                            fields: fields.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(name) = class_name {
                    let body = &stripped[i + 1..i + brace_end];
                    let fields = parse_struct_field_decls(body, custom, typedefs);
                    if !fields.is_empty() {
                        result.push(CStructDef { name, fields });
                    }
                }
                i += brace_end + 1;
                seg_start = i;
                continue;
            }
        }

        if stripped[i..].starts_with(';') {
            seg_start = i + 1;
        }
        i += 1;
    }
    result
}

/// Parse field declarations inside a struct body.
///
/// Handles:
/// - `float x;` → one field
/// - `float x, y, z;` → three fields
/// - `int m[4][4];` → skipped (array declarators)
/// - Nested `struct { … }` → skipped
///
/// Returns only fields whose type resolves to a primitive `CType`.
fn parse_struct_field_decls(
    body: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Vec<(String, CType)> {
    let mut fields = Vec::new();
    let mut i = 0;
    let mut seg_start = 0;

    while i < body.len() {
        if body[i..].starts_with('{') {
            if let Some(end) = find_matching_brace(&body[i..]) {
                i += end + 1;
                seg_start = i;
                continue;
            }
        }
        if body[i..].starts_with(';') {
            let seg = body[seg_start..i].trim();
            if !seg.is_empty() {
                parse_field_segment(seg, custom, typedefs, &mut fields);
            }
            i += 1;
            seg_start = i;
            continue;
        }
        i += 1;
    }
    fields
}

/// Parse a single `;`-delimited field segment like `float x, y, z` or `int flags`.
/// Appends resolved `(name, CType)` pairs to `out`.
fn parse_field_segment(
    seg: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
    out: &mut Vec<(String, CType)>,
) {
    // Skip constructor / method declarations (contain parentheses).
    if seg.contains('(') {
        return;
    }
    let all_words: Vec<&str> = seg.split_whitespace().collect();
    // Strip a leading access-control specifier ("public:" / "private:" / "protected:").
    let start = if all_words
        .first()
        .map(|w| matches!(w.trim_end_matches(':'), "public" | "private" | "protected" | "virtual"))
        .unwrap_or(false)
    {
        1
    } else {
        0
    };
    let words = &all_words[start..];
    if words.len() < 2 {
        return;
    }

    // Split by comma: `float x, y, z` → `["float x", " y", " z"]`
    let seg = words.join(" ");
    let parts: Vec<&str> = seg.split(',').collect();
    let first_words: Vec<&str> = parts[0].split_whitespace().collect();
    if first_words.len() < 2 {
        return;
    }

    // Last word of the first part is the field name (possibly prefixed with `*`)
    let raw_last = *first_words.last().unwrap();
    // Skip array fields like `m[4][4]`
    if raw_last.contains('[') || raw_last.contains('(') {
        return;
    }
    let (first_name, first_stars) = extract_alias_token(raw_last);
    if first_name.is_empty() {
        return;
    }

    let type_words: Vec<&str> = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w != "*")
        .copied()
        .collect();
    let standalone_stars = first_words[..first_words.len() - 1]
        .iter()
        .filter(|&&w| w == "*")
        .count();
    let base_str = type_words.join(" ");
    let total_stars = first_stars + standalone_stars;
    let type_str = if total_stars > 0 {
        format!("{}{}", base_str, "*".repeat(total_stars))
    } else {
        base_str.clone()
    };

    match parse_c_type_str(&type_str, custom, typedefs) {
        Ok(ctype) => {
            out.push((first_name.to_string(), ctype.clone()));
            for part in &parts[1..] {
                let pw: Vec<&str> = part.split_whitespace().collect();
                let extra_stars = pw.iter().filter(|&&w| w == "*").count();
                let raw_name = match pw.iter().find(|&&w| w != "*") {
                    Some(w) => w,
                    None => continue,
                };
                if raw_name.contains('[') || raw_name.contains('(') {
                    continue;
                }
                let (alias, alias_stars) = extract_alias_token(raw_name);
                if alias.is_empty() {
                    continue;
                }
                let stars = alias_stars + extra_stars;
                if stars == 0 {
                    out.push((alias.to_string(), ctype.clone()));
                } else if let Ok(ptr_ct) = parse_c_type_str(
                    &format!("{}{}", base_str, "*".repeat(stars)),
                    custom,
                    typedefs,
                ) {
                    out.push((alias.to_string(), ptr_ct));
                }
            }
        }
        Err(_) => {} // skip fields with unparseable types
    }
}

/// Map a `CType` to the Rust primitive type string used inside a `#[repr(C)]` struct.
/// Returns `None` for types that cannot be represented as a simple Rust field
/// (e.g. nested structs — represented as `ByValueStruct`).
fn ctype_to_rust_field(ct: &CType) -> Option<&'static str> {
    match ct {
        CType::Int | CType::Bool => Some("i32"),
        CType::Long => Some("i64"),
        CType::Float => Some("f32"),
        CType::Double => Some("f64"),
        CType::VoidPtr
        | CType::CharPtr
        | CType::OpaqueStructPtr { .. }
        | CType::Ptr { .. } => Some("*mut i8"),
        // Nested struct-by-value or function pointer → not representable
        CType::ByValueStruct { .. } | CType::FnPtr => None,
        CType::Void => None,
    }
}

/// Generate the Rust expression that reads a handle from the arena and converts
/// it to the Rust type for a struct field.
fn field_from_handle(ct: &CType, handle: &str) -> String {
    match ct {
        CType::Int => format!("((*CB).to_int)({handle}) as i32"),
        CType::Long => format!("((*CB).to_int)({handle})"),
        CType::Float => format!("((*CB).to_float)({handle}) as f32"),
        CType::Double => format!("((*CB).to_float)({handle})"),
        CType::Bool => format!("if {handle} == {TL_TRUE}i64 {{ 1i32 }} else {{ 0i32 }}"),
        _ => format!("{handle} as *mut i8"),
    }
}

/// Generate the Rust expression that wraps a struct field value into a tl handle.
fn field_to_handle(ct: &CType, val: &str) -> String {
    match ct {
        CType::Int => format!("((*CB).make_int)({val} as i64)"),
        CType::Long => format!("((*CB).make_int)({val})"),
        CType::Float => format!("((*CB).make_float)({val} as f64)"),
        CType::Double => format!("((*CB).make_float)({val})"),
        CType::Bool => {
            format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}")
        }
        _ => format!("{val} as i64"),
    }
}

/// Returns `true` if every field of the struct maps to a Rust primitive
/// (i.e. the struct can be represented as `#[repr(C)]` without nested structs).
fn struct_is_bridgeable(def: &CStructDef) -> bool {
    !def.fields.is_empty() && def.fields.iter().all(|(_, ct)| ctype_to_rust_field(ct).is_some())
}

/// Transitively resolve all alias entries until stable (max 32 passes).
/// After this, every value in the map is either a C++ primitive string or an
/// unresolvable type expression (struct name, unknown typedef, etc.).
fn resolve_typedef_map(map: &mut HashMap<String, String>) {
    for _ in 0..32 {
        let mut changed = false;
        let snapshot: HashMap<String, String> = map.clone();
        for val in map.values_mut() {
            let resolved = resolve_one_typedef(val, &snapshot);
            if *val != resolved {
                *val = resolved;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Attempt one resolution step: look up the base type of `expr` in `map`.
/// Handles pointer types: `"DWORD *"` → look up `"DWORD"`, prepend `*` to result.
fn resolve_one_typedef(expr: &str, map: &HashMap<String, String>) -> String {
    // Strip qualifiers for lookup, then restore pointer decoration
    let tokens: Vec<&str> = expr
        .split_whitespace()
        .filter(|t| !matches!(*t, "const" | "volatile" | "restrict" | "__restrict"))
        .collect();
    let has_ptr = tokens.last() == Some(&"*");
    let base_tokens: &[&str] = if has_ptr { &tokens[..tokens.len() - 1] } else { &tokens };
    let base_str = base_tokens.join(" ");
    if let Some(resolved_base) = map.get(&base_str) {
        if has_ptr {
            // Append one `*` to the resolved base (preserving its own trailing `*`)
            return format!("{} *", resolved_base.trim_end_matches('*').trim());
        }
        return resolved_base.clone();
    }
    expr.to_string()
}

/// Recursively scan a C/C++ scope for function declarations.
///
/// - `extern "C" { ... }` → recurse with the same inherited namespace
/// - `namespace X { ... }` → recurse with `ns = Some("X")`
/// - Any `{ ... }` that is neither of the above → skip (struct / class / union body)
/// - `;` at the current level → flush the accumulated text as a declaration candidate
fn scan_scope(text: &str, ns: Option<String>, decls: &mut Vec<(String, Option<String>)>) {
    let mut i = 0;
    let mut seg_start = 0; // start of the current `;`-delimited declaration

    while i < text.len() {
        // Check whether we are at a word boundary (needed for keyword detection)
        let at_boundary = i == 0 || {
            let prev = text[..i].chars().last().unwrap_or(' ');
            !prev.is_alphanumeric() && prev != '_'
        };

        if at_boundary {
            // ── `extern "C" { ... }` ──────────────────────────────────────────
            if text[i..].starts_with("extern") {
                let after_kw = text[i + 6..].trim_start();
                if after_kw.starts_with('"') {
                    if let Some(qclose) = after_kw[1..].find('"') {
                        let rest = after_kw[2 + qclose..].trim_start();
                        if rest.starts_with('{') {
                            if let Some(brace_end) = find_matching_brace(rest) {
                                // Recurse: extern "C" is linkage-only, inherits namespace
                                scan_scope(&rest[1..brace_end], ns.clone(), decls);
                                let consumed = (text.len() - rest.len()) - i + brace_end + 1;
                                i += consumed;
                                seg_start = i;
                                continue;
                            }
                        }
                    }
                }
            }

            // ── `namespace X { ... }` ─────────────────────────────────────────
            if text[i..].starts_with("namespace") {
                let after_kw = text[i + 9..].trim_start();
                let name_end = after_kw
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(after_kw.len());
                let ns_name = &after_kw[..name_end];
                if !ns_name.is_empty() {
                    let rest = after_kw[name_end..].trim_start();
                    if rest.starts_with('{') {
                        if let Some(brace_end) = find_matching_brace(rest) {
                            scan_scope(&rest[1..brace_end], Some(ns_name.to_string()), decls);
                            let consumed = (text.len() - rest.len()) - i + brace_end + 1;
                            i += consumed;
                            seg_start = i;
                            continue;
                        }
                    }
                }
            }
        }

        // ── `{ ... }` that is not extern/namespace → skip (struct / class body) ──
        if text[i..].starts_with('{') {
            if let Some(brace_end) = find_matching_brace(&text[i..]) {
                i += brace_end + 1;
                seg_start = i;
                continue;
            }
        }

        // ── `;` → end of a declaration, emit if it looks like a function ────────
        if text[i..].starts_with(';') {
            let segment = &text[seg_start..i];
            let normalized = segment.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty()
                && normalized.contains('(')
                && normalized.contains(')')
                && !normalized.split_whitespace().any(|w| w == "typedef")
            {
                decls.push((normalized, ns.clone()));
            }
            i += 1;
            seg_start = i;
            continue;
        }

        i += 1;
    }
}

/// Parse a function declaration that may have a leading `extern` keyword,
/// default-parameter macros (e.g. `DEFAULTPARAM(= NULL)`), and a namespace.
fn parse_fn_decl_ns(
    decl: &str,
    namespace: Option<String>,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Result<CFnSig, String> {
    // Strip leading `extern` keyword
    let decl = decl.trim();
    let decl = if decl.starts_with("extern") {
        let after = decl["extern".len()..].trim_start();
        // Don't strip if it's `extern "C"` — those are handled by the block parser
        if after.starts_with('"') {
            decl
        } else {
            after
        }
    } else {
        decl
    };

    // Find outer parentheses (function name is before first `(`)
    let paren_open = decl.find('(').ok_or("no '('")?;
    let paren_close = decl.rfind(')').ok_or("no ')'")?;
    if paren_close < paren_open {
        return Err("')' before '('".to_string());
    }

    let before_paren = decl[..paren_open].trim();
    let params_raw = &decl[paren_open + 1..paren_close];

    // Strip parameter-position macros like `DEFAULTPARAM(= NULL)`
    let params_clean = strip_parameter_macros(params_raw);

    let (ret_str, name) = split_type_and_name(before_paren)?;
    // Skip names that look like C++ operators or are empty
    if name.is_empty() || name.contains("operator") || name.starts_with('~') {
        return Err("not a plain function".to_string());
    }
    let ret = parse_c_type_str(ret_str.trim(), custom, typedefs)?;

    let mut params: Vec<(String, CType)> = Vec::new();
    // Track which raw (pre-strip) params have DEFAULTPARAM to compute n_required
    let raw_param_list = split_params(params_raw);
    let params_str = params_clean.trim();
    if !params_str.is_empty() && params_str != "void" {
        for (idx, p) in split_params(params_str).iter().enumerate() {
            let p = p.trim();
            if p.is_empty() || p == "..." {
                continue;
            }
            // Detect function pointer parameters before attempting type/name split
            if typedef_contains_fn_ptr(p) {
                params.push((format!("_p{idx}"), CType::FnPtr));
                continue;
            }
            let (type_str, pname) = match split_type_and_name(p) {
                Ok(r) => r,
                Err(_) => (p.to_string(), format!("_p{idx}")),
            };
            match parse_c_type_str(type_str.trim(), custom, typedefs) {
                Ok(ct) => params.push((pname, ct)),
                Err(e) => return Err(format!("param {pname}: {e}")),
            }
        }
    }

    // n_required = index of first param that has DEFAULTPARAM in the raw declaration
    // (params after the first optional one are also optional)
    let n_required = raw_param_list
        .iter()
        .position(|p| p.contains(DEFAULTPARAM_MACRO))
        .unwrap_or(params.len());
    // n_required must not exceed actual parsed params count
    let n_required = n_required.min(params.len());

    Ok(CFnSig {
        name,
        params,
        ret,
        namespace,
        n_required,
    })
}

/// Split a parameter list by `,`, respecting nested `()`.
fn split_params(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '[' | '<' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '>' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                result.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// Remove `IDENTIFIER(...)` macro-call patterns from a parameter string.
/// Used to strip default-parameter macros like `DEFAULTPARAM(= NULL)`.
fn strip_parameter_macros(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphabetic() || c == '_' {
            let mut ident = c.to_string();
            while matches!(chars.peek(), Some(nc) if nc.is_alphanumeric() || *nc == '_') {
                ident.push(chars.next().unwrap());
            }
            // Skip whitespace between identifier and possible `(`
            let mut spaces = String::new();
            while matches!(chars.peek(), Some(' ') | Some('\t')) {
                spaces.push(chars.next().unwrap());
            }
            if chars.peek() == Some(&'(') {
                // This looks like a macro call — skip the balanced `(...)`
                chars.next(); // consume `(`
                let mut depth = 1usize;
                for mc in chars.by_ref() {
                    match mc {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                // The macro call is gone — don't emit anything
            } else {
                result.push_str(&ident);
                result.push_str(&spaces);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Strip C/C++ comments and resolve `#ifdef`/`#ifndef`/`#if defined(…)`/
/// `#elif defined(…)`/`#else`/`#endif` preprocessor conditionals.
///
/// Lines excluded by a false condition are replaced with blank lines (preserving
/// line count). All other `#` directives are also replaced with blank lines.
/// Used when loading system typedefs so that platform-conditional definitions
/// (e.g. `#elif defined(_WIN64) … typedef unsigned __int64 ULONG_PTR`) are
/// included when the corresponding macro is listed in `macros`.
fn strip_and_preprocess(src: &str, macros: &[String]) -> String {
    // Stack frames: (branch_active, any_branch_taken, parent_active)
    let mut cond_stack: Vec<(bool, bool, bool)> = Vec::new();
    let mut in_block_comment = false;
    let mut out = String::with_capacity(src.len());

    let currently_active = |stack: &[(bool, bool, bool)]| stack.iter().all(|(a, _, _)| *a);
    let macro_defined = |name: &str| macros.iter().any(|m| m == name);

    // Parse `defined(MACRO)` or `!defined(MACRO)` from a condition expression.
    // Returns `Some((name, negated))` for simple single-macro conditions.
    let parse_defined_cond = |expr: &str| -> Option<(String, bool)> {
        let expr = expr.trim();
        let (expr, negated) = if let Some(rest) = expr.strip_prefix('!') {
            (rest.trim(), true)
        } else {
            (expr, false)
        };
        let rest = expr.strip_prefix("defined")?.trim();
        let name = if let Some(inner) = rest.strip_prefix('(') {
            inner.split(')').next()?.trim().to_string()
        } else {
            rest.split_whitespace().next()?.to_string()
        };
        if name.is_empty() { return None; }
        Some((name, negated))
    };

    // Evaluate a condition expression (handles `||`/`&&`-joined `defined()` clauses).
    // Unknown sub-expressions (function-like macros, integer comparisons, etc.) default
    // to `true` so that content gated by unrecognised conditions is still included —
    // matching the old `strip_comments` behaviour of including everything.
    let eval_cond = |expr: &str| -> bool {
        if expr.contains("||") {
            return expr.split("||").any(|part| {
                if let Some((name, neg)) = parse_defined_cond(part.trim()) {
                    macro_defined(&name) != neg
                } else { true } // unknown sub-term → include
            });
        }
        if expr.contains("&&") {
            return expr.split("&&").all(|part| {
                if let Some((name, neg)) = parse_defined_cond(part.trim()) {
                    macro_defined(&name) != neg
                } else { true } // unknown sub-term → include
            });
        }
        if let Some((name, neg)) = parse_defined_cond(expr) {
            macro_defined(&name) != neg
        } else {
            true // unknown condition → include (same as old strip_comments)
        }
    };

    for line in src.lines() {
        // Strip block comment spans from the visible part of the line.
        let (visible, still_in_comment) = {
            let mut s = String::new();
            let mut in_cmt = in_block_comment;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if in_cmt {
                    if c == '*' && chars.peek() == Some(&'/') { chars.next(); in_cmt = false; }
                } else if c == '/' && chars.peek() == Some(&'*') {
                    chars.next(); in_cmt = true;
                } else if c == '/' && chars.peek() == Some(&'/') {
                    break; // rest is a line comment
                } else {
                    s.push(c);
                }
            }
            (s, in_cmt)
        };
        in_block_comment = still_in_comment;

        let trimmed = visible.trim_start();
        if trimmed.starts_with('#') {
            let dir = trimmed[1..].trim_start();
            if dir.starts_with("ifdef") {
                let name = dir["ifdef".len()..].split_whitespace().next().unwrap_or("");
                let parent = currently_active(&cond_stack);
                let active = parent && macro_defined(name);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("ifndef") {
                let name = dir["ifndef".len()..].split_whitespace().next().unwrap_or("");
                let parent = currently_active(&cond_stack);
                let active = parent && !macro_defined(name);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("if ") || dir == "if" {
                let expr = dir["if".len()..].trim();
                let parent = currently_active(&cond_stack);
                let active = parent && eval_cond(expr);
                cond_stack.push((active, active, parent));
            } else if dir.starts_with("elif") {
                let expr = dir["elif".len()..].trim();
                if let Some(frame) = cond_stack.last_mut() {
                    let (_, any_taken, parent) = *frame;
                    if any_taken {
                        frame.0 = false;
                    } else {
                        let active = parent && eval_cond(expr);
                        frame.0 = active;
                        if active { frame.1 = true; }
                    }
                }
            } else if dir.starts_with("else") {
                if let Some(frame) = cond_stack.last_mut() {
                    let (_, any_taken, parent) = *frame;
                    let active = !any_taken && parent;
                    frame.0 = active;
                    if active { frame.1 = true; }
                }
            } else if dir.starts_with("endif") {
                cond_stack.pop();
            }
            out.push('\n'); // always emit blank line for any # directive
            continue;
        }

        if currently_active(&cond_stack) {
            out.push_str(&visible);
        }
        out.push('\n');
    }
    out
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    // line comment — skip to end of line
                    for c2 in chars.by_ref() {
                        if c2 == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    // block comment — skip to */
                    while let Some(c2) = chars.next() {
                        if c2 == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                        if c2 == '\n' {
                            out.push('\n');
                        }
                    }
                }
                _ => out.push(c),
            }
        } else if c == '#' {
            // preprocessor directive — skip to end of line
            for c2 in chars.by_ref() {
                if c2 == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a C declaration like `"int *foo"` or `"const int* bar"` into `("int*", "foo")`.
///
/// Stars (`*`) attached to the name are moved to the type string so that
/// `parse_c_type_str` can see the full pointer decoration.
fn split_type_and_name(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("cannot split type/name from '{s}'"));
    }

    // Last word may be `*foo`, `**foo`, or just `foo`
    let raw_last = *parts.last().unwrap();
    let star_count = raw_last.chars().take_while(|&c| c == '*').count();
    let name = &raw_last[star_count..];
    if name.is_empty() {
        return Err(format!("cannot extract name from '{s}'"));
    }

    // Stars from the name token belong to the type
    let type_base = parts[..parts.len() - 1].join(" ");
    let type_str = if star_count > 0 {
        format!("{}{}", type_base, "*".repeat(star_count))
    } else {
        type_base
    };

    Ok((type_str, name.to_string()))
}

/// Map a C type string (possibly with trailing `*`) to `CType`.
///
/// Map a C type string to `CType`.
///
/// Resolution order:
///   1. `custom` — user overrides (C alias → tl primitive name)
///   2. `typedefs` — system typedef aliases resolved by `load_system_typedefs`
///   3. Built-in C/C++ primitive names (`int`, `long`, `float`, …)
///
/// Pointer decoration (`*`) in the input is handled before the lookup.
/// Functions whose types cannot be resolved are signalled with `Err`.
fn parse_c_type_str(
    s: &str,
    custom: &HashMap<String, String>,
    typedefs: &HashMap<String, String>,
) -> Result<CType, String> {
    let s = s.trim();

    // Count and strip trailing '*'
    let ptr_count = s.chars().rev().take_while(|&c| c == '*').count();
    let base = s[..s.len() - ptr_count].trim();

    // Detect 'const' qualifier
    let is_const = base.split_whitespace().any(|t| t == "const");

    // Filter qualifiers and MIDL annotations (e.g. `[public]`)
    let tokens: Vec<&str> = base
        .split_whitespace()
        .filter(|t| {
            !matches!(*t, "const" | "volatile" | "restrict" | "__restrict")
                && !t.starts_with('[') // strip MIDL annotations like [public], [unique]
        })
        .collect();

    // Filter signed/unsigned for primitive lookup
    let core: Vec<&str> = tokens
        .iter()
        .filter(|t| !matches!(**t, "unsigned" | "signed"))
        .copied()
        .collect();

    let core_str = core.join(" ");

    // ── 1. User custom type mappings ─────────────────────────────────────────
    if let Some(tl_type) = custom.get(&core_str) {
        let base_ct = match tl_type.as_str() {
            "int" => CType::Int,
            "long" => CType::Long,
            "float" => CType::Float,
            "double" => CType::Double,
            "bool" => CType::Bool,
            "void" => CType::Void,
            other => return Err(format!("custom_type_map: unknown tl type '{other}'")),
        };
        return Ok(if ptr_count > 0 {
            CType::Ptr { inner: Box::new(base_ct), mutable: !is_const }
        } else {
            base_ct
        });
    }

    // ── 2. System typedef aliases ────────────────────────────────────────────
    // Look up the base type name in the pre-resolved typedef map.
    // The resolved value may itself carry a trailing `*` (e.g. HWND → "void*").
    if let Some(resolved) = typedefs.get(&core_str) {
        let resolved_ptrs = resolved.chars().rev().take_while(|&c| c == '*').count();
        let total_ptrs = ptr_count + resolved_ptrs;
        if total_ptrs > 1 {
            return Err("multi-level pointer".to_string());
        }
        let resolved_base = resolved[..resolved.len() - resolved_ptrs].trim();
        let combined = if total_ptrs == 1 {
            format!("{} *", resolved_base)
        } else {
            resolved_base.to_string()
        };
        // Recurse with empty typedefs to avoid infinite loops
        return parse_c_type_str(combined.trim(), custom, &HashMap::new());
    }

    // ── 3. C/C++ built-in primitives ─────────────────────────────────────────
    if ptr_count > 0 {
        if ptr_count > 1 {
            return Err("multi-level pointer".to_string());
        }
        // char* / wchar_t* → tl str
        if matches!(core.as_slice(), ["char"] | ["wchar_t"]) {
            return Ok(CType::CharPtr);
        }
        // void* → opaque integer handle
        if core == ["void"] {
            return Ok(CType::VoidPtr);
        }
        let inner = match core.as_slice() {
            ["bool"] | ["_Bool"] => CType::Bool,
            ["float"] => CType::Float,
            ["double"] | ["long", "double"] => CType::Double,
            ["char"] | ["short"] | ["int"]
            | ["int8_t"] | ["int16_t"] | ["int32_t"]
            | ["uint8_t"] | ["uint16_t"] | ["uint32_t"] => CType::Int,
            ["long"] | ["long", "int"] | ["long", "long"] | ["long", "long", "int"]
            | ["int64_t"] | ["uint64_t"]
            | ["size_t"] | ["ptrdiff_t"] | ["intptr_t"] | ["uintptr_t"]
            | ["__int64"] | ["__int3264"] => CType::Long,
            other => {
                let type_name = other.join(" ");
                if type_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ') {
                    return Ok(CType::OpaqueStructPtr { type_name, mutable: !is_const });
                }
                return Err(format!("unknown pointer base type '{}'", type_name));
            }
        };
        return Ok(CType::Ptr { inner: Box::new(inner), mutable: !is_const });
    }

    match core.as_slice() {
        ["void"] => Ok(CType::Void),
        ["bool"] | ["_Bool"] => Ok(CType::Bool),
        ["float"] => Ok(CType::Float),
        ["double"] | ["long", "double"] => Ok(CType::Double),
        ["char"] | ["short"] | ["int"] | ["wchar_t"]
        | ["int8_t"] | ["int16_t"] | ["int32_t"]
        | ["uint8_t"] | ["uint16_t"] | ["uint32_t"] => Ok(CType::Int),
        ["long"] | ["long", "int"] | ["long", "long"] | ["long", "long", "int"]
        | ["int64_t"] | ["uint64_t"]
        | ["size_t"] | ["ptrdiff_t"] | ["intptr_t"] | ["uintptr_t"]
        | ["__int64"] | ["__int3264"] => Ok(CType::Long),
        other => {
            let type_name = other.join(" ");
            if type_name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ') {
                // Struct/union passed by value: bridge via pointer dereferencing in the shim
                return Ok(CType::ByValueStruct { type_name });
            }
            eprintln!(
                "CppBridge: unknown C type '{}', skipping function",
                type_name
            );
            Err(format!("unknown C type '{}'", type_name))
        }
    }
}

// ── Generated Rust source header ─────────────────────────────────────────────

const WRAPPER_HEADER: &str = r#"// Auto-generated cpp bridge — do not edit.
#![allow(dead_code, unused_variables, non_snake_case, unused_unsafe, static_mut_refs)]

#[repr(C)]
struct TlCallbacks {
    make_int:      unsafe extern "C" fn(i64) -> i64,
    make_float:    unsafe extern "C" fn(f64) -> i64,
    make_bool:     unsafe extern "C" fn(i32) -> i64,
    make_str:      unsafe extern "C" fn(*const u8, i32) -> i64,
    make_list:     unsafe extern "C" fn(*const i64, i32) -> i64,
    make_tuple:    unsafe extern "C" fn(*const i64, i32) -> i64,
    make_dict:     unsafe extern "C" fn(*const i64, *const i64, i32) -> i64,
    make_none:     unsafe extern "C" fn() -> i64,
    is_truthy:     unsafe extern "C" fn(i64) -> i32,
    binop:         unsafe extern "C" fn(i32, i64, i64) -> i64,
    unop:          unsafe extern "C" fn(i32, i64) -> i64,
    call_fn:       unsafe extern "C" fn(i64, *const i64, i32) -> i64,
    get_attr:      unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    set_attr:      unsafe extern "C" fn(i64, *const u8, i32, i64),
    subscript:     unsafe extern "C" fn(i64, i64) -> i64,
    get_global:    unsafe extern "C" fn(*const u8, i32) -> i64,
    iter_from:     unsafe extern "C" fn(i64) -> i64,
    iter_next:     unsafe extern "C" fn(i64) -> i64,
    is_type:       unsafe extern "C" fn(i64, *const u8, i32) -> i64,
    arena_save:    unsafe extern "C" fn() -> u64,
    arena_compact: unsafe extern "C" fn(i64, u64) -> i64,
    compact_many:  unsafe extern "C" fn(*const i64, i32, u64, *mut i64),
    to_int:        unsafe extern "C" fn(i64) -> i64,
    to_float:      unsafe extern "C" fn(i64) -> f64,
    deep_copy:     unsafe extern "C" fn(i64) -> i64,
    to_cstr:       unsafe extern "C" fn(i64) -> *const u8,
    write_handle:  unsafe extern "C" fn(i64, i64),
}

static mut CB: *const TlCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn tl_init(cb: *const TlCallbacks) { CB = cb; }
"#;

// Platform loader inserted into dll-wrapper source.
// RTLD_LAZY is embedded as the literal `1` (standardized value on all POSIX platforms).
const PLATFORM_LOADER: &str = r#"
#[cfg(windows)]
mod _loader {
    extern "system" {
        fn LoadLibraryA(name: *const u8) -> usize;
        fn GetProcAddress(module: usize, name: *const u8) -> usize;
    }
    pub unsafe fn load(path: &str) -> usize {
        let p = format!("{path}\0");
        LoadLibraryA(p.as_ptr())
    }
    pub unsafe fn sym(lib: usize, name: &str) -> usize {
        let n = format!("{name}\0");
        GetProcAddress(lib, n.as_ptr())
    }
}
#[cfg(not(windows))]
mod _loader {
    extern "C" {
        fn dlopen(name: *const u8, flags: i32) -> usize;
        fn dlsym(handle: usize, name: *const u8) -> usize;
    }
    pub unsafe fn load(path: &str) -> usize {
        let p = format!("{path}\0");
        dlopen(p.as_ptr(), 1) // RTLD_LAZY
    }
    pub unsafe fn sym(lib: usize, name: &str) -> usize {
        let n = format!("{name}\0");
        dlsym(lib, n.as_ptr())
    }
}
static mut _DLL_HANDLE: usize = 0;
unsafe fn _dll() -> usize {
    if _DLL_HANDLE == 0 { panic!("cpp bridge DLL not loaded"); }
    _DLL_HANDLE
}
"#;

// ── Code generators ───────────────────────────────────────────────────────────

/// Generate a standalone Rust wrapper for a **dynamic** DLL.
///
/// The wrapper uses OS-level `LoadLibraryA`/`dlopen` to load `dll_path` at
/// `tl_init` time, then resolves each function via `GetProcAddress`/`dlsym`
/// on every call.  All functions follow the `{name}_tl(argc, argv) -> i64`
/// convention so the existing `NativeFnRef` machinery can drive them.
pub fn gen_dll_wrapper(
    dll_path: &str,
    sigs: &[CFnSig],
    struct_defs: &[CStructDef],
) -> String {
    let mut src = String::new();
    src.push_str(WRAPPER_HEADER);

    // Emit #[repr(C)] struct definitions for every bridgeable C struct.
    for def in struct_defs {
        if !struct_is_bridgeable(def) {
            continue;
        }
        let name = &def.name;
        src.push_str(&format!("#[repr(C)] #[derive(Copy, Clone)] struct _Struct_{name} {{ "));
        for (fname, ftype) in &def.fields {
            src.push_str(&format!("{fname}: {}, ", ctype_to_rust_field(ftype).unwrap()));
        }
        src.push_str("}\n");
    }

    src.push_str(PLATFORM_LOADER);

    // Override tl_init to also load the original DLL
    let escaped = dll_path.replace('\\', "\\\\");
    src.push_str(&format!(
        r#"
const _DLL_PATH: &str = "{escaped}";

#[no_mangle]
pub unsafe extern "C" fn tl_init_cpp(cb: *const TlCallbacks) {{
    CB = cb;
    _DLL_HANDLE = _loader::load(_DLL_PATH);
    if _DLL_HANDLE == 0 {{ panic!("cpp bridge: cannot load DLL: {{}}", _DLL_PATH); }}
}}
"#
    ));

    // Re-export tl_init so the standard tl_init call also works
    src.push_str(
        r#"
// Called by the interpreter at module load — initialise callbacks then load DLL.
#[no_mangle]
pub unsafe extern "C" fn tl_init_bridge(cb: *const TlCallbacks) {
    tl_init(cb);
    tl_init_cpp(cb);
}
"#,
    );

    // Build a name→def map for O(1) lookup inside gen_dll_fn.
    let struct_map: HashMap<String, &CStructDef> =
        struct_defs.iter().filter(|d| struct_is_bridgeable(d)).map(|d| (d.name.clone(), d)).collect();

    for sig in sigs {
        src.push_str(&gen_dll_fn(sig, &struct_map));
    }
    src
}

fn gen_dll_fn(sig: &CFnSig, struct_defs: &HashMap<String, &CStructDef>) -> String {
    let mut s = String::new();
    let n = &sig.name;

    // Build the C function-pointer type — pointers use their Rust extern type
    let param_types: Vec<String> = sig
        .params
        .iter()
        .map(|(_, t)| t.rust_extern_type())
        .collect();
    let fn_type = if sig.ret == CType::Void {
        format!("unsafe extern \"C\" fn({})", param_types.join(", "))
    } else {
        format!(
            "unsafe extern \"C\" fn({}) -> {}",
            param_types.join(", "),
            sig.ret.rust_extern_type()
        )
    };

    s.push_str(&format!(
        r#"
#[no_mangle]
pub unsafe extern "C" fn {n}_tl(argv: *const i64, _argc: i32) -> i64 {{
    type _F = {fn_type};
    let _fp: _F = std::mem::transmute(_loader::sym(_dll(), "{n}"));
"#
    ));

    // Marshal each argument
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        let h = format!("*argv.offset({i})");
        match ptype {
            CType::Ptr { inner, mutable: true } => {
                s.push_str(&format!(
                    "    let mut _ptr_{pname}: {} = {};\n",
                    inner.rust_extern_type(),
                    inner.from_handle(&h)
                ));
            }
            CType::Ptr { inner, mutable: false } => {
                s.push_str(&format!(
                    "    let _ptr_{pname}: {} = {};\n",
                    inner.rust_extern_type(),
                    inner.from_handle(&h)
                ));
            }
            CType::CharPtr => {
                s.push_str(&format!("    let _{pname} = ((*CB).to_cstr)({h});\n"));
            }
            CType::ByValueStruct { type_name } => {
                if let Some(def) = struct_defs.get(type_name.as_str()) {
                    // Read each field from the tl instance via get_attr, build C struct
                    for (fname, ftype) in &def.fields {
                        let attr_h = format!("((*CB).get_attr)({h}, b\"{fname}\\0\".as_ptr(), {})", fname.len());
                        s.push_str(&format!(
                            "    let _{pname}_{fname}: {} = {};\n",
                            ctype_to_rust_field(ftype).unwrap(),
                            field_from_handle(ftype, &attr_h)
                        ));
                    }
                    let field_inits: String = def.fields.iter()
                        .map(|(f, _)| format!("{f}: _{pname}_{f}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    s.push_str(&format!(
                        "    let _buf_{pname} = _Struct_{type_name} {{ {field_inits} }};\n"
                    ));
                    s.push_str(&format!(
                        "    let _{pname} = &_buf_{pname} as *const _Struct_{type_name} as *mut i8;\n"
                    ));
                } else {
                    s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
                }
            }
            _ => {
                s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
            }
        }
    }

    // Build call argument list
    let args: Vec<String> = sig
        .params
        .iter()
        .map(|(pname, ptype)| match ptype {
            CType::Ptr { mutable: true, .. } => format!("&mut _ptr_{pname}"),
            CType::Ptr { mutable: false, .. } => format!("&_ptr_{pname}"),
            _ => format!("_{pname}"),
        })
        .collect();

    // Invoke and convert return value
    if sig.ret == CType::Void {
        s.push_str(&format!("    _fp({});\n", args.join(", ")));
    } else if let CType::ByValueStruct { type_name } = &sig.ret {
        s.push_str(&format!("    let _raw_r = _fp({});\n", args.join(", ")));
        if let Some(def) = struct_defs.get(type_name.as_str()) {
            // Read fields from the C buffer and construct a tl class instance
            s.push_str(&format!("    let _rp = _raw_r as *const _Struct_{type_name};\n"));
            for (fname, ftype) in &def.fields {
                s.push_str(&format!(
                    "    let _{fname}_h = {};\n",
                    field_to_handle(ftype, &format!("(*_rp).{fname}"))
                ));
            }
            let n_fields = def.fields.len();
            let args_items: String = def.fields.iter()
                .map(|(f, _)| format!("_{f}_h"))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!("    let _ctor_args = [{args_items}];\n"));
            let nlen = type_name.len();
            s.push_str(&format!(
                "    let _cls = ((*CB).get_global)(b\"{type_name}\\0\".as_ptr(), {nlen});\n"
            ));
            s.push_str(&format!(
                "    let _ret = if _cls != 0 {{ ((*CB).call_fn)(_cls, _ctor_args.as_ptr(), {n_fields}) }} else {{ 0i64 }};\n"
            ));
        } else {
            s.push_str(&format!("    let _ret = {};\n", sig.ret.to_handle("_raw_r")));
        }
    } else {
        s.push_str(&format!("    let _r = _fp({});\n", args.join(", ")));
        s.push_str(&format!("    let _ret = {};\n", sig.ret.to_handle("_r")));
    }

    // Write-back for mutable pointer params (after call, before return)
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        if let CType::Ptr { inner, mutable: true } = ptype {
            let h = format!("*argv.offset({i})");
            let new_h = inner.to_handle(&format!("_ptr_{pname}"));
            s.push_str(&format!("    ((*CB).write_handle)({h}, {new_h});\n"));
        }
    }

    if sig.ret == CType::Void {
        s.push_str("    0i64\n");
    } else {
        s.push_str("    _ret\n");
    }
    s.push_str("}\n");
    s
}

// ── Compiler ──────────────────────────────────────────────────────────────────

/// Compile `rust_src` to a cdylib and return the raw DLL bytes.
///
/// `extra_link_dirs` are passed as `-L` flags (used by `cpp-lib` to locate
/// the static library).
pub fn compile_wrapper(rust_src: &str, extra_link_dirs: &[PathBuf]) -> Result<Vec<u8>, String> {
    let tmp_dir = std::env::temp_dir();
    let rs_path = tmp_dir.join(TMP_RS_NAME);
    let ext = crate::partial_compiler::native_lib_ext();
    let dll_path = tmp_dir.join(format!("{TMP_DLL_STEM}.{ext}"));

    std::fs::write(&rs_path, rust_src)
        .map_err(|e| format!("CppImport: cannot write wrapper source: {e}"))?;

    let mut cmd = Command::new("rustc");
    cmd.args([
        "--edition",
        RUSTC_EDITION,
        "--crate-type",
        "cdylib",
        "-C",
        &format!("opt-level={RUSTC_OPT_LEVEL}"),
    ]);
    for dir in extra_link_dirs {
        cmd.arg("-L").arg(dir);
    }
    // On non-Windows, ensure libdl is linked for dlopen/dlsym
    #[cfg(not(windows))]
    cmd.args(["-l", "dl"]);

    cmd.arg("-o").arg(&dll_path).arg(&rs_path);

    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "rustc not found in PATH (required for import[cpp-dll] / import[cpp-lib])".to_string()
        } else {
            format!("cannot run rustc: {e}")
        }
    })?;

    let _ = std::fs::remove_file(&rs_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("rustc failed compiling cpp bridge:\n{stderr}"));
    }

    let dll_bytes = std::fs::read(&dll_path)
        .map_err(|e| format!("CppImport: cannot read compiled wrapper: {e}"))?;
    let _ = std::fs::remove_file(&dll_path);

    Ok(dll_bytes)
}

// ── MSVC shim (Windows only) ──────────────────────────────────────────────────

/// Paths to the MSVC toolchain.
pub struct MsvcPaths {
    pub vcvarsall: PathBuf,
}

/// Built-in Visual Studio installation candidates searched when no explicit
/// `msvc` path is given in `tl_config.json`.
const MSVC_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Enterprise\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Professional\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2017\Enterprise\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2017\Professional\VC\Auxiliary\Build\vcvarsall.bat",
    r"C:\Program Files (x86)\Microsoft Visual Studio\2017\Community\VC\Auxiliary\Build\vcvarsall.bat",
];

/// Search for `vcvarsall.bat`.
///
/// Checks `extra_paths` (from `tl_config.json` → `msvc_search_paths`) first,
/// then falls back to the built-in `MSVC_CANDIDATES` list.
/// Returns `None` if no installation is found.
pub fn find_msvc_vcvarsall(extra_paths: &[String]) -> Option<MsvcPaths> {
    for p in extra_paths {
        if Path::new(p).exists() {
            return Some(MsvcPaths {
                vcvarsall: PathBuf::from(p),
            });
        }
    }
    for path in MSVC_CANDIDATES {
        if Path::new(path).exists() {
            return Some(MsvcPaths {
                vcvarsall: PathBuf::from(path),
            });
        }
    }
    None
}

/// Generate the MSVC shim C++ source.
///
/// Each `CFnSig::namespace` is used per-function so multiple namespaces can
/// coexist (though in practice all DxLib functions share one namespace).
///
/// `win32_lean_and_mean` controls whether `#define WIN32_LEAN_AND_MEAN` is
/// emitted before the Windows header includes.
pub fn gen_cpp_shim_source(
    sigs: &[CFnSig],
    header_name: &str,
    precompile_macros: &[String],
    win32_lean_and_mean: bool,
) -> String {
    let mut src = String::new();

    if win32_lean_and_mean {
        src.push_str("#define WIN32_LEAN_AND_MEAN\n");
    }
    for m in precompile_macros {
        src.push_str(&format!("#define {m}\n"));
    }
    src.push_str("#include <windows.h>\n");
    src.push_str(&format!("#include \"{header_name}\"\n\n"));
    src.push_str("extern \"C\" {\n\n");

    for sig in sigs {
        let ret_c = sig.ret.c_type_str();

        let params: Vec<String> = sig
            .params
            .iter()
            .enumerate()
            .map(|(i, (name, ct))| {
                let n = if name.is_empty() {
                    format!("p{i}")
                } else {
                    name.clone()
                };
                format!("{} {n}", ct.c_type_str())
            })
            .collect();
        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };

        let args: Vec<String> = sig
            .params
            .iter()
            .enumerate()
            .map(|(i, (name, ct))| {
                let n = if name.is_empty() {
                    format!("p{i}")
                } else {
                    name.clone()
                };
                match ct {
                    // Cast away const on char pointers: DxLib output-buffer params use TCHAR* (non-const),
                    // but our shim declares them as const char*. The explicit (char*) cast suppresses
                    // C2664 when TCHAR=char without losing any type safety at the tl layer.
                    CType::CharPtr => format!("(char*){n}"),
                    // Opaque struct pointer: shim param is void*; cast to real type at the call site.
                    CType::OpaqueStructPtr { type_name, mutable } => {
                        if *mutable {
                            format!("({}*){n}", type_name)
                        } else {
                            format!("(const {}*){n}", type_name)
                        }
                    }
                    // By-value struct: shim param is void*; dereference to pass by value.
                    CType::ByValueStruct { type_name } => {
                        format!("*({type_name}*){n}")
                    }
                    // Function pointer: pass opaque void* as-is (cast to fn ptr type at C++ level).
                    CType::FnPtr => n,
                    _ => n,
                }
            })
            .collect();
        let args_str = args.join(", ");

        // Use per-function namespace from CFnSig
        let callee = match sig.namespace.as_deref() {
            Some(ns) => format!("{ns}::{}({})", sig.name, args_str),
            None => format!("{}({})", sig.name, args_str),
        };

        // Undefine Windows macros that might shadow the function name
        src.push_str(&format!("#undef {}\n", sig.name));

        if sig.ret == CType::Void {
            src.push_str(&format!(
                "__declspec(dllexport) {ret_c} {}({params_str}) {{ {callee}; }}\n",
                sig.name
            ));
        } else if let CType::ByValueStruct { type_name } = &sig.ret {
            // By-value struct return: write result into a per-function static buffer
            // and return a void* pointer to it. Not thread-safe, but sufficient for
            // single-threaded tl usage.
            let name = &sig.name;
            src.push_str(&format!(
                "static {type_name} _ret_buf_{name};\n\
                 __declspec(dllexport) void* {name}({params_str}) {{ _ret_buf_{name} = {callee}; return (void*)&_ret_buf_{name}; }}\n"
            ));
        } else {
            src.push_str(&format!(
                "__declspec(dllexport) {ret_c} {}({params_str}) {{ return ({ret_c}){callee}; }}\n",
                sig.name
            ));
        }
    }

    src.push_str("\n} // extern \"C\"\n");
    src
}

// ── Build config ────────────────────────────────────────────────────────────

/// Build configuration loaded from `tl_config.json`.
pub struct CppBuildConfig {
    /// Explicit path to `vcvarsall.bat`. When `None`, auto-detection is used.
    pub msvc: Option<PathBuf>,
    /// Additional search paths tried before `MSVC_CANDIDATES` when `msvc` is `None`.
    /// Configurable via `"msvc_search_paths": [...]` in `tl_config.json`.
    pub msvc_search_paths: Vec<String>,
    /// Preprocessor macros to `#define` before the header `#include` in the shim.
    pub precompile_macros: Vec<String>,
    /// Target architecture passed to `vcvarsall.bat` (default: `"amd64"`).
    pub target_arch: String,
    /// Extra flags appended to the `cl.exe` invocation (after the fixed base flags).
    pub cl_extra_flags: Vec<String>,
    /// Extra flags appended to the `/link` section of the `cl.exe` invocation.
    pub link_extra_flags: Vec<String>,
    /// System / SDK libraries to link. Defaults to `DEFAULT_SYSTEM_LIBS`.
    pub system_libs: Vec<String>,
    /// Whether to emit `#define WIN32_LEAN_AND_MEAN` in the shim (default: `true`).
    pub win32_lean_and_mean: bool,
    /// Additional C type → tl primitive mappings checked before the built-in table.
    /// Keys are C type names (no `*`, no qualifiers); values are tl primitive names
    /// (`"int"` / `"long"` / `"float"` / `"double"` / `"bool"` / `"void"`).
    pub custom_type_map: HashMap<String, String>,
    /// Suffix patterns used to discover library files next to the header.
    /// Ordered by preference (most specific first). Defaults to `DEFAULT_LIB_PATTERNS`.
    pub lib_patterns: Vec<String>,
    /// OS/SDK system headers to load for typedef alias resolution.
    /// E.g. `["C:/Program Files (x86)/Windows Kits/10/Include/10.0.22621.0/um/Windows.h"]`.
    /// When set, `typedef` aliases in these headers (and their local `#include`s) are
    /// resolved automatically so platform-specific type names are recognised without
    /// hardcoding.  Configurable via `"system_headers": [...]` in `tl_config.json`.
    pub system_headers: Vec<String>,
}

impl Default for CppBuildConfig {
    fn default() -> Self {
        CppBuildConfig {
            msvc: None,
            msvc_search_paths: Vec::new(),
            precompile_macros: Vec::new(),
            target_arch: DEFAULT_TARGET_ARCH.to_string(),
            cl_extra_flags: Vec::new(),
            link_extra_flags: Vec::new(),
            system_libs: DEFAULT_SYSTEM_LIBS.iter().map(|s| s.to_string()).collect(),
            win32_lean_and_mean: true,
            custom_type_map: HashMap::new(),
            lib_patterns: DEFAULT_LIB_PATTERNS.iter().map(|s| s.to_string()).collect(),
            system_headers: Vec::new(),
        }
    }
}

/// Search for `tl_config.json` starting at `start_dir` and walking up to
/// parent directories, then the current working directory.
pub fn load_cpp_config(start_dir: &Path) -> CppBuildConfig {
    let mut config = CppBuildConfig::default();

    let mut search: Vec<PathBuf> = Vec::new();
    let canon = std::fs::canonicalize(start_dir).unwrap_or_else(|_| start_dir.to_path_buf());
    let mut d = canon;
    loop {
        search.push(d.clone());
        match d.parent() {
            Some(p) if p != d => d = p.to_path_buf(),
            _ => break,
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if !search.contains(&cwd) {
            search.push(cwd);
        }
    }

    for dir in &search {
        let cfg_path = dir.join(CONFIG_FILE_NAME);
        if cfg_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&cfg_path) {
                parse_tl_config_json(&text, &mut config);
            }
            break;
        }
    }

    config
}

/// Minimal JSON parser for `tl_config.json` — no external dependencies.
///
/// Expected schema:
/// ```json
/// {
///   "cpp": {
///     "msvc": "...",
///     "msvc_search_paths": ["C:/path/to/vcvarsall.bat"],
///     "precompile_macros": ["MACRO1"],
///     "target_arch": "amd64",
///     "cl_extra_flags": ["/W4"],
///     "link_extra_flags": [],
///     "system_libs": ["winmm.lib"],
///     "win32_lean_and_mean": true,
///     "custom_type_map": { "MY_TYPE": "int" },
///     "lib_patterns": ["_vs2015_x64_md.lib", "_x64.lib"]
///   }
/// }
/// ```
fn parse_tl_config_json(content: &str, config: &mut CppBuildConfig) {
    let cpp_block = match extract_json_object(content, "cpp") {
        Some(b) => b,
        None => return,
    };

    if let Some(v) = extract_json_string(&cpp_block, "msvc") {
        config.msvc = Some(PathBuf::from(v));
    }
    if let Some(v) = extract_json_array(&cpp_block, "msvc_search_paths") {
        config.msvc_search_paths = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "precompile_macros") {
        config.precompile_macros = v;
    }
    if let Some(v) = extract_json_string(&cpp_block, "target_arch") {
        config.target_arch = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "cl_extra_flags") {
        config.cl_extra_flags = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "link_extra_flags") {
        config.link_extra_flags = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "system_libs") {
        config.system_libs = v;
    }
    if let Some(v) = extract_json_bool(&cpp_block, "win32_lean_and_mean") {
        config.win32_lean_and_mean = v;
    }
    if let Some(v) = extract_json_string_map(&cpp_block, "custom_type_map") {
        config.custom_type_map = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "lib_patterns") {
        config.lib_patterns = v;
    }
    if let Some(v) = extract_json_array(&cpp_block, "system_headers") {
        config.system_headers = v;
    }
}

/// Extract the string content of a JSON string value for the given key.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let inner = &after_colon[1..];
    let end = inner.find('"')?;
    Some(
        inner[..end]
            .replace("\\\\", "\\")
            .replace("\\\"", "\"")
            .replace("\\/", "/"),
    )
}

/// Extract a JSON boolean value for the given key.
fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Extract a JSON array of strings for the given key.
fn extract_json_array(json: &str, key: &str) -> Option<Vec<String>> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('[') {
        return None;
    }
    let bracket_end = after_colon.find(']')?;
    let arr_content = &after_colon[1..bracket_end];
    let items = arr_content
        .split(',')
        .filter_map(|item| {
            let t = item.trim();
            if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                Some(t[1..t.len() - 1].to_string())
            } else {
                None
            }
        })
        .collect();
    Some(items)
}

/// Extract a JSON object `{ "key": "value", ... }` as a `HashMap<String, String>`.
/// Only string-valued keys are extracted; non-string values are skipped.
fn extract_json_string_map(json: &str, key: &str) -> Option<HashMap<String, String>> {
    let block = extract_json_object(json, key)?;
    let mut map = HashMap::new();
    let mut rest = block.as_str();
    while let Some(q1) = rest.find('"') {
        rest = &rest[q1 + 1..];
        let q2 = match rest.find('"') {
            Some(i) => i,
            None => break,
        };
        let k = rest[..q2].to_string();
        rest = &rest[q2 + 1..];
        let colon = match rest.find(':') {
            Some(i) => i,
            None => break,
        };
        rest = rest[colon + 1..].trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        rest = &rest[1..];
        let q3 = match rest.find('"') {
            Some(i) => i,
            None => break,
        };
        let v = rest[..q3].to_string();
        rest = &rest[q3 + 1..];
        map.insert(k, v);
    }
    Some(map)
}

/// Extract the object content `{ ... }` for the given key from a JSON string.
fn extract_json_object(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let after_key = &json[pos + needle.len()..];
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim_start();
    if !after_colon.starts_with('{') {
        return None;
    }
    // Find matching closing brace
    let mut depth = 0usize;
    for (i, ch) in after_colon.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after_colon[1..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

// ── tl_XXX.dll generation ────────────────────────────────────────────────────

/// Build `tl_{stem}.dll` next to `header_path` from the given signatures.
///
/// Steps:
///   1. If `tl_{stem}.dll` already exists — return it immediately (permanent cache).
///   2. Find MSVC (`tl_config.json` or auto-detect via `find_msvc_vcvarsall`).
///   3. Compile an MSVC C++ shim (`tl_{stem}_shim.dll`) next to the header.
///   4. Compile a Rust wrapper around the shim → `tl_{stem}.dll`.
///
/// Once generated, `tl_{stem}.dll` is never rebuilt unless it is deleted.
pub fn compile_tl_dll(
    header_path: &Path,
    sigs: &[CFnSig],
    struct_defs: &[CStructDef],
    config: &CppBuildConfig,
) -> Result<(PathBuf, Vec<CFnSig>), String> {
    let header_abs =
        std::fs::canonicalize(header_path).unwrap_or_else(|_| header_path.to_path_buf());
    let header_dir = header_abs.parent().unwrap_or(Path::new("."));
    let stem = header_abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("lib");
    let ext = crate::partial_compiler::native_lib_ext();

    let dll_path = header_dir.join(format!("{TL_DLL_PREFIX}{stem}.{ext}"));
    let shim_path = header_dir.join(format!("{TL_DLL_PREFIX}{stem}{TL_SHIM_SUFFIX}.{ext}"));
    let syms_path = header_dir.join(format!("{TL_DLL_PREFIX}{stem}.{TL_SYMS_EXT}"));

    // Permanent cache: wrapper DLL exists → skip compilation, read saved function list
    if dll_path.exists() {
        eprintln!("CppBridge: loading '{}' (permanent)", dll_path.display());
        let effective = read_syms_file(&syms_path, sigs);
        return Ok((dll_path, effective));
    }

    // Find MSVC toolchain
    let msvc = if let Some(ref p) = config.msvc {
        if !p.exists() {
            return Err(format!(
                "CppBridge: msvc path '{}' not found (check tl_config.json)",
                p.display()
            ));
        }
        MsvcPaths {
            vcvarsall: p.clone(),
        }
    } else {
        find_msvc_vcvarsall(&config.msvc_search_paths).ok_or_else(|| {
            "CppBridge: MSVC not found.\n\
             Install Visual Studio 2017/2019/2022, add paths to tl_config.json:\n\
             {\"cpp\": {\"msvc_search_paths\": [\"C:/path/to/vcvarsall.bat\"]}}"
                .to_string()
        })?
    };

    // Deduplicate by name: extern "C" cannot have overloaded functions.
    // Keep first occurrence of each name.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut effective_sigs: Vec<CFnSig> = sigs
        .iter()
        .filter(|s| seen.insert(s.name.clone()))
        .cloned()
        .collect();

    let header_name = header_abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("header.h");

    // Iterative compile: on C/LNK errors, extract offending function names, remove
    // them, and retry. Repeats up to MAX_COMPILE_PASSES times.
    for pass in 0..MAX_COMPILE_PASSES {
        let shim_src = gen_cpp_shim_source(
            &effective_sigs,
            header_name,
            &config.precompile_macros,
            config.win32_lean_and_mean,
        );
        match compile_msvc_shim(&shim_src, &msvc, &header_abs, &shim_path, config) {
            Ok(()) => break,
            Err(err_msg) => {
                let bad = super::msvc_errors::extract_bad_fn_names(&err_msg);
                if bad.is_empty() || pass == MAX_COMPILE_PASSES - 1 {
                    return Err(err_msg);
                }
                let before = effective_sigs.len();
                effective_sigs.retain(|s| !bad.contains(&s.name));
                eprintln!(
                    "CppBridge: pass {}: removed {} incompatible fn(s) ({}), retrying",
                    pass + 1,
                    before - effective_sigs.len(),
                    bad.iter().cloned().collect::<Vec<_>>().join(", ")
                );
            }
        }
    }

    // Generate Rust wrapper source that loads shim_path at runtime.
    // Use plain path (no \\?\ prefix) so LoadLibraryA can accept it.
    let shim_str = strip_unc_prefix(&shim_path);
    let wrapper_src = gen_dll_wrapper(&shim_str, &effective_sigs, struct_defs);

    // Compile Rust wrapper → tl_{stem}.dll
    let wrapper_bytes = compile_wrapper(&wrapper_src, &[])?;
    std::fs::write(&dll_path, &wrapper_bytes)
        .map_err(|e| format!("CppBridge: cannot write '{}': {e}", dll_path.display()))?;

    // Save the effective function list so future cache hits know what was compiled.
    let syms: String = effective_sigs
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&syms_path, syms);

    eprintln!("CppBridge: generated '{}'", dll_path.display());
    Ok((dll_path, effective_sigs))
}

/// Read the `.syms` companion file and return the subset of `all_sigs` that were compiled.
/// Falls back to all sigs if the file is missing (e.g., first run before the syms file existed).
fn read_syms_file(syms_path: &Path, all_sigs: &[CFnSig]) -> Vec<CFnSig> {
    if let Ok(text) = std::fs::read_to_string(syms_path) {
        let allowed: std::collections::HashSet<&str> = text.lines().collect();
        all_sigs
            .iter()
            .filter(|s| allowed.contains(s.name.as_str()))
            .cloned()
            .collect()
    } else {
        all_sigs.to_vec()
    }
}

/// Compile `cpp_src` into a DLL at `out_dll` using MSVC `cl.exe`.
///
/// Library files are selected from the same directory as `header_path` according
/// to `config.lib_patterns` (most-specific pattern wins when both exist for the
/// same base name).  `config.system_libs` provides the SDK / Windows libs.
/// The target architecture is `config.target_arch`; extra compiler and linker
/// flags come from `config.cl_extra_flags` / `config.link_extra_flags`.
fn compile_msvc_shim(
    cpp_src: &str,
    msvc: &MsvcPaths,
    header_path: &Path,
    out_dll: &Path,
    config: &CppBuildConfig,
) -> Result<(), String> {
    // If shim already exists and source is unchanged, skip recompilation
    let stem = out_dll
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("shim");
    let temp_dir = std::env::temp_dir().join(format!("tl_build_{stem}"));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("CppShim: cannot create temp dir: {e}"))?;

    let cpp_file = temp_dir.join("shim.cpp");
    let prev_src = std::fs::read_to_string(&cpp_file).unwrap_or_default();
    if out_dll.exists() && prev_src == cpp_src {
        eprintln!(
            "CppShim: shim source unchanged, reusing '{}'",
            out_dll.display()
        );
        return Ok(());
    }

    std::fs::write(&cpp_file, cpp_src)
        .map_err(|e| format!("CppShim: cannot write shim.cpp: {e}"))?;

    // Absolute header/lib directory (bat runs from temp_dir).
    // canonicalize() on Windows produces \\?\ UNC paths that cl.exe does not
    // accept for /I or /LIBPATH — strip that prefix to get a plain absolute path.
    let lib_dir = header_path.parent().unwrap_or(Path::new("."));
    let lib_dir_abs = std::fs::canonicalize(lib_dir).unwrap_or_else(|_| lib_dir.to_path_buf());
    let libdir_str = strip_unc_prefix(&lib_dir_abs);

    // Collect libs matching config.lib_patterns, excluding other-family and debug variants.
    let header_stem = header_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let hs_lower = header_stem.to_lowercase();
    let patterns_lc: Vec<String> = config
        .lib_patterns
        .iter()
        .map(|p| p.to_lowercase())
        .collect();

    let mut lib_names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&lib_dir_abs) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("lib") {
                continue;
            }
            if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                let lower = fname.to_lowercase();

                // Accept only files whose name ends with a configured pattern
                let matches_pattern = patterns_lc.iter().any(|pat| lower.ends_with(pat.as_str()));
                if !matches_pattern {
                    continue;
                }

                // Skip debug builds (e.g. DxLib_d.lib, DxLib_x64_d.lib)
                if lower.contains("_d.") {
                    continue;
                }

                // Skip variants from a different library family: e.g. exclude DxLibW_x64.lib
                // when the header stem is DxLib.  Strip any matching pattern suffix to get
                // the base name; if it starts with hs_lower but is longer, it's another family.
                let is_other_family = patterns_lc
                    .iter()
                    .filter_map(|pat| lower.strip_suffix(pat.as_str()))
                    .any(|base| base.starts_with(&hs_lower) && base.len() > hs_lower.len());
                if is_other_family {
                    continue;
                }

                lib_names.push(fname.to_string());
            }
        }
    }
    // Prefer versioned/specific patterns over generic ones for the same base name.
    let deduped = dedup_by_pattern_priority(lib_names, &patterns_lc);

    // Append system libs from config (defaults to DEFAULT_SYSTEM_LIBS)
    let mut final_libs = deduped;
    for syslib in &config.system_libs {
        final_libs.push(syslib.clone());
    }
    let libs_str = final_libs.join(" ");

    let vcvarsall_str = strip_unc_prefix(&msvc.vcvarsall);
    let cpp_str = strip_unc_prefix(&cpp_file);
    let dll_str = strip_unc_prefix(out_dll);
    let bat_file = temp_dir.join("build.bat");

    let extra_cl = config.cl_extra_flags.join(" ");
    let extra_link = config.link_extra_flags.join(" ");
    let arch = &config.target_arch;

    let bat = format!(
        "@echo off\r\n\
         call \"{vcvarsall_str}\" {arch}\r\n\
         cl.exe /nologo /LD /MD /W3 {extra_cl} \
             /I \"{libdir_str}\" \
             /Fe\"{dll_str}\" \
             \"{cpp_str}\" \
             {libs_str} \
             /link /LIBPATH:\"{libdir_str}\" /SUBSYSTEM:WINDOWS /NODEFAULTLIB:LIBCMT {extra_link}\r\n\
         exit /b %ERRORLEVEL%\r\n"
    );
    std::fs::write(&bat_file, to_acp_bytes(&bat))
        .map_err(|e| format!("CppShim: cannot write build.bat: {e}"))?;

    eprintln!("CppShim: compiling '{}' with MSVC …", out_dll.display());

    let output = Command::new("cmd")
        .args(["/c", bat_file.to_str().unwrap_or("build.bat")])
        .current_dir(&temp_dir)
        .output()
        .map_err(|e| format!("CppShim: cannot run cmd.exe: {e}"))?;

    if !output.status.success() || !out_dll.exists() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("CppShim: cl.exe failed:\n{stdout}{stderr}"));
    }

    eprintln!("CppShim: produced '{}'", out_dll.display());
    Ok(())
}

/// Among libs that share a base name, keep only those matching the highest-priority
/// pattern (lowest index in `patterns`).  Libs not matching any pattern are kept as-is.
///
/// Example with `patterns = ["_vs2015_x64_md.lib", "_x64.lib"]`:
///   - `DxLib_vs2015_x64_MD.lib` (index 0) is kept and `DxLib_x64.lib` (index 1) dropped.
///   - `DxThread_x64.lib` has no versioned counterpart → kept.
fn dedup_by_pattern_priority(libs: Vec<String>, patterns: &[String]) -> Vec<String> {
    if patterns.len() <= 1 {
        return libs;
    }

    // For each lib find (priority_index, base_name)
    let match_info = |name: &str| -> Option<(usize, String)> {
        let lower = name.to_lowercase();
        patterns
            .iter()
            .enumerate()
            .find(|(_, pat)| lower.ends_with(pat.as_str()))
            .map(|(i, pat)| (i, lower[..lower.len() - pat.len()].to_string()))
    };

    // Best (lowest index) priority per base name
    let mut best: HashMap<String, usize> = HashMap::new();
    for lib in &libs {
        if let Some((pri, base)) = match_info(lib) {
            let entry = best.entry(base).or_insert(usize::MAX);
            if pri < *entry {
                *entry = pri;
            }
        }
    }

    libs.into_iter()
        .filter(|lib| match match_info(lib) {
            Some((pri, base)) => best.get(&base).map_or(true, |&best_pri| pri <= best_pri),
            None => true,
        })
        .collect()
}

/// Strip the Windows extended-path `\\?\` prefix that `std::fs::canonicalize`
/// adds, because cl.exe does not accept it for `/I` or `/LIBPATH` flags.
fn strip_unc_prefix(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// Convert a UTF-8 string to the system ANSI code page bytes.
///
/// On Japanese Windows the ANSI code page is Shift-JIS (932).  Writing .bat
/// files with this encoding ensures cmd.exe correctly interprets non-ASCII
/// directory paths (e.g. Japanese characters in the DxLib install path).
fn to_acp_bytes(s: &str) -> Vec<u8> {
    #[cfg(windows)]
    {
        extern "system" {
            fn WideCharToMultiByte(
                code_page: u32,
                flags: u32,
                wide_str: *const u16,
                wide_chars: i32,
                mb_str: *mut u8,
                mb_chars: i32,
                default_char: *const u8,
                used_default: *mut i32,
            ) -> i32;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let needed = unsafe {
            WideCharToMultiByte(
                0,
                0,
                wide.as_ptr(),
                wide.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        if needed > 0 {
            let mut buf = vec![0u8; needed as usize];
            unsafe {
                WideCharToMultiByte(
                    0,
                    0,
                    wide.as_ptr(),
                    wide.len() as i32,
                    buf.as_mut_ptr(),
                    needed,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                );
            }
            return buf;
        }
    }
    s.as_bytes().to_vec()
}
