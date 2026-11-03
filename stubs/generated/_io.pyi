# _io.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_io.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import builtins
import codecs
import sys
from _typeshed import FileDescriptorOrPath, MaybeNone, ReadableBuffer, WriteableBuffer
from collections.abc import Callable, Iterable, Iterator
from io import BufferedIOBase, RawIOBase, TextIOBase, UnsupportedOperation as UnsupportedOperation
from os import _Opener
from types import TracebackType
from typing import IO, Any, BinaryIO, Final, Generic, Literal, Protocol, TextIO, TypeVar, overload, type_check_only
from typing_extensions import Self, disjoint_base
DEFAULT_BUFFER_SIZE: int = 8192
open = builtins.open

def open_code(path: str) -> IO[bytes]:
    ...
BlockingIOError = builtins.BlockingIOError

@disjoint_base
class _IOBase:

    def __iter__(self) -> Iterator[bytes]:
        ...

    def __next__(self) -> bytes:
        ...

    def __enter__(self) -> Self:
        ...

    def __exit__(self, exc_type: type[BaseException] | None, exc_val: BaseException | None, exc_tb: TracebackType | None) -> None:
        ...

    def close(self) -> None:
        ...

    def fileno(self) -> int:
        ...

    def flush(self) -> None:
        ...

    def isatty(self) -> bool:
        ...

    def readable(self) -> bool:
        ...
    read: Callable[..., Any]

    def readlines(self, hint: int=-1, /) -> list[bytes]:
        ...

    def seek(self, offset: int, whence: int=0, /) -> int:
        ...

    def seekable(self) -> bool:
        ...

    def tell(self) -> int:
        ...

    def truncate(self, size: int | None=None, /) -> int:
        ...

    def writable(self) -> bool:
        ...
    write: Callable[..., Any]

    def writelines(self, lines: Iterable[ReadableBuffer], /) -> None:
        ...

    def readline(self, size: int | None=-1, /) -> bytes:
        ...

    def __del__(self) -> None:
        ...

    @property
    def closed(self) -> bool:
        ...

    def _checkClosed(self) -> None:
        ...

class _RawIOBase(_IOBase):

    def readall(self) -> bytes:
        ...

    def readinto(self, buffer: WriteableBuffer, /) -> int | MaybeNone:
        ...

    def write(self, b: ReadableBuffer, /) -> int | MaybeNone:
        ...

    def read(self, size: int=-1, /) -> bytes | MaybeNone:
        ...

class _BufferedIOBase(_IOBase):

    def detach(self) -> RawIOBase:
        ...

    def readinto(self, buffer: WriteableBuffer, /) -> int:
        ...

    def write(self, buffer: ReadableBuffer, /) -> int:
        ...

    def readinto1(self, buffer: WriteableBuffer, /) -> int:
        ...

    def read(self, size: int | None=-1, /) -> bytes:
        ...

    def read1(self, size: int=-1, /) -> bytes:
        ...

@disjoint_base
class FileIO(RawIOBase, _RawIOBase, BinaryIO):
    mode: str
    name: Any

    def __init__(self, file: FileDescriptorOrPath, mode: str='r', closefd: bool=True, opener: _Opener | None=None) -> None:
        ...

    @property
    def closefd(self) -> bool:
        ...

    def seek(self, pos: int, whence: int=0, /) -> int:
        ...

    def read(self, size: int | None=-1, /) -> bytes | MaybeNone:
        ...

@disjoint_base
class BytesIO(BufferedIOBase, _BufferedIOBase, BinaryIO):

    def __init__(self, initial_bytes: ReadableBuffer=b'') -> None:
        ...
    name: Any

    def getvalue(self) -> bytes:
        ...

    def getbuffer(self) -> memoryview:
        ...

    def read1(self, size: int | None=-1, /) -> bytes:
        ...

    def readlines(self, size: int | None=None, /) -> list[bytes]:
        ...

    def seek(self, pos: int, whence: int=0, /) -> int:
        ...

@type_check_only
class _BufferedReaderStream(Protocol):

    def read(self, n: int=..., /) -> bytes:
        ...

    def readinto(self, b: memoryview, /) -> int | None:
        ...

    def seek(self, pos: int, whence: int, /) -> int:
        ...

    def tell(self) -> int:
        ...

    def truncate(self, size: int, /) -> int:
        ...

    def flush(self) -> object:
        ...

    def close(self) -> object:
        ...

    @property
    def closed(self) -> bool:
        ...

    def readable(self) -> bool:
        ...

    def seekable(self) -> bool:
        ...

@disjoint_base
class BufferedReader(BufferedIOBase, _BufferedIOBase, BinaryIO, Generic[_BufferedReaderStreamT]):
    raw: Any

    def __init__(self, raw: Any, buffer_size: int=8192) -> None:
        ...

    def peek(self, size: int=0, /) -> bytes:
        ...

    def seek(self, target: int, whence: int=0, /) -> int:
        ...

    def truncate(self, pos: int | None=None, /) -> int:
        ...

