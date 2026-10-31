"""py_methods_via_instance.py — 静的メソッド・クラスメソッドをインスタンスから呼ぶ（known_bugs.md の #5）。

CPython と同じく `self.helper(x)` / `obj.helper(x)` は静的メソッドを（`self` を渡さずに）呼び、`self.make()` は
クラスメソッドに `cls = type(self)`（派生クラスのインスタンスなら派生クラス）を渡す。値として読む形
（`f = self.helper` / `g = self.make`）も同じ。`run()` は CPython 3.12 と同じ出力。
"""


class Shape:
    sides = 0

    def __init__(self, name):
        self.name = name

    @staticmethod
    def double(x):
        return x * 2

    @classmethod
    def kind(cls):
        return cls.__name__ + "/" + str(cls.sides)

    @classmethod
    def make(cls, name):
        return cls(name)

    def describe(self):
        # pandas の `self._validate(..)` と同じ形
        return self.name + ": " + str(self.double(3)) + " " + self.kind()


class Square(Shape):
    sides = 4


class Funcs:
    """関数の形（`f = staticmethod(f)`・タスク 6-13）も同じ。"""

    def plus(x):
        return x + 100

    plus = staticmethod(plus)

    def use(self):
        return self.plus(1)


def run():
    s = Shape("s")
    q = Square("q")
    # 1 メソッドの中から self 経由で
    print("1 describe:", s.describe(), "|", q.describe())

    # 2 外からインスタンス経由で
    print("2 outside:", s.double(5), q.kind(), type(q.make("m")).__name__, q.make("m").name)

    # 3 クラス経由は今までどおり
    print("3 class:", Shape.double(7), Square.kind(), type(Square.make("n")).__name__)

    # 4 値として読む
    f = q.double
    g = q.make
    h = q.kind
    print("4 values:", f(4), type(g("v")).__name__, h())

    # 5 関数の形の staticmethod
    print("5 function form:", Funcs().use())

    # 6 読んだ値の同一性（静的メソッドは同じ関数・クラスメソッドは同じクラスに束縛した同じメソッド）
    print("6 identity:", q.double is q.double, q.make == q.make, q.make == s.make)
