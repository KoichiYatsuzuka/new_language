// cpp_bridge.rs — C++ DLL / static-lib bridge for import[cpp-dll] and import[cpp-lib].
//
// Flow for import[cpp-dll] "lib.dll" with "lib.h":
//   1. parse_header() reads the .h file and extracts extern "C" function signatures.
//   2. gen_dll_wrapper() emits a standalone Rust source file that:
//        - Dynamically loads the original DLL via OS APIs (LoadLibraryA / dlopen).
//        - Exports one `{name}_tl(cb, argc, argv) -> i64` wrapper per function,
//          marshaling between tl handles and native C types.
//   3. compile_wrapper() compiles the source with `rustc --crate-type cdylib`.
//   4. The resulting DLL is loaded by the interpreter via the existing NativeFnRef ABI.
//
// Flow for import[cpp-lib] "lib.lib" with "lib.h":
//   Same as above except the wrapper uses `#[link(name="...", kind="static")]`
//   instead of dynamic loading, and the parent directory is passed to rustc as -L.
//
// Supported C types: int / long / float / double / bool / void / void*
// Not yet supported: const char* (requires a to_str callback addition to TlCallbacks),
//                    struct / union / enum types, function pointers.

use std::path::{Path, PathBuf};
use std::process::Command;

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
}

impl CType {
    /// C type string used in the generated C++ shim source.
    pub fn c_type_str(&self) -> &'static str {
        match self {
            CType::Int     => "int",
            CType::Long    => "long long",
            CType::Float   => "float",
            CType::Double  => "double",
            CType::Bool    => "int",
            CType::Void    => "void",
            CType::VoidPtr => "void*",
        }
    }

    /// Rust type used in the `extern "C"` declaration inside the generated wrapper.
    fn rust_extern_type(&self) -> &'static str {
        match self {
            CType::Int     => "i32",
            CType::Long    => "i64",
            CType::Float   => "f32",
            CType::Double  => "f64",
            CType::Bool    => "i32",
            CType::Void    => "()",
            CType::VoidPtr => "*mut i8",
        }
    }

    /// Rust expression that converts a tl handle (`i64`) to this C type.
    fn from_handle(&self, handle: &str) -> String {
        match self {
            CType::Int     => format!("((*CB).to_int)({handle}) as i32"),
            CType::Long    => format!("((*CB).to_int)({handle})"),
            CType::Float   => format!("((*CB).to_float)({handle}) as f32"),
            CType::Double  => format!("((*CB).to_float)({handle})"),
            CType::Bool    => format!("if {handle} == 1i64 {{ 1i32 }} else {{ 0i32 }}"),
            CType::Void    => "()".to_string(),
            CType::VoidPtr => format!("{handle} as *mut i8"),
        }
    }

    /// Rust expression that wraps a C return value into a tl handle.
    fn to_handle(&self, val: &str) -> String {
        match self {
            CType::Int     => format!("((*CB).make_int)({val} as i64)"),
            CType::Long    => format!("((*CB).make_int)({val})"),
            CType::Float   => format!("((*CB).make_float)({val} as f64)"),
            CType::Double  => format!("((*CB).make_float)({val})"),
            CType::Bool    => format!("if {val} != 0 {{ 1i64 }} else {{ 2i64 }}"),
            CType::Void    => "0i64".to_string(), // TL_NONE
            CType::VoidPtr => format!("{val} as i64"),
        }
    }
}

// ── Function signature ───────────────────────────────────────────────────────

/// A C function signature extracted from a `.h` file.
#[derive(Debug, Clone)]
pub struct CFnSig {
    pub name: String,
    pub params: Vec<(String, CType)>,
    pub ret: CType,
}

// ── Header parser ────────────────────────────────────────────────────────────

