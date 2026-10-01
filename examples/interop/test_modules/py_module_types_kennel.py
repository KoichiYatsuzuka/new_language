# py_module_types.ar から import する Python のモジュール。
# 別のモジュールのクラスを継承する（Corgi）のと、別のモジュールと**同じ名前**のクラス（Dog）を持つ。

from test_modules.py_module_types_zoo import Dog as ZooDog


class Corgi(ZooDog):
    def kind(self):
        return "corgi"


class Dog:
    def kind(self):
        return "kennel dog"
