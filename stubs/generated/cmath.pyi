# cmath.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/cmath.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from typing import Final, SupportsComplex, SupportsFloat, SupportsIndex, TypeAlias
e: Final[float]
pi: Final[float]
inf: Final[float]
infj: Final[complex]
nan: Final[float]
nanj: Final[complex]
tau: Final[float]

def acos(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def acosh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def asin(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def asinh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def atan(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def atanh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def cos(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def cosh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def exp(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def isclose(a: SupportsFloat | SupportsComplex | SupportsIndex | complex, b: SupportsFloat | SupportsComplex | SupportsIndex | complex, *, rel_tol: SupportsFloat=1e-09, abs_tol: SupportsFloat=0.0) -> bool:
    ...

def isinf(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> bool:
    ...

def isnan(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> bool:
    ...

def log(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, base: SupportsFloat | SupportsComplex | SupportsIndex | complex=..., /) -> complex:
    ...

def log10(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def phase(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> float:
    ...

def polar(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> tuple[float, float]:
    ...

def rect(r: SupportsFloat | SupportsIndex, phi: SupportsFloat | SupportsIndex, /) -> complex:
    ...

def sin(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def sinh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def sqrt(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def tan(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def tanh(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> complex:
    ...

def isfinite(z: SupportsFloat | SupportsComplex | SupportsIndex | complex, /) -> bool:
    ...