/// Parse `extern "C"` function declarations from a C/C++ header.
///
/// Recognises two forms:
/// - Block:      `extern "C" { ... }`
/// - Individual: `extern "C" return_type name(params);`
///
/// Functions with unsupported types (`const char*`, struct, etc.) are skipped
/// with a warning printed to stderr.
pub fn parse_header(content: &str) -> Vec<CFnSig> {
    let stripped = strip_comments(content);
    let mut sigs = Vec::new();

    // Collect all text that is inside extern "C" { } blocks, plus standalone
    // extern "C" declarations.
    let mut decls: Vec<String> = Vec::new();

    let s = stripped.as_str();
    let mut i = 0;
    let bytes = s.as_bytes();

    while i < bytes.len() {
        // Look for the token `extern`
        if s[i..].starts_with("extern") {
            let after = &s[i + 6..].trim_start();
            if after.starts_with('"') {
                // Skip the linkage string (e.g. "C")
                if let Some(close) = after[1..].find('"') {
                    let rest = after[1 + close + 1..].trim_start();
                    if rest.starts_with('{') {
                        // Block form: extern "C" { ... }
                        if let Some(end) = find_matching_brace(rest) {
                            let inner = &rest[1..end];
                            for line in inner.lines() {
                                let l = line.trim().trim_end_matches(';').trim();
                                if !l.is_empty() && !l.starts_with('#') {
                                    decls.push(l.to_string());
                                }
                            }
                            i += (s.len() - rest.len()) + end + 1;
                            continue;
                        }
                    } else {
                        // Individual form: extern "C" ret_type name(params);
                        if let Some(sc) = rest.find(';') {
                            decls.push(rest[..sc].trim().to_string());
                            i += (s.len() - rest.len()) + sc + 1;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }

    for decl in &decls {
        match parse_fn_decl(decl) {
            Ok(sig) => sigs.push(sig),
            Err(e) => eprintln!("CppImport: skipping declaration ({e}): {decl}"),
        }
    }

    sigs
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    // line comment — skip to end of line
                    for c2 in chars.by_ref() { if c2 == '\n' { out.push('\n'); break; } }
                }
                Some('*') => {
                    chars.next();
                    // block comment — skip to */
                    while let Some(c2) = chars.next() {
                        if c2 == '*' && chars.peek() == Some(&'/') { chars.next(); break; }
                        if c2 == '\n' { out.push('\n'); }
                    }
                }
                _ => out.push(c),
            }
        } else if c == '#' {
            // preprocessor directive — skip to end of line
            for c2 in chars.by_ref() { if c2 == '\n' { out.push('\n'); break; } }
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
            '}' => { depth -= 1; if depth == 0 { return Some(i); } }
            _ => {}
        }
    }
    None
}

/// Parse a single C function declaration string like `int add(int a, int b)`.
fn parse_fn_decl(decl: &str) -> Result<CFnSig, String> {
    // Find the opening parenthesis for the parameter list
    let paren_open = decl.find('(').ok_or("no '(' found")?;
    let paren_close = decl.rfind(')').ok_or("no ')' found")?;

    let before_paren = decl[..paren_open].trim();
    let params_str   = decl[paren_open + 1..paren_close].trim();

    // The last word before '(' is the function name; everything before is the return type
    let (ret_str, name) = split_type_and_name(before_paren)?;
    let ret = parse_c_type_str(ret_str.trim())?;

    // Parse parameters
    let mut params: Vec<(String, CType)> = Vec::new();
    if !params_str.is_empty() && params_str != "void" {
        for (idx, p) in params_str.split(',').enumerate() {
            let p = p.trim();
            if p.is_empty() { continue; }
            // varargs "..." — skip
            if p == "..." { continue; }
            let (type_str, pname) = match split_type_and_name(p) {
                Ok(r) => r,
                Err(_) => {
                    // unnamed parameter: the whole token is the type
                    (p.to_string(), format!("_p{idx}"))
                }
            };
            let ct = parse_c_type_str(type_str.trim())?;
            params.push((pname, ct));
        }
    }

    Ok(CFnSig { name, params, ret })
}

/// Split `"int foo"` → `("int", "foo")`, `"const unsigned long bar"` → `("const unsigned long", "bar")`.
fn split_type_and_name(s: &str) -> Result<(String, String), String> {
    // Handle pointer suffix on the name: `int *ptr` or `int* ptr`
    let s = s.trim().trim_end_matches('*').trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("cannot split type/name from '{s}'"));
    }
    let name = parts.last().unwrap().to_string();
    let type_str = parts[..parts.len() - 1].join(" ");
    Ok((type_str, name))
}

