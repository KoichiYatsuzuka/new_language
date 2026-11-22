"""py_property_typed.py - getter の戻り値の注釈が property の型になる（python_builtins_plan.md のタスク 6-8）。"""


class Sensor:
    def __init__(self, celsius, readings):
        self._c = celsius
        self._readings = list(readings)

    @property
    def celsius(self) -> float:
        return self._c

    @celsius.setter
    def celsius(self, value: float):
        self._c = value

    @property
    def readings(self) -> list[int]:
        return self._readings

    @property
    def label(self) -> str:
        return "sensor"

    @property
    def raw(self):
        # 注釈の無い getter は Any のまま（Arrow で演算するには型を確かめる）
        return self._c
