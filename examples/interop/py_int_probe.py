"""py_int_probe.py — import[py-int] の値の受け渡しを確かめる（implementation_plans/ffi_value_exchange_plan.md 2-6）。"""
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


def big_text(x) -> str:
    return str(x + 1)


def dec():
    return Decimal("0.1")


def dec_text(d) -> str:
    return str(d * 3)
