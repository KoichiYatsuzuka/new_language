# py_int_slices.ar が import[py-int] で使うモジュール（切り出し・多次元の添字・Ellipsis / NotImplemented を Python へ
# 渡す・pandas_import_plan.md の 2-3）。


class Grid:
    """受け取った添字を repr で返す（numpy の配列・pandas の loc と同じく、添字の形は受け手が決める）。"""

    def __init__(self):
        self.stored: list[str] = []

    def __getitem__(self, key):
        return repr(key)

    def __setitem__(self, key, value):
        self.stored.append(repr(key) + " = " + repr(value))


class Table:
    """ラベルで切り出す（pandas の loc と同じく、終わりのラベルも含む）。"""

    def __init__(self, labels):
        self.labels = list(labels)

    def __getitem__(self, key):
        if isinstance(key, slice):
            start = self.labels.index(key.start) if key.start is not None else 0
            stop = self.labels.index(key.stop) + 1 if key.stop is not None else len(self.labels)
            return self.labels[start:stop:key.step]
        return self.labels.index(key)


class Matrix:
    """2 次元の添字（m[i, j]・m[i, :]・m[:, j]・m[..., j]）。"""

    def __init__(self, rows):
        self.rows = [list(r) for r in rows]

    def __getitem__(self, key):
        if key is Ellipsis:
            return self.tolist()
        i, j = key
        if i is Ellipsis:
            i = slice(None)
        if isinstance(i, slice):
            return [r[j] for r in self.rows[i]]
        return self.rows[i][j]

    def __setitem__(self, key, value):
        i, j = key
        targets = range(len(self.rows))[i] if isinstance(i, slice) else [i]
        for r in targets:
            self.rows[r][j] = value

    def tolist(self):
        return [list(r) for r in self.rows]


def is_ellipsis(x) -> bool:
    return x is Ellipsis


def is_not_implemented(x) -> bool:
    return x is NotImplemented


def make_slice():
    return slice(1, None, 2)
