"""py_class_body_names.py — クラス本体の名前を参照するクラス属性・メソッドの別名を持つクラス（py_class_body_names.ar が CPython で動かす）。"""

SCALE = 100  # クラス本体で後から同じ名前を束縛しても、それより前の式はこの大域を引く


class Temperature:
    LIMIT = 10
    DOUBLE = LIMIT * 2
    SQUARES = [i * i for i in range(LIMIT // 5)]
    FIRST = SCALE  # まだクラス本体に SCALE が無いので大域の 100

    def __init__(self, c):
        self._c = c

    def get_c(self):
        return self._c

    def set_c(self, v):
        if v > Temperature.DOUBLE:
            raise ValueError("too hot")
        self._c = v

    celsius = property(get_c, set_c, None, "degrees Celsius")
    current = get_c  # メソッドの別名（インスタンスから読むと束縛される）
    HANDLERS = {"get": get_c}
    describe = lambda self: "T"  # ラムダの本体はクラス本体の名前を見ない

    def __str__(self):
        return "Temperature(" + str(self._c) + ")"

    __repr__ = __str__
    SCALE = 1


def run():
    t = Temperature(5)
    print(Temperature.LIMIT, Temperature.DOUBLE, Temperature.SQUARES, Temperature.FIRST, Temperature.SCALE)
    print(t.celsius, t.current(), Temperature.HANDLERS["get"](t), t.describe())
    t.celsius = 15
    print(t.celsius, Temperature.celsius.__doc__)
    try:
        t.celsius = 99
    except ValueError as e:
        print(e)
    print(str(t), repr(t), [t])
