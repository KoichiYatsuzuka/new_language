"""py_format.py - 書式指定の小言語を使う Python のモジュール（python_builtins_plan.md のタスク 3-4）。"""


class Vec:
    def __init__(self, x, y):
        self.x = x
        self.y = y

    def __format__(self, spec):
        if spec.endswith("p"):
            return f"<{self.x:{spec[:-1]}}, {self.y:{spec[:-1]}}>"
        return f"({self.x:{spec}}, {self.y:{spec}})"

    def __repr__(self):
        return f"Vec({self.x!r}, {self.y!r})"


def table(rows, width=8):
    lines = []
    for name, value in rows:
        lines.append(f"{name:<{width}}|{value:>{width},.2f}|{value / 1000:.1%}")
    return "\n".join(lines)


def run():
    v = Vec(1.5, -2.25)
    print(f"{v}", f"{v:.2f}", f"{v:6.1fp}", f"{v!r}")
    print(table([("apples", 1234.5), ("pears", 98765.4321)]))
    print("{:>10}|{!a}|{x:^7}".format("right", "naïve", x="mid"))
    print(format(0.000123, "e"), format(10 ** 6, "_"), format(255, "#b"))
    total = 3
    print(f"{total=}", f"{total = :03d}", f"{'é'!a}")
