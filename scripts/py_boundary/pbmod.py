"""Helpers for the Python <-> Arrow boundary measurement (pandas plan 4-1)."""
import time
from typing import Any


def now() -> float:
    return time.perf_counter()


def ints(n: int) -> list[int]:
    return list(range(n))


def floats(n: int) -> list[float]:
    return [i * 0.5 for i in range(n)]


def strs(n: int) -> list[str]:
    return [str(i) for i in range(n)]


def bools(n: int) -> list[bool]:
    return [i % 2 == 0 for i in range(n)]


def anys(n: int) -> list[Any]:
    return list(range(n))


def pairs(n: int) -> list[tuple[int, float]]:
    return [(i, i * 0.5) for i in range(n)]


def strdict(n: int) -> dict[str, int]:
    return {str(i): i for i in range(n)}


def nested(n: int) -> list[list[int]]:
    return [[i, i + 1, i + 2] for i in range(n)]


def identity(x: Any) -> Any:
    return x


def one_int() -> int:
    return 1


def one_float() -> float:
    return 1.5


def one_str() -> str:
    return "abc"


def one_pair() -> tuple[int, float]:
    return (1, 1.5)


def noop() -> None:
    return None


def py_loop(xs: list[int]) -> int:
    s = 0
    for x in xs:
        s += x
    return s


def py_copy(xs: list[int]) -> list[int]:
    return [x for x in xs]


def tolist(a: Any) -> list[float]:
    return a.tolist()
