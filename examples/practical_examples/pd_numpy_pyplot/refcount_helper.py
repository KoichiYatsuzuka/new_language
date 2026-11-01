"""refcount_pandas.ar の補助（pandas_import_plan.md の D-5・タスク 1-1）。

pandas / numpy が出した警告を記録し、refcount_pandas.ar と同じ形を CPython で書いたときの結果（`expected`）を返す。
期待する結果を決め打ちにしないのは、警告の種類が pandas の版で変わるため。
"""
import warnings

import numpy as np
import pandas as pd

_RECORDED = []


def _record(message, category, filename, lineno, file=None, line=None):
    _RECORDED.append(category.__name__)


def start() -> None:
    warnings.simplefilter("always")
    warnings.showwarning = _record


def take() -> list[str]:
    """記録した警告の種類の名前（重複を除いて並べ替えたもの）を返し、記録を空にする。"""
    names = sorted(set(_RECORDED))
    _RECORDED.clear()
    return names


def expected() -> dict[str, list[str]]:
    """refcount_pandas.ar と同じ形を CPython で書いたときの警告と、`resize` の結果。"""
    start()
    out = {}
    df = pd.DataFrame({"a": [1, 2, 3], "b": [4, 5, 6]})
    df["c"] = df["a"] * 2
    out["variable setitem"] = take()
    df["a"][0] = 99
    out["chained setitem"] = take()
    s = df["b"]
    s.fillna(0, inplace=True)
    out["variable inplace"] = take()
    df["b"].fillna(0, inplace=True)
    out["chained inplace"] = take()
    a = np.arange(3)
    try:
        a.resize(5)
        out["variable resize"] = ["ok"]
    except ValueError as e:
        out["variable resize"] = ["ValueError"]
    return out
