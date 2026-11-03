# _lzma.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

CHECK_CRC32: int
CHECK_CRC64: int
CHECK_ID_MAX: int
CHECK_NONE: int
CHECK_SHA256: int
CHECK_UNKNOWN: int
FILTER_ARM: int
FILTER_ARMTHUMB: int
FILTER_DELTA: int
FILTER_IA64: int
FILTER_LZMA1: int
FILTER_LZMA2: int
FILTER_POWERPC: int
FILTER_SPARC: int
FILTER_X86: int
FORMAT_ALONE: int
FORMAT_AUTO: int
FORMAT_RAW: int
FORMAT_XZ: int
class LZMACompressor:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def compress(self, data: Any, /) -> Any: ...
    def flush(self) -> Any: ...
class LZMADecompressor:
    def __init__(self, format: Any = ..., memlimit: Any = ..., filters: Any = ...) -> None: ...
    check: Any
    def decompress(self, data: Any, max_length: Any = ...) -> Any: ...
    eof: Any
    needs_input: Any
    unused_data: Any
class LZMAError:
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
MF_BT2: int
MF_BT3: int
MF_BT4: int
MF_HC3: int
MF_HC4: int
MODE_FAST: int
MODE_NORMAL: int
PRESET_DEFAULT: int
PRESET_EXTREME: int
def is_check_supported(check_id: Any, /) -> Any: ...
