"""py_dec_property_error.py - `@property` は Arrow に対応構文が無いので明示エラーになる。"""


class C:
    def __init__(self, v):
        self.v = v

    @property
    def value(self):
        return self.v
