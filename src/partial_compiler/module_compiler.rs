/// `.arc` compiled module format — writer and reader.
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

use super::llvm_codegen as codegen;
use super::stub_gen;
use crate::ast::Stmt;

const MAGIC: &[u8; 4] = b"TLC\x00";
const VERSION_V0: u32 = 0;
const VERSION_V1: u32 = 1;
/// v2: LLVM bitcode embedded instead of a native DLL.
const VERSION_V2: u32 = 2;

// ---------------------------------------------------------------------------
// Thread-local cache: module_name → (exports, NativePayload)
// Populated by load_tlc(); consumed by exec.rs.
// ---------------------------------------------------------------------------

/// Payload stored in the native cache and embedded in .arc v1/v2.
#[derive(Clone)]
pub enum NativePayload {
    /// v1: raw shared-library bytes, written to a temp file and loaded via libloading.
    Dll(Vec<u8>),
    /// v2: LLVM bitcode, re-JIT'd in-process via inkwell (no temp file needed).
    Bitcode(Vec<u8>),
}

thread_local! {
    static NATIVE_CACHE: RefCell<HashMap<String, (Vec<codegen::FnExport>, NativePayload)>> =
        RefCell::new(HashMap::new());
}

/// Consume and return the cached native data for a module (if any).
pub fn take_native_bytes(module_name: &str) -> Option<(Vec<codegen::FnExport>, NativePayload)> {
    NATIVE_CACHE.with(|c| c.borrow_mut().remove(module_name))
}

/// Insert pre-compiled native data (DLL bytes) into the cache (used by `rs_loader`).
pub fn cache_native(module_name: &str, exports: Vec<codegen::FnExport>, dll_bytes: Vec<u8>) {
    NATIVE_CACHE.with(|c| c.borrow_mut().insert(
        module_name.to_string(),
        (exports, NativePayload::Dll(dll_bytes)),
    ));
}

// ── public API ────────────────────────────────────────────────────────────────

/// Compile `source` (already parsed into `stmts`) and write `.arc` + `.ars`
/// next to the original `source_path`.
///
/// If native compilation succeeds, the `.arc` is written as **version 1**
/// with the DLL bytes embedded inside it.  No separate `_tl.*` file is
/// created.  If `rustc` is unavailable or no functions are eligible, the
/// `.arc` is written as version 0 (source-only) and a warning is printed.
///
/// Returns the paths of the two output files (`.arc`, `.ars`) on success.
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

    let tlc_path = parent.join(format!("{stem}.arc"));
    let tls_path = parent.join(format!("{stem}.ars"));

    let stub = stub_gen::generate_stub(stmts);
    std::fs::write(&tls_path, &stub)?;

    // Attempt native compilation.
    match compile_native(stmts) {
        Ok((payload, exports)) => {
            match &payload {
                NativePayload::Bitcode(bytes) => {
                    write_tlc_v2(source, stem, &exports, bytes, &tlc_path)?;
                    println!(
                        "NativeLib: {} function(s) (LLVM bitcode) embedded in {}",
                        exports.len(), tlc_path.display()
                    );
                }
                NativePayload::Dll(bytes) => {
                    write_tlc_v1(source, stem, &exports, bytes, &tlc_path)?;
                    println!(
                        "NativeLib: {} function(s) (DLL) embedded in {}",
                        exports.len(), tlc_path.display()
                    );
                }
            }
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

/// Load a `.arc` file and return `(module_name, source_text)`.
///
/// If the file is v1, the embedded native data is placed in the
/// thread-local `NATIVE_CACHE` so that `exec.rs` can pick it up when
/// the module is later imported.
pub fn load_tlc(path: &Path) -> std::io::Result<(String, String)> {
    let data = std::fs::read(path)?;
    let (name, source, native_opt) = parse_tlc(&data)
        .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidData, msg))?;

    if let Some((exports, payload)) = native_opt {
        NATIVE_CACHE.with(|c| {
            c.borrow_mut().insert(name.clone(), (exports, payload));
        });
    }

    Ok((name, source))
}

// ── native compilation ────────────────────────────────────────────────────────

/// Compile eligible functions to a `NativePayload`.
///
/// Priority:
///   1. inkwell JIT (feature = "llvm"): produces LLVM bitcode, no external tools.
///   2. clang fallback: produces a DLL via the old text-IR pipeline.
fn compile_native(stmts: &[Stmt]) -> Result<(NativePayload, Vec<codegen::FnExport>), String> {
    // ── inkwell path (preferred) ──────────────────────────────────────────────
    #[cfg(feature = "llvm")]
    {
        match super::inkwell_codegen::get_bitcode(stmts) {
            Ok((bitcode, exports)) => return Ok((NativePayload::Bitcode(bitcode), exports)),
            Err(e) => eprintln!("NativeLib(inkwell): {e}"),
        }
    }

    // ── clang fallback ────────────────────────────────────────────────────────
    let (llvm_ir, exports) = codegen::generate_llvm_module(stmts)
        .ok_or_else(|| "no codegen-eligible functions".to_string())?;

    let fn_names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    eprintln!("NativeLib(clang): compiling {} function(s): {}", fn_names.len(), fn_names.join(", "));

    let tmp_dir  = std::env::temp_dir();
    let ll_path  = tmp_dir.join("ar_native_module.ll");
    let ext      = native_lib_ext();
    let dll_path = tmp_dir.join(format!("ar_native_module.{ext}"));

    std::fs::write(&ll_path, &llvm_ir)
        .map_err(|e| format!("cannot write LLVM IR: {e}"))?;

    let result = invoke_clang(&ll_path, &dll_path);
    let _ = std::fs::remove_file(&ll_path);
    result?;

    let dll_bytes = std::fs::read(&dll_path)
        .map_err(|e| format!("cannot read compiled DLL: {e}"))?;
    let _ = std::fs::remove_file(&dll_path);
    Ok((NativePayload::Dll(dll_bytes), exports))
}

