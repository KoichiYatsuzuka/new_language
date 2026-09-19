"""py_with_clash_error.py — `as` の名前がスコープの他所でも代入されている（明示エラー）。

項目 2 の巻き上げでその名前は宣言済みになるので、ブロック内の `mut` が
`already declared` になる（Arrow は覆い隠しを禁じている）。ブロック外へ束縛すると
**退出時に破棄されない** ＝ リソースが閉じないので、黙って通さず止める。
"""


def g(p, m):
    f = 1
    with open(p, m) as f:
        pass
    return f
