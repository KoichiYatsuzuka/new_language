"""cs_dll_runtime.py — ctypes bridge for import[cs-dll] (Python implementation).

Port of src/interpreter/cs_dll_runtime.rs.

ABI contract (NativeAOT bridge side):
  int/bool args  → c_int64 direct
  float args     → c_int64 bit pattern (IEEE-754 reinterpret)
  string args    → (c_char_p, c_int64) = (UTF-8 ptr, byte length)
  handle args    → c_int64 object id
  void return    → None
  int return     → c_int64 direct
  float return   → c_int64 bit pattern
  string return  → (void** out_ptr, int* out_len) out-params; freed by arrow_bridge_free_str
  handle return  → c_int64 object id
"""

from __future__ import annotations
import ctypes
import struct
from pathlib import Path
from typing import Optional

# ---------------------------------------------------------------------------
# Bridge registry (module-level dict acts as the equivalent of Rust's thread-local)
# ---------------------------------------------------------------------------

_BRIDGE_CACHE: dict[Path, "BridgeLib"] = {}

# ---------------------------------------------------------------------------
# BridgeLib — thin wrapper around ctypes.CDLL
# ---------------------------------------------------------------------------

class BridgeLib:
    def __init__(self, path: Path):
        self.path = path.resolve()
        self._lib = ctypes.CDLL(str(path))
        self._sym_cache: dict[str, Optional[ctypes.CFUNCTYPE]] = {}

    def sym_fn(self, name: str) -> Optional[int]:
        """Return the function pointer address, or None if not found."""
        if name in self._sym_cache:
            return self._sym_cache[name]
        try:
            fn = getattr(self._lib, name)
            addr = ctypes.cast(fn, ctypes.c_void_p).value
            self._sym_cache[name] = addr
            return addr
        except AttributeError:
            self._sym_cache[name] = None
            return None

    def raw_fn(self, name: str) -> Optional[ctypes._NamedFuncPtr]:
        """Return raw ctypes function object, or None."""
        try:
            return getattr(self._lib, name)
        except AttributeError:
            return None

# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def load_bridge(path: Path) -> BridgeLib:
    canon = path.resolve()
    if canon not in _BRIDGE_CACHE:
        _BRIDGE_CACHE[canon] = BridgeLib(path)
    return _BRIDGE_CACHE[canon]

def get_bridge(path: Path) -> Optional[BridgeLib]:
    return _BRIDGE_CACHE.get(path.resolve())

# ---------------------------------------------------------------------------
# Value → i64 conversion
# ---------------------------------------------------------------------------

def value_to_i64(v: object) -> int:
    from .value import TlCsObject
    if isinstance(v, bool):
        return 1 if v else 0
    if isinstance(v, int):
        return v
    if isinstance(v, float):
        # Reinterpret float64 bits as int64
        return struct.unpack("<q", struct.pack("<d", v))[0]
    if isinstance(v, TlCsObject):
        return v.handle
    if v is None:
        return 0
    return 0

def raw_to_value(raw: int, ret_type: Optional[str]) -> object:
    if ret_type == "float":
        return struct.unpack("<d", struct.pack("<q", raw))[0]
    if ret_type == "bool":
        return raw != 0
    if ret_type in ("None", "void"):
        return None
    return raw  # int

def matches_str_ret(ret_type: Optional[str]) -> bool:
    return ret_type == "str"

# ---------------------------------------------------------------------------
# Internal bridge call helpers
# ---------------------------------------------------------------------------

def _build_c_args(args: list, self_handle: Optional[int] = None) -> tuple[list, list]:
    """Return (c_args, keep_alive) where keep_alive holds byte buffers alive."""
    c_args: list = []
    keep_alive: list = []

    if self_handle is not None:
        c_args.append(ctypes.c_int64(self_handle))

    for v in args:
        if isinstance(v, str):
            b = v.encode("utf-8")
            keep_alive.append(b)
            c_args.append(ctypes.c_char_p(b))
            c_args.append(ctypes.c_int64(len(b)))
        else:
            c_args.append(ctypes.c_int64(value_to_i64(v)))

    return c_args, keep_alive


