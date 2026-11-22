"""py_match.py — `match` 文を使う関数（py_match.ar が CPython で動かす）。"""


def classify(n):
    match n:
        case 0:
            return "zero"
        case 1:
            return "one"
        case _:
            return "many"


def word(s):
    match s:
        case "a":
            return "A"
        case "b":
            return "B"
        case _:
            return "?"


def singleton(v):
    match v:
        case None:
            return "none"
        case True:
            return "true"
        case False:
            return "false"
        case _:
            return "other"


def no_default(n):
    # `_` が無く、どれにも当たらない場合は何も起きない（Python と同じ）。
    out = "untouched"
    match n:
        case 0:
            out = "zero"
    return out
