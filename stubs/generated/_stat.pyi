# _stat.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_stat.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from typing import Final
SF_APPEND: int = 262144
SF_ARCHIVED: int = 65536
SF_IMMUTABLE: int = 131072
SF_NOUNLINK: int = 1048576
SF_SNAPSHOT: int = 2097152
ST_MODE: int = 0
ST_INO: int = 1
ST_DEV: int = 2
ST_NLINK: int = 3
ST_UID: int = 4
ST_GID: int = 5
ST_SIZE: int = 6
ST_ATIME: int = 7
ST_MTIME: int = 8
ST_CTIME: int = 9
S_IFIFO: int = 4096
S_IFLNK: int = 40960
S_IFREG: int = 32768
S_IFSOCK: int = 49152
S_IFBLK: int = 24576
S_IFCHR: int = 8192
S_IFDIR: int = 16384
S_IFDOOR: Final[int]
S_IFPORT: Final[int]
S_IFWHT: Final[int]
S_ISUID: int = 2048
S_ISGID: int = 1024
S_ISVTX: int = 512
S_IRWXU: int = 448
S_IRUSR: int = 256
S_IWUSR: int = 128
S_IXUSR: int = 64
S_IRWXG: int = 56
S_IRGRP: int = 32
S_IWGRP: int = 16
S_IXGRP: int = 8
S_IRWXO: int = 7
S_IROTH: int = 4
S_IWOTH: int = 2
S_IXOTH: int = 1
S_ENFMT: int = 1024
S_IREAD: int = 256
S_IWRITE: int = 128
S_IEXEC: int = 64
UF_APPEND: int = 4
UF_COMPRESSED: int = 32
UF_HIDDEN: int = 32768
UF_IMMUTABLE: int = 2
UF_NODUMP: int = 1
UF_NOUNLINK: int = 16
UF_OPAQUE: int = 8

def S_IMODE(mode: int, /) -> int:
    ...

def S_IFMT(mode: int, /) -> int:
    ...

def S_ISBLK(mode: int, /) -> bool:
    ...

def S_ISCHR(mode: int, /) -> bool:
    ...

def S_ISDIR(mode: int, /) -> bool:
    ...

def S_ISDOOR(mode: int, /) -> bool:
    ...

def S_ISFIFO(mode: int, /) -> bool:
    ...

def S_ISLNK(mode: int, /) -> bool:
    ...

def S_ISPORT(mode: int, /) -> bool:
    ...

def S_ISREG(mode: int, /) -> bool:
    ...

def S_ISSOCK(mode: int, /) -> bool:
    ...

def S_ISWHT(mode: int, /) -> bool:
    ...

def filemode(mode: int, /) -> str:
    ...
IO_REPARSE_TAG_SYMLINK: int = 2684354572
IO_REPARSE_TAG_MOUNT_POINT: int = 2684354563
IO_REPARSE_TAG_APPEXECLINK: int = 2147483675
FILE_ATTRIBUTE_ARCHIVE: int = 32
FILE_ATTRIBUTE_COMPRESSED: int = 2048
FILE_ATTRIBUTE_DEVICE: int = 64
FILE_ATTRIBUTE_DIRECTORY: int = 16
FILE_ATTRIBUTE_ENCRYPTED: int = 16384
FILE_ATTRIBUTE_HIDDEN: int = 2
FILE_ATTRIBUTE_INTEGRITY_STREAM: int = 32768
FILE_ATTRIBUTE_NORMAL: int = 128
FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: int = 8192
FILE_ATTRIBUTE_NO_SCRUB_DATA: int = 131072
FILE_ATTRIBUTE_OFFLINE: int = 4096
FILE_ATTRIBUTE_READONLY: int = 1
FILE_ATTRIBUTE_REPARSE_POINT: int = 1024
FILE_ATTRIBUTE_SPARSE_FILE: int = 512
FILE_ATTRIBUTE_SYSTEM: int = 4
FILE_ATTRIBUTE_TEMPORARY: int = 256
FILE_ATTRIBUTE_VIRTUAL: int = 65536
