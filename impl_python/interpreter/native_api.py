"""Native API: handle arena and ArCallbacks for import[rs] (mirrors src/interpreter/native_api.rs).

Values cross the _tl ABI as i64 handles into _ARENA.
The Python side implements all callback functions and passes them to the
loaded DLL via ar_init(cb_ptr).
"""
from __future__ import annotations

import ctypes
import sys
from typing import Any, Optional

# ---------------------------------------------------------------------------
# Handle arena
# ---------------------------------------------------------------------------

_ARENA: dict[int, Any] = {}
_NEXT_HANDLE: int = 3        # 0 = None, 1 = True, 2 = False

TL_NONE:  int = 0
TL_TRUE:  int = 1
TL_FALSE: int = 2


def alloc_handle(v: Any) -> int:
    global _NEXT_HANDLE
    if v is None:
        return TL_NONE
    if v is True:
        return TL_TRUE
    if v is False:
        return TL_FALSE
    h = _NEXT_HANDLE
    _NEXT_HANDLE += 1
    _ARENA[h] = v
    return h


def get_handle(h: int) -> Any:
    if h == TL_NONE:
        return None
    if h == TL_TRUE:
        return True
    if h == TL_FALSE:
        return False
    return _ARENA.get(h)


def free_handle(h: int) -> None:
    _ARENA.pop(h, None)


def _ar_to_i64(v: Any) -> int:
    """Encode a Python value to an i64 handle."""
    return alloc_handle(v)


def _i64_to_ar(h: int) -> Any:
    return get_handle(h)


# ---------------------------------------------------------------------------
# Global variable registry — used by cb_get_global (struct class lookup)
# ---------------------------------------------------------------------------

_GLOBAL_VARS: dict[str, Any] = {}


# ---------------------------------------------------------------------------
# ArCallbacks — ctypes struct matching the Rust #[repr(C)] definition
# ---------------------------------------------------------------------------

# Function-pointer typedefs
_FP_make_int     = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_make_float   = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_double)
_FP_make_bool    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int32)
_FP_make_str     = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)
_FP_make_seq     = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)  # list/tuple
_FP_make_dict    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int32)
_FP_make_none    = ctypes.CFUNCTYPE(ctypes.c_int64)
_FP_is_truthy    = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_int64)
_FP_binop        = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int32, ctypes.c_int64, ctypes.c_int64)
_FP_unop         = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int32, ctypes.c_int64)
_FP_call_fn      = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)
_FP_get_attr     = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)
_FP_set_attr     = ctypes.CFUNCTYPE(None, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32, ctypes.c_int64)
_FP_subscript    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_int64)
_FP_get_global   = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)
_FP_iter_from    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_iter_next    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_is_type      = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32)
_FP_arena_save   = ctypes.CFUNCTYPE(ctypes.c_uint64)
_FP_arena_compact = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_uint64)
_FP_compact_many = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_int32, ctypes.c_uint64, ctypes.c_void_p)
_FP_to_int       = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_to_float     = ctypes.CFUNCTYPE(ctypes.c_double, ctypes.c_int64)
_FP_deep_copy    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_to_cstr      = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_int64)
_FP_write_handle = ctypes.CFUNCTYPE(None, ctypes.c_int64, ctypes.c_int64)
_FP_list_append  = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_int64)
_FP_raise_exc    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64, ctypes.c_int64)
_FP_make_cell    = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_get_cell     = ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_int64)
_FP_set_cell     = ctypes.CFUNCTYPE(None, ctypes.c_int64, ctypes.c_int64)
_FP_call_method  = ctypes.CFUNCTYPE(
    ctypes.c_int64, ctypes.c_int64, ctypes.c_void_p, ctypes.c_int32,
    ctypes.c_void_p, ctypes.c_int32
)


