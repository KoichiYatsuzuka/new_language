# lying_py.py — 型注釈は正しいが実装が嘘をつく Python モジュール。
# Python は注釈を実行時に強制しないため、こういう実装が普通に書けてしまう。
# Arrow の FFI 境界検査（ffi_boundary）がこれを捕まえることを示すための素材。

def get_int() -> int:
    return "I am a string"

def get_int_none() -> int:
    return None

def get_list_of_int() -> list[int]:
    return [1, "two", 3.0]
