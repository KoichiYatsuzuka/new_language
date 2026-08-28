"""py_inherit.py - `import[py]` のクラス継承サポートの検査用モジュール。

★ Arrow の `class` は**継承できない**（基底に置けるのはトレイトだけ）。
Python ではクラス継承が普通なので、**Python モジュールを読み込むときだけ**の特別扱いとして
トレイト継承と同じ仕組み（＝定義時に平坦化する）でクラス継承を成立させている。
"""


class A:
    tag = "A"

    def __init__(self, v):
        self.v = v

    def label(self):
        return "A"

    # 基底のメソッドから `self.label()` を呼ぶ ⇒ サブクラスの実装に動的ディスパッチされる
    def show(self):
        return self.label() + str(self.v)


class B(A):
    """メソッドのオーバーライド + `super()` によるサブクラス `__init__`。"""

    def __init__(self, v, w):
        super().__init__(v)
        self.w = w

    def label(self):
        return "B" + super().label()

    def show2(self):
        return super().show() + "/" + str(self.w)

    def extra(self):
        return self.v * 2


class C(B):
    """3 段継承。"""

    def label(self):
        return "C"


class E(A):
    """`super()` を使わず基底の `__init__` を明示的に呼ぶ古い書き方。"""

    def __init__(self, v, z):
        A.__init__(self, v)
        self.z = z

    def sum(self):
        return self.v + self.z


class M1:
    def m1(self):
        return "m1"


class M2:
    def m2(self):
        return "m2"


class Multi(M1, M2):
    """多重継承（メソッドは先に書いた基底が優先）。"""

    def own(self):
        return "own"
