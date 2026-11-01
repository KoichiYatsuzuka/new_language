"""import[py-int] の参照の数の試験（examples/interop/py_int_refcount.ar・pandas_import_plan.md の D-5・タスク 1-1）。

Arrow で書いた形で CPython から見える参照の数を、同じ形を CPython で書いたときの数（`expected`）と比べる。
数そのものは CPython の版で変わるので、決め打ちの数とは比べない。
pandas は `sys.getrefcount(self)` で連鎖代入を見分け、numpy の `resize` は参照が多いと誤りにするので、
この数が CPython と違うと、Arrow から使ったときだけ振る舞いが変わる。
"""
import sys

SEEN = []


class Probe:
    def count_self(self) -> int:
        return sys.getrefcount(self)

    def __setitem__(self, key, value) -> None:
        SEEN.append(sys.getrefcount(self))


def make() -> Probe:
    return Probe()


def count_arg(x) -> int:
    return sys.getrefcount(x)


def last_seen() -> int:
    return SEEN[-1]


def expected() -> dict[str, int]:
    """examples/interop/py_int_refcount.ar と同じ形を CPython で書いたときの数。"""
    out = {}
    p = make()
    out["method_variable"] = p.count_self()
    out["method_temporary"] = make().count_self()
    out["arg_variable"] = count_arg(p)
    out["arg_temporary"] = count_arg(make())
    p["k"] = 1
    out["setitem_variable"] = last_seen()
    make()["k"] = 1
    out["setitem_temporary"] = last_seen()
    q = p
    r = p
    xs = [p, p]
    out["arg_with_more_holders"] = count_arg(p)
    del q, r, xs
    return out
