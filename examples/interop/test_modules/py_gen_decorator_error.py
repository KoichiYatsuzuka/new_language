"""py_gen_decorator_error.py — ジェネレータへのデコレータ（変換器が明示エラーにする形）。

⚠ `Stmt::GenDef` は `decorators` を持たない（Arrow の `gen` にデコレータ構文が
無い）。黙って捨てるとデコレータが効かなくなるので止める。
`@staticmethod` / `@classmethod` / `@abstractmethod` を付けた `gen` メソッドも同じ。
"""


def mark(f):
    return f


@mark
def g():
    yield 1
