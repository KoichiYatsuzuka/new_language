"""py_type_alias.py — `type X = T`（Python 3.12+ の型エイリアス・項目 10）。

⚠ Arrow の `alias` は**パース時構文**で AST に出せない（`parser.aliases` へ登録され
  `Stmt::Pass` になる）。`new_type` は名目的別型なので `list[int]` として使えなくなる。
  ⇒ 変換器が自前の表を持ち、**型注釈の解決時に透過展開**する。

⚠ 表はモジュール 1 本の変換中だけ有効。別ファイルの同名エイリアスと混ざらない。
"""

type Vec = list[int]
type Name = str
type Pair = Vec  # エイリアスの連鎖（登録時に右辺を解決するので list[int] になる）


def total(xs: Vec) -> int:
    s = 0
    for v in xs:
        s = s + v
    return s


def greet(n: Name) -> Name:
    return "hi " + n


def chained(p: Pair) -> int:
    return len(p)
