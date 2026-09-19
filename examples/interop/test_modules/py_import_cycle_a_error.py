# py_import_cycle_a_error.py — 相互 import（項目 27）。
# Python は部分初期化モジュールを渡して通すが、Arrow の `import[py]` は
# `circular import detected` で止める。

import test_modules.py_import_cycle_b_error as b


def a_val():
    return 1 + b.b_val()
