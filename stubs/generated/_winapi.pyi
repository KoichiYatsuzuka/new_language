# _winapi.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_winapi.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from collections.abc import Sequence
from typing import Any, Final, Literal, NoReturn, final, overload
ABOVE_NORMAL_PRIORITY_CLASS: int = 32768
BELOW_NORMAL_PRIORITY_CLASS: int = 16384
CREATE_BREAKAWAY_FROM_JOB: int = 16777216
CREATE_DEFAULT_ERROR_MODE: int = 67108864
CREATE_NO_WINDOW: int = 134217728
CREATE_NEW_CONSOLE: int = 16
CREATE_NEW_PROCESS_GROUP: int = 512
DETACHED_PROCESS: int = 8
DUPLICATE_CLOSE_SOURCE: int = 1
DUPLICATE_SAME_ACCESS: int = 2
ERROR_ALREADY_EXISTS: int = 183
ERROR_BROKEN_PIPE: int = 109
ERROR_IO_PENDING: int = 997
ERROR_MORE_DATA: int = 234
ERROR_NETNAME_DELETED: int = 64
ERROR_NO_DATA: int = 232
ERROR_NO_SYSTEM_RESOURCES: int = 1450
ERROR_OPERATION_ABORTED: int = 995
ERROR_PIPE_BUSY: int = 231
ERROR_PIPE_CONNECTED: int = 535
ERROR_SEM_TIMEOUT: int = 121
FILE_FLAG_FIRST_PIPE_INSTANCE: int = 524288
FILE_FLAG_OVERLAPPED: int = 1073741824
FILE_GENERIC_READ: int = 1179785
FILE_GENERIC_WRITE: int = 1179926
FILE_MAP_ALL_ACCESS: int = 983071
FILE_MAP_COPY: int = 1
FILE_MAP_EXECUTE: int = 32
FILE_MAP_READ: int = 4
FILE_MAP_WRITE: int = 2
FILE_TYPE_CHAR: int = 2
FILE_TYPE_DISK: int = 1
FILE_TYPE_PIPE: int = 3
FILE_TYPE_REMOTE: int = 32768
FILE_TYPE_UNKNOWN: int = 0
GENERIC_READ: int = 2147483648
GENERIC_WRITE: int = 1073741824
HIGH_PRIORITY_CLASS: int = 128
INFINITE: int = 4294967295
INVALID_HANDLE_VALUE: int = 18446744073709551615
IDLE_PRIORITY_CLASS: int = 64
NORMAL_PRIORITY_CLASS: int = 32
REALTIME_PRIORITY_CLASS: int = 256
NMPWAIT_WAIT_FOREVER: int = 4294967295
MEM_COMMIT: int = 4096
MEM_FREE: int = 65536
MEM_IMAGE: int = 16777216
MEM_MAPPED: int = 262144
MEM_PRIVATE: int = 131072
MEM_RESERVE: int = 8192
NULL: int = 0
OPEN_EXISTING: int = 3
PIPE_ACCESS_DUPLEX: int = 3
PIPE_ACCESS_INBOUND: int = 1
PIPE_READMODE_MESSAGE: int = 2
PIPE_TYPE_MESSAGE: int = 4
PIPE_UNLIMITED_INSTANCES: int = 255
PIPE_WAIT: int = 0
PAGE_EXECUTE: int = 16
PAGE_EXECUTE_READ: int = 32
PAGE_EXECUTE_READWRITE: int = 64
PAGE_EXECUTE_WRITECOPY: int = 128
PAGE_GUARD: int = 256
PAGE_NOACCESS: int = 1
PAGE_NOCACHE: int = 512
PAGE_READONLY: int = 2
PAGE_READWRITE: int = 4
PAGE_WRITECOMBINE: int = 1024
PAGE_WRITECOPY: int = 8
PROCESS_ALL_ACCESS: int = 2097151
PROCESS_DUP_HANDLE: int = 64
SEC_COMMIT: int = 134217728
SEC_IMAGE: int = 16777216
SEC_LARGE_PAGES: int = 2147483648
SEC_NOCACHE: int = 268435456
SEC_RESERVE: int = 67108864
SEC_WRITECOMBINE: int = 1073741824
STARTF_USESHOWWINDOW: int = 1
STARTF_USESTDHANDLES: int = 256
STD_ERROR_HANDLE: int = 4294967284
STD_OUTPUT_HANDLE: int = 4294967285
STD_INPUT_HANDLE: int = 4294967286
STILL_ACTIVE: int = 259
SW_HIDE: int = 0
SYNCHRONIZE: int = 1048576
WAIT_ABANDONED_0: int = 128
WAIT_OBJECT_0: int = 0
WAIT_TIMEOUT: int = 258
LOCALE_NAME_INVARIANT: Final[str]
LOCALE_NAME_MAX_LENGTH: Final[int]
LOCALE_NAME_SYSTEM_DEFAULT: Final[str]
LOCALE_NAME_USER_DEFAULT: Final[str | None]
LCMAP_FULLWIDTH: Final[int]
LCMAP_HALFWIDTH: Final[int]
LCMAP_HIRAGANA: Final[int]
LCMAP_KATAKANA: Final[int]
LCMAP_LINGUISTIC_CASING: Final[int]
LCMAP_LOWERCASE: Final[int]
LCMAP_SIMPLIFIED_CHINESE: Final[int]
LCMAP_TITLECASE: Final[int]
LCMAP_TRADITIONAL_CHINESE: Final[int]
LCMAP_UPPERCASE: Final[int]
COPYFILE2_CALLBACK_CHUNK_STARTED: int = 1
COPYFILE2_CALLBACK_CHUNK_FINISHED: int = 2
COPYFILE2_CALLBACK_STREAM_STARTED: int = 3
COPYFILE2_CALLBACK_STREAM_FINISHED: int = 4
COPYFILE2_CALLBACK_POLL_CONTINUE: int = 5
COPYFILE2_CALLBACK_ERROR: int = 6
COPYFILE2_PROGRESS_CONTINUE: int = 0
COPYFILE2_PROGRESS_CANCEL: int = 1
COPYFILE2_PROGRESS_STOP: int = 2
COPYFILE2_PROGRESS_QUIET: int = 3
COPYFILE2_PROGRESS_PAUSE: int = 4
COPY_FILE_FAIL_IF_EXISTS: int = 1
COPY_FILE_RESTARTABLE: int = 2
COPY_FILE_OPEN_SOURCE_FOR_WRITE: int = 4
COPY_FILE_ALLOW_DECRYPTED_DESTINATION: int = 8
COPY_FILE_COPY_SYMLINK: int = 2048
COPY_FILE_NO_BUFFERING: int = 4096
COPY_FILE_REQUEST_SECURITY_PRIVILEGES: int = 8192
COPY_FILE_RESUME_FROM_PAUSE: int = 16384
COPY_FILE_NO_OFFLOAD: int = 262144
COPY_FILE_REQUEST_COMPRESSED_TRAFFIC: int = 268435456
ERROR_ACCESS_DENIED: int = 5
ERROR_PRIVILEGE_NOT_HELD: int = 1314

