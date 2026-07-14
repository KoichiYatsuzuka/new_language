# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""ctypes-based DLL loader and value marshaling for cpp-dll / cpp-lib imports.

Replaces the Rust 'generate wrapper DLL + compile' approach with direct ctypes
dispatch: Python's ctypes handles Arrow value ↔ C type conversion natively,
so no intermediate Rust wrapper is needed.
"""
from __future__ import annotations
import ctypes
import sys
from pathlib import Path
from typing import Callable, Optional

from .types import (
    CType, CInt, CLong, CFloat, CDouble, CBool, CVoid, CVoidPtr, CCharPtr,
    CPtr, COpaqueStructPtr, CByValueStruct, CFnPtr, CFnSig, CStructDef,
)


# ── ctypes type mapping ───────────────────────────────────────────────────────

def _to_ctypes(ct: CType):
    """Convert a CType to the corresponding ctypes type (or None for void return)."""
    match ct:
        case CInt() | CBool():
            return ctypes.c_int
        case CLong():
            return ctypes.c_int64
        case CFloat():
            return ctypes.c_float
        case CDouble():
            return ctypes.c_double
        case CVoid():
            return None
        case CVoidPtr() | COpaqueStructPtr() | CByValueStruct() | CFnPtr():
            return ctypes.c_void_p
        case CCharPtr():
            return ctypes.c_char_p
        case CPtr(inner=inner, mutable=_):
            inner_ct = _to_ctypes(inner)
            if inner_ct is None:
                return ctypes.c_void_p
            return ctypes.POINTER(inner_ct)
        case _:
            return ctypes.c_void_p


# ── Value → C argument conversion ────────────────────────────────────────────

def _value_to_c(v: object, ct: CType) -> object:
    """Convert a Arrow runtime value to a ctypes-compatible C argument."""
    match ct:
        case CInt() | CBool():
            if isinstance(v, bool):
                return int(v)
            if isinstance(v, (int, float)):
                return int(v)
            return 0

        case CLong():
            if isinstance(v, bool):
                return int(v)
            if isinstance(v, (int, float)):
                return int(v)
            return 0

        case CFloat() | CDouble():
            if isinstance(v, (int, float)) and not isinstance(v, bool):
                return float(v)
            if isinstance(v, bool):
                return float(v)
            return 0.0

        case CCharPtr():
            if isinstance(v, str):
                return v.encode("utf-8")
            if isinstance(v, (bytes, bytearray)):
                return bytes(v)
            return b""

        case CVoidPtr() | COpaqueStructPtr() | CByValueStruct():
            if isinstance(v, int) and not isinstance(v, bool):
                return v
            return None

        case CFnPtr():
            # Function pointers: pass as opaque int / void*
            if isinstance(v, int):
                return v
            return None

        case CPtr(inner=inner, mutable=mutable):
            # Mutable pointer: create a writable ctypes container
            inner_cttype = _to_ctypes(inner)
            if inner_cttype is None:
                return None
            raw = _value_to_c(v, inner)
            try:
                container = inner_cttype(raw)  # type: ignore[call-arg]
                return ctypes.byref(container)
            except Exception:
                return None

        case _:
            return None


def _c_to_value(result: object, ct: CType) -> object:
    """Convert a ctypes return value to a Arrow runtime value."""
    match ct:
        case CInt() | CLong():
            if result is None:
                return 0
            return int(result)  # type: ignore[arg-type]

        case CFloat() | CDouble():
            if result is None:
                return 0.0
            return float(result)  # type: ignore[arg-type]

        case CBool():
            return bool(result)

        case CVoid():
            return None

        case CCharPtr():
            if result is None:
                return ""
            if isinstance(result, bytes):
                return result.decode("utf-8", errors="replace")
            return str(result)

        case CVoidPtr() | COpaqueStructPtr() | CByValueStruct() | CFnPtr():
            if result is None:
                return 0
            try:
                return int(result)  # type: ignore[arg-type]
            except (TypeError, ValueError):
                return 0

        case CPtr(inner=inner, mutable=_):
            # Return values for pointer types are rare; treat as opaque int
            if result is None:
                return 0
            try:
                return int(result)  # type: ignore[arg-type]
            except (TypeError, ValueError):
                return 0

        case _:
            return None


# ── Mutable pointer writeback ─────────────────────────────────────────────────

class _MutPtrBox:
    """Holds a ctypes container for a mutable (output) pointer parameter."""
    def __init__(self, ct_type, initial_value: object) -> None:
        self._ct_type = ct_type
        try:
            self._container = ct_type(initial_value)  # type: ignore[call-arg]
        except Exception:
            self._container = ct_type()  # type: ignore[call-arg]
        self._byref = ctypes.byref(self._container)

    def read_back(self, inner_ct: CType) -> object:
        return _c_to_value(self._container.value, inner_ct)


def _struct_ptr_arg(v: object, type_name: str, raw_layouts: dict,
                    mutable: bool, cleanups: list) -> object:
    """Resolve a struct-pointer argument (mirrors resolve_typed_ptr_arg).

    - TlInstance with a structurally compatible raw layout → zero-copy:
      pass a pointer into the instance's raw block (C writes are visible
      in the instance immediately).
    - Other TlInstance → shadow conversion: copy the fields into a temp
      buffer laid out per the target struct; for mutable params, read the
      buffer back into the instance after the call.
    - int → treated as a raw address (opaque handle path).
    """
    from ..value import TlInstance, raw_layouts_compatible

    target_layout = raw_layouts.get(type_name)
    if isinstance(v, TlInstance):
        inst_layout = v.cls.raw_layout
        if (v.raw is not None and inst_layout is not None and target_layout is not None
                and raw_layouts_compatible(inst_layout, target_layout)):
            # Zero-copy: raw block == C struct layout
            buf = (ctypes.c_ubyte * len(v.raw)).from_buffer(v.raw)
            return ctypes.cast(buf, ctypes.c_void_p)
        if target_layout is not None:
            # Shadow conversion: encode fields (declaration order) into a temp block
            shadow = bytearray(target_layout.total_bytes)
            import struct as _struct
            for i, desc in enumerate(target_layout.fields):
                if not v.slot_initialized(i):
                    raise RuntimeError(
                        f"CppBridge: field {i} of '{v.cls.name}' is uninitialized"
                    )
                val = v.field_value(i)
                if desc.is_float:
                    _struct.pack_into(desc.fmt, shadow, desc.byte_offset, float(val))
                else:
                    masked = int(val) & ((1 << (desc.width * 8)) - 1)
                    _struct.pack_into(desc.fmt_store, shadow, desc.byte_offset, masked)
            buf = (ctypes.c_ubyte * len(shadow)).from_buffer(shadow)
            if mutable:
                def _apply_back(inst=v, shadow=shadow, layout=target_layout):
                    for i, desc in enumerate(layout.fields):
                        raw_v = _struct.unpack_from(desc.fmt, shadow, desc.byte_offset)[0]
                        inst.store_field(i, float(raw_v) if desc.is_float else int(raw_v), True)
                cleanups.append(_apply_back)
            return ctypes.cast(buf, ctypes.c_void_p)
        raise RuntimeError(
            f"CppBridge: cannot pass '{v.cls.name}' instance as '{type_name}*' "
            f"(struct layout unknown)"
        )
    if isinstance(v, int) and not isinstance(v, bool):
        return v  # raw address / handle
    return None


def _make_ctypes_wrapper(lib: ctypes.CDLL, sig: CFnSig,
                         raw_layouts: Optional[dict] = None) -> Optional[Callable]:
    """Build a Python callable that calls sig.name in lib via ctypes.

    The wrapper takes (args: list, kwargs: dict) and returns
    (return_value, writebacks) where writebacks is a list of
    (param_index, new_value) for mutable PRIMITIVE pointer parameters —
    the interpreter assigns these back to named `mut` variables
    (mirrors the Rust write-back path in eval/native.rs).
    Struct-pointer write-back happens inside the wrapper itself
    (zero-copy shares memory; shadow conversion is applied back here).
    """
    raw_layouts = raw_layouts or {}
    # Retrieve the function from the DLL
    try:
        fn = getattr(lib, sig.name)
    except AttributeError:
        return None  # symbol not exported → skip

    # Set restype (None = void)
    fn.restype = _to_ctypes(sig.ret)

    def wrapper(args: list, kwargs: dict) -> tuple:
        # Pad args to n_required minimum (treat missing as None → 0)
        n_params = len(sig.params)
        padded = list(args) + [None] * (n_params - len(args))
        padded = padded[:n_params]

        c_args = []
        mut_boxes: list[tuple[int, _MutPtrBox, CType]] = []
        cleanups: list = []   # shadow write-back closures (struct ptrs)
        keepalive: list = []  # buffers that must outlive the call

        for i, (_, ct) in enumerate(sig.params):
            v = padded[i]
            if isinstance(ct, CPtr) and ct.mutable:
                inner_cttype = _to_ctypes(ct.inner)
                if inner_cttype is not None:
                    box = _MutPtrBox(inner_cttype, _value_to_c(v, ct.inner))
                    mut_boxes.append((i, box, ct.inner))
                    c_args.append(box._byref)
                else:
                    c_args.append(None)
            elif isinstance(ct, COpaqueStructPtr):
                arg = _struct_ptr_arg(v, ct.type_name, raw_layouts, ct.mutable, cleanups)
                keepalive.append(arg)
                c_args.append(arg)
            else:
                c_args.append(_value_to_c(v, ct))

        try:
            raw = fn(*c_args)
        except Exception as e:
            raise RuntimeError(f"CppBridge: call to '{sig.name}' failed: {e}") from e

        ret_val = _c_to_value(raw, sig.ret)

        # Struct shadow write-back (mutable struct ptr args, non-zero-copy path)
        for apply_back in cleanups:
            apply_back()

        # Primitive out-ptr write-back values → interpreter assigns to mut vars
        writebacks = [(i, box.read_back(inner_ct)) for i, box, inner_ct in mut_boxes]
        return ret_val, writebacks

    return wrapper


# ── Typed ABI (`{name}_typed` entry points of Rust-built wrapper DLLs) ───────
#
# Mirrors the unified typed ABI in src/interpreter/eval/native.rs:
#   status: u32 = fn(args: *const u64, ret: *mut u64, err: *mut ErrSlot)
# status 0 = OK, 1 = raise (err holds "TypeName"/"message" strings that live
# in the DLL's static memory).

class _ErrSlot(ctypes.Structure):
    _fields_ = [
        ("type_ptr", ctypes.c_void_p),
        ("type_len", ctypes.c_uint64),
        ("msg_ptr", ctypes.c_void_p),
        ("msg_len", ctypes.c_uint64),
    ]

    def to_error_string(self) -> str:
        def read(p, n):
            if not p or n == 0:
                return ""
            return ctypes.string_at(p, int(n)).decode("utf-8", errors="replace")
        type_name = read(self.type_ptr, self.type_len) or "RuntimeError"
        msg = read(self.msg_ptr, self.msg_len)
        return f"{type_name}: {msg}"


def _typed_param_spec(sig: CFnSig, raw_layouts: dict) -> Optional[tuple[list, str]]:
    """Build the typed-ABI param spec (mirrors build_cpp_typed_sig).

    Returns (params, ret_kind) where each param is one of:
      ("i64",) / ("f64",) /
      ("ptr", type_name, mutable, by_value) /
      ("out", fmt, fmt_store, width, is_float)
    and ret_kind is "void" | "i64" | "f64". None = not typed-eligible.
    """
    import struct as _struct
    match sig.ret:
        case CVoid():
            ret_kind = "void"
        case CInt() | CLong():
            ret_kind = "i64"
        case CFloat() | CDouble():
            ret_kind = "f64"
        case _:
            return None
    params: list = []
    for _, ct in sig.params:
        match ct:
            case CInt() | CLong():
                params.append(("i64",))
            case CFloat() | CDouble():
                params.append(("f64",))
            case COpaqueStructPtr(type_name=tn, mutable=mut):
                if tn not in raw_layouts:
                    return None
                params.append(("ptr", tn, mut, False))
            case CByValueStruct(type_name=tn):
                if tn not in raw_layouts:
                    return None
                params.append(("ptr", tn, False, True))
            # Primitive write pointer (`int*` / `double*`): OutPtr slot.
            # Width follows the rust_extern_type convention
            # (Int→i32, Long→i64 — known LLP64 caveat).
            case CPtr(inner=inner, mutable=True):
                match inner:
                    case CInt():
                        params.append(("out", "<i", "<I", 4, False))
                    case CLong():
                        params.append(("out", "<q", "<Q", 8, False))
                    case CFloat():
                        params.append(("out", "<f", "<f", 4, True))
                    case CDouble():
                        params.append(("out", "<d", "<d", 8, True))
                    case _:
                        return None
            case _:
                return None
    return params, ret_kind


def _make_typed_wrapper(lib: ctypes.CDLL, sig: CFnSig,
                        raw_layouts: dict) -> Optional[Callable]:
    """Build a wrapper over the `{name}_typed` entry of a Rust-built wrapper DLL.

    Same (args, kwargs) -> (ret, writebacks) protocol as _make_ctypes_wrapper.
    """
    import struct as _struct
    spec = _typed_param_spec(sig, raw_layouts)
    if spec is None:
        return None
    param_spec, ret_kind = spec
    try:
        fn = getattr(lib, f"{sig.name}_typed")
    except AttributeError:
        return None
    fn.argtypes = [ctypes.POINTER(ctypes.c_uint64),
                   ctypes.POINTER(ctypes.c_uint64),
                   ctypes.POINTER(_ErrSlot)]
    fn.restype = ctypes.c_uint32

    def wrapper(args: list, kwargs: dict) -> tuple:
        from ..value import TlInstance, raw_layouts_compatible

        n = len(param_spec)
        padded = (list(args) + [None] * n)[:n]
        slots = (ctypes.c_uint64 * max(n, 1))()
        out_locals: dict[int, object] = {}   # index → single-element c_uint64 buffer
        cleanups: list = []
        keepalive: list = []

        for i, p in enumerate(param_spec):
            v = padded[i]
            kind = p[0]
            if kind == "i64":
                if isinstance(v, bool) or not isinstance(v, int):
                    raise RuntimeError(
                        f"TypeError: argument {i} of '{sig.name}' must be int"
                    )
                slots[i] = v & 0xFFFFFFFFFFFFFFFF
            elif kind == "f64":
                if isinstance(v, bool) or not isinstance(v, (int, float)):
                    raise RuntimeError(
                        f"TypeError: argument {i} of '{sig.name}' must be float"
                    )
                slots[i] = int.from_bytes(_struct.pack("<d", float(v)), "little")
            elif kind == "ptr":
                _tag, tn, mut, _by_value = p
                target_layout = raw_layouts[tn]
                if isinstance(v, TlInstance):
                    inst_layout = v.cls.raw_layout
                    if (v.raw is not None and inst_layout is not None
                            and raw_layouts_compatible(inst_layout, target_layout)):
                        buf = (ctypes.c_ubyte * len(v.raw)).from_buffer(v.raw)
                        keepalive.append(buf)
                        slots[i] = ctypes.addressof(buf)
                    else:
                        # Shadow conversion (+ write-back when mutable)
                        shadow = bytearray(target_layout.total_bytes)
                        for j, desc in enumerate(target_layout.fields):
                            if not v.slot_initialized(j):
                                raise RuntimeError(
                                    f"CppBridge: field {j} of '{v.cls.name}' is uninitialized"
                                )
                            val = v.field_value(j)
                            if desc.is_float:
                                _struct.pack_into(desc.fmt, shadow, desc.byte_offset, float(val))
                            else:
                                masked = int(val) & ((1 << (desc.width * 8)) - 1)
                                _struct.pack_into(desc.fmt_store, shadow, desc.byte_offset, masked)
                        buf = (ctypes.c_ubyte * len(shadow)).from_buffer(shadow)
                        keepalive.append((buf, shadow))
                        slots[i] = ctypes.addressof(buf)
                        if mut:
                            def _apply_back(inst=v, shadow=shadow, layout=target_layout):
                                for j, desc in enumerate(layout.fields):
                                    raw_v = _struct.unpack_from(desc.fmt, shadow, desc.byte_offset)[0]
                                    inst.store_field(j, float(raw_v) if desc.is_float else int(raw_v), True)
                            cleanups.append(_apply_back)
                elif isinstance(v, int) and not isinstance(v, bool):
                    slots[i] = v  # raw address / handle
                else:
                    raise RuntimeError(
                        f"TypeError: argument {i} of '{sig.name}' must be a "
                        f"'{tn}' instance or pointer"
                    )
            else:  # "out"
                _tag, fmt, fmt_store, width, is_float = p
                local = (ctypes.c_uint64 * 1)()
                if is_float:
                    if isinstance(v, bool) or not isinstance(v, (int, float)):
                        raise RuntimeError(
                            f"TypeError: argument {i} of '{sig.name}' must be float"
                        )
                    enc = _struct.pack(fmt, float(v if v is not None else 0.0))
                else:
                    iv = v if isinstance(v, int) and not isinstance(v, bool) else 0
                    enc = _struct.pack(fmt_store, iv & ((1 << (width * 8)) - 1))
                ctypes.memmove(local, enc, len(enc))
                out_locals[i] = local
                keepalive.append(local)
                slots[i] = ctypes.addressof(local)

        ret_slot = ctypes.c_uint64(0)
        err = _ErrSlot()
        status = fn(slots, ctypes.byref(ret_slot), ctypes.byref(err))
        if status != 0:
            raise RuntimeError(err.to_error_string())

        # Struct shadow write-back (success only)
        for apply_back in cleanups:
            apply_back()

        # OutPtr write-back values (decoded with the same width rules)
        writebacks = []
        for i, local in out_locals.items():
            _tag, fmt, fmt_store, width, is_float = param_spec[i]
            raw_bytes = bytes((ctypes.c_ubyte * 8).from_buffer(local))
            val = _struct.unpack_from(fmt, raw_bytes, 0)[0]
            writebacks.append((i, float(val) if is_float else int(val)))

        if ret_kind == "void":
            ret_val: object = None
        elif ret_kind == "i64":
            r = ret_slot.value
            ret_val = r - (1 << 64) if r >= (1 << 63) else r
        else:
            ret_val = _struct.unpack("<d", _struct.pack("<Q", ret_slot.value))[0]
        return ret_val, writebacks

    return wrapper


# ── Public API ────────────────────────────────────────────────────────────────

def load_cpp_dll(
    dll_path: Path,
    sigs: list,         # list[CFnSig]
    struct_defs: list,  # list[CStructDef]
    module_name: str,
) -> object:            # TlNamespace
    """Load a C DLL with ctypes and return a TlNamespace with callable wrappers.

    Works for both cpp-dll (pre-built) and cpp-lib (shim built by compiler.py).
    """
    from ..value import TlNamespace, TlClass, TlFunction
    from ..builtins import _make_native

    try:
        lib = ctypes.CDLL(str(dll_path))
    except OSError as e:
        raise RuntimeError(f"CppImport: cannot load DLL '{dll_path}': {e}") from e

    # Rust-built wrapper DLLs need bridge initialisation (loads the underlying
    # shim DLL and registers the callback table). Prefer ar_init_bridge,
    # fall back to ar_init / hv_init (mirrors load_cpp_wrapper_dll in exec/modules.rs).
    _init_bridge_dll(lib)

    members: dict[str, object] = {}

    # Raw layouts of structs whose C layout is fully known (complete)
    raw_layouts: dict[str, object] = {}
    for sdef in struct_defs:
        layout = sdef.raw_layout()
        if layout is not None:
            raw_layouts[sdef.name] = layout

    # Build TlClass stubs for C structs so Arrow code can construct instances
    for sdef in struct_defs:
        field_names = [fname for fname, _ in sdef.fields]
        field_mutability = {fname: True for fname in field_names}
        field_index = {fname: i for i, fname in enumerate(field_names)}
        field_mutability_vec = [True] * len(field_names)

        # __init__ body: self.field = field for each field
        from ...ast import StmtAttrAssign, ExprIdent, ExprAttr
        init_body: list[object] = [
            StmtAttrAssign(
                target=ExprAttr(
                    object=ExprIdent("self"),
                    attr=fname,
                ),
                value=ExprIdent(fname),
            )
            for fname in field_names
        ]
        from ...ast import Param as AstParam
        init_params = [
            AstParam(name="self", mutable=True, type_ann=None, default=None)
        ] + [
            AstParam(name=fname, mutable=False, type_ann=None, default=None)
            for fname in field_names
        ]
        init_fn = TlFunction(
            name="__init__",
            params=init_params,
            body=init_body,
        )

        # When the C/C++ struct layout is fully known, instances use a
        # C-ABI-compliant raw block (field widths and offsets match C, so
        # the block can be passed directly as a struct pointer).
        cls = TlClass(
            name=sdef.name,
            bases=[],
            methods={"__init__": [init_fn]},
            gen_methods={},
            field_defaults=[],
            class_vars={},
            field_mutability=field_mutability,
            field_access={},
            method_access={},
            static_method_names=set(),
            class_method_names=set(),
            static_vars={},
            field_index=field_index,
            field_count=len(field_names),
            field_mutability_vec=field_mutability_vec,
            raw_layout=raw_layouts.get(sdef.name),
        )
        members[sdef.name] = cls

    # Build callable wrappers for each exported function.
    # Prefer the plain symbol (Python-built shim / plain C DLL); fall back to
    # the `{name}_typed` unified-ABI entry of Rust-built wrapper DLLs.
    for sig in sigs:
        wrapper = _make_ctypes_wrapper(lib, sig, raw_layouts)
        if wrapper is None:
            wrapper = _make_typed_wrapper(lib, sig, raw_layouts)
        if wrapper is not None:
            nc = _make_native(sig.name, _wrap_drop_writebacks(wrapper))
            # The interpreter's cpp call path uses these to run the runtime
            # mut check and assign primitive out-ptr write-backs to variables.
            nc.cpp_sig = sig
            nc.cpp_call = wrapper
            members[sig.name] = nc

    if not members:
        print(
            f"CppImport: warning: no callable symbols found in '{dll_path}'",
            file=sys.stderr,
        )

    return TlNamespace(name=module_name, members=members)


def _init_bridge_dll(lib: ctypes.CDLL) -> None:
    """Call the wrapper DLL's bridge-init entry with the Python arena callbacks."""
    from ..native_api import make_ar_callbacks, ArCallbacks

    init_fn = None
    for name in ("ar_init_bridge", "hv_init_bridge", "ar_init", "hv_init"):
        try:
            init_fn = getattr(lib, name)
            break
        except AttributeError:
            continue
    if init_fn is None:
        return
    cb = make_ar_callbacks()
    init_fn.argtypes = [ctypes.POINTER(ArCallbacks)]
    init_fn.restype = None
    init_fn(ctypes.byref(cb))
    # Keep the callback table alive for the DLL's lifetime
    _BRIDGE_CALLBACKS.append(cb)


_BRIDGE_CALLBACKS: list = []


def _wrap_drop_writebacks(wrapper: Callable) -> Callable:
    """Adapt a (ret, writebacks) wrapper to a plain-return callable.

    Used when a cpp function is called without argument AST context
    (e.g. as a stored callback) — named-variable write-back is impossible
    there, so primitive out-ptr write-backs are dropped (safe side,
    mirroring the Rust no-CallArg path).
    """
    def call(args: list, kwargs: dict) -> object:
        ret, _writebacks = wrapper(args, kwargs)
        return ret
    return call
