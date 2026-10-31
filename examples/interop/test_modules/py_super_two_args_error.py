"""py_super_two_args_error.py - 定義中でないクラスを super に渡す（タスク 6-2・変換の誤り）。"""


class Base:
    def name(self):
        return "base"


class Middle(Base):
    def name(self):
        return "middle"


class Leaf(Middle):
    def name(self):
        # Middle を飛ばして Base の name を呼ぶ形（MRO の途中から探す）
        return super(Middle, self).name()