class ArCallbacks(ctypes.Structure):
    _fields_ = [
        ("make_int",      _FP_make_int),
        ("make_float",    _FP_make_float),
        ("make_bool",     _FP_make_bool),
        ("make_str",      _FP_make_str),
        ("make_list",     _FP_make_seq),
        ("make_tuple",    _FP_make_seq),
        ("make_dict",     _FP_make_dict),
        ("make_none",     _FP_make_none),
        ("is_truthy",     _FP_is_truthy),
        ("binop",         _FP_binop),
        ("unop",          _FP_unop),
        ("call_fn",       _FP_call_fn),
        ("get_attr",      _FP_get_attr),
        ("set_attr",      _FP_set_attr),
        ("subscript",     _FP_subscript),
        ("get_global",    _FP_get_global),
        ("iter_from",     _FP_iter_from),
        ("iter_next",     _FP_iter_next),
        ("is_type",       _FP_is_type),
        ("arena_save",    _FP_arena_save),
        ("arena_compact", _FP_arena_compact),
        ("compact_many",  _FP_compact_many),
        ("to_int",        _FP_to_int),
        ("to_float",      _FP_to_float),
        ("deep_copy",     _FP_deep_copy),
        ("to_cstr",       _FP_to_cstr),
        ("write_handle",  _FP_write_handle),
        ("list_append",   _FP_list_append),
        ("raise_exc",     _FP_raise_exc),
        ("make_cell",     _FP_make_cell),
        ("get_cell",      _FP_get_cell),
        ("set_cell",      _FP_set_cell),
        ("call_method",   _FP_call_method),
    ]


# ---------------------------------------------------------------------------
# String buffer for to_cstr — must stay alive as long as the handle lives
# ---------------------------------------------------------------------------

_CSTR_BUFS: dict[int, bytes] = {}


# ---------------------------------------------------------------------------
# Callback implementations
# ---------------------------------------------------------------------------

def _cb_make_int(n: int) -> int:
    return alloc_handle(n)

def _cb_make_float(f: float) -> int:
    return alloc_handle(f)

def _cb_make_bool(b: int) -> int:
    return TL_TRUE if b else TL_FALSE

def _cb_make_str(p: Optional[int], l: int) -> int:
    if p is None or p == 0:
        return alloc_handle("")
    try:
        raw = (ctypes.c_uint8 * l).from_address(p)
        s = bytes(raw).decode("utf-8", errors="replace")
    except Exception:
        s = ""
    return alloc_handle(s)

def _cb_make_list(p: Optional[int], n: int) -> int:
    if p is None or p == 0 or n == 0:
        return alloc_handle([])
    arr = (ctypes.c_int64 * n).from_address(p)
    items = [get_handle(arr[i]) for i in range(n)]
    return alloc_handle(items)

def _cb_make_tuple(p: Optional[int], n: int) -> int:
    if p is None or p == 0 or n == 0:
        return alloc_handle(())
    arr = (ctypes.c_int64 * n).from_address(p)
    items = tuple(get_handle(arr[i]) for i in range(n))
    return alloc_handle(items)

def _cb_make_dict(kp: Optional[int], vp: Optional[int], n: int) -> int:
    if n == 0:
        return alloc_handle({})
    karr = (ctypes.c_int64 * n).from_address(kp)  # type: ignore[arg-type]
    varr = (ctypes.c_int64 * n).from_address(vp)   # type: ignore[arg-type]
    d = {get_handle(karr[i]): get_handle(varr[i]) for i in range(n)}
    return alloc_handle(d)

def _cb_make_none() -> int:
    return TL_NONE

def _cb_is_truthy(h: int) -> int:
    v = get_handle(h)
    return 1 if v else 0

def _cb_binop(op: int, lh: int, rh: int) -> int:
    # Minimal implementation for the most common ops used in native wrappers
    l = get_handle(lh)
    r = get_handle(rh)
    try:
        result: Any
        if op == 0:   result = l + r
        elif op == 1: result = l - r
        elif op == 2: result = l * r
        elif op == 3: result = l / r
        elif op == 4: result = l // r
        elif op == 5: result = l % r
        elif op == 6: result = l ** r
        elif op == 7: result = l == r
        elif op == 8: result = l != r
        elif op == 9: result = l < r
        elif op == 10: result = l > r
        elif op == 11: result = l <= r
        elif op == 12: result = l >= r
        else:         result = None
    except Exception:
        result = None
    return alloc_handle(result)

def _cb_unop(op: int, h: int) -> int:
    v = get_handle(h)
    try:
        if op == 0: return alloc_handle(-v)
        if op == 1: return TL_FALSE if v else TL_TRUE
        if op == 2: return alloc_handle(~v)
    except Exception:
        pass
    return TL_NONE

