# lifetime_py.py — 値がいつ捨てられたかを数える（known_bugs.md #25 の例題 nested_native_call_lifetime.ar）
#
# Tracker を作ると数が 1 増え、捨てられる（参照が無くなる）と __del__ が 1 減らす。

_alive = 0


class Tracker:
    def __init__(self) -> None:
        global _alive
        _alive += 1

    def __del__(self) -> None:
        global _alive
        _alive -= 1


def make() -> Tracker:
    return Tracker()


def alive() -> int:
    return _alive


# 渡された物を生かしたまま、今の生きている数と同じ長さの文字列を返す（copy_alloc_speed_plan.md 7-15 の例題
# for_loop_rewind.ar）。コンパイル済みコードは `len(..)` を生の int で受けるので、受けたローカルがハンドルにならない
def alive_mark(t: Tracker) -> str:
    return "x" * _alive
