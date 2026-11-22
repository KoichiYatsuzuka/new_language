# py_home_shapes.py — よそのモジュールのクラスを返す・受けるモジュール（examples/interop/py_home_classes.ar）

import test_modules.py_home_points as pts
from test_modules.py_home_points import Point


def origin() -> Point:
    return Point(0, 0)


def shift(p: Point, d: int) -> Point:
    return Point(p.x + d, p.y + d)


def vec(n: int) -> pts.vector:
    return pts.vector(n)


class Point2:
    """同じ短い名前ではない、別のクラス。"""

    def __init__(self) -> None:
        self.x = 1


def other() -> Point2:
    return Point2()
