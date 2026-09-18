"""py_global_error.py — `global`（変換器が明示エラーにする形）。

⚠⚠ **以前は黙って無視していた**。その結果、内側の代入が外側に届かないまま
静かに動く（＝内側のローカルを書き換えるだけ）誤変換になっていた。
⚠ `nonlocal` も同じ扱い。Arrow は外側を `mut` で宣言すれば内側から書き換えられる。
"""

COUNT = 0


def bump():
    global COUNT
    COUNT = COUNT + 1
    return COUNT
