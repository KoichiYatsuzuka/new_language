"""py_cached_property.py - functools.cached_property（python_builtins_plan.md のタスク 6-5）。"""
from functools import cached_property

CALLS = []


class Circle:
    def __init__(self, r):
        self.r = r

    @cached_property
    def area(self) -> float:
        """Area, computed once per instance."""
        CALLS.append(self.r)
        return 3.0 * self.r * self.r

    @cached_property
    def tags(self):
        return ["round"]


def run():
    a = Circle(2)
    b = Circle(3)
    print(a.area, a.area, b.area, CALLS)
    a.tags.append("small")
    print(a.tags, b.tags)
    a.area = 1.5
    print(a.area, CALLS)
    p = Circle.area
    print(type(p).__name__, p.attrname, p.__doc__, p.func.__name__)
    print(p.__get__(None, Circle) is p)