/// Map a C type string to `CType`, returning an error for unsupported types.
fn parse_c_type_str(s: &str) -> Result<CType, String> {
    // Strip leading/trailing whitespace and pointer stars
    let s = s.trim();
    let is_ptr = s.ends_with('*');
    let base = s.trim_end_matches('*').trim();

    // Determine base type (ignore const/unsigned/signed qualifiers)
    let tokens: Vec<&str> = base.split_whitespace()
        .filter(|t| !matches!(*t, "const" | "volatile" | "restrict" | "__restrict"))
        .collect();

    if is_ptr {
        // Only void* is supported for now
        if tokens == ["void"] { return Ok(CType::VoidPtr); }
        return Err(format!("pointer to non-void type: {s}"));
    }

    // Remove signed/unsigned qualifiers to get the base integer kind
    let core: Vec<&str> = tokens.iter()
        .filter(|t| !matches!(**t, "unsigned" | "signed"))
        .copied()
        .collect();

    match core.as_slice() {
        ["void"]                                                    => Ok(CType::Void),
        ["bool"] | ["_Bool"]                                        => Ok(CType::Bool),
        ["float"]                                                   => Ok(CType::Float),
        ["double"] | ["long", "double"]                             => Ok(CType::Double),
        ["char"] | ["short"] | ["int"] |
        ["int8_t"] | ["int16_t"] | ["int32_t"] |
        ["uint8_t"] | ["uint16_t"] | ["uint32_t"]                  => Ok(CType::Int),
        ["long"] | ["long", "int"] |
        ["long", "long"] | ["long", "long", "int"] |
        ["int64_t"] | ["uint64_t"] |
        ["size_t"] | ["ptrdiff_t"] | ["intptr_t"] | ["uintptr_t"]  => Ok(CType::Long),
        other => Err(format!("unsupported type: {}", other.join(" "))),
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
}

static mut CB: *const TlCallbacks = std::ptr::null();

#[no_mangle]
pub unsafe extern "C" fn tl_init(cb: *const TlCallbacks) { CB = cb; }
"#;

// Platform loader inserted into dll-wrapper source
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
/// on every call.  All functions follow the `{name}_tl(cb, argc, argv) -> i64`
/// convention so the existing `NativeFnRef` machinery can drive them.
pub fn gen_dll_wrapper(dll_path: &str, sigs: &[CFnSig]) -> String {
    let mut src = String::new();
    src.push_str(WRAPPER_HEADER);
    src.push_str(PLATFORM_LOADER);

    // Override tl_init to also load the original DLL
    let escaped = dll_path.replace('\\', "\\\\");
    src.push_str(&format!(r#"
const _DLL_PATH: &str = "{escaped}";

#[no_mangle]
pub unsafe extern "C" fn tl_init_cpp(cb: *const TlCallbacks) {{
    CB = cb;
    _DLL_HANDLE = _loader::load(_DLL_PATH);
    if _DLL_HANDLE == 0 {{ panic!("cpp bridge: cannot load DLL: {{}}", _DLL_PATH); }}
}}
"#));

    // Re-export tl_init so the standard tl_init call also works
    src.push_str(r#"
// Called by the interpreter at module load — initialise callbacks then load DLL.
#[no_mangle]
pub unsafe extern "C" fn tl_init_bridge(cb: *const TlCallbacks) {
    tl_init(cb);
    tl_init_cpp(cb);
}
"#);

    for sig in sigs {
        src.push_str(&gen_dll_fn(sig));
    }
    src
}

fn gen_dll_fn(sig: &CFnSig) -> String {
    let mut s = String::new();
    let n = &sig.name;

    // Build the C function-pointer type
    let param_types: Vec<&str> = sig.params.iter()
        .map(|(_, t)| t.rust_extern_type())
        .collect();
    let fn_type = if sig.ret == CType::Void {
        format!("unsafe extern \"C\" fn({})", param_types.join(", "))
    } else {
        format!("unsafe extern \"C\" fn({}) -> {}", param_types.join(", "), sig.ret.rust_extern_type())
    };

    s.push_str(&format!(r#"
#[no_mangle]
pub unsafe extern "C" fn {n}_tl(argv: *const i64, _argc: i32) -> i64 {{
    type _F = {fn_type};
    let _fp: _F = std::mem::transmute(_loader::sym(_dll(), "{n}"));
"#));

    // Marshal each argument
    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        let h = format!("*argv.offset({i})");
        s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
    }

    // Invoke
    let args: Vec<String> = sig.params.iter().map(|(n, _)| format!("_{n}")).collect();
    if sig.ret == CType::Void {
        s.push_str(&format!("    _fp({});\n", args.join(", ")));
        s.push_str("    0i64\n");
    } else {
        s.push_str(&format!("    let _r = _fp({});\n", args.join(", ")));
        s.push_str(&format!("    {}\n", sig.ret.to_handle("_r")));
    }
    s.push_str("}\n");
    s
}

/// Generate a standalone Rust wrapper for a **static** `.lib` / `.a`.
///
/// The returned tuple is `(rust_source, lib_dir)` where `lib_dir` is the
/// directory that must be passed to rustc as `-L` so the linker can find the
/// static library.
pub fn gen_lib_wrapper(lib_path: &str, lib_stem: &str, sigs: &[CFnSig]) -> (String, PathBuf) {
    let lib_dir = Path::new(lib_path).parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let mut src = String::new();
    src.push_str(WRAPPER_HEADER);

    // Static link directive
    src.push_str(&format!(
        "\n#[link(name = \"{lib_stem}\", kind = \"static\")]\nextern \"C\" {{\n"
    ));
    for sig in sigs {
        let param_types: Vec<String> = sig.params.iter()
            .map(|(pn, pt)| format!("{pn}: {}", pt.rust_extern_type()))
            .collect();
        if sig.ret == CType::Void {
            src.push_str(&format!("    fn {}({});\n", sig.name, param_types.join(", ")));
        } else {
            src.push_str(&format!(
                "    fn {}({}) -> {};\n",
                sig.name, param_types.join(", "), sig.ret.rust_extern_type()
            ));
        }
    }
    src.push_str("}\n");

    for sig in sigs {
        src.push_str(&gen_lib_fn(sig));
    }

    (src, lib_dir)
}

fn gen_lib_fn(sig: &CFnSig) -> String {
    let mut s = String::new();
    let n = &sig.name;

    s.push_str(&format!(r#"
#[no_mangle]
pub unsafe extern "C" fn {n}_tl(argv: *const i64, _argc: i32) -> i64 {{
"#));

    for (i, (pname, ptype)) in sig.params.iter().enumerate() {
        let h = format!("*argv.offset({i})");
        s.push_str(&format!("    let _{pname} = {};\n", ptype.from_handle(&h)));
    }

    let args: Vec<String> = sig.params.iter().map(|(n, _)| format!("_{n}")).collect();
    if sig.ret == CType::Void {
        s.push_str(&format!("    {n}({});\n", args.join(", ")));
        s.push_str("    0i64\n");
    } else {
        s.push_str(&format!("    let _r = {n}({});\n", args.join(", ")));
        s.push_str(&format!("    {}\n", sig.ret.to_handle("_r")));
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
    let rs_path  = tmp_dir.join("_tl_cpp_bridge.rs");
    let ext      = crate::partial_compiler::native_lib_ext();
    let dll_path = tmp_dir.join(format!("_tl_cpp_bridge.{ext}"));

    std::fs::write(&rs_path, rust_src)
        .map_err(|e| format!("CppImport: cannot write wrapper source: {e}"))?;

    let mut cmd = Command::new("rustc");
    cmd.args(["--edition", "2021", "--crate-type", "cdylib", "-C", "opt-level=2"]);
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

/// Search common Visual Studio installation paths for `vcvarsall.bat`.
/// Returns `None` if no MSVC installation is found.
pub fn find_msvc_vcvarsall() -> Option<MsvcPaths> {
    let candidates = [
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
    for path in &candidates {
        if Path::new(path).exists() {
            return Some(MsvcPaths { vcvarsall: PathBuf::from(path) });
        }
    }
    None
}

/// Strip common architecture/compiler suffixes from a lib stem to get the
/// likely C++ namespace name.  E.g. `"DxLib_x64"` → `"DxLib"`.
pub fn lib_namespace_from_stem(stem: &str) -> String {
    let lower = stem.to_lowercase();
    for suffix in &["_x64", "_x86", "_win64", "_win32", "_64", "_32",
                    "_vc", "_vs2022", "_vs2019", "_vs2017", "_vs2015",
                    "_md", "_mt", "_d", "_debug", "_release"] {
        if lower.ends_with(suffix) {
            let stripped = &stem[..stem.len() - suffix.len()];
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    stem.to_string()
}

/// Generate a C++ shim source file that:
/// 1. Includes the real C++ header by **filename only** (e.g. `"DxLib.h"`).
///    The caller is responsible for passing the containing directory as a
///    `/I` include-path argument to the compiler — this avoids embedding
///    non-ASCII directory paths inside the source file.
/// 2. Exports a flat `extern "C"` wrapper for each signature in `sigs`.
///
/// Each wrapper calls through the given C++ `namespace` (e.g. `"DxLib"`), or
/// calls the function directly when `namespace` is `None`.
///
/// Special handling:
/// - `void` return: call is a statement, wrapper returns nothing.
/// - All other returns: explicit `return` with implicit C numeric conversion.
pub fn gen_cpp_shim_source(
    sigs: &[CFnSig],
    header_name: &str,   // just the filename, e.g. "DxLib.h" — no directory part
    namespace: Option<&str>,
) -> String {
    let mut src = String::new();

    src.push_str("#define WIN32_LEAN_AND_MEAN\n");
    src.push_str("#include <windows.h>\n");
    src.push_str(&format!("#include \"{header_name}\"\n\n"));
    src.push_str("extern \"C\" {\n\n");

    for sig in sigs {
        let ret_c = sig.ret.c_type_str();

        // Parameter list: "int x1, int y1, ..."  or "void" if empty
        let params: Vec<String> = sig.params.iter().enumerate().map(|(i, (name, ct))| {
            let n = if name.is_empty() { format!("p{i}") } else { name.clone() };
            format!("{} {n}", ct.c_type_str())
        }).collect();
        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };

        // Argument list: "x1, y1, ..." or "" if empty
        let args: Vec<String> = sig.params.iter().enumerate().map(|(i, (name, _))| {
            if name.is_empty() { format!("p{i}") } else { name.clone() }
        }).collect();
        let args_str = args.join(", ");

        // Callee: "DxLib::FuncName" or just "FuncName"
        let callee = match namespace {
            Some(ns) => format!("{ns}::{}({})", sig.name, args_str),
            None     => format!("{}({})", sig.name, args_str),
        };

        if sig.ret == CType::Void {
            src.push_str(&format!(
                "__declspec(dllexport) {ret_c} {}({params_str}) {{ {callee}; }}\n",
                sig.name
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

/// Compile `cpp_src` into a DLL using MSVC `cl.exe` (via `vcvarsall.bat`).
///
/// All `.lib` files in the same directory as `lib_path` are linked automatically
/// (the main lib + any codec / helper libs stored alongside it), plus the
/// standard Windows DirectX / multimedia libs.
///
/// Returns the path to the produced `.dll` inside a temp directory.
///
/// Implementation notes:
/// - Writes a `.bat` file instead of inlining the command, avoiding Windows'
///   32 767-char `CreateProcess` command-line limit.
/// - Converts the lib directory to its 8.3 short path so the `.bat` stays
///   pure ASCII even when the user's path contains non-ASCII characters
///   (e.g. the Japanese DxLib installation directory).
pub fn compile_cpp_shim(
    cpp_src: &str,
    msvc: &MsvcPaths,
    lib_path: &Path,
) -> Result<PathBuf, String> {
    let lib_dir = lib_path.parent().unwrap_or(Path::new("."));

    // Temp dir name uses only the lib filename (ASCII) not the full path.
    let lib_stem = lib_path.file_stem().and_then(|s| s.to_str()).unwrap_or("shim");
    let temp_dir = std::env::temp_dir().join(format!("tl_shim_{lib_stem}"));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("CppShim: cannot create temp dir: {e}"))?;

    let cpp_file = temp_dir.join("shim.cpp");
    let dll_file = temp_dir.join("shim.dll");
    let bat_file = temp_dir.join("build.bat");

    // Skip recompilation if shim.dll is newer than both the .lib and the shim source.
    let lib_mtime = std::fs::metadata(lib_path).and_then(|m| m.modified()).ok();
    let dll_mtime = std::fs::metadata(&dll_file).and_then(|m| m.modified()).ok();
    let prev_src  = std::fs::read_to_string(&cpp_file).unwrap_or_default();
    if let (Some(lib_t), Some(dll_t)) = (lib_mtime, dll_mtime) {
        if dll_t > lib_t && prev_src == cpp_src {
            eprintln!("CppShim: using cached shim DLL");
            return Ok(dll_file);
        }
    }

    std::fs::write(&cpp_file, cpp_src)
        .map_err(|e| format!("CppShim: cannot write shim.cpp: {e}"))?;

    // Use the full lib directory path. The bat file is written in the system
    // ANSI code page (Shift-JIS on Japanese Windows) via to_acp_bytes(), so
    // non-ASCII characters in the path are correctly encoded for cmd.exe.
    let libdir_str = lib_dir.to_string_lossy().into_owned();

    // Collect lib filenames only (not full paths).
    // The linker resolves them via /LIBPATH:lib_dir.
    //
    // Include only: the explicitly specified main lib, AND any other libs
    // whose name contains "x64" or "_64" (i.e. architecture-matched codec
    // libs like celt_x64.lib, libpng16_x64.lib, etc.).
    // This avoids accidentally picking up x86 variants (e.g. DxLib.lib)
    // that reside in the same directory, which would cause LNK1112.
    let main_lib_name = lib_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut lib_names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(lib_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("lib") {
                if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                    let lower = fname.to_lowercase();
                    // Include: the specified main lib, OR x64 codec/helper libs
                    // that are not old-CRT variants and not alternative DxLib
                    // flavours (DxLibW = wide-char, extra DxLib_vs2015 variants,
                    // etc. — these pull in __iob_func or duplicate CRT symbols).
                    let is_old_crt = lower.contains("vs2012") || lower.contains("vs2013")
                                  || lower.contains("vs2010") || lower.contains("vs2008");
                    let is_x64 = lower.contains("x64") || lower.contains("_64");
                    // Any lib starting with "dxlib" other than the explicitly
                    // specified main lib should be excluded — DxLibW_*, DxLib_vs2015_*,
                    // etc. all add conflicting or unsatisfiable symbols.
                    let is_other_dxlib = lower.starts_with("dxlib") && fname != main_lib_name;
                    if fname == main_lib_name || (is_x64 && !is_old_crt && !is_other_dxlib) {
                        lib_names.push(fname.to_string());
                    }
                }
            }
        }
    }
    // Standard Windows multimedia / DirectX libs present in every MSVC env.
    for syslib in &[
        "winmm.lib", "imm32.lib", "ws2_32.lib", "dxguid.lib",
        "d3d9.lib", "d3d11.lib", "dxgi.lib", "dinput8.lib", "d3dcompiler.lib",
    ] {
        lib_names.push(syslib.to_string());
    }
    let libs_str = lib_names.join(" ");

    let vcvarsall_str = msvc.vcvarsall.to_string_lossy().into_owned();
    let cpp_str       = cpp_file.to_string_lossy().into_owned();
    let dll_str       = dll_file.to_string_lossy().into_owned();

    // Write a .bat file to avoid CreateProcess command-line length limits.
    // No output redirect: Rust captures stdout/stderr via output() below.
    //
    // /I passes the lib directory so shim.cpp can use `#include "DxLib.h"`
    // (filename only) without embedding non-ASCII paths in the source file.
    //
    // The bat is written in the system ANSI code page (Shift-JIS on Japanese
    // Windows) via to_acp_bytes() so cmd.exe correctly interprets non-ASCII
    // characters in /I and /LIBPATH arguments.
    let bat = format!(
        "@echo off\r\n\
         call \"{vcvarsall_str}\" amd64\r\n\
         cl.exe /nologo /LD /MD /W3 \
             /I \"{libdir_str}\" \
             /Fe\"{dll_str}\" \
             \"{cpp_str}\" \
             {libs_str} \
             /link /LIBPATH:\"{libdir_str}\" /SUBSYSTEM:WINDOWS /NODEFAULTLIB:LIBCMT\r\n\
         exit /b %ERRORLEVEL%\r\n"
    );
    std::fs::write(&bat_file, to_acp_bytes(&bat))
        .map_err(|e| format!("CppShim: cannot write build.bat: {e}"))?;

    eprintln!("CppShim: compiling shim DLL with MSVC …");

    let output = Command::new("cmd")
        .args(["/c", bat_file.to_str().unwrap_or("build.bat")])
        .current_dir(&temp_dir)
        .output()
        .map_err(|e| format!("CppShim: cannot run cmd.exe: {e}"))?;

    if !output.status.success() || !dll_file.exists() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("CppShim: cl.exe failed:\n{stdout}{stderr}"));
    }

    eprintln!("CppShim: produced '{dll_str}'");
    Ok(dll_file)
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
                code_page:   u32,
                flags:       u32,
                wide_str:    *const u16,
                wide_chars:  i32,
                mb_str:      *mut u8,
                mb_chars:    i32,
                default_char: *const u8,
                used_default: *mut i32,
            ) -> i32;
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let needed = unsafe {
            WideCharToMultiByte(0, 0, wide.as_ptr(), wide.len() as i32,
                                std::ptr::null_mut(), 0,
                                std::ptr::null(), std::ptr::null_mut())
        };
        if needed > 0 {
            let mut buf = vec![0u8; needed as usize];
            unsafe {
                WideCharToMultiByte(0, 0, wide.as_ptr(), wide.len() as i32,
                                    buf.as_mut_ptr(), needed,
                                    std::ptr::null(), std::ptr::null_mut());
            }
            return buf;
        }
    }
    s.as_bytes().to_vec()
}

