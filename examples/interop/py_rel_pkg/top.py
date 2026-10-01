# top.py — Python の相対 import（`from .m import x` / `from . import m`）の入口。
#
# ★ 変換器は相対 import を Arrow の相対 import（`origin.level`）へそのまま写す（2026-10-02）。
#   探索の起点は**この .py のディレクトリ**で、規則は Arrow 側と同じ（src/module_path.rs）。
#   ⚠ 以前は「相対 import は未対応」という変換エラーだった。

from .helper import double
from . import consts
from .inner.deep import quad


def run(x):
    return double(x) + quad(x) + consts.OFFSET
