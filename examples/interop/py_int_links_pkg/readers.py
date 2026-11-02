"""注釈でだけ使われるよそのクラス（pandas の `TextFileReader` の役）。"""
from .frames import Frame


class Reader:
    def __init__(self, path: str) -> None:
        self.path = path

    def get(self) -> Frame:
        return Frame(1)
