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
use crate::cpp_header::CppBuildConfig;
use crate::cpp_header::types::{CStructDef, CFnSig, CType};

// ── Compiler constants ────────────────────────────────────────────────────────

const TMP_RS_NAME: &str = "_tl_cpp_bridge.rs";
const TMP_DLL_STEM: &str = "_tl_cpp_bridge";
const RUSTC_EDITION: &str = "2021";
const RUSTC_OPT_LEVEL: &str = "2";
const HV_DLL_PREFIX: &str = "ar_";
const TL_SYMS_EXT: &str = "syms";
const TL_SHIM_SUFFIX: &str = "_shim";
const MAX_COMPILE_PASSES: usize = 5;
/// `.syms` の 1 行目に書く恒久キャッシュの刻印の接頭辞（`cache_stamp`）。
const SYMS_STAMP_PREFIX: &str = "#stamp ";
// RTLD_LAZY = 1 on all POSIX platforms; embedded as a literal in the PLATFORM_LOADER string.
#[allow(dead_code)]
const RTLD_LAZY: i32 = 1;

// ── MSVC toolchain ────────────────────────────────────────────────────────────

/// MSVC ツールチェーンへのパス。
pub struct MsvcPaths {
    pub vcvarsall: PathBuf,
}

/// Built-in Visual Studio installation candidates searched when no explicit
/// `msvc` path is given in `ar_config.json`.
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
/// `extra_paths`（`ar_config.json` の `msvc_search_paths`）を先に確認し、見つからなければ組み込み `MSVC_CANDIDATES` リストにフォールバックする。
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

        // Wrapper name and export mechanism.
        //
        // Namespaced functions (DxLib-style): the real function lives in a C++
        // namespace, so a global-scope extern "C" wrapper with the same name does
        // not collide — plain `__declspec(dllexport)` works.
        //
        // Namespace-less functions (plain C headers, `extern "C" int f(V3*);`):
        // the header already declares `f` at global scope with C linkage. Defining
        // an extern "C" wrapper `f(void*)` alongside it is an illegal overload
        // (C2733), and defining it with identical parameter types would shadow the
        // library symbol and recurse into itself. Instead, define the wrapper under
        // a unique internal name and export it under the real name via the linker
        // (`/EXPORT:f=ar_shim_f` — extern "C" symbols are undecorated on x64).
        let (def_name, dllexport, export_pragma) = if sig.namespace.is_some() {
            (sig.name.clone(), "__declspec(dllexport) ", None)
        } else {
            let shim_name = format!("ar_shim_{}", sig.name);
            let pragma = format!(
                "#pragma comment(linker, \"/EXPORT:{}={shim_name}\")\n",
                sig.name
            );
            (shim_name, "", Some(pragma))
        };

        if sig.ret == CType::Void {
            src.push_str(&format!(
                "{dllexport}{ret_c} {def_name}({params_str}) {{ {callee}; }}\n"
            ));
        } else if let CType::ByValueStruct { type_name } = &sig.ret {
            // By-value struct return: write result into a per-function static buffer
            // and return a void* pointer to it. Not thread-safe, but sufficient for
            // single-threaded tl usage.
            let name = &sig.name;
            src.push_str(&format!(
                "static {type_name} _ret_buf_{name};\n\
                 {dllexport}void* {def_name}({params_str}) {{ _ret_buf_{name} = {callee}; return (void*)&_ret_buf_{name}; }}\n"
            ));
        } else {
            src.push_str(&format!(
                "{dllexport}{ret_c} {def_name}({params_str}) {{ return ({ret_c}){callee}; }}\n"
            ));
        }
        if let Some(pragma) = export_pragma {
            src.push_str(&pragma);
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

    let dll_path = header_dir.join(format!("{HV_DLL_PREFIX}{stem}.{ext}"));
    let shim_path = header_dir.join(format!("{HV_DLL_PREFIX}{stem}{TL_SHIM_SUFFIX}.{ext}"));
    let syms_path = header_dir.join(format!("{HV_DLL_PREFIX}{stem}.{TL_SYMS_EXT}"));

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
    // 中継 DLL はシムを**ファイル名だけ**で指し、自分と同じディレクトリから読む（生成ローダーの `resolve`）。
    // ⚠ 以前は絶対パスを埋め込んでいたので、生成物を別のチェックアウトへ持っていくと元の場所のシムを読んでいた。
    let shim_name = format!("{HV_DLL_PREFIX}{stem}{TL_SHIM_SUFFIX}.{ext}");

    // Permanent cache: reuse the wrapper DLL only while its stamp matches what we would generate now
    // (ffi_value_exchange_plan.md 1-5). A stale DLL (generator, header or config changed) is rebuilt.
    let stamp = cache_stamp(&effective_sigs, struct_defs, header_name, &shim_name, config);
    if dll_path.exists() {
        match read_syms_file(&syms_path, sigs) {
            Some((Some(saved), effective)) if saved == stamp => {
                eprintln!("CppBridge: loading '{}' (permanent)", dll_path.display());
                return Ok((dll_path, effective));
            }
            _ => eprintln!(
                "CppBridge: '{}' is stale (generator, header or build config changed), rebuilding",
                dll_path.display()
            ),
        }
    }

    // Find MSVC toolchain
    let msvc = if let Some(ref p) = config.msvc {
        if !p.exists() {
            return Err(format!(
                "CppBridge: msvc path '{}' not found (check ar_config.json)",
                p.display()
            ));
        }
        MsvcPaths {
            vcvarsall: p.clone(),
        }
    } else {
        find_msvc_vcvarsall(&config.msvc_search_paths).ok_or_else(|| {
            "CppBridge: MSVC not found.\n\
             Install Visual Studio 2017/2019/2022, add paths to ar_config.json:\n\
             {\"cpp\": {\"msvc_search_paths\": [\"C:/path/to/vcvarsall.bat\"]}}"
                .to_string()
        })?
    };

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

    // Generate Rust wrapper source that loads the shim (next to the wrapper) at runtime.
    let wrapper_src = gen_dll_wrapper(&shim_name, &effective_sigs, struct_defs);

    // Compile Rust wrapper → tl_{stem}.dll
    let wrapper_bytes = compile_wrapper(&wrapper_src, &[])?;
    std::fs::write(&dll_path, &wrapper_bytes)
        .map_err(|e| format!("CppBridge: cannot write '{}': {e}", dll_path.display()))?;

    // Save the stamp and the effective function list so future cache hits know what was compiled.
    let names: Vec<&str> = effective_sigs.iter().map(|s| s.name.as_str()).collect();
    let syms = format!("{SYMS_STAMP_PREFIX}{stamp}\n{}", names.join("\n"));
    std::fs::write(&syms_path, syms)
        .map_err(|e| format!("CppBridge: cannot write '{}': {e}", syms_path.display()))?;

    eprintln!("CppBridge: generated '{}'", dll_path.display());
    Ok((dll_path, effective_sigs))
}

