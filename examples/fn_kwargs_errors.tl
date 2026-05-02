# fn_kwargs_errors.tl
# キーワード引数まわりの StaticTypeError を意図的に起こすサンプル。
# 型検査フェーズで複数のエラーを検出し、exit(1) で停止するはず。

fn add(a: int, b: int) -> int:
    pass

fn greet(name: str, repeat: int) -> None:
    pass

# ── エラー 1: 存在しないキーワード引数名 ─────────────────────────
# 'z' は add のパラメータに存在しない
add(a=1, z=99)

# ── エラー 2: キーワード引数の型不一致 ──────────────────────────
# 'a' は int 型なのに str を渡している
add(a="not_an_int", b=2)

# ── エラー 3: 引数の個数不一致（キーワード引数でも個数は検査される）──
# add は 2 引数なのに 1 つしか渡していない
add(a=1)

# ── エラー 4: 存在しないキーワード引数名（別の関数）────────────────
# 'msg' は greet のパラメータに存在しない
greet(msg="Hello", repeat=1)
