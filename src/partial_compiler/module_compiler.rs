/// `.tlc` compiled module format — writer and reader.
///
/// # File format (version 0)
///
/// Embeds only the source text.
///
/// ```text
/// [4 bytes]  magic    : b"TLC\x00"
/// [4 bytes]  version  : u32 LE  (0)
/// [4 bytes]  name_len : u32 LE
/// [name_len] name     : UTF-8 module name
/// [4 bytes]  src_len  : u32 LE
/// [src_len]  source   : UTF-8 source text
/// ```
///
/// # File format (version 1)
///
/// Extends version 0 with an embedded native shared library (DLL/SO/dylib).
///
/// ```text
/// [4 bytes]  magic    : b"TLC\x00"
/// [4 bytes]  version  : u32 LE  (1)
/// [4 bytes]  name_len : u32 LE
/// [name_len] name     : UTF-8 module name
/// [4 bytes]  src_len  : u32 LE
/// [src_len]  source   : UTF-8 source text
/// [4 bytes]  n_fns    : u32 LE  (number of natively compiled functions)
/// for each fn:
///   [4 bytes]       fn_name_len : u32 LE
///   [fn_name_len]   fn_name     : UTF-8
///   [4 bytes]       n_params    : u32 LE
/// [4 bytes]  dll_len  : u32 LE
/// [dll_len]  dll_bytes: raw shared-library bytes
/// ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::ast::Stmt;
use super::codegen;
use super::stub_gen;

const MAGIC: &[u8; 4] = b"TLC\x00";
const VERSION_V0: u32 = 0;
const VERSION_V1: u32 = 1;

// ---------------------------------------------------------------------------
// Thread-local cache: module_name → (exports, dll_bytes)
// Populated by load_tlc() when it reads a v1 file; consumed by exec.rs.
// ---------------------------------------------------------------------------

thread_local! {
    static NATIVE_CACHE: RefCell<HashMap<String, (Vec<codegen::FnExport>, Vec<u8>)>> =
        RefCell::new(HashMap::new());
}

/// Consume and return the cached native data for a module (if any).
/// Called once per `import[tl]` evaluation; subsequent calls return `None`.
pub fn take_native_bytes(module_name: &str) -> Option<(Vec<codegen::FnExport>, Vec<u8>)> {
    NATIVE_CACHE.with(|c| c.borrow_mut().remove(module_name))
}

// ── public API ────────────────────────────────────────────────────────────────

/// Compile `source` (already parsed into `stmts`) and write `.tlc` + `.tls`
/// next to the original `source_path`.
///
/// If native compilation succeeds, the `.tlc` is written as **version 1**
/// with the DLL bytes embedded inside it.  No separate `_tl.*` file is
/// created.  If `rustc` is unavailable or no functions are eligible, the
/// `.tlc` is written as version 0 (source-only) and a warning is printed.
///
/// Returns the paths of the two output files (`.tlc`, `.tls`) on success.
pub fn compile(
    source: &str,
    stmts: &[Stmt],
    source_path: &Path,
) -> std::io::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");

    let parent = source_path.parent().unwrap_or(Path::new("."));

    let tlc_path = parent.join(format!("{stem}.tlc"));
    let tls_path = parent.join(format!("{stem}.tls"));

    let stub = stub_gen::generate_stub(stmts);
    std::fs::write(&tls_path, &stub)?;

    // Attempt native compilation.
    match compile_native(stmts) {
        Ok((dll_bytes, exports)) => {
            write_tlc_v1(source, stem, &exports, &dll_bytes, &tlc_path)?;
            println!(
                "NativeLib: {} function(s) embedded in {}",
                exports.len(),
                tlc_path.display()
            );
        }
        Err(e) => {
            eprintln!("NativeLib: skipped ({e})");
            write_tlc_v0(source, stem, &tlc_path)?;
        }
    }

    Ok((tlc_path, tls_path))
}

/// Platform native-library extension (`dll` / `so` / `dylib`).
/// Used when extracting the embedded DLL to a temp file at runtime.
pub fn native_lib_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// Load a `.tlc` file and return `(module_name, source_text)`.
///
/// If the file is v1, the embedded native data is placed in the
/// thread-local `NATIVE_CACHE` so that `exec.rs` can pick it up when
/// the module is later imported.
pub fn load_tlc(path: &Path) -> std::io::Result<(String, String)> {
    let data = std::fs::read(path)?;
    let (name, source, native_opt) = parse_tlc(&data)
        .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidData, msg))?;

    if let Some((exports, dll_bytes)) = native_opt {
        NATIVE_CACHE.with(|c| {
            c.borrow_mut().insert(name.clone(), (exports, dll_bytes));
        });
    }

    Ok((name, source))
}

// ── native compilation ────────────────────────────────────────────────────────

