"""py_static_class_funcs_error.py - クラス本体の外で定義した関数を staticmethod で包む（タスク 6-13・変換の誤り）。"""


def helper(x):
    return x + 1


class Box:
    bump = staticmethod(helper)
