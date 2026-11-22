"""py_key_error_args.py — 実行時の KeyError の `args` はキーそのもの（known_bugs.md の #9）。

`d[k]` / `del d[k]` / `d.pop(k)` / `s.remove(x)` / `"{k}".format()` の KeyError は `args == (k,)`・`e.args[0] is k`。
空の `d.popitem()` / `s.pop()` は文言の文字列が引数。`str(e)` はどれもキーの `repr`。`run()` は CPython 3.12 と同じ出力。
"""


class K:
    """`__eq__` / `__hash__` の無いキー（同一性で比べる）。"""

    def __repr__(self):
        return "K()"


def show(label, f):
    try:
        f()
        print(label, "no error")
    except KeyError as e:
        print(label, repr(e), e.args, str(e))


def run():
    d = {"a": 1, (1, 2): 3}
    s = {1, 2}
    show("1 read:", lambda: d["zz"])
    show("1 int key:", lambda: d[7])
    show("1 tuple key:", lambda: d[(3, 4)])

    def delete():
        del d["gone"]

    show("2 del:", delete)
    show("3 pop:", lambda: d.pop("nope"))
    show("4 remove:", lambda: s.remove(9))
    show("5 format:", lambda: "{name}".format())
    show("6 popitem:", lambda: {}.popitem())
    show("6 set pop:", lambda: set().pop())

    # 7 キーは同じ物（`is`）
    k = K()
    try:
        {}[k]
    except KeyError as e:
        print("7 identity:", e.args[0] is k, repr(e))

    # 8 捕まえずに投げ直しても同じ args
    try:
        try:
            d["again"]
        except KeyError:
            raise
    except KeyError as e:
        print("8 reraise:", e.args)

    # 9 呼び出した関数の中の誤りも（関数の境界で例外になる）
    def inner():
        return d["deep"]

    try:
        inner()
    except KeyError as e:
        print("9 nested:", e.args)

    # 10 文言の引用符を含むキー
    show("10 quote:", lambda: d["it's"])
