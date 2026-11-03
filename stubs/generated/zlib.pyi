# zlib.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

DEFLATED: int
DEF_BUF_SIZE: int
DEF_MEM_LEVEL: int
MAX_WBITS: int
ZLIB_RUNTIME_VERSION: str
ZLIB_VERSION: str
Z_BEST_COMPRESSION: int
Z_BEST_SPEED: int
Z_BLOCK: int
Z_DEFAULT_COMPRESSION: int
Z_DEFAULT_STRATEGY: int
Z_FILTERED: int
Z_FINISH: int
Z_FIXED: int
Z_FULL_FLUSH: int
Z_HUFFMAN_ONLY: int
Z_NO_COMPRESSION: int
Z_NO_FLUSH: int
Z_PARTIAL_FLUSH: int
Z_RLE: int
Z_SYNC_FLUSH: int
Z_TREES: int
def adler32(data: Any, value: Any = ..., /) -> Any: ...
def compress(data: Any, /, level: Any = ..., wbits: Any = ...) -> Any: ...
def compressobj(level: Any = ..., method: Any = ..., wbits: Any = ..., memLevel: Any = ..., strategy: Any = ..., zdict: Any = ...) -> Any: ...
def crc32(data: Any, value: Any = ..., /) -> Any: ...
def decompress(data: Any, /, wbits: Any = ..., bufsize: Any = ...) -> Any: ...
def decompressobj(wbits: Any = ..., zdict: Any = ...) -> Any: ...
class error:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    def __str__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