def CloseHandle(handle: int, /) -> None:
    ...

@overload
def ConnectNamedPipe(handle: int, overlapped: Literal[True]) -> Overlapped:
    ...

@overload
def ConnectNamedPipe(handle: int, overlapped: Literal[False]=False) -> None:
    ...

@overload
def ConnectNamedPipe(handle: int, overlapped: bool) -> Overlapped | None:
    ...

def CreateFile(file_name: str, desired_access: int, share_mode: int, security_attributes: int, creation_disposition: int, flags_and_attributes: int, template_file: int, /) -> int:
    ...

def CreateFileMapping(file_handle: int, security_attributes: int, protect: int, max_size_high: int, max_size_low: int, name: str, /) -> int:
    ...

def CreateJunction(src_path: str, dst_path: str, /) -> None:
    ...

def CreateNamedPipe(name: str, open_mode: int, pipe_mode: int, max_instances: int, out_buffer_size: int, in_buffer_size: int, default_timeout: int, security_attributes: int, /) -> int:
    ...

def CreatePipe(pipe_attrs: Any, size: int, /) -> tuple[int, int]:
    ...

def CreateProcess(application_name: str | None, command_line: str | None, proc_attrs: Any, thread_attrs: Any, inherit_handles: bool, creation_flags: int, env_mapping: dict[str, str], current_directory: str | None, startup_info: Any, /) -> tuple[int, int, int, int]:
    ...

