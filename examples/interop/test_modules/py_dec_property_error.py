"""py_dec_property_error.py - `@property` を内側に書いた形（property を別のデコレータで包む）は表せないので明示エラーになる。

`@property` / `@x.setter` は**いちばん外側**のときだけ変換できる（python_builtins_plan.md のタスク 6-1）。
"""


def logged(f):
    return f


class C:
    def __init__(self, v):
        self.v = v

    @logged
    @property
    def value(self):
        return self.v