@type_check_only
class _BufferedWriterStream(Protocol):

    def write(self, b: WriteableBuffer, /) -> int | None:
        ...

    def seek(self, pos: int, whence: int, /) -> int:
        ...

    def tell(self) -> int:
        ...

    def truncate(self, size: int, /) -> int:
        ...

    def flush(self) -> object:
        ...

    def close(self) -> object:
        ...

    @property
    def closed(self) -> bool:
        ...

    def writable(self) -> bool:
        ...

    def seekable(self) -> bool:
        ...

@disjoint_base
class BufferedWriter(BufferedIOBase, _BufferedIOBase, BinaryIO, Generic[_BufferedWriterStreamT]):
    raw: Any

    def __init__(self, raw: Any, buffer_size: int=8192) -> None:
        ...

    def write(self, buffer: ReadableBuffer, /) -> int:
        ...

    def seek(self, target: int, whence: int=0, /) -> int:
        ...

    def truncate(self, pos: int | None=None, /) -> int:
        ...

@disjoint_base
class BufferedRandom(BufferedIOBase, _BufferedIOBase, BinaryIO):
    mode: str
    name: Any
    raw: RawIOBase

    def __init__(self, raw: RawIOBase, buffer_size: int=8192) -> None:
        ...

    def seek(self, target: int, whence: int=0, /) -> int:
        ...

    def peek(self, size: int=0, /) -> bytes:
        ...

    def truncate(self, pos: int | None=None, /) -> int:
        ...

@disjoint_base
class BufferedRWPair(BufferedIOBase, _BufferedIOBase, Generic[_BufferedReaderStreamT, _BufferedWriterStreamT]):

    def __init__(self, reader: Any, writer: Any, buffer_size: int=8192, /) -> None:
        ...

    def peek(self, size: int=0, /) -> bytes:
        ...

class _TextIOBase(_IOBase):
    encoding: str
    errors: str | None
    newlines: str | tuple[str, ...] | None

    def __iter__(self) -> Iterator[str]:
        ...

    def __next__(self) -> str:
        ...

    def detach(self) -> BinaryIO:
        ...

    def write(self, s: str, /) -> int:
        ...

    def writelines(self, lines: Iterable[str], /) -> None:
        ...

    def readline(self, size: int=-1, /) -> str:
        ...

    def readlines(self, hint: int=-1, /) -> list[str]:
        ...

    def read(self, size: int | None=-1, /) -> str:
        ...

@type_check_only
class _WrappedBuffer(Protocol):

    @property
    def name(self) -> Any:
        ...

    @property
    def closed(self) -> bool:
        ...

    def read(self, size: int=..., /) -> ReadableBuffer:
        ...

    def write(self, b: bytes, /) -> object:
        ...

    def flush(self) -> object:
        ...

    def close(self) -> object:
        ...

    def seekable(self) -> bool:
        ...

    def readable(self) -> bool:
        ...

    def writable(self) -> bool:
        ...

    def truncate(self, size: int, /) -> int:
        ...

    def fileno(self) -> int:
        ...

    def isatty(self) -> bool:
        ...

@disjoint_base
class TextIOWrapper(TextIOBase, _TextIOBase, TextIO, Generic[_BufferT_co]):

    def __init__(self, buffer: Any, encoding: str | None=None, errors: str | None=None, newline: str | None=None, line_buffering: bool=False, write_through: bool=False) -> None:
        ...

    @property
    def buffer(self) -> Any:
        ...

    @property
    def line_buffering(self) -> bool:
        ...

    @property
    def write_through(self) -> bool:
        ...

    def reconfigure(self, *, encoding: str | None=None, errors: str | None=None, newline: str | None=None, line_buffering: bool | None=None, write_through: bool | None=None) -> None:
        ...

    def readline(self, size: int=-1, /) -> str:
        ...

    def detach(self) -> Any:
        ...

    def seek(self, cookie: int, whence: int=0, /) -> int:
        ...

    def truncate(self, pos: int | None=None, /) -> int:
        ...

@disjoint_base
class StringIO(TextIOBase, _TextIOBase, TextIO):

    def __init__(self, initial_value: str | None='', newline: str | None='\n') -> None:
        ...
    name: Any

    def getvalue(self) -> str:
        ...

    @property
    def line_buffering(self) -> bool:
        ...

    def seek(self, pos: int, whence: int=0, /) -> int:
        ...

    def truncate(self, pos: int | None=None, /) -> int:
        ...

@disjoint_base
class IncrementalNewlineDecoder:

    def __init__(self, decoder: codecs.IncrementalDecoder | None, translate: bool, errors: str='strict') -> None:
        ...

    def decode(self, input: ReadableBuffer | str, final: bool=False) -> str:
        ...

    @property
    def newlines(self) -> str | tuple[str, ...] | None:
        ...

    def getstate(self) -> tuple[bytes, int]:
        ...

    def reset(self) -> None:
        ...

    def setstate(self, state: tuple[bytes, int], /) -> None:
        ...

@overload
def text_encoding(encoding: None, stacklevel: int=2, /) -> Literal['locale', 'utf-8']:
    ...

@overload
def text_encoding(encoding: Any, stacklevel: int=2, /) -> Any:
    ...
