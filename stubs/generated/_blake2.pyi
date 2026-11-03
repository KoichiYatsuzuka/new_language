# _blake2.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_blake2.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from typing import ClassVar, Final, final
from typing_extensions import Self
BLAKE2B_MAX_DIGEST_SIZE: int = 64
BLAKE2B_MAX_KEY_SIZE: int = 64
BLAKE2B_PERSON_SIZE: int = 16
BLAKE2B_SALT_SIZE: int = 16
BLAKE2S_MAX_DIGEST_SIZE: int = 32
BLAKE2S_MAX_KEY_SIZE: int = 32
BLAKE2S_PERSON_SIZE: int = 8
BLAKE2S_SALT_SIZE: int = 8

@final
class blake2b:
    MAX_DIGEST_SIZE: ClassVar[int] = 64
    MAX_KEY_SIZE: ClassVar[int] = 64
    PERSON_SIZE: ClassVar[int] = 16
    SALT_SIZE: ClassVar[int] = 16
    block_size: int
    digest_size: int
    name: str

    def __init__(self, data: ReadableBuffer=b'', /, *, digest_size: int=64, key: ReadableBuffer=b'', salt: ReadableBuffer=b'', person: ReadableBuffer=b'', fanout: int=1, depth: int=1, leaf_size: int=0, node_offset: int=0, node_depth: int=0, inner_size: int=0, last_node: bool=False, usedforsecurity: bool=True) -> None:
        ...

    def copy(self) -> Self:
        ...

    def digest(self) -> bytes:
        ...

    def hexdigest(self) -> str:
        ...

    def update(self, data: ReadableBuffer, /) -> None:
        ...

@final
class blake2s:
    MAX_DIGEST_SIZE: ClassVar[int] = 32
    MAX_KEY_SIZE: ClassVar[int] = 32
    PERSON_SIZE: ClassVar[int] = 8
    SALT_SIZE: ClassVar[int] = 8
    block_size: int
    digest_size: int
    name: str

    def __init__(self, data: ReadableBuffer=b'', /, *, digest_size: int=32, key: ReadableBuffer=b'', salt: ReadableBuffer=b'', person: ReadableBuffer=b'', fanout: int=1, depth: int=1, leaf_size: int=0, node_offset: int=0, node_depth: int=0, inner_size: int=0, last_node: bool=False, usedforsecurity: bool=True) -> None:
        ...

    def copy(self) -> Self:
        ...

    def digest(self) -> bytes:
        ...

    def hexdigest(self) -> str:
        ...

    def update(self, data: ReadableBuffer, /) -> None:
        ...
