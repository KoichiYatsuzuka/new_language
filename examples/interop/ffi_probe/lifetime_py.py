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
