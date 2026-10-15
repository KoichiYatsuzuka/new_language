"""py_int_probe.py — import[py-int] の値の受け渡しを確かめる（implementation_plans/ffi_value_exchange_plan.md 2-6）。"""
import array
from decimal import Decimal


def make_set() -> set[int]:
    return {1, 2, 3}


def make_frozenset() -> frozenset[str]:
    return frozenset({"a", "b"})


def set_of_tuples() -> set[tuple[int, int]]:
    return {(1, 2), (3, 4)}


def kind(x) -> str:
    return type(x).__name__


def size(x) -> int:
    return len(x)


def contains(s, x) -> bool:
    return x in s


def big():
    return 2 ** 70


def big_declared() -> int:      # declared int, but does not fit in Arrow's int (plan 7-9)
    return 2 ** 70


def big_in_list() -> list[int]:
    return [1, 2 ** 70]


def pairs() -> set[tuple[int, int]]:           # element types are kept in the stub (plan 7-8)
    return {(1, 2), (3, 4)}


def frozen() -> frozenset[str]:
    return frozenset({"a"})


def table() -> dict[str, list[int]]:
    return {"a": [1, 2]}


def lying_pairs() -> set[tuple[int, int]]:     # breaks its own annotation: the boundary sees the elements now
    return {(1, "x")}


def lying_table() -> dict[str, int]:
    return {"a": "x"}


def fail(why: str) -> int:                     # raises in Python: Arrow sees a ForeignError (plan 7-24)
    raise ValueError(why)


class Raiser:
    def boom(self) -> int:
        raise KeyError("k")


def make_raiser() -> Raiser:
    return Raiser()


def big_text(x) -> str:
    return str(x + 1)


def dec():
    return Decimal("0.1")


def dec_text(d) -> str:
    return str(d * 3)


# bytes (task 5-3)
def make_bytes() -> bytes:
    return b"hi\x00\xff"


def make_bytearray() -> bytearray:
    return bytearray(b"ab")


def make_view() -> memoryview:
    return memoryview(b"xyz")


def int_view():
    return memoryview(array.array("i", [1, 2]))


def decode_utf8(b) -> str:
    return b.decode("utf-8")
