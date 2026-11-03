# _winapi.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

ABOVE_NORMAL_PRIORITY_CLASS: int
BELOW_NORMAL_PRIORITY_CLASS: int
COPYFILE2_CALLBACK_CHUNK_FINISHED: int
COPYFILE2_CALLBACK_CHUNK_STARTED: int
COPYFILE2_CALLBACK_ERROR: int
COPYFILE2_CALLBACK_POLL_CONTINUE: int
COPYFILE2_CALLBACK_STREAM_FINISHED: int
COPYFILE2_CALLBACK_STREAM_STARTED: int
COPYFILE2_PROGRESS_CANCEL: int
COPYFILE2_PROGRESS_CONTINUE: int
COPYFILE2_PROGRESS_PAUSE: int
COPYFILE2_PROGRESS_QUIET: int
COPYFILE2_PROGRESS_STOP: int
COPY_FILE_ALLOW_DECRYPTED_DESTINATION: int
COPY_FILE_COPY_SYMLINK: int
COPY_FILE_FAIL_IF_EXISTS: int
COPY_FILE_NO_BUFFERING: int
COPY_FILE_NO_OFFLOAD: int
COPY_FILE_OPEN_SOURCE_FOR_WRITE: int
COPY_FILE_REQUEST_COMPRESSED_TRAFFIC: int
COPY_FILE_REQUEST_SECURITY_PRIVILEGES: int
COPY_FILE_RESTARTABLE: int
COPY_FILE_RESUME_FROM_PAUSE: int
CREATE_BREAKAWAY_FROM_JOB: int
CREATE_DEFAULT_ERROR_MODE: int
CREATE_NEW_CONSOLE: int
CREATE_NEW_PROCESS_GROUP: int
CREATE_NO_WINDOW: int
def CloseHandle(handle: Any, /) -> Any: ...
def ConnectNamedPipe(handle: Any, overlapped: Any = ...) -> Any: ...
def CopyFile2(existing_file_name: Any, new_file_name: Any, flags: Any, progress_routine: Any = ...) -> Any: ...
def CreateFile(file_name: Any, desired_access: Any, share_mode: Any, security_attributes: Any, creation_disposition: Any, flags_and_attributes: Any, template_file: Any, /) -> Any: ...
def CreateFileMapping(file_handle: Any, security_attributes: Any, protect: Any, max_size_high: Any, max_size_low: Any, name: Any, /) -> Any: ...
def CreateJunction(src_path: Any, dst_path: Any, /) -> Any: ...
def CreateNamedPipe(name: Any, open_mode: Any, pipe_mode: Any, max_instances: Any, out_buffer_size: Any, in_buffer_size: Any, default_timeout: Any, security_attributes: Any, /) -> Any: ...
def CreatePipe(pipe_attrs: Any, size: Any, /) -> Any: ...
def CreateProcess(application_name: Any, command_line: Any, proc_attrs: Any, thread_attrs: Any, inherit_handles: Any, creation_flags: Any, env_mapping: Any, current_directory: Any, startup_info: Any, /) -> Any: ...
DETACHED_PROCESS: int
DUPLICATE_CLOSE_SOURCE: int
DUPLICATE_SAME_ACCESS: int
def DuplicateHandle(source_process_handle: Any, source_handle: Any, target_process_handle: Any, desired_access: Any, inherit_handle: Any, options: Any = ..., /) -> Any: ...
ERROR_ACCESS_DENIED: int
ERROR_ALREADY_EXISTS: int
ERROR_BROKEN_PIPE: int
ERROR_IO_PENDING: int
ERROR_MORE_DATA: int
ERROR_NETNAME_DELETED: int
ERROR_NO_DATA: int
ERROR_NO_SYSTEM_RESOURCES: int
ERROR_OPERATION_ABORTED: int
ERROR_PIPE_BUSY: int
ERROR_PIPE_CONNECTED: int
ERROR_PRIVILEGE_NOT_HELD: int
ERROR_SEM_TIMEOUT: int
def ExitProcess(ExitCode: Any, /) -> Any: ...
FILE_FLAG_FIRST_PIPE_INSTANCE: int
FILE_FLAG_OVERLAPPED: int
FILE_GENERIC_READ: int
FILE_GENERIC_WRITE: int
FILE_MAP_ALL_ACCESS: int
FILE_MAP_COPY: int
FILE_MAP_EXECUTE: int
FILE_MAP_READ: int
FILE_MAP_WRITE: int
FILE_TYPE_CHAR: int
FILE_TYPE_DISK: int
FILE_TYPE_PIPE: int
FILE_TYPE_REMOTE: int
FILE_TYPE_UNKNOWN: int
GENERIC_READ: int
GENERIC_WRITE: int
def GetACP() -> Any: ...
def GetCurrentProcess() -> Any: ...
def GetExitCodeProcess(process: Any, /) -> Any: ...
def GetFileType(handle: Any) -> Any: ...
def GetLastError() -> Any: ...
def GetModuleFileName(module_handle: Any, /) -> Any: ...
def GetStdHandle(std_handle: Any, /) -> Any: ...
def GetVersion() -> Any: ...
HIGH_PRIORITY_CLASS: int
IDLE_PRIORITY_CLASS: int
INFINITE: int
INVALID_HANDLE_VALUE: int
LCMAP_FULLWIDTH: int
LCMAP_HALFWIDTH: int
LCMAP_HIRAGANA: int
LCMAP_KATAKANA: int
LCMAP_LINGUISTIC_CASING: int
LCMAP_LOWERCASE: int
LCMAP_SIMPLIFIED_CHINESE: int
LCMAP_TITLECASE: int
LCMAP_TRADITIONAL_CHINESE: int
LCMAP_UPPERCASE: int
def LCMapStringEx(locale: Any, flags: Any, src: Any) -> Any: ...
LOCALE_NAME_INVARIANT: str
LOCALE_NAME_MAX_LENGTH: int
LOCALE_NAME_SYSTEM_DEFAULT: str
LOCALE_NAME_USER_DEFAULT: Any
MEM_COMMIT: int
MEM_FREE: int
MEM_IMAGE: int
MEM_MAPPED: int
MEM_PRIVATE: int
MEM_RESERVE: int
def MapViewOfFile(file_map: Any, desired_access: Any, file_offset_high: Any, file_offset_low: Any, number_bytes: Any, /) -> Any: ...
NMPWAIT_WAIT_FOREVER: int
NORMAL_PRIORITY_CLASS: int
NULL: int
def NeedCurrentDirectoryForExePath(exe_name: Any, /) -> Any: ...
OPEN_EXISTING: int
def OpenFileMapping(desired_access: Any, inherit_handle: Any, name: Any, /) -> Any: ...
def OpenProcess(desired_access: Any, inherit_handle: Any, process_id: Any, /) -> Any: ...
class Overlapped:
    def __init__(self) -> None: ...
    def GetOverlappedResult(self, wait: Any, /) -> Any: ...
    def cancel(self) -> Any: ...
    event: Any
    def getbuffer(self) -> Any: ...
