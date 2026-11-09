# py_home_points.py — クラスを宣言するモジュール（examples/interop/py_home_classes.ar）


class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    def total(self) -> int:
        return self.x + self.y


class vector:
    """小文字のクラス（numpy の `ndarray` の役）。"""

    def __init__(self, n: int) -> None:
        self.n = n

    def size(self) -> int:
        return self.n
