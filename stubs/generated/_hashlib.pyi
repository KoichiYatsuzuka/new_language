# _hashlib.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_hashlib.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from collections.abc import Callable
from types import ModuleType
from typing import AnyStr, Protocol, TypeAlias, final, overload, type_check_only
from typing_extensions import Self, disjoint_base
openssl_md_meth_names: frozenset[str]

@type_check_only
class _HashObject(Protocol):

    @property
    def digest_size(self) -> int:
        ...

    @property
    def block_size(self) -> int:
        ...

    @property
    def name(self) -> str:
        ...

    def copy(self) -> Self:
        ...

    def digest(self) -> bytes:
        ...

    def hexdigest(self) -> str:
        ...

    def update(self, obj: ReadableBuffer, /) -> None:
        ...

@disjoint_base
class HASH:

    @property
    def digest_size(self) -> int:
        ...

    @property
    def block_size(self) -> int:
        ...

    @property
    def name(self) -> str:
        ...

    def copy(self) -> Self:
        ...

    def digest(self) -> bytes:
        ...

    def hexdigest(self) -> str:
        ...

    def update(self, obj: ReadableBuffer, /) -> None:
        ...

class UnsupportedDigestmodError(ValueError):
    ...

class HASHXOF(HASH):

    def digest(self, length: int) -> bytes:
        ...

    def hexdigest(self, length: int) -> str:
        ...

@final
class HMAC:

    @property
    def digest_size(self) -> int:
        ...

    @property
    def block_size(self) -> int:
        ...

    @property
    def name(self) -> str:
        ...

    def copy(self) -> Self:
        ...

    def digest(self) -> bytes:
        ...

    def hexdigest(self) -> str:
        ...

    def update(self, msg: ReadableBuffer) -> None:
        ...

@overload
def compare_digest(a: ReadableBuffer, b: ReadableBuffer, /) -> bool:
    ...

@overload
def compare_digest(a: AnyStr, b: AnyStr, /) -> bool:
    ...

def get_fips_mode() -> int:
    ...

def hmac_new(key: ReadableBuffer, msg: ReadableBuffer=b'', digestmod: str | Callable[[], _HashObject] | ModuleType | None=None) -> HMAC:
    ...

def new(name: str, string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_md5(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha1(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha224(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha256(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha384(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha512(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha3_224(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha3_256(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha3_384(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_sha3_512(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASH:
    ...

def openssl_shake_128(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASHXOF:
    ...

def openssl_shake_256(string: ReadableBuffer=b'', *, usedforsecurity: bool=True) -> HASHXOF:
    ...

def hmac_digest(key: ReadableBuffer, msg: ReadableBuffer, digest: str) -> bytes:
    ...

def pbkdf2_hmac(hash_name: str, password: ReadableBuffer, salt: ReadableBuffer, iterations: int, dklen: int | None=None) -> bytes:
    ...

def scrypt(password: ReadableBuffer, *, salt: ReadableBuffer, n: int, r: int, p: int, maxmem: int=0, dklen: int=64) -> bytes:
    ...
