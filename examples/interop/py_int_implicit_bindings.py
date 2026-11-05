"""import[py] の暗黙の束縛の試験（examples/interop/py_int_implicit_bindings.ar・pandas_import_plan.md の D-11・2-10）。"""


class Frame:
    def __init__(self, n: int) -> None:
        self.n: int = n
        self.cols: dict = {}

    def __setitem__(self, key: str, value: int) -> None:
        self.cols[key] = value

    def add(self, k: int) -> None:
        self.n += k

    def width(self) -> int:
        return len(self.cols)


class Failed(Exception):
    def __init__(self, msg: str) -> None:
        super().__init__(msg)
        self.retries: int = 0


def chunks() -> list[Frame]:
    return [Frame(1), Frame(2)]


def stream():
    # 戻り値の注釈が無いので、Arrow からは要素の型が分からない（実行時に検査する形）
    yield Frame(10)
    yield Frame(20)


def boom() -> None:
    raise Failed("boom")
