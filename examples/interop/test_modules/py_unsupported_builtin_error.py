# py_unsupported_builtin_error.py — Arrow に無い組み込みを参照するモジュール（python_builtins_plan.md のタスク 1-1）。
#
# 使う側: examples/interop/py_unsupported_builtin_error.ar
# format / exit はどちらも Arrow の実行時に無い。exit は呼ばれない関数の中だが、それでも誤り。


def show(x):
    return format(x, ",")


def never_called():
    exit(1)
