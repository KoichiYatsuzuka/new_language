# py_bound_method.py — Python の束縛メソッド（python_builtins_plan.md のタスク 1-8）。
#
# 使う側: examples/interop/py_bound_method.ar


class Scale:
    def __init__(self, k):
        self.k = k

    def apply(self, x):
        return x * self.k


def run():
    s = Scale(3)
    f = s.apply
    g = getattr(s, "apply")
    return [f(2), g(4), list(map(s.apply, [1, 2])), sorted([3, 1, 2], key=s.apply)]
