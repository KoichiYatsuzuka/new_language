"""py_super_error.py - 基底クラスの無いクラスで `super()` を使うと明示エラーになる。"""


class A:
    def f(self):
        return super().f()
