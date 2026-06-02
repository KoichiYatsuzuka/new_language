# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""ctypes-based DLL loader and value marshaling for cpp-dll / cpp-lib imports.

Replaces the Rust 'generate wrapper DLL + compile' approach with direct ctypes
dispatch: Python's ctypes handles Havakyrie value ↔ C type conversion natively,
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
    """Convert a Havakyrie runtime value to a ctypes-compatible C argument."""
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
    """Convert a ctypes return value to a Havakyrie runtime value."""
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


def _make_ctypes_wrapper(lib: ctypes.CDLL, sig: CFnSig) -> Optional[Callable]:
    """Build a Python callable that calls sig.name in lib via ctypes.

    The wrapper takes (args: list, kwargs: dict) and returns a Havakyrie value.
    Mutable pointer parameters are written back after the call by returning
    a tuple (return_value, wb1, wb2, ...) when there are writebacks.
    """
    # Retrieve the function from the DLL
    try:
        fn = getattr(lib, sig.name)
    except AttributeError:
        return None  # symbol not exported → skip

    # Set up argtypes and restype
    argtypes = []
    for _, ct in sig.params:
        match ct:
            case CPtr(mutable=True):
                # For mutable ptrs we pass byref; ctypes infers the type
                argtypes.append(None)  # placeholder; bypassed via byref
            case _:
                argtypes.append(_to_ctypes(ct))

    # Set restype (None = void)
    fn.restype = _to_ctypes(sig.ret)

    mut_indices = [i for i, (_, ct) in enumerate(sig.params)
                   if isinstance(ct, CPtr) and ct.mutable]

    def wrapper(args: list, kwargs: dict) -> object:
        # Pad args to n_required minimum (treat missing as None → 0)
        n_params = len(sig.params)
        padded = list(args) + [None] * (n_params - len(args))
        padded = padded[:n_params]

        c_args = []
        mut_boxes: list[tuple[int, _MutPtrBox, CType]] = []

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
            else:
                c_args.append(_value_to_c(v, ct))

        try:
            raw = fn(*c_args)
        except Exception as e:
            raise RuntimeError(f"CppBridge: call to '{sig.name}' failed: {e}") from e

        ret_val = _c_to_value(raw, sig.ret)

        if not mut_boxes:
            return ret_val

        # Return (ret_val, writeback1, writeback2, ...) as a TlTuple
        from ..value import TlTuple
        parts = [ret_val] + [box.read_back(inner_ct) for _, box, inner_ct in mut_boxes]
        return TlTuple(values=parts)

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

    members: dict[str, object] = {}

    # Build TlClass stubs for C structs so Havakyrie code can construct instances
    for sdef in struct_defs:
        from ...ast import Param, Stmt, Expr, Accessibility, FieldKind
        from ..value import TlClass

        field_names = [fname for fname, _ in sdef.fields]
        field_mutability = {fname: True for fname in field_names}

        # __init__ body: self.field = field for each field
        from ...ast import StmtAttrAssign, ExprIdent, ExprAttr
        from ...token import Span
        init_body: list[object] = [
            StmtAttrAssign(
                target=ExprAttr(
                    object=ExprIdent("self"),
                    attr=fname,
                    span=Span.unknown() if hasattr(Span, "unknown") else None,  # type: ignore[arg-type]
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
        )
        members[sdef.name] = cls

    # Build callable wrappers for each exported function
    for sig in sigs:
        wrapper = _make_ctypes_wrapper(lib, sig)
        if wrapper is not None:
            members[sig.name] = _make_native(sig.name, wrapper)

    if not members:
        print(
            f"CppImport: warning: no callable symbols found in '{dll_path}'",
            file=sys.stderr,
        )

    return TlNamespace(name=module_name, members=members)
