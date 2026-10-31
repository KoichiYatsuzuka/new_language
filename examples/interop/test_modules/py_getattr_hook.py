"""py_getattr_hook.py — Python のクラスの `__getattr__`（known_bugs.md の #11）。

ふつうの読みが AttributeError になったときだけ `__getattr__(name)` を呼ぶ（無い名前・`del` した属性・
AttributeError を投げた property の getter）。見つかる名前（フィールド・メソッド・property）は呼ばない。
`getattr(o, k, d)` / `hasattr(o, k)` / `o.k(..)` も同じ。`__getattr__` の中から本当の属性を読むのは
`object.__getattribute__(self, name)` / `super().__getattribute__(name)`。`run()` / `errors()` は CPython 3.12 と同じ出力。
"""


class Dyn:
    def __init__(self):
        self.real = 1
        self.asked = []

    def __getattr__(self, name):
        self.asked.append(name)
        if name.startswith("no_"):
            raise AttributeError("Dyn refuses " + name)
        return "dyn:" + name

    def method(self):
        return "method"


class Frame:
    """pandas の `NDFrame.__getattr__` と同じ形: 列の名前を属性として読み、ほかは `object.__getattribute__`。"""

    def __init__(self, cols):
        self._cols = cols

    def __getitem__(self, key):
        return self._cols[key]

    def __getattr__(self, name):
        if not name.startswith("_") and name in self._cols:
            return self[name]
        return object.__getattribute__(self, name)


class Calls:
    """呼べる値を返す `__getattr__`（`o.greet("x")` は読んでから呼ぶ）。"""

    def __getattr__(self, name):
        def call(*args):
            return name + str(list(args))
        return call


class Child(Dyn):
    """`__getattr__` を受け継ぐ。"""

    def extra(self):
        return self.whatever


class Lazy:
    """property の getter が AttributeError を投げると `__getattr__` が呼ばれる（CPython と同じ）。"""

    def __init__(self):
        self.calls = 0

    @property
    def value(self):
        raise AttributeError("not ready")

    def __getattr__(self, name):
        self.calls += 1
        return "fallback:" + name


class Super:
    def __init__(self):
        self.x = 1

    def __getattr__(self, name):
        if name == "alias":
            return super().__getattribute__("x")
        return super().__getattribute__(name)


def run():
    # 1 無い名前だけが `__getattr__` へ行く（フィールド・メソッドは呼ばない）
    d = Dyn()
    print("1 read:", d.real, d.method(), d.zz, d.asked)

    # 2 getattr / hasattr
    print("2 getattr:", getattr(d, "yy", "default"), getattr(d, "no_q", "default"), hasattr(d, "a"), hasattr(d, "no_b"))

    # 3 del したフィールドは `__getattr__` へ
    del d.real
    print("3 deleted:", d.real)

    # 4 pandas の形（列の名前・ほかは object.__getattribute__）
    f = Frame({"price": [1, 2], "qty": [3]})
    print("4 frame:", f.price, f.qty, hasattr(f, "volume"), hasattr(f, "_cols"))

    # 5 呼べる値を返す・受け継ぐ
    c = Calls()
    print("5 call:", c.greet("x", 2), c.other())
    ch = Child()
    print("5 child:", ch.extra(), ch.real, ch.asked)

    # 6 property の getter の AttributeError
    lz = Lazy()
    print("6 property:", lz.value, lz.calls)

    # 7 super().__getattribute__
    s = Super()
    print("7 super:", s.alias, s.x, hasattr(s, "nothing"))


def errors():
    def refused():
        return Dyn().no_x

    def frame_missing():
        return Frame({}).volume

    def bypass_missing():
        return object.__getattribute__(Dyn(), "zz")

    def bad_name():
        return object.__getattribute__(Dyn(), 3)

    for fn in [refused, frame_missing, bypass_missing, bad_name]:
        try:
            fn()
        except (AttributeError, TypeError) as e:
            print("E", fn.__name__ + ":", type(e).__name__, e)


def read_missing():
    return Frame({"a": 1}).b
