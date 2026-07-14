# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""C++ shim compiler: generate and compile the extern-C wrapper DLL (mirrors compiler.rs).

For cpp-lib: generates a C++ shim that re-exports static-lib symbols as
             plain extern "C", then compiles it with MSVC cl.exe (Windows)
             or GCC/Clang (Linux/Mac).
For cpp-dll: no compilation needed — ctypes loads the DLL directly.
"""
from __future__ import annotations
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Optional

from .types import CFnSig, CStructDef, CType, CPtr, CCharPtr, CVoid, CByValueStruct

HV_DLL_PREFIX = "ar_"
TL_SYMS_EXT = "syms"
TL_SHIM_SUFFIX = "_shim"
MAX_COMPILE_PASSES = 5

MSVC_CANDIDATES = [
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
]


def find_msvc_vcvarsall(extra_paths: list) -> Optional[Path]:
    """Search for vcvarsall.bat. Returns its path or None."""
    for p in extra_paths:
        if Path(p).exists():
            return Path(p)
    for p in MSVC_CANDIDATES:
        if Path(p).exists():
            return Path(p)
    return None


def native_lib_ext() -> str:
    """Return the platform shared-library extension."""
    if sys.platform == "win32":
        return "dll"
    if sys.platform == "darwin":
        return "dylib"
    return "so"


# ── C++ shim source generator ─────────────────────────────────────────────────

def gen_cpp_shim_source(
    sigs: list,         # list[CFnSig]
    header_name: str,
    precompile_macros: list,    # list[str]
    win32_lean_and_mean: bool,
) -> str:
    """Generate a C++ shim source that re-exports sigs as plain extern "C" functions.

    Mirrors gen_cpp_shim_source from compiler.rs.
    """
    src = []
    if win32_lean_and_mean:
        src.append("#define WIN32_LEAN_AND_MEAN\n")
    for m in precompile_macros:
        src.append(f"#define {m}\n")
    src.append("#include <windows.h>\n")
    src.append(f'#include "{header_name}"\n\n')
    src.append('extern "C" {\n\n')

    for sig in sigs:
        ret_c = _ctype_c_str(sig.ret)
        params_parts = []
        for i, (pname, ct) in enumerate(sig.params):
            n = pname if pname else f"p{i}"
            params_parts.append(f"{_ctype_c_str(ct)} {n}")
        params_str = ", ".join(params_parts) if params_parts else "void"

        # Build call arguments (with casts for special types)
        args_parts = []
        for i, (pname, ct) in enumerate(sig.params):
            n = pname if pname else f"p{i}"
            match ct:
                case CCharPtr():
                    args_parts.append(f"(char*){n}")
                case _ if hasattr(ct, "type_name") and hasattr(ct, "mutable"):
                    # COpaqueStructPtr
                    if ct.mutable:  # type: ignore[union-attr]
                        args_parts.append(f"({ct.type_name}*){n}")  # type: ignore[union-attr]
                    else:
                        args_parts.append(f"(const {ct.type_name}*){n}")  # type: ignore[union-attr]
                case CByValueStruct(type_name=tn):
                    args_parts.append(f"*({tn}*){n}")
                case _:
                    args_parts.append(n)
        args_str = ", ".join(args_parts)

        # Callee with optional namespace prefix
        if sig.namespace:
            callee = f"{sig.namespace}::{sig.name}({args_str})"
        else:
            callee = f"{sig.name}({args_str})"

        # Undefine any Windows macro that might shadow the name
        src.append(f"#undef {sig.name}\n")

        # Wrapper name and export mechanism (mirrors compiler.rs).
        #
        # Namespaced functions (DxLib-style): the real function lives in a C++
        # namespace, so a global-scope extern "C" wrapper with the same name does
        # not collide — plain `__declspec(dllexport)` works.
        #
        # Namespace-less functions (plain C headers, `extern "C" int f(V3*);`):
        # the header already declares `f` at global scope with C linkage. Defining
        # an extern "C" wrapper `f(void*)` alongside it is an illegal overload
        # (C2733), and defining it with identical parameter types would shadow the
        # library symbol and recurse into itself. Instead, define the wrapper under
        # a unique internal name and export it under the real name via the linker
        # (`/EXPORT:f=ar_shim_f` — extern "C" symbols are undecorated on x64).
        if sig.namespace:
            def_name = sig.name
            dllexport = "__declspec(dllexport) "
            export_pragma = None
        else:
            def_name = f"ar_shim_{sig.name}"
            dllexport = ""
            export_pragma = (
                f'#pragma comment(linker, "/EXPORT:{sig.name}={def_name}")\n'
            )

        if isinstance(sig.ret, CVoid):
            src.append(
                f'{dllexport}{ret_c} {def_name}({params_str}) '
                f'{{ {callee}; }}\n'
            )
        elif isinstance(sig.ret, CByValueStruct):
            tn = sig.ret.type_name
            src.append(
                f"static {tn} _ret_buf_{sig.name};\n"
                f"{dllexport}void* {def_name}({params_str}) "
                f"{{ _ret_buf_{sig.name} = {callee}; "
                f"return (void*)&_ret_buf_{sig.name}; }}\n"
            )
        else:
            src.append(
                f"{dllexport}{ret_c} {def_name}({params_str}) "
                f"{{ return ({ret_c}){callee}; }}\n"
            )
        if export_pragma is not None:
            src.append(export_pragma)

    src.append("\n} // extern \"C\"\n")
    return "".join(src)


def _ctype_c_str(ct: CType) -> str:
    """Return the C type string for use in generated shim source."""
    from .types import (CInt, CLong, CFloat, CDouble, CBool, CVoidPtr,
                        COpaqueStructPtr, CFnPtr)
    match ct:
        case CInt(): return "int"
        case CLong(): return "long long"
        case CFloat(): return "float"
        case CDouble(): return "double"
        case CBool(): return "int"
        case CVoid(): return "void"
        case CVoidPtr(): return "void*"
        case CCharPtr(): return "const char*"
        case CPtr(inner=inner, mutable=mutable):
            return f"{_ctype_c_str(inner)}*" if mutable else f"const {_ctype_c_str(inner)}*"
        case COpaqueStructPtr(): return "void*"
        case CByValueStruct(): return "void*"
        case CFnPtr(): return "void*"
        case _: return "void*"


# ── MSVC shim compilation ─────────────────────────────────────────────────────

def _compile_msvc_shim(
    cpp_src: str,
    vcvarsall: Path,
    header_path: Path,
    out_dll: Path,
    config,             # CppBuildConfig
) -> None:
    """Compile cpp_src with MSVC cl.exe into out_dll. Raises RuntimeError on failure."""
    stem = out_dll.stem
    tmp_dir = Path(tempfile.gettempdir()) / f"ar_build_{stem}"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    cpp_file = tmp_dir / "shim.cpp"
    # Skip recompile if source and DLL are both unchanged
    prev_src = cpp_file.read_text(encoding="utf-8", errors="replace") if cpp_file.exists() else ""
    if out_dll.exists() and prev_src == cpp_src:
        print(f"CppShim: shim unchanged, reusing '{out_dll}'", file=sys.stderr)
        return

    cpp_file.write_text(cpp_src, encoding="utf-8")

    lib_dir = header_path.parent.resolve()
    libdir_str = str(lib_dir)

    # Collect .lib files matching config.lib_patterns
    header_stem = header_path.stem.lower()
    patterns_lc = [p.lower() for p in config.lib_patterns]
    lib_names: list[str] = []
    for entry in lib_dir.iterdir():
        if entry.suffix.lower() != ".lib":
            continue
        lower = entry.name.lower()
        if any(lower.endswith(pat) for pat in patterns_lc):
            if "_d." not in lower:  # skip debug builds
                lib_names.append(entry.name)

    final_libs = lib_names + list(config.system_libs)
    libs_str = " ".join(f'"{libdir_str}\\{lib}"' for lib in final_libs)

    extra_cl = " ".join(config.cl_extra_flags)
    extra_link = " ".join(config.link_extra_flags)
    arch = config.target_arch

    vcvarsall_str = str(vcvarsall)
    cpp_str = str(cpp_file)
    dll_str = str(out_dll)

    bat_content = (
        f"@echo off\r\n"
        f'call "{vcvarsall_str}" {arch}\r\n'
        f'cl.exe /nologo /LD /MD /W3 {extra_cl} '
        f'/I "{libdir_str}" '
        f'/Fe"{dll_str}" '
        f'"{cpp_str}" '
        f'{libs_str} '
        f'/link /LIBPATH:"{libdir_str}" /SUBSYSTEM:WINDOWS /NODEFAULTLIB:LIBCMT {extra_link}\r\n'
        f"exit /b %ERRORLEVEL%\r\n"
    )

    bat_file = tmp_dir / "build.bat"
    bat_file.write_bytes(bat_content.encode("mbcs", errors="replace"))

    print(f"CppShim: compiling '{out_dll.name}' with MSVC …", file=sys.stderr)
    result = subprocess.run(
        ["cmd", "/c", str(bat_file)],
        cwd=tmp_dir,
        capture_output=True,
    )

    if result.returncode != 0 or not out_dll.exists():
        out = result.stdout.decode("mbcs", errors="replace")
        err = result.stderr.decode("mbcs", errors="replace")
        raise RuntimeError(f"CppShim: cl.exe failed:\n{out}{err}")

    print(f"CppShim: produced '{out_dll}'", file=sys.stderr)


# ── GCC/Clang shim compilation (Linux / Mac) ─────────────────────────────────

def _compile_gcc_shim(
    c_src: str,
    header_path: Path,
    out_so: Path,
    config,             # CppBuildConfig
) -> None:
    """Compile c_src with GCC/g++ into a shared library. Raises RuntimeError on failure."""
    lib_dir = header_path.parent.resolve()
    tmp_dir = Path(tempfile.gettempdir())
    cpp_file = tmp_dir / f"ar_shim_{out_so.stem}.cpp"
    cpp_file.write_text(c_src, encoding="utf-8")

    compiler = "g++"
    cmd = [
        compiler, "-O2", "-shared", "-fPIC",
        f"-I{lib_dir}",
        "-o", str(out_so),
        str(cpp_file),
        f"-L{lib_dir}",
        "-Wl,--whole-archive",
    ]
    # Add any .a static libs
    for entry in lib_dir.iterdir():
        if entry.suffix == ".a":
            cmd.append(str(entry))
    cmd += ["-Wl,--no-whole-archive"]

    print(f"CppShim(gcc): compiling '{out_so.name}' …", file=sys.stderr)
    result = subprocess.run(cmd, capture_output=True)
    cpp_file.unlink(missing_ok=True)

    if result.returncode != 0 or not out_so.exists():
        err = result.stderr.decode(errors="replace")
        raise RuntimeError(f"CppShim(gcc): g++ failed:\n{err}")

    print(f"CppShim(gcc): produced '{out_so}'", file=sys.stderr)


# ── Full compile_tl_dll pipeline (cpp-lib) ───────────────────────────────────

def compile_tl_dll(
    header_path: Path,
    sigs: list,         # list[CFnSig]
    struct_defs: list,  # list[CStructDef]
    config,             # CppBuildConfig
) -> tuple:             # (Path, list[CFnSig])
    """Full build pipeline for cpp-lib: generate shim → compile → return DLL path.

    Mirrors compile_tl_dll from compiler.rs. The compiled DLL is cached permanently
    next to the header as ar_{stem}.dll so subsequent imports skip compilation.
    """
    ext = native_lib_ext()
    header_abs = header_path.resolve()
    header_dir = header_abs.parent
    stem = header_abs.stem
    dll_path = header_dir / f"{HV_DLL_PREFIX}{stem}.{ext}"
    shim_path = header_dir / f"{HV_DLL_PREFIX}{stem}{TL_SHIM_SUFFIX}.{ext}"
    syms_path = header_dir / f"{HV_DLL_PREFIX}{stem}.{TL_SYMS_EXT}"

    # Deduplicate by name
    seen: set[str] = set()
    effective_sigs = [s for s in sigs if not seen.__contains__(s.name) and not seen.add(s.name)]  # type: ignore[func-returns-value]

    # Permanent cache: wrapper already exists → read saved symbols and return
    if dll_path.exists():
        print(f"CppBridge: loading '{dll_path}' (permanent)", file=sys.stderr)
        effective_sigs = _read_syms_file(syms_path, effective_sigs)
        return dll_path, effective_sigs

    header_name = header_abs.name

    if sys.platform == "win32":
        # Find MSVC
        vcvarsall = (
            Path(str(config.msvc)) if config.msvc else
            find_msvc_vcvarsall(config.msvc_search_paths)
        )
        if vcvarsall is None:
            raise RuntimeError(
                "CppBridge: MSVC not found.\n"
                "Install Visual Studio 2017/2019/2022 or set msvc path in ar_config.json."
            )

        # Iterative compile: remove incompatible functions and retry
        for pass_n in range(MAX_COMPILE_PASSES):
            shim_src = gen_cpp_shim_source(
                effective_sigs, header_name,
                config.precompile_macros, config.win32_lean_and_mean,
            )
            try:
                _compile_msvc_shim(shim_src, vcvarsall, header_abs, shim_path, config)
                break
            except RuntimeError as e:
                bad = _extract_bad_fn_names(str(e))
                if not bad or pass_n == MAX_COMPILE_PASSES - 1:
                    raise
                before = len(effective_sigs)
                effective_sigs = [s for s in effective_sigs if s.name not in bad]
                print(
                    f"CppBridge: pass {pass_n+1}: removed {before - len(effective_sigs)} "
                    f"incompatible fn(s) ({', '.join(bad)}), retrying",
                    file=sys.stderr,
                )

        # Generate a thin Rust-free loader: just copy the shim DLL as the final DLL
        # (On Windows with ctypes we can load the MSVC shim directly)
        import shutil
        shutil.copy2(shim_path, dll_path)

    else:
        # Linux / Mac: generate C++ shim and compile with g++
        shim_src = gen_cpp_shim_source(
            effective_sigs, header_name,
            config.precompile_macros, False,
        )
        _compile_gcc_shim(shim_src, header_abs, dll_path, config)

    # Save effective function list
    syms_path.write_text("\n".join(s.name for s in effective_sigs), encoding="utf-8")
    print(f"CppBridge: generated '{dll_path}'", file=sys.stderr)
    return dll_path, effective_sigs


def _read_syms_file(syms_path: Path, all_sigs: list) -> list:
    """Filter all_sigs to only those listed in the .syms file."""
    if syms_path.exists():
        allowed = set(syms_path.read_text(encoding="utf-8").splitlines())
        return [s for s in all_sigs if s.name in allowed]
    return all_sigs


def _extract_bad_fn_names(error_msg: str) -> set:
    """Extract function names that caused MSVC errors (best-effort regex)."""
    import re
    bad: set[str] = set()
    # Look for patterns like: error C2039: 'FnName'
    for m in re.finditer(r"error\s+\w+:\s+'(\w+)'", error_msg):
        bad.add(m.group(1))
    # Also: unresolved external symbol _FnName
    for m in re.finditer(r"unresolved external symbol[^\n]*?(\w+)", error_msg):
        bad.add(m.group(1))
    return bad
