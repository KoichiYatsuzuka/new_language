"""py_generator.py — `def` + `yield` → Arrow の `gen`（項目 9）。

判定は**そのフレームだけ**（`frame_has_yield`）。入れ子の `def` が持つ `yield` は
内側を `gen` にするだけで外側には影響しない —— Python の規則と同じ。

⚠ B13 でジェネレータが**真に遅延**になったので、無限ジェネレータ + `break` も
  CPython と同じに動く（以前は先行評価で止まらなかった）。
"""


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
