# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""Build configuration loaded from hv_config.json (mirrors cpp_bridge/config.rs)."""
from __future__ import annotations
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

CONFIG_FILE_NAME = "hv_config.json"

DEFAULT_SYSTEM_LIBS = [
    "winmm.lib", "imm32.lib", "ws2_32.lib", "dxguid.lib",
    "d3d9.lib", "d3d11.lib", "dxgi.lib", "dinput8.lib", "d3dcompiler.lib",
]
DEFAULT_LIB_PATTERNS = ["_vs2015_x64_md.lib", "_x64.lib"]
DEFAULT_TARGET_ARCH = "amd64"


@dataclass
class CppBuildConfig:
    """C++ build configuration loaded from hv_config.json."""
    msvc: Optional[Path] = None
    msvc_search_paths: list = field(default_factory=list)      # list[str]
    precompile_macros: list = field(default_factory=list)       # list[str]
    target_arch: str = DEFAULT_TARGET_ARCH
    cl_extra_flags: list = field(default_factory=list)          # list[str]
    link_extra_flags: list = field(default_factory=list)        # list[str]
    system_libs: list = field(default_factory=lambda: list(DEFAULT_SYSTEM_LIBS))
    win32_lean_and_mean: bool = True
    custom_type_map: dict = field(default_factory=dict)         # dict[str, str]
    lib_patterns: list = field(default_factory=lambda: list(DEFAULT_LIB_PATTERNS))
    system_headers: list = field(default_factory=list)          # list[str]


def load_cpp_config(start_dir: Path) -> CppBuildConfig:
    """Search for hv_config.json from start_dir upwards and parse it."""
    config = CppBuildConfig()

    # Build search path: start_dir and all parents, plus cwd
    search: list[Path] = []
    try:
        canon = start_dir.resolve()
    except Exception:
        canon = start_dir
    d = canon
    while True:
        search.append(d)
        parent = d.parent
        if parent == d:
            break
        d = parent
    try:
        cwd = Path.cwd()
        if cwd not in search:
            search.append(cwd)
    except Exception:
        pass

    for directory in search:
        cfg_path = directory / CONFIG_FILE_NAME
        if cfg_path.exists():
            try:
                text = cfg_path.read_text(encoding="utf-8")
                _parse_config_json(text, config)
            except Exception:
                pass
            break

    return config


def _parse_config_json(text: str, config: CppBuildConfig) -> None:
    """Apply hv_config.json contents to config in-place."""
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return

    cpp = data.get("cpp")
    if not isinstance(cpp, dict):
        return

    if "msvc" in cpp:
        config.msvc = Path(cpp["msvc"])
    if "msvc_search_paths" in cpp:
        config.msvc_search_paths = list(cpp["msvc_search_paths"])
    if "precompile_macros" in cpp:
        config.precompile_macros = list(cpp["precompile_macros"])
    if "target_arch" in cpp:
        config.target_arch = str(cpp["target_arch"])
    if "cl_extra_flags" in cpp:
        config.cl_extra_flags = list(cpp["cl_extra_flags"])
    if "link_extra_flags" in cpp:
        config.link_extra_flags = list(cpp["link_extra_flags"])
    if "system_libs" in cpp:
        config.system_libs = list(cpp["system_libs"])
    if "win32_lean_and_mean" in cpp:
        config.win32_lean_and_mean = bool(cpp["win32_lean_and_mean"])
    if "custom_type_map" in cpp:
        config.custom_type_map = dict(cpp["custom_type_map"])
    if "lib_patterns" in cpp:
        config.lib_patterns = list(cpp["lib_patterns"])
    if "system_headers" in cpp:
        config.system_headers = list(cpp["system_headers"])