/// 恒久キャッシュの刻印（ffi_value_exchange_plan.md 1-5）。シムと中継 DLL の**生成ソース**と、
/// ビルドに効く設定から計算する。ヘッダの関数・構造体、ラッパーの生成内容、設定のどれが変わっても値が変わる。
///
/// - 生成物の置き場所には依存しない（ヘッダもシムもファイル名で入る）。別のチェックアウトでも同じ値になる
/// - ⚠ 実装ライブラリ（`.lib`）の中身は含めない。ライブラリだけを作り直したときは生成物（`ar_*`）を消すこと
/// - ハッシュは FNV-1a 64（`exec/mod.rs` の `simple_hash` と同じ）。Rust の版で値が変わらないものを使う
fn cache_stamp(
    sigs: &[CFnSig],
    struct_defs: &[CStructDef],
    header_name: &str,
    shim_name: &str,
    config: &CppBuildConfig,
) -> String {
    let shim_src = gen_cpp_shim_source(
        sigs,
        header_name,
        &config.precompile_macros,
        config.win32_lean_and_mean,
    );
    let wrapper_src = gen_dll_wrapper(shim_name, sigs, struct_defs);
    let cfg = format!(
        "{}|{:?}|{:?}|{:?}|{:?}",
        config.target_arch,
        config.cl_extra_flags,
        config.link_extra_flags,
        config.system_libs,
        config.lib_patterns
    );
    let mut h: u64 = 0xcbf29ce484222325;
    for b in shim_src.bytes().chain(wrapper_src.bytes()).chain(cfg.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// `.syms` コンパニオンファイルを読み込み、`(刻印, コンパイル済みの all_sigs のサブセット)` を返す。
/// ファイルが無ければ `None`。1 行目が刻印でない（1-5 より前の）ファイルは刻印が `None`（作り直しになる）。
fn read_syms_file(syms_path: &Path, all_sigs: &[CFnSig]) -> Option<(Option<String>, Vec<CFnSig>)> {
    let text = std::fs::read_to_string(syms_path).ok()?;
    let mut lines = text.lines().peekable();
    let stamp = match lines.peek() {
        Some(first) if first.starts_with(SYMS_STAMP_PREFIX) => {
            let s = first[SYMS_STAMP_PREFIX.len()..].trim().to_string();
            lines.next();
            Some(s)
        }
        _ => None,
    };
    let allowed: std::collections::HashSet<&str> = lines.collect();
    let effective = all_sigs
        .iter()
        .filter(|s| allowed.contains(s.name.as_str()))
        .cloned()
        .collect();
    Some((stamp, effective))
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
    let temp_dir = std::env::temp_dir().join(format!("ar_build_{stem}"));
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
            Some((pri, base)) => best.get(&base).is_none_or(|&best_pri| pri <= best_pri),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(name: &str, ret: CType) -> CFnSig {
        CFnSig {
            name: name.to_string(),
            params: vec![("x".to_string(), CType::Int)],
            ret,
            namespace: None,
            n_required: 1,
        }
    }

    fn rec() -> CStructDef {
        CStructDef {
            name: "Rec".to_string(),
            fields: vec![("id".to_string(), CType::Int), ("score".to_string(), CType::Double)],
            complete: true,
        }
    }

    /// 刻印は同じ入力なら同じ値（置き場所に依存しない・ヘッダもシムもファイル名で入る）。
    #[test]
    fn cache_stamp_is_deterministic() {
        let cfg = CppBuildConfig::default();
        let sigs = vec![sig("f", CType::Int)];
        let a = cache_stamp(&sigs, &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
        let b = cache_stamp(&sigs, &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    /// 関数・構造体・設定のどれが変わっても刻印が変わる（古い生成物を使い続けない）。
    #[test]
    fn cache_stamp_changes_with_inputs() {
        let cfg = CppBuildConfig::default();
        let base = cache_stamp(&[sig("f", CType::Int)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
        let more_fns =
            cache_stamp(&[sig("f", CType::Int), sig("g", CType::Int)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
        let other_ret = cache_stamp(&[sig("f", CType::Double)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg);
        let no_struct = cache_stamp(&[sig("f", CType::Int)], &[], "lib.h", "ar_lib_shim.dll", &cfg);
        let mut cfg2 = CppBuildConfig::default();
        cfg2.cl_extra_flags.push("/DFOO".to_string());
        let other_cfg = cache_stamp(&[sig("f", CType::Int)], &[rec()], "lib.h", "ar_lib_shim.dll", &cfg2);
        for other in [&more_fns, &other_ret, &no_struct, &other_cfg] {
            assert_ne!(&base, other);
        }
    }

    /// `.syms` の 1 行目の刻印を読み、残りを関数名として扱う。刻印の無い（1-5 より前の）ファイルは刻印 `None`。
    #[test]
    fn read_syms_file_parses_stamp() {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let with_stamp = dir.join(format!("ar_test_syms_stamp_{pid}.syms"));
        let legacy = dir.join(format!("ar_test_syms_legacy_{pid}.syms"));
        std::fs::write(&with_stamp, "#stamp 0123456789abcdef\nf\ng").unwrap();
        std::fs::write(&legacy, "f\ng").unwrap();
        let all = vec![sig("f", CType::Int), sig("g", CType::Int), sig("h", CType::Int)];

        let (stamp, eff) = read_syms_file(&with_stamp, &all).unwrap();
        assert_eq!(stamp.as_deref(), Some("0123456789abcdef"));
        assert_eq!(eff.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["f", "g"]);

        let (stamp, eff) = read_syms_file(&legacy, &all).unwrap();
        assert_eq!(stamp, None);
        assert_eq!(eff.len(), 2);

        assert!(read_syms_file(&dir.join(format!("ar_test_syms_missing_{pid}.syms")), &all).is_none());
        let _ = std::fs::remove_file(&with_stamp);
        let _ = std::fs::remove_file(&legacy);
    }
}
