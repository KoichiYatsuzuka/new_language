# _msi.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

def CreateRecord(count: Any, /) -> Any: ...
def FCICreate(cabname: Any, files: Any, /) -> Any: ...
MSICOLINFO_NAMES: int
MSICOLINFO_TYPES: int
MSIDBOPEN_CREATE: int
MSIDBOPEN_CREATEDIRECT: int
MSIDBOPEN_DIRECT: int
MSIDBOPEN_PATCHFILE: int
MSIDBOPEN_READONLY: int
MSIDBOPEN_TRANSACT: int
class MSIError:
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
MSIMODIFY_ASSIGN: int
MSIMODIFY_DELETE: int
MSIMODIFY_INSERT: int
MSIMODIFY_INSERT_TEMPORARY: int
MSIMODIFY_MERGE: int
MSIMODIFY_REFRESH: int
MSIMODIFY_REPLACE: int
MSIMODIFY_SEEK: int
MSIMODIFY_UPDATE: int
MSIMODIFY_VALIDATE: int
MSIMODIFY_VALIDATE_DELETE: int
MSIMODIFY_VALIDATE_FIELD: int
MSIMODIFY_VALIDATE_NEW: int
def OpenDatabase(path: Any, persist: Any, /) -> Any: ...
PID_APPNAME: int
PID_AUTHOR: int
PID_CHARCOUNT: int
PID_CODEPAGE: int
PID_COMMENTS: int
PID_CREATE_DTM: int
PID_KEYWORDS: int
PID_LASTAUTHOR: int
PID_LASTPRINTED: int
PID_LASTSAVE_DTM: int
PID_PAGECOUNT: int
PID_REVNUMBER: int
PID_SECURITY: int
PID_SUBJECT: int
PID_TEMPLATE: int
PID_TITLE: int
PID_WORDCOUNT: int
def UuidCreate() -> Any: ...
