# py_getattr.ar から import する Python のモジュール（名前で属性を読み書きする）。


class Handler:
    def __init__(self):
        self.log = ""

    def on_start(self):
        self.log = self.log + "started;"

    def on_stop(self):
        self.log = self.log + "stopped;"

    def dispatch(self, event):
        # 名前を組み立ててメソッドを引く（getattr の典型的な使い方）。
        # ⚠ クラスから引いて self を渡す（Arrow にはまだ束縛メソッドが無い・python_builtins_plan.md の 1-8）。
        method = getattr(type(self), "on_" + event, None)
        if method is None:
            self.log = self.log + "unknown " + event + ";"
            return False
        method(self)
        return True


def run():
    h = Handler()
    results = [h.dispatch("start"), h.dispatch("pause"), h.dispatch("stop")]
    return results, h.log


def rename(h, new_log):
    setattr(h, "log", new_log)
    return getattr(h, "log"), hasattr(h, "dispatch"), hasattr(h, "missing")
