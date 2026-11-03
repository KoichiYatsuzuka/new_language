# _ctypes.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_ctypes.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import _typeshed
import builtins
import sys
from _typeshed import ReadableBuffer, StrOrBytesPath, WriteableBuffer
from abc import abstractmethod
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from ctypes import CDLL, ArgumentError as ArgumentError, c_void_p
from types import GenericAlias
from typing import Any, ClassVar, Final, Generic, Literal, SupportsIndex, TypeAlias, TypeVar, final, overload, type_check_only
from typing_extensions import Self
FUNCFLAG_CDECL: int = 1
FUNCFLAG_PYTHONAPI: int = 4
FUNCFLAG_USE_ERRNO: int = 8
FUNCFLAG_USE_LASTERROR: int = 16
RTLD_GLOBAL: Final[int]
RTLD_LOCAL: Final[int]
CTYPES_MAX_ARGCOUNT: Final[int]
SIZEOF_TIME_T: Final[int]

class COMError(Exception):
    hresult: int
    text: str | None
    details: tuple[str | None, str | None, str | None, int | None, int | None]

    def __init__(self, hresult: int, text: str | None, details: tuple[str | None, str | None, str | None, int | None, int | None]) -> None:
        ...

def CopyComPointer(src: _PointerLike, dst: _PointerLike | _CArgObject) -> int:
    ...
FUNCFLAG_HRESULT: int = 2
FUNCFLAG_STDCALL: int = 0

def FormatError(code: int=...) -> str:
    ...

def get_last_error() -> int:
    ...

def set_last_error(value: int) -> int:
    ...

def LoadLibrary(name: str, load_flags: int=0, /) -> int:
    ...

def FreeLibrary(handle: int, /) -> None:
    ...

@type_check_only
class _CData:
    _b_base_: int
    _b_needsfree_: bool
    _objects: Mapping[Any, int] | None

    def __buffer__(self, flags: int, /) -> memoryview:
        ...

    def __ctypes_from_outparam__(self, /) -> Self:
        ...

@type_check_only
class _PyCSimpleType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def __mul__(self: type[Any], value: int, /) -> type[Array[Any]]:
        ...

    def __rmul__(self: type[Any], value: int, /) -> type[Array[Any]]:
        ...

class _SimpleCData(_CData, Generic[_T], metaclass=_PyCSimpleType):
    value: Any

    def __init__(self, value: Any=...) -> None:
        ...

    def __ctypes_from_outparam__(self, /) -> Any:
        ...

@type_check_only
class _CanCastTo(_CData):
    ...

@type_check_only
class _PointerLike(_CanCastTo):
    ...

@type_check_only
class _PyCPointerType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def set_type(self, type: type, /) -> None:
        ...

    def __mul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

    def __rmul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

class _Pointer(_PointerLike, _CData, Generic[_CT], metaclass=_PyCPointerType):
    _type_: type[Any]
    contents: Any

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __getitem__(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def __setitem__(self, key: int, value: Any, /) -> None:
        ...

@overload
def POINTER(type: None, /) -> type[c_void_p]:
    ...

@overload
def POINTER(type: type[Any], /) -> type[_Pointer[Any]]:
    ...

def pointer(obj: Any, /) -> _Pointer[Any]:
    ...

@final
@type_check_only
class _CArgObject:
    ...

def byref(obj: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]), offset: int=0) -> _CArgObject:
    ...

@type_check_only
class _PyCFuncPtrType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def __mul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

    def __rmul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

class CFuncPtr(_PointerLike, _CData, metaclass=_PyCFuncPtrType):
    restype: type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]] | Callable[[int], Any] | None
    argtypes: Sequence[type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]]]
    errcheck: Callable[[_CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]) | None, CFuncPtr, tuple[_CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]), ...]], _SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]]
    _flags_: ClassVar[int]

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        ...

@final
@type_check_only
class _CField(Generic[_CT, _GetT, _SetT]):
    offset: int
    size: int

    def __get__(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def __set__(self, instance: Any, value: Any, /) -> None:
        ...

@type_check_only
class _UnionType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def __mul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

    def __rmul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

class Union(_CData, metaclass=_UnionType):
    _fields_: ClassVar[Sequence[tuple[str, type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]]] | tuple[str, type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]], int]]]
    _pack_: ClassVar[int]
    _anonymous_: ClassVar[Sequence[str]]

    def __init__(self, *args: Any, **kw: Any) -> None:
        ...

@type_check_only
class _PyCStructType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def __mul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

    def __rmul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

class Structure(_CData, metaclass=_PyCStructType):
    _fields_: ClassVar[Sequence[tuple[str, type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]]] | tuple[str, type[_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]], int]]]
    _pack_: ClassVar[int]
    _anonymous_: ClassVar[Sequence[str]]

    def __init__(self, *args: Any, **kw: Any) -> None:
        ...

@type_check_only
class _PyCArrayType(_CTypeBaseType):

    def from_address(self: type[_typeshed.Self], value: int, /) -> _typeshed.Self:
        ...

    def from_buffer(self: type[_typeshed.Self], obj: WriteableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_buffer_copy(self: type[_typeshed.Self], buffer: ReadableBuffer, offset: int=0, /) -> _typeshed.Self:
        ...

    def from_param(self: type[_typeshed.Self], value: Any, /) -> _typeshed.Self | _CArgObject:
        ...

    def in_dll(self: type[_typeshed.Self], dll: CDLL, name: str, /) -> _typeshed.Self:
        ...

    def __mul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

    def __rmul__(cls: type[Any], other: int) -> type[Array[Any]]:
        ...

class Array(_CData, Generic[_CT], metaclass=_PyCArrayType):

    @property
    @abstractmethod
    def _length_(self) -> int:
        ...

    @_length_.setter
    def _length_(self, value: int) -> None:
        ...

    @property
    @abstractmethod
    def _type_(self) -> type[Any]:
        ...

    @_type_.setter
    def _type_(self, value: type[Any]) -> None:
        ...

    @property
    def raw(self) -> bytes:
        ...

    @raw.setter
    def raw(self, value: ReadableBuffer) -> None:
        ...
    value: Any

    def __init__(self, *args: Any) -> None:
        ...

    def __getitem__(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def __setitem__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Iterator[Any]:
        ...

    def __len__(self) -> int:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

def addressof(obj: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]), /) -> int:
    ...

def alignment(obj_or_type: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]) | type[_CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any])], /) -> int:
    ...

def get_errno() -> int:
    ...

def resize(obj: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]), size: int, /) -> None:
    ...

def set_errno(value: int, /) -> int:
    ...

def sizeof(obj_or_type: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]) | type[_CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any])], /) -> int:
    ...

def PyObj_FromPtr(address: int, /) -> Any:
    ...

def Py_DECREF(o: Any, /) -> Any:
    ...

def Py_INCREF(o: Any, /) -> Any:
    ...

def buffer_info(o: _CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any]) | type[_CData | (_SimpleCData[Any] | _Pointer[Any] | CFuncPtr | Union | Structure | Array[Any])], /) -> tuple[str, int, tuple[int, ...]]:
    ...

def call_cdeclfunction(address: int, arguments: tuple[Any, ...], /) -> Any:
    ...

def call_function(address: int, arguments: tuple[Any, ...], /) -> Any:
    ...
