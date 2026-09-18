"""py_except_tuple_error.py — `except (A, B):`（変換器が明示エラーにする形）。

⚠⚠ **以前は黙って `"Exception"` に潰していた** ＝ 指定した 2 型だけでなく
**何でも捕まえるハンドラ**に化けていた。`except mod.Err:` も同じ経路。
"""


def parse(s):
    try:
        return int(s)
    except (ValueError, TypeError):
        return -1
