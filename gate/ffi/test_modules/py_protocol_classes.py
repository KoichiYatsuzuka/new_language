# py_protocol_classes.py — `typing.Protocol` のクラス（examples/interop/py_protocol_classes.ar）

from typing import Iterator, Protocol, TypeVar

_T_co = TypeVar("_T_co", covariant=True)


class SupportsRead(Protocol[_T_co]):
    """`read(n)` で `_T_co` を返す物（型変数つきの protocol）。"""

    def read(self, n: int) -> _T_co: ...


class _Closer(Protocol):
    """`close()` を持つ物（名前が `_` で始まる protocol）。"""

    def close(self) -> None: ...


class _Nested(Protocol[_T_co]):
    """自分を参照する protocol（numpy の `_NestedSequence` と同じ形）。"""

    def __len__(self, /) -> int: ...

    def __getitem__(self, index: int, /) -> "_T_co | _Nested[_T_co]": ...

    def __iter__(self, /) -> "Iterator[_T_co | _Nested[_T_co]]": ...


class StrReader:
    def read(self, n: int) -> str:
        return "x" * n

    def close(self) -> None:
        pass


class Nothing:
    pass


def take(r: SupportsRead[str], n: int) -> str:
    return r.read(n)


def shut(c: _Closer) -> int:
    c.close()
    return 1


def depth(xs: _Nested[int]) -> int:
    first = xs[0]
    return 1 + depth(first) if hasattr(first, "__len__") else 1


def reader() -> StrReader:
    return StrReader()


def nothing() -> Nothing:
    return Nothing()
