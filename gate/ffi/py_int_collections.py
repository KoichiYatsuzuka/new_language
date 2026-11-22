# py_int_collections.py — py_int_collections.ar から CPython で使うモジュール（pandas_import_plan.md の D-12・2-7）。


def make_scores() -> dict[str, list[int]]:
    return {"a": [3, 1], "b": [2]}


def untyped_list():
    return [1, "two", 3]


def append_twice(xs: list[int], x: int) -> None:
    xs.append(x)
    xs.append(x)


def same(a, b) -> bool:
    return a is b


class Registry:
    """渡された list を持ち続けて、後から書き換える。"""

    def __init__(self) -> None:
        self.kept: list[list[int]] = []

    def add(self, xs: list[int]) -> None:
        self.kept.append(xs)

    def push_all(self, x: int) -> None:
        for xs in self.kept:
            xs.append(x)
