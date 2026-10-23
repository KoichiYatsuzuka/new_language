"""py_del.py — Python の `del o.a` / `del d[k]`（known_bugs.md の #2 / #3）。

`del o.a` は属性を未代入に戻す（以後の読みは AttributeError・hasattr は偽）。property の deleter・
cached_property の計算し直し・`__delattr__` も CPython と同じ。`del d[k]` は辞書・リスト（スライスも）・
`__delitem__`。`run()` / `errors()` は CPython 3.12 と同じ出力。
"""
from functools import cached_property


class Series:
    """pandas の `Series._reset_cacher` と同じ形: キャッシュを `del` で消し、`hasattr` で有無を見る。"""

    def __init__(self, name):
        self.name = name

    def set_cacher(self, ref):
        self._cacher = ref

    def reset_cacher(self):
        if hasattr(self, "_cacher"):
            del self._cacher

    def cacher(self):
        return getattr(self, "_cacher", None)


class Temp:
    def __init__(self, c):
        self._c = c
        self.log = []

    @property
    def celsius(self):
        return self._c

    @celsius.deleter
    def celsius(self):
        self.log.append("deleted")
        del self._c


class ReadOnly:
    def __init__(self):
        self._v = 1

    @property
    def v(self):
        return self._v


class SetOnly:
    def __get__(self, obj, owner=None):
        return 7

    def __set__(self, obj, value):
        pass


class HasSetOnly:
    d = SetOnly()


class Area:
    def __init__(self, w, h):
        self.w = w
        self.h = h
        self.calls = 0

    @cached_property
    def area(self):
        self.calls += 1
        return self.w * self.h


class Bag:
    def __init__(self):
        self.items = {}

    def __setitem__(self, k, v):
        self.items[k] = v

    def __delitem__(self, k):
        del self.items[k]


class Guarded:
    def __init__(self):
        self.x = 1
        self.seen = []

    def __delattr__(self, name):
        self.seen.append(name)


def run():
    # 1 属性を消すと読めなくなる（pandas の `_reset_cacher`）。また代入できる
    s = Series("a")
    s.set_cacher("ref")
    print("1 before:", hasattr(s, "_cacher"), s.cacher())
    s.reset_cacher()
    print("1 after:", hasattr(s, "_cacher"), s.cacher())
    s.reset_cacher()
    s.set_cacher("ref2")
    print("1 again:", s.cacher())

    # 2 property の deleter
    t = Temp(20)
    del t.celsius
    print("2 deleter:", t.log, hasattr(t, "_c"))

    # 3 cached_property は消すと計算し直す
    a = Area(2, 3)
    print("3 first:", a.area, a.area, a.calls)
    a.w = 5
    del a.area
    print("3 recomputed:", a.area, a.calls)

    # 4 辞書
    d = {"a": 1, "b": 2, "c": 3}
    del d["b"]
    print("4 dict:", d)
    try:
        del d["zz"]
    except KeyError as e:
        print("4 missing:", type(e).__name__, e)

    # 5 リスト（添字・負の添字・スライス・ステップ・負のステップ）
    xs = list(range(10))
    del xs[0]
    del xs[-1]
    print("5 index:", xs)
    del xs[1:3]
    print("5 slice:", xs)
    ys = list(range(10))
    del ys[::3]
    print("5 step:", ys)
    zs = list(range(10))
    del zs[8:2:-2]
    print("5 negative step:", zs)
    ws = [1, 2, 3]
    del ws[:]
    print("5 all:", ws)

    # 6 `__delitem__`
    b = Bag()
    b["x"] = 1
    b["y"] = 2
    del b["x"]
    print("6 delitem:", b.items)

    # 7 ターゲットが複数（左から順に）
    m = {"p": [1, 2, 3], "q": 0}
    del m["p"][0], m["q"]
    print("7 multiple:", m)
    n = [10, 20, 30, 40]
    del (n[0], n[0])
    print("7 tuple:", n)

    # 8 `__delattr__` があればそれが呼ばれる（ここでは消さない）
    g = Guarded()
    del g.x
    print("8 delattr:", g.seen, g.x)

    # 9 別名から消しても同じ物（Python のコードの中は共有・known_bugs.md の #1）
    s2 = Series("b")
    s2.set_cacher("r")
    alias = s2
    del alias._cacher
    print("9 alias:", hasattr(s2, "_cacher"))
    reg = {"s": s2}
    s2.set_cacher("back")
    del reg["s"]._cacher
    print("9 through dict:", hasattr(s2, "_cacher"))


def errors():
    def twice():
        s = Series("c")
        s.set_cacher(1)
        del s._cacher
        del s._cacher

    def read_after():
        t = Temp(1)
        del t.celsius
        return t.celsius

    def no_deleter():
        r = ReadOnly()
        del r.v

    def set_only():
        h = HasSetOnly()
        del h.d

    def method():
        s = Series("c")
        del s.reset_cacher

    def missing():
        s = Series("c")
        del s.nothing

    def out_of_range():
        xs = [1, 2]
        del xs[5]

    def str_index():
        xs = [1, 2]
        del xs["a"]

    def zero_step():
        xs = [1, 2]
        del xs[::0]

    def tuple_item():
        t = (1, 2)
        del t[0]

    def str_item():
        s = "ab"
        del s[0]

    def set_item():
        s = {1}
        del s[1]

    def no_delitem():
        s = Series("c")
        del s[0]

    for f in [twice, read_after, no_deleter, set_only, method, missing, out_of_range, str_index, zero_step,
              tuple_item, str_item, set_item, no_delitem]:
        try:
            f()
        except (AttributeError, IndexError, TypeError, ValueError) as e:
            print("E", f.__name__ + ":", type(e).__name__, e)


def read_deleted():
    s = Series("d")
    s.set_cacher("x")
    del s._cacher
    return s._cacher