def _cb_call_fn(fn_h: int, args_p: Optional[int], n: int) -> int:
    fn = get_handle(fn_h)
    if fn is None:
        return TL_NONE

    # Decode arguments from handle array
    args: list = []
    if args_p and n > 0:
        arr = (ctypes.c_int64 * n).from_address(args_p)
        args = [get_handle(arr[i]) for i in range(n)]

    try:
        from .value import TlClass, TlInstance
        from .builtins import _NativeCallable
        if isinstance(fn, TlClass):
            # Instantiate: create TlInstance, fill field_defaults, call __init__
            fields: dict = {}
            for fname, default_val, is_mut in fn.field_defaults:
                fields[fname] = [default_val, is_mut]
            inst = TlInstance(cls=fn, fields=fields, immutable=False)
            if "__init__" in fn.methods:
                init_fn = fn.methods["__init__"][0]
                if isinstance(init_fn, _NativeCallable):
                    init_fn.call([inst] + args, {})
            return alloc_handle(inst)

        if isinstance(fn, _NativeCallable):
            result = fn.call(args, {})
            return alloc_handle(result)

        if callable(fn):
            result = fn(args, {})
            return alloc_handle(result)
    except Exception:
        pass
    return TL_NONE

def _cb_get_attr(obj_h: int, name_p: Optional[int], name_l: int) -> int:
    obj = get_handle(obj_h)
    if obj is None or name_p is None:
        return TL_NONE
    try:
        raw = (ctypes.c_uint8 * name_l).from_address(name_p)
        name = bytes(raw).decode("utf-8", errors="replace")
    except Exception:
        return TL_NONE
    try:
        from .value import TlInstance
        if isinstance(obj, TlInstance):
            # TlInstance.fields is {name: [value, is_mutable]}
            entry = obj.fields.get(name)
            if entry is not None:
                return alloc_handle(entry[0])
            return TL_NONE
    except Exception:
        pass
    try:
        return alloc_handle(getattr(obj, name, None))
    except Exception:
        return TL_NONE

def _cb_set_attr(obj_h: int, name_p: Optional[int], name_l: int, val_h: int) -> None:
    obj = get_handle(obj_h)
    if obj is None or name_p is None:
        return
    try:
        raw = (ctypes.c_uint8 * name_l).from_address(name_p)
        name = bytes(raw).decode("utf-8", errors="replace")
    except Exception:
        return
    val = get_handle(val_h)
    try:
        from .value import TlInstance
        if isinstance(obj, TlInstance):
            # TlInstance.fields is {name: [value, is_mutable]}
            if name in obj.fields:
                obj.fields[name][0] = val
            else:
                obj.fields[name] = [val, True]
            return
    except Exception:
        pass
    try:
        setattr(obj, name, val)
    except Exception:
        pass

def _cb_subscript(obj_h: int, idx_h: int) -> int:
    obj = get_handle(obj_h)
    idx = get_handle(idx_h)
    try:
        return alloc_handle(obj[idx])
    except Exception:
        return TL_NONE

def _cb_get_global(name_p: Optional[int], name_l: int) -> int:
    if name_p is None or name_p == 0:
        return TL_NONE
    try:
        raw = (ctypes.c_uint8 * name_l).from_address(name_p)
        name = bytes(raw).decode("utf-8", errors="replace")
        v = _GLOBAL_VARS.get(name)
        if v is not None:
            return alloc_handle(v)
    except Exception:
        pass
    return TL_NONE

def _cb_iter_from(h: int) -> int:
    v = get_handle(h)
    try:
        return alloc_handle(iter(v))
    except Exception:
        return TL_NONE

def _cb_iter_next(h: int) -> int:
    it = get_handle(h)
    try:
        return alloc_handle(next(it))
    except StopIteration:
        return TL_NONE
    except Exception:
        return TL_NONE

def _cb_is_type(_h: int, _p: Optional[int], _l: int) -> int:
    return TL_FALSE

def _cb_arena_save() -> int:
    return _NEXT_HANDLE

def _cb_arena_compact(h: int, _snap: int) -> int:
    return h

def _cb_compact_many(_src: Optional[int], _n: int, _snap: int, _dst: Optional[int]) -> None:
    pass

def _cb_to_int(h: int) -> int:
    v = get_handle(h)
    if isinstance(v, bool):
        return int(v)
    if isinstance(v, (int, float)):
        return int(v)
    return 0

def _cb_to_float(h: int) -> float:
    v = get_handle(h)
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return float(v)
    if isinstance(v, bool):
        return float(v)
    return 0.0

def _cb_deep_copy(h: int) -> int:
    import copy
    v = get_handle(h)
    try:
        return alloc_handle(copy.deepcopy(v))
    except Exception:
        return alloc_handle(v)

def _cb_to_cstr(h: int) -> Optional[int]:
    v = get_handle(h)
    if v is None:
        return 0
    s = str(v) if not isinstance(v, str) else v
    b = s.encode("utf-8") + b"\x00"
    _CSTR_BUFS[h] = b
    return ctypes.cast(ctypes.c_char_p(b), ctypes.c_void_p).value

