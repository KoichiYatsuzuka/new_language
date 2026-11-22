# py_file_methods.py — Python のファイルのメソッド（python_builtins_plan.md のタスク 5-4）。
#
# 使う側: examples/interop/py_file_methods.ar（CPython と同じ出力）


def write_sample(path):
    # newline="": CPython も改行を変換しない（Windows の既定は \n を \r\n にする。Arrow は変換しない）
    f = open(path, "w", encoding="utf-8", newline="")
    n = f.write("alpha\n")
    f.writelines(["beta\n", "γάμμα\n", "delta"])
    print(n, f.tell(), f.closed, f.mode, f.name.endswith(".txt"))
    f.close()
    f.close()  # 2 度目も誤りにしない
    print(f.closed)


def read_sample(path):
    with open(path, encoding="utf-8") as f:
        print(repr(f.readline()), repr(f.readline(3)), repr(f.readline()))
        print(f.readlines())
        print(repr(f.read()), repr(f.readline()), f.readlines())


def iterate(path):
    with open(path, encoding="utf-8") as f:
        i = 0
        for line in f:
            print(i, line.rstrip("\n"))
            if i == 1:
                break
            i += 1
        # 行の途中で抜けても、続きを readline で読める（1 行ずつ進む）
        print(repr(f.readline()))
    with open(path, encoding="utf-8") as f:
        print([line.rstrip("\n") for line in f], f.read(2) == "")
    with open(path, encoding="utf-8") as f:
        print(repr(f.read(3)), repr(f.read(4)), f.tell())
        print(list(iter(f.readline, ""))[-1])


def binary(path):
    with open(path, "wb") as f:  # 同じ関数の中で続けて読む（with の終わりで閉じて書き戻す）
        print(f.write(b"ab\ncd"))
    with open(path, "rb") as f:
        print(f.readline(), f.read(), f.read())


def errors(path):
    f = open(path, encoding="utf-8")
    f.close()
    for action in ("read", "readline", "iterate"):
        try:
            if action == "read":
                f.read()
            elif action == "readline":
                f.readline()
            else:
                for _ in f:
                    pass
        except ValueError as e:
            print(action, e)
    g = open(path, "a", encoding="utf-8")
    try:
        g.read()
    except OSError as e:
        print("OSError", e)
    try:
        g.write(3)
    except TypeError as e:
        print(e)
    g.close()


def run(path):
    write_sample(path)
    read_sample(path)
    iterate(path)
    errors(path)
    binary(path)
