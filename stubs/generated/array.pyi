# array.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/array.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer, SupportsRead, SupportsWrite
from collections.abc import Iterable, MutableSequence
from types import GenericAlias
from typing import Any, ClassVar, Literal, SupportsIndex, TypeAlias, TypeVar, overload
from typing_extensions import Self, deprecated, disjoint_base
typecodes: str

@disjoint_base
class array(MutableSequence[_T]):

    @property
    def typecode(self) -> Literal['b', 'B', 'h', 'H', 'i', 'I', 'l', 'L', 'q', 'Q'] | Literal['f', 'd'] | Literal['u']:
        ...

    @property
    def itemsize(self) -> int:
        ...

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def append(self, v: Any, /) -> None:
        ...

    def buffer_info(self) -> tuple[int, int]:
        ...

    def byteswap(self) -> None:
        ...

    def count(self, v: Any, /) -> int:
        ...

    def extend(self, bb: Iterable[Any], /) -> None:
        ...

    def frombytes(self, buffer: ReadableBuffer, /) -> None:
        ...

    def fromfile(self, f: SupportsRead[bytes], n: int, /) -> None:
        ...

    def fromlist(self, list: list[Any], /) -> None:
        ...

    def fromunicode(self, ustr: str, /) -> None:
        ...

    def index(self, v: Any, start: int=0, stop: int=sys.maxsize, /) -> int:
        ...

    def insert(self, i: int, v: Any, /) -> None:
        ...

    def pop(self, i: int=-1, /) -> Any:
        ...

    def remove(self, v: Any, /) -> None:
        ...

    def tobytes(self) -> bytes:
        ...

    def tofile(self, f: SupportsWrite[bytes], /) -> None:
        ...

    def tolist(self) -> list[Any]:
        ...

    def tounicode(self) -> str:
        ...
    __hash__: ClassVar[None]

    def __contains__(self, value: object, /) -> bool:
        ...

    def __len__(self) -> int:
        ...

    def __getitem__(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def __setitem__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __delitem__(self, key: SupportsIndex | slice[SupportsIndex | None], /) -> None:
        ...

    def __add__(self, value: array[Any], /) -> array[Any]:
        ...

    def __eq__(self, value: object, /) -> bool:
        ...

    def __ge__(self, value: array[Any], /) -> bool:
        ...

    def __gt__(self, value: array[Any], /) -> bool:
        ...

    def __iadd__(self, value: array[Any], /) -> Self:
        ...

    def __imul__(self, value: int, /) -> Self:
        ...

    def __le__(self, value: array[Any], /) -> bool:
        ...

    def __lt__(self, value: array[Any], /) -> bool:
        ...

    def __mul__(self, value: int, /) -> array[Any]:
        ...

    def __rmul__(self, value: int, /) -> array[Any]:
        ...

    def __copy__(self) -> array[Any]:
        ...

    def __deepcopy__(self, unused: Any, /) -> array[Any]:
        ...

    def __buffer__(self, flags: int, /) -> memoryview:
        ...

    def __release_buffer__(self, buffer: memoryview, /) -> None:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...
ArrayType = array
