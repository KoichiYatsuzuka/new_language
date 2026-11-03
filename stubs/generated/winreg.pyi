# winreg.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/winreg.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer, Unused
from types import TracebackType
from typing import Any, Final, Literal, TypeAlias, final, overload
from typing_extensions import Self

def CloseKey(hkey: HKEYType | int, /) -> None:
    ...

def ConnectRegistry(computer_name: str | None, key: HKEYType | int, /) -> HKEYType:
    ...

def CreateKey(key: HKEYType | int, sub_key: str | None, /) -> HKEYType:
    ...

def CreateKeyEx(key: HKEYType | int, sub_key: str | None, reserved: int=0, access: int=131078) -> HKEYType:
    ...

def DeleteKey(key: HKEYType | int, sub_key: str, /) -> None:
    ...

def DeleteKeyEx(key: HKEYType | int, sub_key: str, access: int=256, reserved: int=0) -> None:
    ...

def DeleteValue(key: HKEYType | int, value: str, /) -> None:
    ...

def EnumKey(key: HKEYType | int, index: int, /) -> str:
    ...

def EnumValue(key: HKEYType | int, index: int, /) -> tuple[str, Any, int]:
    ...

def ExpandEnvironmentStrings(string: str, /) -> str:
    ...

def FlushKey(key: HKEYType | int, /) -> None:
    ...

def LoadKey(key: HKEYType | int, sub_key: str, file_name: str, /) -> None:
    ...

def OpenKey(key: HKEYType | int, sub_key: str | None, reserved: int=0, access: int=131097) -> HKEYType:
    ...

def OpenKeyEx(key: HKEYType | int, sub_key: str | None, reserved: int=0, access: int=131097) -> HKEYType:
    ...

def QueryInfoKey(key: HKEYType | int, /) -> tuple[int, int, int]:
    ...

def QueryValue(key: HKEYType | int, sub_key: str | None, /) -> str:
    ...

def QueryValueEx(key: HKEYType | int, name: str, /) -> tuple[Any, int]:
    ...

def SaveKey(key: HKEYType | int, file_name: str, /) -> None:
    ...

def SetValue(key: HKEYType | int, sub_key: str | None, type: int, value: str, /) -> None:
    ...

@overload
def SetValueEx(key: HKEYType | int, value_name: str | None, reserved: Unused, type: Literal[4, 5], value: int | None, /) -> None:
    ...

@overload
def SetValueEx(key: HKEYType | int, value_name: str | None, reserved: Unused, type: Literal[1, 2], value: str | None, /) -> None:
    ...

@overload
def SetValueEx(key: HKEYType | int, value_name: str | None, reserved: Unused, type: Literal[7], value: list[str] | None, /) -> None:
    ...

@overload
def SetValueEx(key: HKEYType | int, value_name: str | None, reserved: Unused, type: Literal[0, 3, 8, 9, 10, 11], value: ReadableBuffer | None, /) -> None:
    ...

@overload
def SetValueEx(key: HKEYType | int, value_name: str | None, reserved: Unused, type: int, value: int | str | list[str] | ReadableBuffer | None, /) -> None:
    ...

def DisableReflectionKey(key: HKEYType | int, /) -> None:
    ...

def EnableReflectionKey(key: HKEYType | int, /) -> None:
    ...

def QueryReflectionKey(key: HKEYType | int, /) -> bool:
    ...
HKEY_CLASSES_ROOT: Final[int]
HKEY_CURRENT_USER: Final[int]
HKEY_LOCAL_MACHINE: Final[int]
HKEY_USERS: Final[int]
HKEY_PERFORMANCE_DATA: Final[int]
HKEY_CURRENT_CONFIG: Final[int]
HKEY_DYN_DATA: Final[int]
KEY_ALL_ACCESS: int = 983103
KEY_WRITE: int = 131078
KEY_READ: int = 131097
KEY_EXECUTE: int = 131097
KEY_QUERY_VALUE: int = 1
KEY_SET_VALUE: int = 2
KEY_CREATE_SUB_KEY: int = 4
KEY_ENUMERATE_SUB_KEYS: int = 8
KEY_NOTIFY: int = 16
KEY_CREATE_LINK: int = 32
KEY_WOW64_64KEY: int = 256
KEY_WOW64_32KEY: int = 512
REG_BINARY: int = 3
REG_DWORD: int = 4
REG_DWORD_LITTLE_ENDIAN: int = 4
REG_DWORD_BIG_ENDIAN: int = 5
REG_EXPAND_SZ: int = 2
REG_LINK: int = 6
REG_MULTI_SZ: int = 7
REG_NONE: int = 0
REG_QWORD: int = 11
REG_QWORD_LITTLE_ENDIAN: int = 11
REG_RESOURCE_LIST: int = 8
REG_FULL_RESOURCE_DESCRIPTOR: int = 9
REG_RESOURCE_REQUIREMENTS_LIST: int = 10
REG_SZ: int = 1
REG_CREATED_NEW_KEY: int = 1
REG_LEGAL_CHANGE_FILTER: int = 268435471
REG_LEGAL_OPTION: int = 31
REG_NOTIFY_CHANGE_ATTRIBUTES: int = 2
REG_NOTIFY_CHANGE_LAST_SET: int = 4
REG_NOTIFY_CHANGE_NAME: int = 1
REG_NOTIFY_CHANGE_SECURITY: int = 8
REG_NO_LAZY_FLUSH: int = 4
REG_OPENED_EXISTING_KEY: int = 2
REG_OPTION_BACKUP_RESTORE: int = 4
REG_OPTION_CREATE_LINK: int = 2
REG_OPTION_NON_VOLATILE: int = 0
REG_OPTION_OPEN_LINK: int = 8
REG_OPTION_RESERVED: int = 0
REG_OPTION_VOLATILE: int = 1
REG_REFRESH_HIVE: int = 2
REG_WHOLE_HIVE_VOLATILE: int = 1
error = OSError

@final
class HKEYType:

    def __bool__(self) -> bool:
        ...

    def __int__(self) -> int:
        ...

    def __enter__(self) -> Self:
        ...

    def __exit__(self, exc_type: type[BaseException] | None, exc_value: BaseException | None, traceback: TracebackType | None, /) -> bool | None:
        ...

    def Close(self) -> None:
        ...

    def Detach(self) -> int:
        ...

    def __hash__(self) -> int:
        ...

    @property
    def handle(self) -> int:
        ...
