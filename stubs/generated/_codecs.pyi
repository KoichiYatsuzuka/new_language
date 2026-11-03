# _codecs.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_codecs.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import codecs
import sys
from _typeshed import ReadableBuffer
from collections.abc import Callable
from typing import Literal, TypeAlias, final, overload, type_check_only

@final
@type_check_only
class _EncodingMap:

    def size(self) -> int:
        ...

def register(search_function: Callable[[str], codecs.CodecInfo | None], /) -> None:
    ...

def unregister(search_function: Callable[[str], codecs.CodecInfo | None], /) -> None:
    ...

def register_error(errors: str, handler: Callable[[UnicodeError], tuple[str | bytes, int]], /) -> None:
    ...

def lookup_error(name: str, /) -> Callable[[UnicodeError], tuple[str | bytes, int]]:
    ...

@overload
def encode(obj: ReadableBuffer, encoding: Literal['base64', 'base_64', 'base64_codec', 'bz2', 'bz2_codec', 'hex', 'hex_codec', 'quopri', 'quotedprintable', 'quoted_printable', 'quopri_codec', 'uu', 'uu_codec', 'zip', 'zlib', 'zlib_codec'], errors: str='strict') -> bytes:
    ...

@overload
def encode(obj: str, encoding: Literal['rot13', 'rot_13'], errors: str='strict') -> str:
    ...

@overload
def encode(obj: str, encoding: str='utf-8', errors: str='strict') -> bytes:
    ...

@overload
def decode(obj: ReadableBuffer, encoding: Literal['base64', 'base_64', 'base64_codec', 'bz2', 'bz2_codec', 'hex', 'hex_codec', 'quopri', 'quotedprintable', 'quoted_printable', 'quopri_codec', 'uu', 'uu_codec', 'zip', 'zlib', 'zlib_codec'], errors: str='strict') -> bytes:
    ...

@overload
def decode(obj: str, encoding: Literal['rot13', 'rot_13'], errors: str='strict') -> str:
    ...

@overload
def decode(obj: str, encoding: Literal['unicode_escape', 'unicode-escape', 'raw_unicode_escape', 'raw-unicode-escape'], errors: str='strict') -> str:
    ...

@overload
def decode(obj: str, encoding: Literal['hex', 'hex_codec'], errors: str='strict') -> bytes:
    ...

@overload
def decode(obj: ReadableBuffer, encoding: str='utf-8', errors: str='strict') -> str:
    ...

def lookup(encoding: str, /) -> codecs.CodecInfo:
    ...

def charmap_build(map: str, /) -> dict[int, int] | _EncodingMap:
    ...

def ascii_decode(data: ReadableBuffer, errors: str | None=None, /) -> tuple[str, int]:
    ...

def ascii_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def charmap_decode(data: ReadableBuffer, errors: str | None=None, mapping: dict[int, int] | _EncodingMap | None=None, /) -> tuple[str, int]:
    ...

def charmap_encode(str: str, errors: str | None=None, mapping: dict[int, int] | _EncodingMap | None=None, /) -> tuple[bytes, int]:
    ...

def escape_decode(data: str | ReadableBuffer, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def escape_encode(data: bytes, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def latin_1_decode(data: ReadableBuffer, errors: str | None=None, /) -> tuple[str, int]:
    ...

def latin_1_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def raw_unicode_escape_decode(data: str | ReadableBuffer, errors: str | None=None, final: bool=True, /) -> tuple[str, int]:
    ...

def raw_unicode_escape_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def readbuffer_encode(data: str | ReadableBuffer, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def unicode_escape_decode(data: str | ReadableBuffer, errors: str | None=None, final: bool=True, /) -> tuple[str, int]:
    ...

def unicode_escape_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_16_be_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_16_be_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_16_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_16_encode(str: str, errors: str | None=None, byteorder: int=0, /) -> tuple[bytes, int]:
    ...

def utf_16_ex_decode(data: ReadableBuffer, errors: str | None=None, byteorder: int=0, final: bool=False, /) -> tuple[str, int, int]:
    ...

def utf_16_le_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_16_le_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_32_be_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_32_be_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_32_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_32_encode(str: str, errors: str | None=None, byteorder: int=0, /) -> tuple[bytes, int]:
    ...

def utf_32_ex_decode(data: ReadableBuffer, errors: str | None=None, byteorder: int=0, final: bool=False, /) -> tuple[str, int, int]:
    ...

def utf_32_le_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_32_le_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_7_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_7_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def utf_8_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def utf_8_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def mbcs_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def mbcs_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def code_page_decode(codepage: int, data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def code_page_encode(code_page: int, str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...

def oem_decode(data: ReadableBuffer, errors: str | None=None, final: bool=False, /) -> tuple[str, int]:
    ...

def oem_encode(str: str, errors: str | None=None, /) -> tuple[bytes, int]:
    ...
