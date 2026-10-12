"""py_dec_property_error.py - setter に property と違う名前を付けた形は表さずに明示エラーにする。

`@x.setter def set_x` は CPython では `x` を写した別の property `set_x` を作り、`x` は getter だけのまま残る。
意図と違うことがほとんどなので、変換器は誤りにする（python_builtins_plan.md のタスク 6-1）。
⚠ 以前は `@logged @property`（property を包むデコレータ）をここで踏ませていたが、タスク 6-6 で変換できるようになった。
"""


class C:
    def __init__(self, v):
        self._v = v

    @property
    def value(self):
        return self._v

    @value.setter
    def set_value(self, v):
        self._v = v
