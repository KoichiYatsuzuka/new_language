// compiler.rs — Compile the generated Rust wrapper, drive MSVC for the C++ shim,
// and orchestrate the full tl_{stem}.dll build pipeline.
//
// Public API:
//   compile_wrapper   — compile a Rust cdylib source string → raw DLL bytes
//   compile_tl_dll    — full pipeline: parse → shim → wrapper → tl_XXX.dll
//   find_msvc_vcvarsall — locate vcvarsall.bat
//   gen_cpp_shim_source — generate the MSVC C++ shim source
//   MsvcPaths         — paths to the MSVC toolchain

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::codegen::gen_dll_wrapper;
use super::config::CppBuildConfig;
use super::types::{CStructDef, CFnSig, CType};

// ── Compiler constants ────────────────────────────────────────────────────────

const TMP_RS_NAME: &str = "_tl_cpp_bridge.rs";
const TMP_DLL_STEM: &str = "_tl_cpp_bridge";
const RUSTC_EDITION: &str = "2021";
const RUSTC_OPT_LEVEL: &str = "2";
const TL_DLL_PREFIX: &str = "tl_";
const TL_SYMS_EXT: &str = "syms";
const TL_SHIM_SUFFIX: &str = "_shim";
const MAX_COMPILE_PASSES: usize = 5;
// RTLD_LAZY = 1 on all POSIX platforms; embedded as a literal in the PLATFORM_LOADER string.
#[allow(dead_code)]
const RTLD_LAZY: i32 = 1;

// ── MSVC toolchain ────────────────────────────────────────────────────────────

/// MSVC ツールチェーンへのパス。
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

/// `vcvarsall.bat` を検索する。
/// `extra_paths`（`tl_config.json` の `msvc_search_paths`）を先に確認し、見つからなければ組み込み `MSVC_CANDIDATES` リストにフォールバックする。
/// インストールが見つからない場合は `None` を返す。
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

// ── Rust wrapper compiler ─────────────────────────────────────────────────────

/// `rust_src` を cdylib としてコンパイルし、生の DLL バイト列を返す。
/// `extra_link_dirs` は `-L` フラグとして渡される（`cpp-lib` がスタティックライブラリを探すために使用）。
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

// ── MSVC C++ shim generator ───────────────────────────────────────────────────

/// MSVC シム C++ ソースを生成する。
/// 各関数の名前空間には `CFnSig::namespace` を使用する（実際には全 DxLib 関数が同一名前空間）。
/// `win32_lean_and_mean` が `true` の場合、Windows ヘッダのインクルード前に `#define WIN32_LEAN_AND_MEAN` を出力する。
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

// ── Full build pipeline ───────────────────────────────────────────────────────

/// `header_path` の隣に `tl_{stem}.dll` を生成する。
/// 手順: (1) 既存 DLL があれば即座に返す (2) MSVC を検索 (3) C++ シム DLL をコンパイル (4) Rust ラッパーを DLL に変換。
/// 生成後は削除しない限り再ビルドしない。
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
                let bad = super::super::msvc_errors::extract_bad_fn_names(&err_msg);
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

/// `.syms` コンパニオンファイルを読み込み、コンパイル済みの `all_sigs` のサブセットを返す。
/// ファイルが存在しない場合（初回実行前など）は全シグネチャにフォールバックする。
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

// ── MSVC shim compiler ────────────────────────────────────────────────────────

/// `cpp_src` を MSVC `cl.exe` を使って `out_dll` に DLL としてコンパイルする。
/// ライブラリは `header_path` と同じディレクトリから `config.lib_patterns` に従って選択される（同名で複数存在する場合は最も具体的なパターンが優先）。
/// `config.system_libs` は SDK / Windows システムライブラリを提供し、アーキテクチャは `config.target_arch`、追加フラグは `config.cl_extra_flags` / `config.link_extra_flags` から取得する。
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

// ── Utilities ─────────────────────────────────────────────────────────────────

/// 同じベース名を持つライブラリのうち、最高優先度（`patterns` の最小インデックス）のパターンに一致するものだけを残す。どのパターンにも一致しないライブラリはそのまま保持する。
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

/// `std::fs::canonicalize` が付加する Windows 拡張パスプレフィックス `\\?\` を除去する。
/// `cl.exe` は `/I` や `/LIBPATH` フラグでこのプレフィックスを受け付けないため必要。
pub(crate) fn strip_unc_prefix(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// UTF-8 文字列をシステムの ANSI コードページバイト列に変換する。
/// 日本語 Windows では ANSI コードページは Shift-JIS（932）であり、.bat ファイルをこのエンコーディングで書き込むことで
/// cmd.exe が非 ASCII ディレクトリパス（DxLib インストールパス中の日本語文字など）を正しく解釈できる。
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
