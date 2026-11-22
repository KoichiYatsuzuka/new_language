"""`from typing import Any` in a Python module: the annotation is Arrow's `Any`, not a class of this module."""
from typing import Any


def echo(x: Any) -> Any:
    return x


class Box:
    value: Any

    def __init__(self, value: Any) -> None:
        self.value = value

    def get(self) -> Any:
        return self.value

    def put(self, value: Any) -> None:
        self.value = value
