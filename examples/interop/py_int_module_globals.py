# py_int_module_globals.ar の補助（pandas_import_plan.md の 1-4）。モジュールの大域を関数が書き換える。
count: int = 0
label: str = "start"
untyped = 3


def bump() -> None:
    global count
    count += 1


def relabel(text: str) -> None:
    global label
    label = text
