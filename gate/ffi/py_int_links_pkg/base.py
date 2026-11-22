"""ほかのファイルの基底（pandas の `NDFrame` の役）。"""
from typing import Self


class Base:
    def head(self, n: int = 2) -> Self:
        return self

    def count(self) -> int:
        return 3
