"""py_object.py - 組み込みのクラス object と番兵（python_builtins_plan.md のタスク 6-10）。"""

MISSING = object()
OTHER = object()


class Config:
    def __init__(self, timeout=MISSING):
        self.timeout = timeout
        self.cache = {"t": timeout}

    def has_timeout(self):
        return self.timeout is not MISSING


class Base(object):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.ready = True


class Plain:
    def __init__(self):
        super().__init__()
        self.n = 1


class Extra:
    def __init__(self, a):
        super().__init__(a)


def pick(x=MISSING):
    return x


def run():
    # フィールド・辞書の値・リストを通っても同じ物（写さない）
    c = Config()
    print(c.timeout is MISSING, c.cache["t"] is MISSING, c.has_timeout(), Config(3).has_timeout())
    xs = []
    xs.append(MISSING)
    print(xs[0] is MISSING, MISSING in xs, pick() is MISSING, pick(None) is MISSING)
    # 同一性で比べる（別々の object() は等しくない・辞書のキーも別）
    print(MISSING == MISSING, MISSING == OTHER, MISSING != OTHER, MISSING is OTHER)
    d = {MISSING: "missing", OTHER: "other"}
    print(len(d), d[MISSING], d[OTHER], OTHER in [MISSING], MISSING in {MISSING})
    print(type(MISSING).__name__, type(MISSING) is object, hash(MISSING) == hash(MISSING), id(MISSING) == id(OTHER))
    print(repr(MISSING).startswith("<object object at 0x"), str(MISSING) == repr(MISSING))
    # すべての値は object の派生
    print(isinstance(1, object), isinstance("s", object), isinstance(None, object), isinstance(c, object))
    print(issubclass(Config, object), issubclass(int, object), object, type(object))
    # 基底が object のクラスの super().__init__()
    print(Base().ready, Base(**{}).ready, Plain().n)
    for bad in (lambda: Extra(1), lambda: setattr(OTHER, "x", 1), lambda: object(1)):
        try:
            bad()
        except (TypeError, AttributeError) as e:
            print(type(e).__name__ + ":", e)
