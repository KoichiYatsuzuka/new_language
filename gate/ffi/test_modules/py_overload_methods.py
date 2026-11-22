"""`@overload` on methods: the type checker picks the first variant the call fits."""
from typing import Literal, overload


class Store:
    def __init__(self) -> None:
        self.items = {"a": 1}

    @overload
    def get(self, key: str) -> int: ...

    @overload
    def get(self, key: str, default: str) -> str: ...

    def get(self, key, default=None):
        if default is None:
            return self.items[key]
        return default

    @overload
    def drop(self, key: str, inplace: Literal[False] = ...) -> "Store": ...

    @overload
    def drop(self, key: str, inplace: Literal[True]) -> None: ...

    def drop(self, key, inplace=False):
        target = self if inplace else Store()
        target.items.pop(key, None)
        return None if inplace else target

    def size(self) -> int:
        return len(self.items)


def spread(*values: int, scale: float = 1.0) -> float:
    return sum(values) * scale
