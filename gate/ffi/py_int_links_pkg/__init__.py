"""py_int_links.ar の補助のパッケージ: 型の再エクスポート・よそのモジュールのクラス・ほかのファイルの基底・@overload。

pandas と同じ形（`pandas/__init__.py` が `from pandas.core.frame import DataFrame` を再エクスポートし、`read_csv` は
`@overload` で戻り値の型が変わり、`DataFrame` のメソッドの多くは別のファイルの基底 `NDFrame` にある）を小さく真似る
（pandas_import_plan.md の 1-3）。
"""
from .frames import Frame, read
from .readers import Reader
