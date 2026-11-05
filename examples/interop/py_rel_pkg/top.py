# top.py — Python の相対 import（`from .m import x` / `from . import m`）の入口。
#
# CPython が実行する（探索の起点はこの .py のパッケージ）。

from .helper import double
from . import consts
from .inner.deep import quad


def run(x) -> int:
    return double(x) + quad(x) + consts.OFFSET
