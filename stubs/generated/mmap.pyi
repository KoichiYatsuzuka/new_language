# mmap.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/mmap.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import os
import sys
from _typeshed import ReadableBuffer, Unused
from collections.abc import Iterator
from typing import Final, Literal, NoReturn, SupportsIndex, overload
from typing_extensions import Self, disjoint_base
ACCESS_DEFAULT: int = 0
ACCESS_READ: int = 1
ACCESS_WRITE: int = 2
ACCESS_COPY: int = 3
ALLOCATIONGRANULARITY: Final[int]
PAGESIZE: Final[int]

@disjoint_base
class mmap:

    def __init__(self, fileno: int, length: int, tagname: str | None=None, access: int=0, offset: int=0) -> None:
        ...

    def close(self) -> None:
        ...

    def flush(self, offset: int=0, size: int=..., /) -> None:
        ...

    def move(self, dest: int, src: int, count: int, /) -> None:
        ...

    def read_byte(self) -> int:
        ...

    def readline(self) -> bytes:
        ...

    def resize(self, newsize: int, /) -> None:
        ...

    def seek(self, pos: int, whence: Literal[0, 1, 2]=os.SEEK_SET, /) -> None:
        ...

    def size(self) -> int:
        ...

    def tell(self) -> int:
        ...

    def write_byte(self, byte: int, /) -> None:
        ...

    def __len__(self) -> int:
        ...
    closed: bool

    def find(self, view: ReadableBuffer, start: int=..., end: int=..., /) -> int:
        ...

    def rfind(self, view: ReadableBuffer, start: int=..., end: int=..., /) -> int:
        ...

    def read(self, n: int | None=None, /) -> bytes:
        ...

    def write(self, bytes: ReadableBuffer, /) -> int:
        ...

    def __getitem__(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def __delitem__(self, key: SupportsIndex | slice[SupportsIndex | None], /) -> NoReturn:
        ...

    def __setitem__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __contains__(self, o: object, /) -> bool:
        ...

    def __iter__(self) -> Iterator[int]:
        ...

    def __enter__(self) -> Self:
        ...

    def __exit__(self, exc_type: Unused, exc_value: Unused, traceback: Unused, /) -> None:
        ...

    def __buffer__(self, flags: int, /) -> memoryview:
        ...

    def __release_buffer__(self, buffer: memoryview, /) -> None:
        ...
