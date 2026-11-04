# py_int_arrow_objects.py — py_int_arrow_objects.ar から CPython で使うモジュール（pandas_import_plan.md の 2-6）。
# Arrow のインスタンス・クラス・ジェネレータを、Python の物と同じように扱う。


def describe(p):
    return f"{type(p).__name__}({p.x}, {p.y}) str={p} repr={p!r}"


def by_x(points):
    return [str(p) for p in sorted(points, key=lambda p: p.x)]


def move(p, dx):
    p.x = p.x + dx          # 書ける包みなら Arrow のインスタンスが変わる
    return p


def call_method(p, dx):
    p.shift(dx)             # `mut self` のメソッド
    return p.x


def same(a, b):
    return a is b


def keep(holder, p):
    holder.append(p)


def length_and_items(bag):
    return len(bag), bag[0], 2 in bag, list(bag)


def add(a, b):
    return a + b


def make(cls, x, y):
    return cls(x, y)        # Arrow のクラスを Python から呼ぶ


def collect(gen):
    return [v * 10 for v in gen]


def total_of(make_gen, n):
    return sum(make_gen(n))  # Arrow のジェネレータ関数を Python から呼ぶ


def try_set(p):
    try:
        p.x = 99
        return "changed"
    except TypeError:
        return "TypeError"


def as_key(p):
    d = {p: "found"}
    return d[p]


def hasattrs(p):
    return hasattr(p, "x"), hasattr(p, "nope"), "shift" in dir(p)


def is_iterable(p):
    return hasattr(type(p), "__iter__")


def from_thread(p):
    """別の Python のスレッドから Arrow のインスタンスを読む（Arrow の値はスレッドをまたげない）。"""
    import threading
    out = []

    def run():
        try:
            out.append(p.x)
        except RuntimeError:
            out.append("RuntimeError")

    t = threading.Thread(target=run)
    t.start()
    t.join()
    return out[0]


def shift_it(p):
    p.shift(1)
