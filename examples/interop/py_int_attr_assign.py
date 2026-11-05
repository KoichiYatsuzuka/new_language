"""import[py] の属性への代入の試験（examples/interop/py_int_attr_assign.ar・pandas_import_plan.md の 2-9）。

Arrow の `o.x = v` は CPython の `setattr` を呼ぶ。`__setattr__` の中から見える参照の数を、同じ形を CPython で
書いたときの数（`expected`）と比べる（数そのものは CPython の版で変わるので決め打ちしない・D-5）。
"""
import sys

count: int = 0
label: str = "start"
SEEN = []


def read_count() -> int:
    return count


class Box:
    def __init__(self) -> None:
        self.value: int = 1

    @property
    def doubled(self) -> int:
        return self.value * 2


class Probe:
    def __setattr__(self, name, value) -> None:
        SEEN.append((sys.getrefcount(self), sys.getrefcount(value)))
        object.__setattr__(self, name, value)


class Payload:
    pass


def make() -> Probe:
    return Probe()


def last_seen() -> list[int]:
    return list(SEEN[-1])


def expected() -> dict[str, list[int]]:
    """examples/interop/py_int_attr_assign.ar と同じ形を CPython で書いたときの数（受け手, 値）。"""
    out = {}
    p = make()
    v = Payload()
    p.x = v
    out["variable"] = last_seen()
    make().x = Payload()
    out["temporary"] = last_seen()
    return out
