"""py_dec_static_class_error.py - `@staticmethod` と `@classmethod` の併用は明示エラーになる。

Arrow の `static fn` と `class_method fn` は排他なので、両方を立てられない。
"""


class C:
    @staticmethod
    @classmethod
    def f(a):
        return a
