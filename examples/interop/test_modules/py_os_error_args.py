"""py_os_error_args.py — OSError の errno / strerror / filename と args（known_bugs.md の #9）。

`open` の失敗は CPython と同じ `[Errno 2] No such file or directory: 'p'`・`args == (errno, strerror)`・`filename` はパス。
`OSError(errno, strerror[, filename])` で作っても同じ（`filename` があれば `args` は最初の 2 つ）。
`run()` は CPython 3.12 と同じ出力（パスは相対で、リポジトリの根から実行する）。
"""

TMP = "examples/interop/_tmp_py_os_error.txt"


def show(label, e):
    print(label, type(e).__name__, "|", str(e), "|", repr(e), "|", e.args, e.errno, e.strerror, e.filename, e.filename2)


def run():
    # 1 open の失敗
    try:
        open("examples/interop/no_such_dir_kb9/nope.txt")
    except FileNotFoundError as e:
        show("1 missing:", e)
    try:
        open("examples/interop/no_such_dir_kb9/new.txt", "w")
    except OSError as e:
        show("1 missing dir:", e)
    f = open(TMP, "w")
    f.write("x")
    f.close()
    try:
        open(TMP, "x")
    except FileExistsError as e:
        show("1 exists:", e)

    # 2 作るとき
    # ⚠ 5 つの形（filename2）は Windows の CPython だけ `[WinError None] ..` になる（winerror の欄）ので書かない。
    for args in [(), ("m",), (2, "No such"), (2, "No such", "f"), (2, "x", None), (13, "denied", "p"), (999, "odd"),
                 (1, 2, 3, 4, 5, 6), (None, None), ("a", "b")]:
        show("2 " + repr(args) + ":", OSError(*args))
    show("2 subclass:", FileNotFoundError(2, "No such file or directory", "p"))
    show("2 one arg:", PermissionError("plain"))

    # 3 Python の派生クラス（super().__init__ で入れる）
    class PathError(OSError):
        def __init__(self, path):
            super().__init__(13, "denied", path)

    show("3 derived:", PathError("p"))

    # 4 投げて捕まえる
    try:
        raise FileNotFoundError(2, "No such file or directory", "q")
    except OSError as e:
        print("4 raised:", e.errno, e.filename, e)


def winerror():
    return OSError(2, "x", "f", 5)
