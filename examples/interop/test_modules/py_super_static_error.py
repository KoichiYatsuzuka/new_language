"""py_super_static_error.py — 静的メソッドの中の super()（変換の誤り・known_bugs.md の #6）。

引数なしの `super()` の受け手は今の関数の最初の引数。静的メソッドには受け手が無い（CPython も呼ぶと
最初の引数を受け手にして `TypeError: super(type, obj): obj must be an instance or subtype of type`）。
"""


class Base:
    @staticmethod
    def helper(x):
        return x + 1


class Sub(Base):
    @staticmethod
    def helper(x):
        return super().helper(x) * 10
