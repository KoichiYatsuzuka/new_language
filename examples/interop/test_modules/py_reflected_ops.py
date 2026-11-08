# py_reflected_ops.py — Arrow の値が左・Python の物が右の演算子（examples/interop/py_reflected_ops.ar）


class Money:
    """右に来ても足し算・引き算・大小比較ができる。"""

    def __init__(self, n):
        self.n = n

    def __radd__(self, other):
        return Money(other + self.n)

    def __rsub__(self, other):
        return Money(other - self.n)

    def __gt__(self, other):
        return self.n > other

    def __lt__(self, other):
        return self.n < other

    def __eq__(self, other):
        return isinstance(other, Money) and self.n == other.n

    def __hash__(self):
        return hash(self.n)


class Declines:
    """`__radd__` が `NotImplemented` を返す（相手の型を扱わない）。"""

    def __radd__(self, other):
        return NotImplemented


class Plain:
    """演算子を 1 つも持たない。"""


class Bag:
    """`__contains__` が無く、`__iter__` だけを持つ（`in` は要素を回して探す）。"""

    def __init__(self, items):
        self.items = items

    def __iter__(self):
        return iter(self.items)


def money(n: int) -> Money:
    return Money(n)


def declines() -> Declines:
    return Declines()


def plain() -> Plain:
    return Plain()


def bag() -> Bag:
    return Bag([1, 2, 3])
