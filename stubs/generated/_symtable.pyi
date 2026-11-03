# _symtable.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

CELL: int
DEF_ANNOT: int
DEF_BOUND: int
DEF_FREE: int
DEF_FREE_CLASS: int
DEF_GLOBAL: int
DEF_IMPORT: int
DEF_LOCAL: int
DEF_NONLOCAL: int
DEF_PARAM: int
FREE: int
GLOBAL_EXPLICIT: int
GLOBAL_IMPLICIT: int
LOCAL: int
SCOPE_MASK: int
SCOPE_OFF: int
TYPE_ANNOTATION: int
TYPE_CLASS: int
TYPE_FUNCTION: int
TYPE_MODULE: int
TYPE_TYPE_ALIAS: int
TYPE_TYPE_PARAM: int
TYPE_TYPE_VAR_BOUND: int
USE: int
def symtable(source: Any, filename: Any, startstr: Any, /) -> Any: ...
