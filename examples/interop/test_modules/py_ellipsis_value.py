"""py_ellipsis_value.py - Ellipsis（`...`）の値（python_builtins_plan.md のタスク 6-11）。"""


class Grid:
    def __init__(self, fill=...):
        self.fill = fill
        self.marks = {"any": ...}

    def has_fill(self):
        return self.fill is not ...


def pick(x=...):
    return x


def stub():
    ...


def run():
    # `...` は Ellipsis（None ではない）。フィールド・辞書・リストを通っても同じ物
    g = Grid()
    print(g.fill is ..., g.marks["any"] is Ellipsis, g.has_fill(), Grid(None).has_fill())
    print(pick() is ..., pick(None) is ..., None is ..., stub() is None)
    xs = [...]
    print(xs[0] is ..., ... in xs, ... == Ellipsis, ... != None, {...: 1}[Ellipsis])
    # 表示・型
    print(..., repr(...), str(Ellipsis), [..., 1], type(...).__name__, type(...) is type(Ellipsis))
    print(type(...)() is ..., isinstance(..., object), hash(...) == hash(Ellipsis), bool(...))
    for bad in (lambda: type(...)(1), lambda: setattr(..., "x", 1)):
        try:
            bad()
        except (TypeError, AttributeError) as e:
            print(type(e).__name__ + ":", e)
