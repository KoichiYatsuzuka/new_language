# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
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
    """C/C++ struct/class definition extracted from a header file.

    `complete` is True when `fields` contains ALL layout members of the
    C/C++ side. It is False when array/bitfield/nested-struct/unresolved
    fields were skipped, or for unions and classes with inheritance —
    such structs cannot get a raw layout.
    """
    name: str
    fields: list        # list[tuple[str, CType]]
    complete: bool = False

    def raw_layout(self):
        """Build the raw-block layout (C ABI area of TlInstance.raw) for this struct.

        Conditions: the field list is complete and every field has a fixed
        primitive width. C `int`→int32, `float`→float32, `double`→float64.
        C `long` is platform-dependent (Windows LLP64 = 4B / Linux LP64 = 8B)
        and excluded; `bool` is 1 byte in C++ but the existing mirror assumes
        i32, so it is excluded as ambiguous.
        """
        if not self.complete:
            return None
        from ..value import RawLayout
        anns: list[tuple[str, str]] = []
        for name, ct in self.fields:
            if isinstance(ct, CInt):
                ann = "int32"
            elif isinstance(ct, CFloat):
                ann = "float32"
            elif isinstance(ct, CDouble):
                ann = "float64"
            else:
                return None
            anns.append((name, ann))
        return RawLayout.from_fields(anns)


def ctype_to_tl_str(ct: CType) -> str:
    """Return the Arrow type string for a CType (static type-check stubs only —
    runtime marshaling is decided separately from the C signature).

    - Primitive pointers (`int*` / `double*` etc.) are annotated with the
      POINTEE type (`double*` → "float") so write-back matches the value type.
    - Struct pointers / by-value structs are "Any": binding them nominally
      would break both shadow conversion of structurally compatible classes
      and the int-handle path. Mutability checking (`mut`) works independently
      of the type annotation via Param.mutable.
    """
    match ct:
        case CInt() | CLong():
            return "int"
        case CFloat() | CDouble():
            return "float"
        case CBool():
            return "bool"
        case CVoid():
            return "None"
        case CVoidPtr():
            return "int"
        case COpaqueStructPtr() | CByValueStruct():
            return "Any"
        case CCharPtr():
            return "str"
        case CPtr(inner=inner):
            return ctype_to_tl_str(inner)
        case CFnPtr():
            return "function"
        case _:
            return "int"
