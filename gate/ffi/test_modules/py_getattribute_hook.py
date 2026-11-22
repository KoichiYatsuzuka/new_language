"""py_getattribute_hook.py — Python のクラスの `__getattribute__`（known_bugs.md の #12）。

`__getattribute__(name)` は**すべての**明示の読みで呼ばれる（フィールド・メソッド・property・`getattr` / `hasattr`・
`o.m(..)` のメソッド呼び出し・特殊メソッドを明示して呼ぶ `o.__len__()`）。`AttributeError` を投げれば `__getattr__` へ。
`str(o)` / `len(o)` / `o + 1` / `for x in o` が内部で引く特殊メソッドは通らない（CPython は型から直接引く）。
本当の属性を読むのは `object.__getattribute__(self, name)` / `super().__getattribute__(name)`。`run()` は CPython 3.12 と同じ出力。
"""


class Logged:
    def __init__(self):
        object.__setattr__(self, "seen", [])
        self.x = 1

    def __getattribute__(self, name):
        object.__getattribute__(self, "seen").append(name)
        return object.__getattribute__(self, name)

    def method(self):
        return "m"

    @property
    def prop(self):
        return "p"

    def __len__(self):
        return 3

    def __str__(self):
        return "Logged!"

    def __add__(self, other):
        return 100 + other

    def __iter__(self):
        return iter([7, 8])


class Redirect:
    """名前で答えを変える（`virtual_*` は計算した値・ほかは本当の属性）。"""

    def __init__(self):
        self.real = "real"

    def __getattribute__(self, name):
        if name.startswith("virtual_"):
            return "v:" + name[len("virtual_"):]
        return super().__getattribute__(name)


class Fallback:
    """`__getattribute__` が AttributeError を投げると `__getattr__` が呼ばれる。"""

    def __init__(self):
        self.kept = 1

    def __getattribute__(self, name):
        if name == "hidden":
            raise AttributeError("hidden by __getattribute__")
        return object.__getattribute__(self, name)

    def __getattr__(self, name):
        return "fallback:" + name


class Child(Logged):
    """基底の `__getattribute__` を受け継ぐ。"""

    def extra(self):
        return self.x + 10


def seen(o):
    return object.__getattribute__(o, "seen")


def run():
    # 1 明示の読み・呼び出しはすべて通る
    o = Logged()
    v = (o.x, o.method(), o.prop, getattr(o, "x"), hasattr(o, "x"))
    print("1 explicit:", v, seen(o))

    # 2 内部で引く特殊メソッドは通らない・明示して呼べば通る
    o2 = Logged()
    implicit = (str(o2), len(o2), o2 + 1, [i for i in o2])
    print("2 implicit:", implicit, seen(o2))
    explicit = (o2.__len__(), o2.__str__())
    print("2 explicit dunder:", explicit, seen(o2))

    # 3 名前で答えを変える・super().__getattribute__
    r = Redirect()
    print("3 redirect:", r.virtual_color, r.real, getattr(r, "virtual_size"), hasattr(r, "virtual_any"))

    # 4 __getattribute__ の AttributeError は __getattr__ へ
    f = Fallback()
    print("4 fallback:", f.kept, f.hidden, getattr(f, "hidden", "default"), f.missing)

    # 5 受け継ぐ（メソッドの中の self.x も通る）
    c = Child()
    print("5 child:", c.extra(), seen(c))
