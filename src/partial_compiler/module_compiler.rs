/// `.arc` compiled module — writer, native compilation, and the runtime cache of embedded DLLs.
///
/// ⚠ The file format (and the reader) lives in `crate::arc_format`: the import processing reads the
///   embedded source with it, in the VS Code extension's wasm too (editor_import_resolution_plan.md 3-2).
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use super::llvm_codegen as codegen;
use super::stub_gen;
use crate::arc_format::{MAGIC, VERSION_V0, VERSION_V1};
use crate::ast::Stmt;
use crate::type_check::AstAnnotations;

// ---------------------------------------------------------------------------
// Thread-local cache: module_name → (exports, NativePayload)
// Populated by load_tlc(); consumed by exec.rs.
// ---------------------------------------------------------------------------

/// Payload stored in the native cache and embedded in .arc v1.
#[derive(Clone)]
pub enum NativePayload {
    /// v1: raw shared-library bytes, written to a temp file and loaded via libloading.
    Dll(Vec<u8>),
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
///
/// `annotations` は型検査が生成した AST 型解決層の注釈（#16 段階(c)）。codegen が
/// node-id 索引で解決型を引き、自前の型再導出を置き換えるために使う。
pub fn compile(
    source: &str,
    stmts: &[Stmt],
    source_path: &Path,
    annotations: &AstAnnotations,
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
    match compile_native(stmts, annotations) {
        Ok((payload, exports)) => {
            let NativePayload::Dll(bytes) = &payload;
            write_tlc_native(source, stem, &exports, bytes, VERSION_V1, &tlc_path)?;
            println!(
                "NativeLib: {} function(s) (DLL) embedded in {}",
                exports.len(), tlc_path.display()
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

/// Load a `.arc` file and return `(module_name, source_text)`.
///
/// If the file is v1, the embedded native data is placed in the
/// thread-local `NATIVE_CACHE` so that `exec.rs` can pick it up when
/// the module is later imported.
///
/// ⚠ 「`.arc` が古いかどうか」の判定には使わない（古いと判った後に DLL がキャッシュに残る＝
///   新しいソース × 古い DLL という最悪の組み合わせになる）。判定は形式を読むだけの
///   `crate::arc_format::read` で行う（`parser/imports/ar_modules.rs`）。
pub fn load_tlc(path: &Path) -> std::io::Result<(String, String)> {
    let arc = crate::arc_format::read(path)?;
    let name = arc.module_name;

    if let Some((exports, dll)) = arc.native {
        let exports = exports
            .into_iter()
            .map(|(name, n_params)| codegen::FnExport { name, n_params })
            .collect();
        NATIVE_CACHE.with(|c| {
            c.borrow_mut().insert(name.clone(), (exports, NativePayload::Dll(dll)));
        });
    }

    Ok((name, arc.source))
}

// ── native compilation ────────────────────────────────────────────────────────

/// Compile eligible functions to a `NativePayload`.
///
/// Emits LLVM IR text via `llvm_codegen`, then compiles it to a shared library
/// with the external `clang` driver.
fn compile_native(
    stmts: &[Stmt],
    annotations: &AstAnnotations,
) -> Result<(NativePayload, Vec<codegen::FnExport>), String> {
    let (llvm_ir, exports) = codegen::generate_llvm_module(stmts, annotations)
        .ok_or_else(|| "no codegen-eligible functions".to_string())?;

    let fn_names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    eprintln!("NativeLib(clang): compiling {} function(s): {}", fn_names.len(), fn_names.join(", "));

    let tmp_dir  = std::env::temp_dir();
    let ll_path  = tmp_dir.join("ar_native_module.ll");
    let ext      = native_lib_ext();
    let dll_path = tmp_dir.join(format!("ar_native_module.{ext}"));

    std::fs::write(&ll_path, &llvm_ir)
        .map_err(|e| format!("cannot write LLVM IR: {e}"))?;

    // 開発用フック（#16 段階(c)）: `AR_DUMP_LL=<path>` を指定すると生成 IR をそこへも保存する。
    // codegen 変更が IR を変えていないこと（byte-identical）を差分で検証するために使う。
    if let Ok(dump) = std::env::var("AR_DUMP_LL") {
        if !dump.is_empty() {
            let _ = std::fs::write(&dump, &llvm_ir);
        }
    }

    let result = invoke_clang(&ll_path, &dll_path);
    let _ = std::fs::remove_file(&ll_path);
    result?;

    let dll_bytes = std::fs::read(&dll_path)
        .map_err(|e| format!("cannot read compiled DLL: {e}"))?;
    let _ = std::fs::remove_file(&dll_path);
    Ok((NativePayload::Dll(dll_bytes), exports))
}

/// Invoke `clang -O3 -shared` to compile `ll_path` → `dll_path`.
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
        .is_ok_and(|o| o.status.success())
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

/// `[u32 LE length][bytes]` を書き出す。
fn write_len_prefixed(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// ソーステキストのみを埋め込んだ v0 形式の `.arc` ファイルを書き出す。
fn write_tlc_v0(source: &str, module_name: &str, path: &Path) -> std::io::Result<()> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION_V0.to_le_bytes());
    write_len_prefixed(&mut buf, module_name.as_bytes());
    write_len_prefixed(&mut buf, source.as_bytes());
    std::fs::write(path, buf)
}

/// ネイティブペイロード(DLL バイト列)付きの `.arc` を書き出す。
fn write_tlc_native(
    source: &str,
    module_name: &str,
    exports: &[codegen::FnExport],
    payload: &[u8],
    version: u32,
    path: &Path,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&version.to_le_bytes());
    write_len_prefixed(&mut buf, module_name.as_bytes());
    write_len_prefixed(&mut buf, source.as_bytes());
    buf.extend_from_slice(&(exports.len() as u32).to_le_bytes());
    for exp in exports {
        write_len_prefixed(&mut buf, exp.name.as_bytes());
        buf.extend_from_slice(&(exp.n_params as u32).to_le_bytes());
    }
    write_len_prefixed(&mut buf, payload);
    std::fs::write(path, buf)
}
