# py_int_truthiness.ar が import[py] で使うモジュール（CPython の物の真偽・pandas_import_plan.md の 2-2）。


class Box:
    """__len__ だけを持つ（真偽は要素の数で決まる）。"""

    def __init__(self, n):
        self.items = list(range(n))

    def __len__(self):
        return len(self.items)


class Flag:
    """__bool__ と __len__ の両方を持つ（CPython は __bool__ を先に使う）。"""

    def __init__(self, on, n):
        self.on = on
        self.n = n

    def __bool__(self):
        return self.on

    def __len__(self):
        return self.n


class Plain:
    """どちらも持たない（常に真）。"""


class Ambiguous:
    """__bool__ が投げる（numpy の配列の「真偽が曖昧」と同じ形）。"""

    def __bool__(self):
        raise ValueError("the truth value is ambiguous")


class WrongBool:
    """__bool__ が bool 以外を返す（CPython が TypeError にする）。"""

    def __bool__(self):
        return 1


class Vec:
    """単項の - と ~ を持つ。"""

    def __init__(self, xs):
        self.xs = list(xs)

    def __neg__(self):
        return Vec(-x for x in self.xs)

    def __invert__(self):
        return Vec(~x for x in self.xs)

    def values(self):
        return self.xs


calls = []


class Counted:
    """__bool__ が呼ばれた回数を calls に残す。"""

    def __bool__(self):
        calls.append("bool")
        return False


def untyped(kind):
    """戻り値の注釈が無い（Arrow からは型が分からない）。"""
    if kind == "empty":
        return Box(0)
    return Box(3)
