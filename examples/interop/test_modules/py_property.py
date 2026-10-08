"""py_property.py - @property / @x.setter / @Base.x.getter と組み込みの property（python_builtins_plan.md のタスク 6-1）。"""


class Temperature:
    def __init__(self, celsius):
        self.celsius = celsius  # setter を通る（フィールドではない）

    @property
    def celsius(self):
        return self._c

    @celsius.setter
    def celsius(self, value):
        if value < -273.15:
            raise ValueError("below absolute zero")
        self._c = value  # __init__ では代入していないフィールド

    @celsius.deleter
    def celsius(self):
        self._c = 0

    @property
    def fahrenheit(self):
        return self._c * 9 / 5 + 32


class Frame:
    """pandas の DataFrame の形: _constructor を property にし、self._constructor(..) で作り直す。"""

    def __init__(self, data):
        self.data = list(data)

    @property
    def _constructor(self):
        return Frame

    @property
    def shape(self):
        return (len(self.data),)

    def head(self, n):
        return self._constructor(self.data[:n])


class Base:
    @property
    def name(self):
        return "base"


class Child(Base):
    @Base.name.getter
    def name(self):
        return "child of " + Base.name.fget(self)


def run():
    t = Temperature(25)
    print(t.celsius)
    print(t.fahrenheit)
    t.celsius = 100
    print(t.fahrenheit)
    try:
        t.celsius = -300
    except ValueError as e:
        print(e)
    try:
        t.fahrenheit = 0
    except AttributeError as e:
        print(e)
    p = Temperature.celsius
    print(type(p).__name__, p is Temperature.celsius)
    print(p.fset is not None, p.fdel is not None)
    p.fdel(t)
    print(t.celsius)
    h = Frame([1, 2, 3, 4]).head(2)
    print(h.data, h.shape)
    print(Child().name)
    print(Base().name)
