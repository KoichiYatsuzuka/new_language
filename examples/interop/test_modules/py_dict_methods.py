"""py_dict_methods.py - dict のメソッド（python_builtins_plan.md のタスク 3-6）。"""


class Key:
    def __init__(self, name):
        self.name = name

    def __eq__(self, other):
        return isinstance(other, Key) and self.name.lower() == other.name.lower()

    def __hash__(self):
        return hash(self.name.lower())


def attempt(label, f):
    try:
        print(label, f())
    except (KeyError, TypeError, ValueError) as e:
        print(label, type(e).__name__ + ":", e)


def run():
    d = {"a": 1, "b": 2}
    print(d.get("a"), d.get("z"), d.get("z", 0), d.get("a", 0))
    print(d.pop("a"), d, d.pop("z", -1))
    attempt("pop missing:", lambda: d.pop("z"))
    print(d.setdefault("c", 3), d.setdefault("c", 99), d.setdefault("n"), d)
    d.update({"b": 20, "x": 1})
    d.update([("y", 2), ["z", 3]], w=4)
    d.update(v=5)
    print(d)
    print(d.popitem(), d.popitem(), d)
    e = d.copy()
    e["new"] = 1
    print(len(d), len(e), "new" in d)
    e.clear()
    print(e, len(e))
    attempt("popitem empty:", lambda: e.popitem())
    attempt("get no args:", lambda: d.get())
    attempt("get 3 args:", lambda: d.get(1, 2, 3))
    attempt("get kw:", lambda: d.get("a", default=1))
    attempt("update 2:", lambda: d.update({}, {}))
    attempt("update bad:", lambda: d.update([1]))
    attempt("update len:", lambda: d.update([(1, 2, 3)]))
    attempt("clear arg:", lambda: d.clear(1))
    # __hash__ / __eq__ を尊重する
    k = {Key("Name"): "v"}
    print(k.get(Key("NAME")), k.pop(Key("name")), len(k))
    # 数えてから表にする形
    counts = {}
    for w in "a b a c b a".split():
        counts[w] = counts.get(w, 0) + 1
    print(counts)
    groups = {}
    for w in ["apple", "avocado", "banana"]:
        groups.setdefault(w[0], []).append(w)
    print(groups)
