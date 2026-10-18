# py_int_functions.py - Python functions used from Arrow through import[py-int] (ffi_value_exchange_plan.md 7-30).
# Annotated parameters and returns get their types on the Arrow side; everything else is Any.
from typing import Optional


def scale(x: float, by: int = 2) -> float:
    return x * by


def greet(name: str, punct: Optional[str] = None) -> str:
    return "hello " + name + (punct or "")


def join(*parts: str, sep: str = "") -> str:
    return sep.join(parts)


def untyped(a, b=10):
    return a + b


def takes_alias(x: "SomeAlias") -> int:   # a name this file does not declare -> Any
    return len(x)