def _cb_write_handle(dst_h: int, src_h: int) -> None:
    src = get_handle(src_h)
    _ARENA[dst_h] = src

def _cb_list_append(list_h: int, val_h: int) -> int:
    lst = get_handle(list_h)
    val = get_handle(val_h)
    if isinstance(lst, list):
        lst.append(val)
    return list_h

def _cb_raise_exc(_cls_h: int, _msg_h: int) -> int:
    return TL_NONE

def _cb_make_cell(h: int) -> int:
    v = get_handle(h)
    return alloc_handle([v])  # cell as a 1-element list

def _cb_get_cell(h: int) -> int:
    cell = get_handle(h)
    if isinstance(cell, list) and cell:
        return alloc_handle(cell[0])
    return TL_NONE

def _cb_set_cell(cell_h: int, val_h: int) -> None:
    cell = get_handle(cell_h)
    val = get_handle(val_h)
    if isinstance(cell, list):
        if cell:
            cell[0] = val
        else:
            cell.append(val)

def _cb_call_method(obj_h: int, name_p: Optional[int], name_l: int,
                    args_p: Optional[int], n: int) -> int:
    obj = get_handle(obj_h)
    if obj is None or name_p is None:
        return TL_NONE
    try:
        raw = (ctypes.c_uint8 * name_l).from_address(name_p)
        name = bytes(raw).decode("utf-8", errors="replace")
    except Exception:
        return TL_NONE
    args = []
    if args_p and n > 0:
        arr = (ctypes.c_int64 * n).from_address(args_p)
        args = [get_handle(arr[i]) for i in range(n)]
    try:
        method = getattr(obj, name)
        return alloc_handle(method(*args))
    except Exception:
        return TL_NONE


# ---------------------------------------------------------------------------
# Build and cache the singleton ArCallbacks instance
# ---------------------------------------------------------------------------

_CALLBACKS: Optional[ArCallbacks] = None


def make_ar_callbacks() -> ArCallbacks:
    """Return the singleton ArCallbacks struct wired to our Python arena."""
    global _CALLBACKS
    if _CALLBACKS is not None:
        return _CALLBACKS

    cb = ArCallbacks()
    cb.make_int      = _FP_make_int(_cb_make_int)
    cb.make_float    = _FP_make_float(_cb_make_float)
    cb.make_bool     = _FP_make_bool(_cb_make_bool)
    cb.make_str      = _FP_make_str(_cb_make_str)
    cb.make_list     = _FP_make_seq(_cb_make_list)
    cb.make_tuple    = _FP_make_seq(_cb_make_tuple)
    cb.make_dict     = _FP_make_dict(_cb_make_dict)
    cb.make_none     = _FP_make_none(_cb_make_none)
    cb.is_truthy     = _FP_is_truthy(_cb_is_truthy)
    cb.binop         = _FP_binop(_cb_binop)
    cb.unop          = _FP_unop(_cb_unop)
    cb.call_fn       = _FP_call_fn(_cb_call_fn)
    cb.get_attr      = _FP_get_attr(_cb_get_attr)
    cb.set_attr      = _FP_set_attr(_cb_set_attr)
    cb.subscript     = _FP_subscript(_cb_subscript)
    cb.get_global    = _FP_get_global(_cb_get_global)
    cb.iter_from     = _FP_iter_from(_cb_iter_from)
    cb.iter_next     = _FP_iter_next(_cb_iter_next)
    cb.is_type       = _FP_is_type(_cb_is_type)
    cb.arena_save    = _FP_arena_save(_cb_arena_save)
    cb.arena_compact = _FP_arena_compact(_cb_arena_compact)
    cb.compact_many  = _FP_compact_many(_cb_compact_many)
    cb.to_int        = _FP_to_int(_cb_to_int)
    cb.to_float      = _FP_to_float(_cb_to_float)
    cb.deep_copy     = _FP_deep_copy(_cb_deep_copy)
    cb.to_cstr       = _FP_to_cstr(_cb_to_cstr)
    cb.write_handle  = _FP_write_handle(_cb_write_handle)
    cb.list_append   = _FP_list_append(_cb_list_append)
    cb.raise_exc     = _FP_raise_exc(_cb_raise_exc)
    cb.make_cell     = _FP_make_cell(_cb_make_cell)
    cb.get_cell      = _FP_get_cell(_cb_get_cell)
    cb.set_cell      = _FP_set_cell(_cb_set_cell)
    cb.call_method   = _FP_call_method(_cb_call_method)

    _CALLBACKS = cb
    return cb
