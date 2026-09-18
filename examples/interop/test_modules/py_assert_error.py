"""py_assert_error.py — `assert`（変換器が明示エラーにする形）。

⚠ 以前は末尾の catch-all で `unsupported Python statement` になり、
  **どの構文で落ちたのか判らなかった**。専用の文言にした。
"""


def positive(n):
    assert n > 0, "must be positive"
    return n
