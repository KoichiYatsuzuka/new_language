# _json.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_json.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from collections.abc import Callable
from typing import Any, final
from typing_extensions import Self

@final
class make_encoder:

    @property
    def sort_keys(self) -> bool:
        ...

    @property
    def skipkeys(self) -> bool:
        ...

    @property
    def key_separator(self) -> str:
        ...

    @property
    def indent(self) -> str | None:
        ...

    @property
    def markers(self) -> dict[int, Any] | None:
        ...

    @property
    def default(self) -> Callable[[Any], Any]:
        ...

    @property
    def encoder(self) -> Callable[[str], str]:
        ...

    @property
    def item_separator(self) -> str:
        ...

    def __init__(self, markers: dict[int, Any] | None, default: Callable[[Any], Any], encoder: Callable[[str], str], indent: str | None, key_separator: str, item_separator: str, sort_keys: bool, skipkeys: bool, allow_nan: bool) -> None:
        ...

    def __call__(self, obj: object, _current_indent_level: int) -> Any:
        ...

@final
class make_scanner:
    object_hook: Any
    object_pairs_hook: Any
    parse_int: Any
    parse_constant: Any
    parse_float: Any
    strict: bool

    def __init__(self, context: make_scanner) -> None:
        ...

    def __call__(self, string: str, index: int) -> tuple[Any, int]:
        ...

def encode_basestring(s: str, /) -> str:
    ...

def encode_basestring_ascii(s: str, /) -> str:
    ...

def scanstring(string: str, end: int, strict: bool=True) -> tuple[str, int]:
    ...
