# py_int_identity.ar が import[py-int] で使うモジュール（CPython の物どうしの === は同一性・pandas_import_plan.md の 5-1）。


class Node:
    pass


shared = Node()


def same() -> Node:
    """毎回同じオブジェクトを返す。"""
    return shared


def fresh() -> Node:
    """毎回新しいオブジェクトを返す。"""
    return Node()


class Strict:
    """__eq__ を呼ばれたら投げる（=== が __eq__ を呼ばないことを確かめる）。"""

    def __eq__(self, other):
        raise RuntimeError("__eq__ must not be called by ===")

    __hash__ = object.__hash__


class Always:
    """__eq__ が何にでも True を返す（== と === の違いを見る）。"""

    def __eq__(self, other):
        return True

    __hash__ = object.__hash__


def is_same(a, b) -> bool:
    return a is b
