"""py_varargs_tuple.py - Python の *args はタプル（python_builtins_plan.md のタスク 1-16）。"""


def collect(first, *rest):
    return rest


def describe(*args, **kwargs):
    return f"{args!r} {type(args).__name__} {len(args)} {sorted(kwargs)}"


def extend(*items):
    return items + ("end",)


class Logger:
    def __init__(self):
        self.lines = []

    def log(self, fmt, *values):
        self.lines.append(fmt % values)
        return values


def run():
    print(collect(1, 2, 3), collect(1))
    print(describe(), describe(1, "a", k=2))
    print(extend(1, 2), isinstance(extend(), tuple))
    lg = Logger()
    print(lg.log("%d-%s", 7, "x"), lg.lines)

    def inner(*xs):
        return xs[::-1]

    print(inner(1, 2, 3))
