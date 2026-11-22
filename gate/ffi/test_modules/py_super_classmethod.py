"""py_super_classmethod.py — クラスメソッド・静的メソッドと super()（known_bugs.md の #6）。

CPython と同じく、引数なしの `super()` の受け手は今の関数の最初の引数（クラスメソッドなら `cls`）で、基底の
クラスメソッドは `cls` を派生クラスのまま受け取る。`run()` / `errors()` は CPython 3.12 と同じ出力。
"""


class Base:
    def __init__(self, name):
        self.name = name

    @classmethod
    def kind(cls):
        return "Base(" + cls.__name__ + ")"

    @classmethod
    def make(cls, name, suffix=""):
        return cls(name + suffix)

    @staticmethod
    def helper(x):
        return x + 1

    def hello(self):
        return "hello " + self.name


class Sub(Base):
    @classmethod
    def kind(cls):
        return "Sub<" + super().kind()

    @classmethod
    def make(cls, name, suffix=""):
        # pandas の `_simple_new` / `_from_sequence` と同じ形: 基底のクラスメソッドへ回して派生のインスタンスを作る
        return super().make(name.upper(), suffix=suffix)

    @classmethod
    def two_arg(cls):
        return super(Sub, cls).kind()

    @classmethod
    def call_static(cls):
        return super().helper(1)

    @classmethod
    def spread(cls, *args, **kwargs):
        return super().make(*args, **kwargs)

    @staticmethod
    def helper(x):
        return x * 10

    def hello(self):
        return "sub " + super().hello()

    def inst_calls_cm(self):
        return super().kind()

    def inst_calls_static(self):
        return super().helper(5)


class Leaf(Sub):
    @classmethod
    def kind(cls):
        return "Leaf<" + super().kind()


def run():
    print("1 classmethod:", Sub.kind(), "|", Sub("x").kind())
    m = Sub.make("ab", suffix="!")
    print("2 makes the subclass:", type(m).__name__, m.name)
    print("3 two-arg:", Sub.two_arg())
    print("4 static from classmethod:", Sub.call_static())
    s = Sub.spread("cd", suffix="?")
    print("5 spread:", type(s).__name__, s.name)
    print("6 instance methods:", Sub("y").hello(), "|", Sub("z").inst_calls_cm(), "|", Sub("w").inst_calls_static())
    print("7 three levels:", Leaf.kind(), "|", type(Leaf.make("q")).__name__)
