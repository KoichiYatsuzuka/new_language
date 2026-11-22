# py_nested_class.py — クラス本体の中のクラス（入れ子のクラス・フェーズ10 10-18）。
#
# 使う側: examples/interop/py_nested_class.ar


class Node:
    # 最上位の Node（下の Tree.Node とは別のクラス）。
    def kind(self):
        return "top-level Node"


class Tree:
    class Node:
        def __init__(self, value):
            self.value = value

        def kind(self):
            return "Tree.Node"

    # 同じ本体で先に定義した入れ子のクラスを基底にする（素の名前 Node で引ける）。
    class Leaf(Node):
        def kind(self):
            return "Tree.Leaf of " + str(self.value)

    def __init__(self):
        self.root = Tree.Node(1)

    def leaf(self, v):
        # self 経由でも入れ子のクラスを引ける。
        return self.Leaf(v)


class Graph:
    # 別のクラスの同名の入れ子のクラス（Tree.Node とぶつからない）。
    class Node:
        def kind(self):
            return "Graph.Node"


class Config:
    class Section:
        class Entry:
            def __init__(self, key):
                self.key = key


def summary():
    # 入れ子のクラスを Python の中で使う。
    t = Tree()
    return ", ".join([Node().kind(), t.root.kind(), t.leaf(5).kind(), Graph.Node().kind()])
