# _blake2.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

BLAKE2B_MAX_DIGEST_SIZE: int
BLAKE2B_MAX_KEY_SIZE: int
BLAKE2B_PERSON_SIZE: int
BLAKE2B_SALT_SIZE: int
BLAKE2S_MAX_DIGEST_SIZE: int
BLAKE2S_MAX_KEY_SIZE: int
BLAKE2S_PERSON_SIZE: int
BLAKE2S_SALT_SIZE: int
class blake2b:
    def __init__(self, data: Any = ..., /, *, digest_size: Any = ..., key: Any = ..., salt: Any = ..., person: Any = ..., fanout: Any = ..., depth: Any = ..., leaf_size: Any = ..., node_offset: Any = ..., node_depth: Any = ..., inner_size: Any = ..., last_node: Any = ..., usedforsecurity: Any = ...) -> None: ...
    MAX_DIGEST_SIZE: int
    MAX_KEY_SIZE: int
    PERSON_SIZE: int
    SALT_SIZE: int
    block_size: Any
    def copy(self) -> Any: ...
    def digest(self) -> Any: ...
    digest_size: Any
    def hexdigest(self) -> Any: ...
    name: Any
    def update(self, data: Any, /) -> Any: ...
class blake2s:
    def __init__(self, data: Any = ..., /, *, digest_size: Any = ..., key: Any = ..., salt: Any = ..., person: Any = ..., fanout: Any = ..., depth: Any = ..., leaf_size: Any = ..., node_offset: Any = ..., node_depth: Any = ..., inner_size: Any = ..., last_node: Any = ..., usedforsecurity: Any = ...) -> None: ...
    MAX_DIGEST_SIZE: int
    MAX_KEY_SIZE: int
    PERSON_SIZE: int
    SALT_SIZE: int
    block_size: Any
    def copy(self) -> Any: ...
    def digest(self) -> Any: ...
    digest_size: Any
    def hexdigest(self) -> Any: ...
    name: Any
    def update(self, data: Any, /) -> Any: ...
