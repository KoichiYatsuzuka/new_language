# py_int_lazy_for.ar が import[py] で使うモジュール（for で CPython の反復子から 1 つずつ取り出す・
# pandas_import_plan.md の 2-4）。

log = []


def counting(n):
    """取り出されるたびに log に残す生成器。"""
    for i in range(n):
        log.append("yield %d" % i)
        yield i


def guarded(limit):
    """limit 個より先まで取り出されたら投げる（全部を先に取り出すと必ず投げる）。"""
    for i in range(limit):
        yield i
    raise RuntimeError("consumed past the limit")


def failing():
    """1 つ返したあとで投げる。"""
    yield 1
    raise ValueError("broken in the middle")


class Countdown:
    """__iter__ / __next__ を持つ反復子。"""

    def __init__(self, n):
        self.n = n

    def __iter__(self):
        return self

    def __next__(self):
        if self.n <= 0:
            raise StopIteration
        self.n -= 1
        return self.n


class Shelf:
    """__iter__ が生成器を返す（反復できる物）。"""

    def __init__(self, names):
        self.names = list(names)

    def __iter__(self):
        for name in self.names:
            yield name.upper()


def pairs():
    yield ("a", 1)
    yield ("b", 2)


class Item:
    n: int

    def __init__(self, n: int):
        self.n = n


def items(n):
    """CPython の物（Item）を返す生成器。"""
    for i in range(n):
        yield Item(i)
