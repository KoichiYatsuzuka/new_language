"""py_setattr.py — Python のクラスの `__setattr__`（known_bugs.md の #10）。

`o.x = v` / `setattr(o, "x", v)` / `o.x += 1` は、クラスが `__setattr__` を定義していればそれを呼ぶ（`__init__` の中の
`self.x = ..` も）。上書きを飛ばして本当に入れるのは `object.__setattr__(self, name, value)` か
`super().__setattr__(name, value)`。`__delattr__` と `object.__delattr__` も同じ。`run()` / `errors()` は
CPython 3.12 と同じ出力。
"""
from functools import cached_property


class Logged:
    """代入を記録してから入れる（pandas の `NDFrame.__setattr__` と同じ形: 最後に `object.__setattr__`）。"""

    def __init__(self):
        self.log = []
        self.x = 1

    def __setattr__(self, name, value):
        if name != "log":
            self.log.append(name + "=" + repr(value))
        object.__setattr__(self, name, value)


class Frozen:
    """`__init__` だけが `object.__setattr__` で入れ、あとの代入は拒む（pandas の `NoNewAttributesMixin` の形）。"""

    def __init__(self, v):
        object.__setattr__(self, "v", v)

    def __setattr__(self, name, value):
        raise AttributeError("Frozen is read-only: " + name)

    def __delattr__(self, name):
        raise AttributeError("Frozen cannot delete: " + name)


class Swallow:
    """何も入れない `__setattr__`（入れなければ属性は無いまま）。"""

    def __init__(self):
        self.a = 1

    def __setattr__(self, name, value):
        pass


class Base:
    def __init__(self):
        self.seen = []
        self.n = 0

    def __setattr__(self, name, value):
        if name != "seen":
            self.seen.append(name)
        super().__setattr__(name, value)


class Derived(Base):
    """基底の `__setattr__` を受け継ぐ。"""

    def bump(self):
        self.n += 1


class Upper(Base):
    """自分でも上書きして `super().__setattr__` で基底へ回す。"""

    def __setattr__(self, name, value):
        if isinstance(value, str):
            value = value.upper()
        super().__setattr__(name, value)


class Plain:
    def __init__(self):
        self.k = 1


class Child(Plain):
    """基底は `__setattr__` を書いていない（`super().__setattr__` は `object` のものに届く）。"""

    def __setattr__(self, name, value):
        super().__setattr__(name, value * 10)


class Temp:
    def __init__(self):
        self.calls = []
        self._c = 0

    @property
    def celsius(self):
        return self._c

    @celsius.setter
    def celsius(self, v):
        self._c = v

    def __setattr__(self, name, value):
        if name != "calls":
            self.calls.append(name)
        object.__setattr__(self, name, value)


class Area:
    def __init__(self, w):
        self.hooked = 0
        self.w = w

    @cached_property
    def area(self):
        return self.w * 2

    def __setattr__(self, name, value):
        object.__setattr__(self, "hooked", 0 if name == "hooked" else self.hooked + 1)
        if name != "hooked":
            object.__setattr__(self, name, value)


class Tracked:
    def __init__(self):
        self.gone = []
        self.x = 1
        self.y = 2

    def __delattr__(self, name):
        self.gone.append(name)
        object.__delattr__(self, name)


def run():
    # 1 代入（`__init__` の中も）が `__setattr__` を通る
    o = Logged()
    o.x = 2
    setattr(o, "x", 3)
    o.x += 4
    print("1 log:", o.log, o.x)

    # 2 拒む `__setattr__`。`object.__setattr__` は通る
    f = Frozen(5)
    print("2 frozen:", f.v)
    object.__setattr__(f, "v", 6)
    print("2 bypass:", f.v)

    # 3 何も入れない `__setattr__`
    s = Swallow()
    s.a = 2
    print("3 swallow:", hasattr(s, "a"))

    # 4 受け継いだ `__setattr__`・`super().__setattr__`
    d = Derived()
    d.bump()
    d.bump()
    print("4 derived:", d.seen, d.n)
    u = Upper()
    u.n = "hi"
    print("4 upper:", u.seen, u.n)
    c = Child()
    print("4 child:", c.k)

    # 5 property の setter は `object.__setattr__` の先で呼ばれる
    t = Temp()
    t.celsius = 30
    print("5 property:", t.calls, t.celsius)

    # 6 cached_property が覚える値は `__setattr__` を通らない（CPython はインスタンスの __dict__ へ直に入れる）
    a = Area(4)
    before = a.hooked
    print("6 cached:", a.area, a.area, a.hooked == before)

    # 7 `__delattr__` と `object.__delattr__`
    tr = Tracked()
    del tr.x
    del tr.y
    print("7 delattr:", tr.gone, hasattr(tr, "x"), hasattr(tr, "y"))


def errors():
    def frozen_set():
        f = Frozen(1)
        f.v = 2

    def frozen_setattr():
        f = Frozen(1)
        setattr(f, "v", 2)

    def frozen_del():
        f = Frozen(1)
        del f.v

    def bad_name():
        o = Plain()
        object.__setattr__(o, 1, 2)

    for fn in [frozen_set, frozen_setattr, frozen_del, bad_name]:
        try:
            fn()
        except (AttributeError, TypeError) as e:
            print("E", fn.__name__ + ":", type(e).__name__, e)


def assign_frozen():
    f = Frozen(1)
    f.v = 2
