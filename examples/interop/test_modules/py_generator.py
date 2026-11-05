"""py_generator.py — ジェネレータ（関数・入れ子・メソッド・無限）（py_generator.ar が CPython で動かす）。"""


def two():
    yield 1
    yield 2


def squares(xs):
    for x in xs:
        yield x * x


def filtered(xs):
    for x in xs:
        if x > 1:
            yield x


def counter():
    n = 0
    while True:
        yield n
        n = n + 1


def outer():
    # 入れ子の `def` が持つ yield は**内側だけ**を gen にする。
    def inner():
        yield 7

    return inner()


def not_a_gen(xs):
    return len(xs)


class Bag:
    def __init__(self, xs):
        self.xs = xs

    def each(self):
        for x in self.xs:
            yield x

    def size(self):
        return len(self.xs)