def _call_bridge_fn(bridge: BridgeLib, sym_name: str,
                    args: list, self_handle: Optional[int] = None) -> int:
    """Call a bridge function returning i64 (or void, which we treat as 0)."""
    fn = bridge.raw_fn(sym_name)
    if fn is None:
        raise RuntimeError(f"CsDll: bridge '{bridge.path.name}' has no export '{sym_name}'")

    fn.restype = ctypes.c_int64
    c_args, keep_alive = _build_c_args(args, self_handle)
    fn.argtypes = [type(a) for a in c_args]
    result = fn(*c_args)
    _ = keep_alive  # prevent GC
    return int(result)


def _call_returning_str(bridge: BridgeLib, sym_name: str,
                        args: list, self_handle: Optional[int] = None) -> str:
    """Call a bridge function that returns a UTF-8 string via (void** out_ptr, int* out_len)."""
    fn = bridge.raw_fn(sym_name)
    if fn is None:
        raise RuntimeError(f"CsDll: bridge '{bridge.path.name}' has no export '{sym_name}'")

    fn.restype = None  # void

    c_args, keep_alive = _build_c_args(args, self_handle)

    # out-params: byte** (= c_void_p*) and int*
    out_ptr = ctypes.c_void_p(0)
    out_len = ctypes.c_int(0)

    fn.argtypes = [type(a) for a in c_args] + [
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_int),
    ]
    fn(*c_args, ctypes.byref(out_ptr), ctypes.byref(out_len))
    _ = keep_alive

    if not out_ptr.value or out_len.value < 0:
        return ""

    # Read string bytes
    s = ctypes.string_at(out_ptr.value, out_len.value).decode("utf-8", errors="replace")

    # Free the C# heap buffer
    free_fn = bridge.raw_fn("arrow_bridge_free_str")
    if free_fn is not None:
        free_fn.restype = None
        free_fn.argtypes = [ctypes.c_void_p]
        free_fn(out_ptr.value)

    return s

# ---------------------------------------------------------------------------
# Public dispatch functions
# ---------------------------------------------------------------------------

def call_constructor(bridge: BridgeLib, class_name: str, args: list) -> int:
    """Calls {ClassName}_new_{argc} or {ClassName}_new. Returns i64 handle."""
    numbered = f"{class_name}_new_{len(args)}"
    fallback = f"{class_name}_new"

    sym_name = numbered if bridge.sym_fn(numbered) is not None else fallback
    if bridge.sym_fn(sym_name) is None:
        raise RuntimeError(
            f"CsDll: no constructor export for '{class_name}' "
            f"(tried '{numbered}' and '{fallback}')"
        )
    return _call_bridge_fn(bridge, sym_name, args)


def call_static(bridge: BridgeLib, class_name: str, method: str,
                args: list, ret_type: Optional[str]) -> object:
    """Call static bridge method {ClassName}_{method}."""
    sym_name = f"{class_name}_{method}"
    if bridge.sym_fn(sym_name) is None:
        raise RuntimeError(f"CsDll: bridge has no export '{sym_name}'")

    if matches_str_ret(ret_type):
        return _call_returning_str(bridge, sym_name, args)

    raw = _call_bridge_fn(bridge, sym_name, args)
    return raw_to_value(raw, ret_type)


def call_instance(bridge: BridgeLib, class_name: str, handle: int,
                  method: str, args: list, ret_type: Optional[str]) -> object:
    """Call instance bridge method {ClassName}_inst_{method}(handle, args...)."""
    sym_name = f"{class_name}_inst_{method}"
    if bridge.sym_fn(sym_name) is None:
        raise RuntimeError(f"CsDll: bridge has no export '{sym_name}'")

    if matches_str_ret(ret_type):
        return _call_returning_str(bridge, sym_name, args, self_handle=handle)

    raw = _call_bridge_fn(bridge, sym_name, args, self_handle=handle)

    if ret_type in ("None", "void"):
        return None
    return raw_to_value(raw, ret_type)


def release_handle(bridge: BridgeLib, handle: int) -> None:
    fn = bridge.raw_fn("arrow_bridge_release")
    if fn is not None:
        fn.restype = None
        fn.argtypes = [ctypes.c_int64]
        fn(ctypes.c_int64(handle))
