"""py_bare_super_error.py - `super()` を「メソッドを呼ぶ」以外の形で使うと明示エラーになる。"""


class A:
    pass


class B(A):
    def f(self):
        s = super()
        return s
