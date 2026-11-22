"""Values py_value_kinds.ar reads from Python: the built-in types themselves and their look-alikes."""
import collections
import decimal
import enum


class MyInt(int):
    pass


class MyFloat(float):
    pass


class MyStr(str):
    pass


class Color(enum.IntEnum):
    RED = 1


class Indexable:
    def __index__(self):
        return 7


class Truth:
    """A class named `bool` that is not one: it stays a Python object."""


Truth.__name__ = "bool"

Point = collections.namedtuple("Point", "x y")


def big():
    return 2 ** 70


def negative_big():
    return -(2 ** 63) - 1


def edge():
    return (2 ** 63 - 1, -(2 ** 63))


def my_int():
    return MyInt(5)


def my_float():
    return MyFloat(1.5)


def my_str():
    return MyStr("s")


def color():
    return Color.RED


def indexable():
    return Indexable()


def named_bool():
    return Truth()


def surrogate():
    return "a\ud800"


def point():
    return Point(1, 2)


def nested():
    return (1, 2.5, "x", None, True, (2 ** 70, (3,)))


def tenth():
    return decimal.Decimal("0.1")


def nothing():
    return None


def empty():
    return ()


def japanese():
    return "日本語"