/// Invoke `clang -O3 -shared` to compile `ll_path` → `dll_path` (fallback path).
fn invoke_clang(ll_path: &Path, dll_path: &Path) -> Result<(), String> {
    let out_str = dll_path.to_str().unwrap_or("output");
    let in_str  = ll_path.to_str().unwrap_or("");
    let mut args: Vec<&str> = vec!["-O3", "-shared", "-o", out_str, in_str];
    #[cfg(not(target_os = "windows"))]
    args.push("-fPIC");
    #[cfg(target_os = "windows")]
    args.extend_from_slice(&["-Wno-dll-attribute-on-redeclaration"]);

    // Try clang from PATH first; fall back to llvm.path in ar_config.json.
    let clang_exe = if Command::new("clang").arg("--version").output()
        .map_or(false, |o| o.status.success())
    {
        std::path::PathBuf::from("clang")
    } else {
        let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        let from_config = std::env::current_dir().ok()
            .and_then(|d| std::fs::read_to_string(d.join("ar_config.json")).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|j| j.get("llvm")?.get("path")?.as_str().map(|s| s.to_string()))
            .map(|p| std::path::PathBuf::from(p).join("bin").join(format!("clang{ext}")));
        from_config.filter(|p| p.exists()).unwrap_or_else(|| std::path::PathBuf::from("clang"))
    };

    let output = Command::new(&clang_exe).args(&args).output();
    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!("clang failed:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
            Err("clang not found in PATH (or llvm.path in ar_config.json)".to_string()),
        Err(e) => Err(format!("cannot run clang: {e}")),
    }
}

// ── writers ───────────────────────────────────────────────────────────────────

/// ソーステキストのみを埋め込んだ v0 形式の `.arc` ファイルを書き出す。
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

/// ソーステキストとネイティブ DLL バイト列を埋め込んだ v1 形式の `.arc` ファイルを書き出す。
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

/// v2: identical layout to v1 but uses LLVM bitcode bytes instead of DLL bytes.
fn write_tlc_v2(
    source: &str,
    module_name: &str,
    exports: &[codegen::FnExport],
    bitcode: &[u8],
    path: &Path,
) -> std::io::Result<()> {
    let name_bytes = module_name.as_bytes();
    let src_bytes  = source.as_bytes();
    let mut buf    = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION_V2.to_le_bytes());
    buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(&(src_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(src_bytes);
    buf.extend_from_slice(&(exports.len() as u32).to_le_bytes());
    for exp in exports {
        let nb = exp.name.as_bytes();
        buf.extend_from_slice(&(nb.len() as u32).to_le_bytes());
        buf.extend_from_slice(nb);
        buf.extend_from_slice(&(exp.n_params as u32).to_le_bytes());
    }
    buf.extend_from_slice(&(bitcode.len() as u32).to_le_bytes());
    buf.extend_from_slice(bitcode);
    std::fs::write(path, buf)
}

// ── reader ────────────────────────────────────────────────────────────────────

/// Returns `(module_name, source, Option<(exports, NativePayload)>)`.
fn parse_tlc(
    data: &[u8],
) -> Result<(String, String, Option<(Vec<codegen::FnExport>, NativePayload)>), String> {
    let mut pos = 0;

    if data.len() < 4 || &data[..4] != MAGIC {
        return Err("not a valid .arc file (bad magic)".into());
    }
    pos += 4;

    let version = read_u32(data, &mut pos)?;
    if version > VERSION_V2 {
        return Err(format!("unsupported .arc version {version}"));
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

    // version 1 or 2: parse fn export table (identical layout)
    let n_fns = read_u32(data, &mut pos)? as usize;
    let mut exports = Vec::with_capacity(n_fns);
    for _ in 0..n_fns {
        let fn_name_len   = read_u32(data, &mut pos)? as usize;
        let fn_name_bytes = read_bytes(data, &mut pos, fn_name_len)?;
        let fn_name       = String::from_utf8(fn_name_bytes.to_vec())
            .map_err(|_| "function name is not valid UTF-8".to_string())?;
        let n_params = read_u32(data, &mut pos)? as usize;
        exports.push(codegen::FnExport { name: fn_name, n_params, class_name: None });
    }

    let payload_len   = read_u32(data, &mut pos)? as usize;
    let payload_bytes = read_bytes(data, &mut pos, payload_len)?.to_vec();
    let payload = if version == VERSION_V1 {
        NativePayload::Dll(payload_bytes)
    } else {
        NativePayload::Bitcode(payload_bytes)
    };

    Ok((module_name, source, Some((exports, payload))))
}

/// バイト列の現在位置から u32 をリトルエンディアンで読み取り、位置を4バイト進める。
fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    if data.len() < *pos + 4 {
        return Err("unexpected end of .arc data".into());
    }
    let v = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

/// バイト列の現在位置から `len` バイトのスライスを返し、位置を進める。
fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], String> {
    if data.len() < *pos + len {
        return Err("unexpected end of .arc data".into());
    }
    let slice = &data[*pos..*pos + len];
    *pos += len;
    Ok(slice)
}
