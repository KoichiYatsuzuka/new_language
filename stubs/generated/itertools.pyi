# itertools.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/itertools.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import MaybeNone
from collections.abc import Callable, Iterable, Iterator
from types import GenericAlias
from typing import Any, Generic, Literal, SupportsComplex, SupportsFloat, SupportsIndex, SupportsInt, TypeAlias, TypeVar, overload
from typing_extensions import Self, disjoint_base

@disjoint_base
class count(Generic[_N]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __next__(self) -> Any:
        ...

    def __iter__(self) -> Self:
        ...

@disjoint_base
class cycle(Generic[_T]):

    def __init__(self, iterable: Iterable[Any], /) -> None:
        ...

    def __next__(self) -> Any:
        ...

    def __iter__(self) -> Self:
        ...

@disjoint_base
class repeat(Generic[_T]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __next__(self) -> Any:
        ...

    def __iter__(self) -> Self:
        ...

    def __length_hint__(self) -> int:
        ...

@disjoint_base
class accumulate(Generic[_T]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class chain(Generic[_T]):

    def __init__(self, *iterables: Iterable[Any]) -> None:
        ...

    def __next__(self) -> Any:
        ...

    def __iter__(self) -> Self:
        ...

    @classmethod
    def from_iterable(cls: type[Any], iterable: Iterable[Iterable[Any]], /) -> chain[Any]:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

@disjoint_base
class compress(Generic[_T]):

    def __init__(self, data: Iterable[Any], selectors: Iterable[Any]) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class dropwhile(Generic[_T]):

    def __init__(self, predicate: Callable[[Any], object][Any], iterable: Iterable[Any], /) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class filterfalse(Generic[_T]):

    def __init__(self, function: Callable[[Any], object][Any] | None, iterable: Iterable[Any], /) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class groupby(Generic[_T_co, _S_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> tuple[Any, Iterator[Any]]:
        ...

@disjoint_base
class islice(Generic[_T]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class starmap(Generic[_T_co]):

    def __init__(self, function: Callable[..., Any], iterable: Iterable[Iterable[Any]], /) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class takewhile(Generic[_T]):

    def __init__(self, predicate: Callable[[Any], object][Any], iterable: Iterable[Any], /) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

def tee(iterable: Iterable[Any], n: int=2, /) -> tuple[Iterator[Any], ...]:
    ...

@disjoint_base
class zip_longest(Generic[_T_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class product(Generic[_T_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class permutations(Generic[_T_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class combinations(Generic[_T_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class combinations_with_replacement(Generic[_T_co]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class pairwise(Generic[_T_co]):

    def __init__(self, iterable: Iterable[Any], /) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...

@disjoint_base
class batched(Generic[_T_co]):

    def __init__(self, iterable: Iterable[Any], n: int) -> None:
        ...

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> Any:
        ...
