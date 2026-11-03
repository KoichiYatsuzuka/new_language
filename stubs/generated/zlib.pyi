# zlib.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/zlib.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from typing import Any, Final, final, type_check_only
from typing_extensions import Self
DEFLATED: int = 8
DEF_MEM_LEVEL: Final[int]
DEF_BUF_SIZE: int = 16384
MAX_WBITS: Final[int]
ZLIB_VERSION: Final[str]
ZLIB_RUNTIME_VERSION: Final[str]
Z_NO_COMPRESSION: int = 0
Z_PARTIAL_FLUSH: int = 1
Z_BEST_COMPRESSION: int = 9
Z_BEST_SPEED: int = 1
Z_BLOCK: int = 5
Z_DEFAULT_COMPRESSION: int = -1
Z_DEFAULT_STRATEGY: int = 0
Z_FILTERED: int = 1
Z_FINISH: int = 4
Z_FIXED: int = 4
Z_FULL_FLUSH: int = 3
Z_HUFFMAN_ONLY: int = 2
Z_NO_FLUSH: int = 0
Z_RLE: int = 3
Z_SYNC_FLUSH: int = 2
Z_TREES: int = 6

class error(Exception):
    ...

@final
@type_check_only
class _Compress:

    def __copy__(self) -> Self:
        ...

    def __deepcopy__(self, memo: Any, /) -> Self:
        ...

    def compress(self, data: ReadableBuffer, /) -> bytes:
        ...

    def flush(self, mode: int=4, /) -> bytes:
        ...

    def copy(self) -> _Compress:
        ...

@final
@type_check_only
class _Decompress:

    @property
    def unused_data(self) -> bytes:
        ...

    @property
    def unconsumed_tail(self) -> bytes:
        ...

    @property
    def eof(self) -> bool:
        ...

    def __copy__(self) -> Self:
        ...

    def __deepcopy__(self, memo: Any, /) -> Self:
        ...

    def decompress(self, data: ReadableBuffer, /, max_length: int=0) -> bytes:
        ...

    def flush(self, length: int=16384, /) -> bytes:
        ...

    def copy(self) -> _Decompress:
        ...

def adler32(data: ReadableBuffer, value: int=1, /) -> int:
    ...

def compress(data: ReadableBuffer, /, level: int=-1, wbits: int=15) -> bytes:
    ...

def compressobj(level: int=-1, method: int=8, wbits: int=15, memLevel: int=8, strategy: int=0, zdict: ReadableBuffer | None=None) -> _Compress:
    ...

def crc32(data: ReadableBuffer, value: int=0, /) -> int:
    ...

def decompress(data: ReadableBuffer, /, wbits: int=15, bufsize: int=16384) -> bytes:
    ...

def decompressobj(wbits: int=15, zdict: ReadableBuffer=b'') -> _Decompress:
    ...
