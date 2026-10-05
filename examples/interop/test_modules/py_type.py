# py_type.ar から import する Python のモジュール（type(x) を Python の中で使う）。


class Shape:
    def __init__(self, size):
        self.size = size

    def scaled(self, k):
        # type(self)(...) で同じクラスを作り直す（派生クラスでも派生のまま）
        return type(self)(self.size * k)

    def describe(self):
        return type(self).__name__ + "(" + str(self.size) + ")"


class Square(Shape):
    pass


def names():
    return [type(3).__name__, type("s").__name__, type([1]).__name__, type(None).__name__,
            Square(2).describe(), Square(2).scaled(3).describe()]


def same_class(a, b):
    return type(a) is type(b)
