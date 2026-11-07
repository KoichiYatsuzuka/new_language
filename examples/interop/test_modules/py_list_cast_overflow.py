"""A list whose elements are not known statically, holding an int that does not fit in 64 bits."""
from typing import Any


def bigs() -> list[Any]:
    return [1, 2 ** 70]