/// Compile eligible functions to a temporary DLL, read it back, and return
/// the raw bytes together with the export table.
fn compile_native(stmts: &[Stmt]) -> Result<(Vec<u8>, Vec<codegen::FnExport>), String> {
    let (rust_src, exports) = codegen::generate_rust_module(stmts)
        .ok_or_else(|| "no codegen-eligible functions".to_string())?;

    let fn_names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    eprintln!(
        "NativeLib: compiling {} function(s): {}",
        fn_names.len(),
        fn_names.join(", ")
    );

    let tmp_dir = std::env::temp_dir();
    let rs_path = tmp_dir.join("tl_native_module.rs");
    let ext = native_lib_ext();
    let dll_path = tmp_dir.join(format!("tl_native_module.{ext}"));

    std::fs::write(&rs_path, &rust_src)
        .map_err(|e| format!("cannot write temp source: {e}"))?;

    let compile_result = invoke_rustc(&rs_path, &dll_path);
    let _ = std::fs::remove_file(&rs_path);
    compile_result?;

    let dll_bytes = std::fs::read(&dll_path)
        .map_err(|e| format!("cannot read compiled DLL: {e}"))?;
    let _ = std::fs::remove_file(&dll_path);

    Ok((dll_bytes, exports))
}

/// Invoke `rustc --crate-type cdylib` to compile `rs_path` → `dll_path`.
fn invoke_rustc(rs_path: &Path, dll_path: &Path) -> Result<(), String> {
    let output = Command::new("rustc")
        .args([
            "--edition", "2021",
            "--crate-type", "cdylib",
            "-C", "opt-level=3",
            "-o", dll_path.to_str().unwrap_or("output"),
            rs_path.to_str().unwrap_or(""),
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(format!("rustc failed:\n{stderr}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err("rustc not found in PATH".to_string())
        }
        Err(e) => Err(format!("cannot run rustc: {e}")),
    }
}

// ── writers ───────────────────────────────────────────────────────────────────

fn write_tlc_v0(source: &str, module_name: &str, path: &Path) -> std::io::Result<()> {
    let name_bytes = module_name.as_bytes();
    let src_bytes = source.as_bytes();

    let mut buf = Vec::with_capacity(4 + 4 + 4 + name_bytes.len() + 4 + src_bytes.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION_V0.to_le_bytes());
    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&(src_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(src_bytes);

    std::fs::write(path, buf)
}

fn write_tlc_v1(
    source: &str,
    module_name: &str,
    exports: &[codegen::FnExport],
    dll_bytes: &[u8],
    path: &Path,
) -> std::io::Result<()> {
    let name_bytes = module_name.as_bytes();
    let src_bytes = source.as_bytes();

    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION_V1.to_le_bytes());
    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&(src_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(src_bytes);

    buf.extend_from_slice(&(exports.len() as u32).to_le_bytes());
    for exp in exports {
        let fn_name_bytes = exp.name.as_bytes();
        buf.extend_from_slice(&(fn_name_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(fn_name_bytes);
        buf.extend_from_slice(&(exp.n_params as u32).to_le_bytes());
    }

    buf.extend_from_slice(&(dll_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(dll_bytes);

    std::fs::write(path, buf)
}

// ── reader ────────────────────────────────────────────────────────────────────

/// Returns `(module_name, source, Option<(exports, dll_bytes)>)`.
fn parse_tlc(
    data: &[u8],
) -> Result<(String, String, Option<(Vec<codegen::FnExport>, Vec<u8>)>), String> {
    let mut pos = 0;

    if data.len() < 4 || &data[..4] != MAGIC {
        return Err("not a valid .tlc file (bad magic)".into());
    }
    pos += 4;

    let version = read_u32(data, &mut pos)?;
    if version > VERSION_V1 {
        return Err(format!("unsupported .tlc version {version}"));
    }

    let name_len = read_u32(data, &mut pos)? as usize;
    let name_bytes = read_bytes(data, &mut pos, name_len)?;
    let module_name = String::from_utf8(name_bytes.to_vec())
        .map_err(|_| "module name is not valid UTF-8".to_string())?;

    let src_len = read_u32(data, &mut pos)? as usize;
    let src_bytes = read_bytes(data, &mut pos, src_len)?;
    let source = String::from_utf8(src_bytes.to_vec())
        .map_err(|_| "source is not valid UTF-8".to_string())?;

    if version == VERSION_V0 {
        return Ok((module_name, source, None));
    }

    // version == 1: parse native section
    let n_fns = read_u32(data, &mut pos)? as usize;
    let mut exports = Vec::with_capacity(n_fns);
    for _ in 0..n_fns {
        let fn_name_len = read_u32(data, &mut pos)? as usize;
        let fn_name_bytes = read_bytes(data, &mut pos, fn_name_len)?;
        let fn_name = String::from_utf8(fn_name_bytes.to_vec())
            .map_err(|_| "function name is not valid UTF-8".to_string())?;
        let n_params = read_u32(data, &mut pos)? as usize;
        exports.push(codegen::FnExport { name: fn_name, n_params });
    }

    let dll_len = read_u32(data, &mut pos)? as usize;
    let dll_bytes = read_bytes(data, &mut pos, dll_len)?.to_vec();

    Ok((module_name, source, Some((exports, dll_bytes))))
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    if data.len() < *pos + 4 {
        return Err("unexpected end of .tlc data".into());
    }
    let v = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], String> {
    if data.len() < *pos + len {
        return Err("unexpected end of .tlc data".into());
    }
    let slice = &data[*pos..*pos + len];
    *pos += len;
    Ok(slice)
}
