"""クラスと @overload の関数（pandas の `DataFrame` / `read_csv` の役）。"""
from __future__ import annotations

from typing import TYPE_CHECKING, Literal, overload

from .base import Base

if TYPE_CHECKING:
    from .readers import Reader


class Frame(Base):
    def __init__(self, rows: int) -> None:
        self.rows = rows

    def width(self) -> int:
        return 2


@overload
def read(path: str, *, lazy: Literal[True]) -> Reader: ...
@overload
def read(path: str, *, lazy: Literal[False] = ...) -> Frame: ...
def read(path: str, *, lazy: bool = False) -> Frame | Reader:
    from .readers import Reader

    return Reader(path) if lazy else Frame(3)
