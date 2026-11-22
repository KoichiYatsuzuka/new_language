# py_numeric_classes.py — 境界で Arrow の数になるクラスと、ならないクラス（examples/interop/py_numpy_types.ar）


class Count:
    """`__index__` を持つ（容器でない）: 境界で Arrow の `int` になる（numpy の `int64` と同じ）。"""

    def __init__(self, n):
        self.n = n

    def __index__(self) -> int:
        return self.n


class Grid:
    """`__index__` も持つが容器（`__len__` / `__getitem__`）: Python の物のまま届く（numpy の `ndarray` と同じ）。"""

    def __init__(self, n):
        self.n = n

    def __index__(self) -> int:
        return self.n

    def __len__(self) -> int:
        return self.n

    def __getitem__(self, i: int) -> int:
        return i

    def total(self) -> int:
        return self.n * 2


def count(n: int) -> Count:
    return Count(n)


def grid(n: int) -> Grid:
    return Grid(n)
