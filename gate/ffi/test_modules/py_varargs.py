"""py_varargs.py — `*args` / `**kwargs` を受ける関数（py_varargs.ar が CPython で動かす）。"""


# --- 1. `*args` の基本 ---
def va(*xs):
    out = []
    for v in xs:
        out.append(v)
    return out


def va_len(*xs):
    return len(xs)


def va_index(*xs):
    return xs[0]


def va_sum(*xs):
    t = 0
    for v in xs:
        t = t + v
    return t


# --- 2. 通常引数 + `*args` ---
def mixed(base, *rest):
    t = base
    for v in rest:
        t = t + v
    return t


# --- 3. `**kwargs` の基本 ---
def kw(**opts):
    return opts


def kw_len(**opts):
    return len(opts)


def kw_get(k, **opts):
    return opts[k]


def kw_keys(**opts):
    out = []
    for k in opts.keys():
        out.append(k)
    return out


# --- 4. 両方 ---
def both(a, *rest, **opts):
    n = 0
    for v in rest:
        n = n + v
    return (a, n, opts)


# --- 5. メソッドでも同じ ---
class C:
    def __init__(self, *xs):
        self.xs = []
        for v in xs:
            self.xs.append(v)

    def m(self, *ys):
        out = []
        for v in self.xs:
            out.append(v)
        for v in ys:
            out.append(v)
        return out

    def k(self, **o):
        return o


# --- 6. ★`*args` の中身は list（CPython は tuple） ---
def raw(*xs):
    return xs
