"""import[py] の例外の試験（examples/interop/py_int_exceptions.ar・pandas_import_plan.md の 2-1）。

組み込みの例外と、Python で定義した例外（組み込みの例外を継承するもの・同じファイルの例外を継承するもの）を投げる。
"""


class AppError(Exception):
    """Python で定義した例外（利用者の欄を持つ）。"""

    def __init__(self, message: str, code: int) -> None:
        super().__init__(message)
        self.code: int = code


class ParseFailed(ValueError):
    """組み込みの例外（ValueError）を継承する（pandas の ParserError と同じ形）。"""


class DeepFailed(ParseFailed):
    """同じファイルの例外を継承する。"""


def fail_value(why: str) -> int:
    raise ValueError(why)


def fail_key() -> int:
    d: dict[str, int] = {}
    return d["missing"]


def fail_os() -> None:
    open("no/such/file/py_int_exceptions.txt")


def fail_app(code: int) -> int:
    raise AppError("app failed", code)


def fail_parse() -> int:
    raise ParseFailed("bad row 3")


def fail_deep() -> int:
    raise DeepFailed("deeper")


class Plain:
    """例外ではないクラス（`except px.Plain:` は静的な誤り・py_int_exceptions_error.ar）。"""
