# py_import_stdlib_error.py — `import[py]` が**通さない** import（項目 27）。
#
# `os` は C 実装を含む標準ライブラリで、翻訳対象の `.py` が検索パスに無い。
# 変換器はここを黙って捨てず、明示エラーで止める。

import os


def cwd():
    return os.getcwd()
