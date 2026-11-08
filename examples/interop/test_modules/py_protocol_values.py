# py_protocol_values.py — CPython の物と protocol（examples/interop/py_protocol_values.ar）


class Bag:
    """`__len__` だけを持つ。"""

    def __len__(self) -> int:
        return 4


class Lazy:
    """どの属性の読みでも例外を投げる。型テストはこれを呼ばない（CPython 3.12 の `isinstance` と同じ）。"""

    def __getattr__(self, name):
        raise RuntimeError("must not run: " + name)


def bag() -> Bag:
    return Bag()


def lazy() -> Lazy:
    return Lazy()