def DuplicateHandle(source_process_handle: int, source_handle: int, target_process_handle: int, desired_access: int, inherit_handle: bool, options: int=0, /) -> int:
    ...

def ExitProcess(ExitCode: int, /) -> NoReturn:
    ...

def GetACP() -> int:
    ...

def GetFileType(handle: int) -> int:
    ...

def GetCurrentProcess() -> int:
    ...

def GetExitCodeProcess(process: int, /) -> int:
    ...

def GetLastError() -> int:
    ...

def GetModuleFileName(module_handle: int, /) -> str:
    ...

def GetStdHandle(std_handle: int, /) -> int:
    ...

def GetVersion() -> int:
    ...

def MapViewOfFile(file_map: int, desired_access: int, file_offset_high: int, file_offset_low: int, number_bytes: int, /) -> int:
    ...

def OpenProcess(desired_access: int, inherit_handle: bool, process_id: int, /) -> int:
    ...

def PeekNamedPipe(handle: int, size: int=0, /) -> tuple[int, int] | tuple[bytes, int, int]:
    ...

def LCMapStringEx(locale: str, flags: int, src: str) -> str:
    ...

def UnmapViewOfFile(address: int, /) -> None:
    ...

@overload
def ReadFile(handle: int, size: int, overlapped: Literal[True]) -> tuple[Overlapped, int]:
    ...

@overload
def ReadFile(handle: int, size: int, overlapped: Literal[False]=False) -> tuple[bytes, int]:
    ...

@overload
def ReadFile(handle: int, size: int, overlapped: int | bool) -> tuple[Any, int]:
    ...

def SetNamedPipeHandleState(named_pipe: int, mode: int | None, max_collection_count: int | None, collect_data_timeout: int | None, /) -> None:
    ...

def TerminateProcess(handle: int, exit_code: int, /) -> None:
    ...

def VirtualQuerySize(address: int, /) -> int:
    ...

def WaitForMultipleObjects(handle_seq: Sequence[int], wait_flag: bool, milliseconds: int=4294967295, /) -> int:
    ...

def WaitForSingleObject(handle: int, milliseconds: int, /) -> int:
    ...

def WaitNamedPipe(name: str, timeout: int, /) -> None:
    ...

@overload
def WriteFile(handle: int, buffer: ReadableBuffer, overlapped: Literal[True]) -> tuple[Overlapped, int]:
    ...

@overload
def WriteFile(handle: int, buffer: ReadableBuffer, overlapped: Literal[False]=False) -> tuple[int, int]:
    ...

@overload
def WriteFile(handle: int, buffer: ReadableBuffer, overlapped: int | bool) -> tuple[Any, int]:
    ...

@final
class Overlapped:
    event: int

    def GetOverlappedResult(self, wait: bool, /) -> tuple[int, int]:
        ...

    def cancel(self) -> None:
        ...

    def getbuffer(self) -> bytes | None:
        ...

def OpenFileMapping(desired_access: int, inherit_handle: bool, name: str, /) -> int:
    ...

def CopyFile2(existing_file_name: str, new_file_name: str, flags: int, progress_routine: int | None=None) -> int:
    ...

def NeedCurrentDirectoryForExePath(exe_name: str, /) -> bool:
    ...
