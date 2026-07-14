# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""C/C++ bridge for import[cpp-dll] and import[cpp-lib] (mirrors src/interpreter/cpp_bridge/).

Sub-modules by role:
  types         — CType enum, CStructDef, CFnSig, ctype_to_tl_str
  header_parser — parse_header_full, collect_included_headers
  config        — CppBuildConfig, load_cpp_config
  compiler      — gen_cpp_shim_source, compile_tl_dll (cpp-lib pipeline)
  loader        — load_cpp_dll (ctypes dispatch)
"""
from .types import (
    CType, CInt, CLong, CFloat, CDouble, CBool, CVoid, CVoidPtr, CCharPtr,
    CPtr, COpaqueStructPtr, CByValueStruct, CFnPtr,
    CFnSig, CStructDef, ctype_to_tl_str,
)
from .header_parser import parse_header_full, collect_included_headers
from .config import CppBuildConfig, load_cpp_config
from .compiler import compile_tl_dll, find_msvc_vcvarsall, native_lib_ext
from .loader import load_cpp_dll

__all__ = [
    "CType", "CInt", "CLong", "CFloat", "CDouble", "CBool", "CVoid",
    "CVoidPtr", "CCharPtr", "CPtr", "COpaqueStructPtr", "CByValueStruct",
    "CFnPtr", "CFnSig", "CStructDef", "ctype_to_tl_str",
    "parse_header_full", "collect_included_headers",
    "CppBuildConfig", "load_cpp_config",
    "compile_tl_dll", "find_msvc_vcvarsall", "native_lib_ext",
    "load_cpp_dll",
]
