"""py_shared_objects.py - Python のコードの中は格納で写さず参照を共有する（known_bugs.md の #1・A 案）。"""


class P:
    def __init__(self, x):
        self.x = x


class E:
    pass


class Node:
    def __init__(self, name):
        self.name = name
        self.children = []
        self.parent = None

    def add(self, child):
        self.children.append(child)
        child.parent = self


class Registry:
    def __init__(self):
        self.items = []

    def register(self, item):
        self.items.append(item)
        return item


def run():
    # 1. 格納した後に元を書き換える（別名からの書き換え）
    a = P(1)
    xs = [a]
    a.x = 2
    print("1 list alias:", xs[0].x)
    d = {"k": a}
    a.x = 3
    print("1 dict alias:", d["k"].x)
    # 2. 受け取った物を書き換えて呼び出し元に見える
    reg = Registry()
    item = reg.register(P(10))
    item.x = 11
    print("2 registry:", reg.items[0].x)
    # 3. 格納の後の is
    b = P(1)
    holder = P(b)
    print("3 field is:", holder.x is b, "list is:", [b][0] is b)
    # 4. 既定の ==
    print("4 eq:", P(1) == P(1), E() == E(), b == b)
    # 5. 辞書のキー・in
    print("5 dict keys:", len({P(1): 1, P(1): 2}), len({E(): 1, E(): 2}), "in:", P(1) in [P(1)])
    # 6. 親子の参照（循環）
    root = Node("root")
    kid = Node("kid")
    root.add(kid)
    print("6 parent:", root.children[0].parent is root, kid.parent.name, len(kid.parent.children))
    kid.name = "renamed"
    print("6 renamed:", root.children[0].name)


def run2():
    # 7. 名前への代入（mut の宣言）でも共有
    a = P(1)
    y = a
    y.x = 5
    print("7 local alias:", a.x, y is a)
    # 8. リストの内包・dict の値のリスト
    ps = [P(i) for i in range(2)]
    qs = [p for p in ps]
    qs[0].x = 9
    print("8 comprehension:", ps[0].x, qs[0] is ps[0])
    lst = [1]
    box = {"k": lst}
    lst.append(2)
    print("8 dict list:", box["k"])
    # 9. 辞書のキーの同一性
    k = P(1)
    d = {k: "v"}
    print("9 key lookup:", d[k], k in d, P(1) in d)
