"""py_static_class_funcs.py - staticmethod / classmethod を関数として使う形（python_builtins_plan.md のタスク 6-13）。"""


class Temp:
    def __init__(self, deg):
        self.deg = deg

    def to_f(c):
        return c * 9 / 5 + 32
    to_f = staticmethod(to_f)

    def parse(cls, text):
        return cls(float(text.rstrip("C")))
    parse = classmethod(parse)

    def double(x):
        return x * 2
    twice = staticmethod(double)

    def describe(self):
        return f"{self.deg}C = {Temp.to_f(self.deg)}F"


class Hot(Temp):
    pass


def run():
    print(Temp.to_f(100), Temp.to_f(37))
    t = Temp.parse("25C")
    print(type(t).__name__, t.deg, t.describe())
    h = Hot.parse("40C")
    print(type(h).__name__, h.deg)
    print(Temp.twice(21), Temp.twice("ab"), Temp.double(4))
