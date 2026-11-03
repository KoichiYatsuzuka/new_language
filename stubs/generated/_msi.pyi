# _msi.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_msi.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from typing import Final, type_check_only

class MSIError(Exception):
    ...

@type_check_only
class _View:

    def Execute(self, params: _Record | None=...) -> None:
        ...

    def GetColumnInfo(self, kind: int) -> _Record:
        ...

    def Fetch(self) -> _Record:
        ...

    def Modify(self, mode: int, record: _Record) -> None:
        ...

    def Close(self) -> None:
        ...
    __new__: None
    __init__: None

@type_check_only
class _SummaryInformation:

    def GetProperty(self, field: int) -> int | bytes | None:
        ...

    def GetPropertyCount(self) -> int:
        ...

    def SetProperty(self, field: int, value: int | str) -> None:
        ...

    def Persist(self) -> None:
        ...
    __new__: None
    __init__: None

@type_check_only
class _Database:

    def OpenView(self, sql: str) -> _View:
        ...

    def Commit(self) -> None:
        ...

    def GetSummaryInformation(self, updateCount: int) -> _SummaryInformation:
        ...

    def Close(self) -> None:
        ...
    __new__: None
    __init__: None

@type_check_only
class _Record:

    def GetFieldCount(self) -> int:
        ...

    def GetInteger(self, field: int) -> int:
        ...

    def GetString(self, field: int) -> str:
        ...

    def SetString(self, field: int, str: str) -> None:
        ...

    def SetStream(self, field: int, stream: str) -> None:
        ...

    def SetInteger(self, field: int, int: int) -> None:
        ...

    def ClearData(self) -> None:
        ...
    __new__: None
    __init__: None

def UuidCreate() -> str:
    ...

def FCICreate(cabname: str, files: list[tuple[str, str]], /) -> None:
    ...

def OpenDatabase(path: str, persist: int, /) -> _Database:
    ...

def CreateRecord(count: int, /) -> _Record:
    ...
MSICOLINFO_NAMES: Final[int]
MSICOLINFO_TYPES: Final[int]
MSIDBOPEN_CREATE: Final[int]
MSIDBOPEN_CREATEDIRECT: Final[int]
MSIDBOPEN_DIRECT: Final[int]
MSIDBOPEN_PATCHFILE: Final[int]
MSIDBOPEN_READONLY: Final[int]
MSIDBOPEN_TRANSACT: Final[int]
MSIMODIFY_ASSIGN: Final[int]
MSIMODIFY_DELETE: Final[int]
MSIMODIFY_INSERT: Final[int]
MSIMODIFY_INSERT_TEMPORARY: Final[int]
MSIMODIFY_MERGE: Final[int]
MSIMODIFY_REFRESH: Final[int]
MSIMODIFY_REPLACE: Final[int]
MSIMODIFY_SEEK: Final[int]
MSIMODIFY_UPDATE: Final[int]
MSIMODIFY_VALIDATE: Final[int]
MSIMODIFY_VALIDATE_DELETE: Final[int]
MSIMODIFY_VALIDATE_FIELD: Final[int]
MSIMODIFY_VALIDATE_NEW: Final[int]
PID_APPNAME: Final[int]
PID_AUTHOR: Final[int]
PID_CHARCOUNT: Final[int]
PID_CODEPAGE: Final[int]
PID_COMMENTS: Final[int]
PID_CREATE_DTM: Final[int]
PID_KEYWORDS: Final[int]
PID_LASTAUTHOR: Final[int]
PID_LASTPRINTED: Final[int]
PID_LASTSAVE_DTM: Final[int]
PID_PAGECOUNT: Final[int]
PID_REVNUMBER: Final[int]
PID_SECURITY: Final[int]
PID_SUBJECT: Final[int]
PID_TEMPLATE: Final[int]
PID_TITLE: Final[int]
PID_WORDCOUNT: Final[int]
