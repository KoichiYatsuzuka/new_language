# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""C type model, struct definitions, and function signatures (mirrors cpp_bridge/types.rs)."""
from __future__ import annotations
from dataclasses import dataclass
from typing import Optional


@dataclass
class CInt:
    """int, short, char, uint32_t, DWORD, WORD, etc."""

@dataclass
class CLong:
    """long long, int64_t, size_t, LONGLONG, etc."""

@dataclass
class CFloat:
    """float"""

@dataclass
class CDouble:
    """double"""

@dataclass
class CBool:
    """bool, BOOL"""

@dataclass
class CVoid:
    """void (return type only)"""

@dataclass
class CVoidPtr:
    """void* — opaque pointer stored as int"""

@dataclass
class CCharPtr:
    """char* / const char* — mapped to str"""

@dataclass
class CPtr:
    """T* or const T* pointer parameter.
    mutable=True → output param written back after the call.
    mutable=False → read-only input param."""
    inner: "CType"
    mutable: bool

@dataclass
class COpaqueStructPtr:
    """Struct/union pointer — marshaled as void*."""
    type_name: str
    mutable: bool

@dataclass
class CByValueStruct:
    """Struct passed by value — treated as opaque int handle."""
    type_name: str

@dataclass
class CFnPtr:
    """Function pointer parameter — passed as opaque void*."""


CType = (CInt | CLong | CFloat | CDouble | CBool | CVoid | CVoidPtr | CCharPtr |
         CPtr | COpaqueStructPtr | CByValueStruct | CFnPtr)


@dataclass
class CFnSig:
    """C function signature extracted from a header file."""
    name: str
    params: list        # list[tuple[str, CType]]
    ret: CType
    namespace: Optional[str] = None
    n_required: int = 0  # index of first optional param (0 = all required)


@dataclass
class CStructDef:
    """C struct/union definition extracted from a header file."""
    name: str
    fields: list        # list[tuple[str, CType]]


def ctype_to_tl_str(ct: CType) -> str:
    """Return the Havakyrie type string for a CType (mirrors ctype_to_tl_str in exec.rs)."""
    match ct:
        case CInt() | CLong():
            return "int"
        case CFloat() | CDouble():
            return "float"
        case CBool():
            return "bool"
        case CVoid():
            return "None"
        case CVoidPtr() | COpaqueStructPtr() | CByValueStruct():
            return "int"
        case CCharPtr():
            return "str"
        case CPtr(inner=inner, mutable=mutable):
            return f"mut {ctype_to_tl_str(inner)}" if mutable else ctype_to_tl_str(inner)
        case CFnPtr():
            return "function"
        case _:
            return "int"
