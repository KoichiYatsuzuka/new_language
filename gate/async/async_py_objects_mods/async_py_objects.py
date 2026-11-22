# async_py_objects.py — async_py_objects.ar から CPython で使うモジュール（pandas_import_plan.md の 2-8）。
import threading


class Counter:
    def __init__(self):
        self.n = 0
        self.threads = set()

    def bump(self):
        self.n += 1
        self.threads.add(threading.get_ident())
        return self.n


def call(f):
    """Arrow の関数を Python から呼ぶ（その中で async のタスクを待つ）。"""
    return f()


def main_thread_id():
    return threading.get_ident()


class Box:
    """Arrow のインスタンスを持ち続け、後で属性を読む。"""

    def __init__(self, item):
        self.item = item

    def read(self):
        return self.item.x
