# py_class_body_error.py — クラス本体の変換しない文（フェーズ10 10-18）。
#
# 使う側: examples/interop/py_class_body_error.ar


class Settings:
    debug = False
    if debug:
        level = 10
    else:
        level = 20
