"""py_loop_else_error.py — `for ... else:`（変換器が明示エラーにする形）。

Python のループ `else` は「**`break` せずに回り切ったときだけ**走る」節。
「ループが終わったら走る」ではないので、本体の後ろに繋ぐ変換もできない。
⚠ `while ... else:` もまったく同じ経路（`loop_else_error`）で止まる。
"""


def find(xs, t):
    for x in xs:
        if x == t:
            break
    else:
        return "not found"
    return "found"
