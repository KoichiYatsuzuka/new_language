"""py_with_cm_error.py — `__enter__` / `__exit__` を持つクラスの `with`（明示エラー）。

- `__enter__` が**別の値**を返す形は `mut x = EXPR` では再現できない
  （`open()` のようにマネージャ自身を返すものだけが一致する）。
- `__exit__` の副作用（ロック解放・commit / rollback）は `Drop` では走らない。

⇒ 黙って落とさず止める。判定は**同じモジュールで定義されたクラス**に限る。
"""


class Lock:
    def __enter__(self):
        return self

    def __exit__(self, a, b, c):
        return False


def g():
    with Lock() as l:
        pass
    return 1
