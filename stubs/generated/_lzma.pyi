# _lzma.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_lzma.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from collections.abc import Mapping, Sequence
from typing import Any, Final, TypeAlias, final
from typing_extensions import Self
FORMAT_AUTO: int = 0
FORMAT_XZ: int = 1
FORMAT_ALONE: int = 2
FORMAT_RAW: int = 3
CHECK_NONE: int = 0
CHECK_CRC32: int = 1
CHECK_CRC64: int = 4
CHECK_SHA256: int = 10
CHECK_ID_MAX: int = 15
CHECK_UNKNOWN: int = 16
FILTER_LZMA1: Final[int]
FILTER_LZMA2: int = 33
FILTER_DELTA: int = 3
FILTER_X86: int = 4
FILTER_IA64: int = 6
FILTER_ARM: int = 7
FILTER_ARMTHUMB: int = 8
FILTER_SPARC: int = 9
FILTER_POWERPC: int = 5
MF_HC3: int = 3
MF_HC4: int = 4
MF_BT2: int = 18
MF_BT3: int = 19
MF_BT4: int = 20
MODE_FAST: int = 1
MODE_NORMAL: int = 2
PRESET_DEFAULT: int = 6
PRESET_EXTREME: Final[int]

@final
class LZMADecompressor:

    def __init__(self, format: int=0, memlimit: int | None=None, filters: Sequence[Mapping[str, Any]] | None=None) -> None:
        ...

    def decompress(self, data: ReadableBuffer, max_length: int=-1) -> bytes:
        ...

    @property
    def check(self) -> int:
        ...

    @property
    def eof(self) -> bool:
        ...

    @property
    def unused_data(self) -> bytes:
        ...

    @property
    def needs_input(self) -> bool:
        ...

@final
class LZMACompressor:

    def __init__(self, format: int=1, check: int=-1, preset: int | None=None, filters: Sequence[Mapping[str, Any]] | None=None) -> None:
        ...

    def compress(self, data: ReadableBuffer, /) -> bytes:
        ...

    def flush(self) -> bytes:
        ...

class LZMAError(Exception):
    ...

def is_check_supported(check_id: int, /) -> bool:
    ...
