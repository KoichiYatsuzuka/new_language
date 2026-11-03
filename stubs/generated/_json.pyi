# _json.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

def encode_basestring(*args: Any, **kwargs: Any) -> Any: ...
def encode_basestring_ascii(*args: Any, **kwargs: Any) -> Any: ...
class make_encoder:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __call__(self, *args: Any, **kwargs: Any) -> Any: ...
    default: Any
    encoder: Any
    indent: Any
    item_separator: Any
    key_separator: Any
    markers: Any
    skipkeys: Any
    sort_keys: Any
class make_scanner:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __call__(self, *args: Any, **kwargs: Any) -> Any: ...
    object_hook: Any
    object_pairs_hook: Any
    parse_constant: Any
    parse_float: Any
    parse_int: Any
    strict: Any
def scanstring(*args: Any, **kwargs: Any) -> Any: ...
