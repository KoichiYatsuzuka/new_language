"""py_match.py — `match` 文（値 / ワイルドカードのサブセット・項目 8）。

Arrow の `match` は**値の等価比較**か**型検査**のどちらかしか持たず、構造的
パターンマッチングが無い。⇒ `case <リテラル>:` と `case _:` だけを写す。

⚠ 意味論は Python と一致する（実機確認済み）: アームはフォールスルーせず、
  どれにも当たらなければ何もしない。
"""


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
