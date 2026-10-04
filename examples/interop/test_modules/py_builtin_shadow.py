# py_builtin_shadow.ar から import する Python のモジュール。最上位で組み込みの名前を束縛し直す。
# CPython はモジュールの大域が組み込みを隠す（builtins は名前の探索の最後の段）。

id = 7
str = "shadowed str"


def len(x):
    return 42


def read_back():
    return id, str, len([1, 2, 3])