PAGE_EXECUTE: int
PAGE_EXECUTE_READ: int
PAGE_EXECUTE_READWRITE: int
PAGE_EXECUTE_WRITECOPY: int
PAGE_GUARD: int
PAGE_NOACCESS: int
PAGE_NOCACHE: int
PAGE_READONLY: int
PAGE_READWRITE: int
PAGE_WRITECOMBINE: int
PAGE_WRITECOPY: int
PIPE_ACCESS_DUPLEX: int
PIPE_ACCESS_INBOUND: int
PIPE_READMODE_MESSAGE: int
PIPE_TYPE_MESSAGE: int
PIPE_UNLIMITED_INSTANCES: int
PIPE_WAIT: int
PROCESS_ALL_ACCESS: int
PROCESS_DUP_HANDLE: int
def PeekNamedPipe(handle: Any, size: Any = ..., /) -> Any: ...
REALTIME_PRIORITY_CLASS: int
def ReadFile(handle: Any, size: Any, overlapped: Any = ...) -> Any: ...
SEC_COMMIT: int
SEC_IMAGE: int
SEC_LARGE_PAGES: int
SEC_NOCACHE: int
SEC_RESERVE: int
SEC_WRITECOMBINE: int
STARTF_USESHOWWINDOW: int
STARTF_USESTDHANDLES: int
STD_ERROR_HANDLE: int
STD_INPUT_HANDLE: int
STD_OUTPUT_HANDLE: int
STILL_ACTIVE: int
SW_HIDE: int
SYNCHRONIZE: int
def SetNamedPipeHandleState(named_pipe: Any, mode: Any, max_collection_count: Any, collect_data_timeout: Any, /) -> Any: ...
def TerminateProcess(handle: Any, exit_code: Any, /) -> Any: ...
def UnmapViewOfFile(address: Any, /) -> Any: ...
def VirtualQuerySize(address: Any, /) -> Any: ...
WAIT_ABANDONED_0: int
WAIT_OBJECT_0: int
WAIT_TIMEOUT: int
def WaitForMultipleObjects(handle_seq: Any, wait_flag: Any, milliseconds: Any = ..., /) -> Any: ...
def WaitForSingleObject(handle: Any, milliseconds: Any, /) -> Any: ...
def WaitNamedPipe(name: Any, timeout: Any, /) -> Any: ...
def WriteFile(handle: Any, buffer: Any, overlapped: Any = ...) -> Any: ...
