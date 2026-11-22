# py_int_callbacks.ar が import[py] で使うモジュール（Arrow の関数を Python から呼ぶ・pandas_import_plan.md の 2-5）。
import threading


def apply(f, x):
    """渡された関数を 1 回呼ぶ。"""
    return f(x)


def apply_kw(f):
    """キーワード引数で呼ぶ。"""
    return f(1, scale=10)


def sort_by(xs, key):
    """組み込みの sorted に key として渡す（C の関数から Arrow へ呼び戻す）。"""
    return sorted(xs, key=key)


def map_all(f, xs):
    return list(map(f, xs))


handlers = []


def subscribe(f):
    """呼ぶ物を持ち続ける（後で呼ぶ）。"""
    handlers.append(f)


def fire(value):
    """持っている呼ぶ物を全部呼ぶ。"""
    return [h(value) for h in handlers]


def call_and_catch(f):
    """Arrow の関数が投げた例外を Python で捕まえる。"""
    try:
        f()
    except ValueError as e:
        return "ValueError: " + str(e)
    except Exception as e:
        return type(e).__name__ + ": " + str(e)
    return "no exception"


def call_through(f):
    """例外を捕まえずに通す（Arrow へ戻る）。"""
    return f()


def describe(f):
    return f.__name__ + " " + repr(f)


def make_adder(n):
    """Python の関数を Arrow へ返す（Arrow から Python の物として呼べる）。"""
    return lambda x: x + n


def call_from_thread(f):
    """別のスレッドから呼ぶ（Arrow の値はスレッドをまたげないので RuntimeError）。"""
    result = []

    def run():
        try:
            f()
            result.append("called")
        except RuntimeError as e:
            result.append("RuntimeError: " + str(e))

    t = threading.Thread(target=run)
    t.start()
    t.join()
    return result[0]


def call_nested(f, depth):
    """Arrow → Python → Arrow → Python … の入れ子。"""
    if depth == 0:
        return 0
    return f(depth) + 1
