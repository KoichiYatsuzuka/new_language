"""py_except_tuple_error.py — `except (A, (B, C)):`（組の入れ子・変換器が明示エラーにする形）。

⚠ 組（`except (A, B):`）と修飾名（`except mod.Err:`）は python_builtins_plan.md のタスク 4-4 / 4-3 で通るようにした。
  残る誤りは**組の入れ子**だけ。CPython 3.12 も実行時に
  `TypeError: catching classes that do not inherit from BaseException is not allowed` で止める。
⚠⚠ さらに以前は型を黙って `"Exception"` に潰していた ＝ 指定した型だけでなく**何でも捕まえるハンドラ**に化けていた。
"""


def parse(s):
    try:
        return int(s)
    except (ValueError, (TypeError, KeyError)):
        return -1
