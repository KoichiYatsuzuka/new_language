"""py_bytes.py - bytes のリテラル b".." と bytes / frozenset の組み込み（python_builtins_plan.md のタスク 3-1）。"""

MAGIC = b"\x89PNG\r\n"


def header_ok(data, sig=b"\x89PNG"):
    return data.startswith(sig)


def run():
    print(MAGIC, len(MAGIC), header_ok(MAGIC), header_ok(b"GIF89a"))
    line = b"name=caf\xc3\xa9; tags=a,b"
    fields = line.split(b"; ")
    print(fields, [f.split(b"=", 1)[0].decode() for f in fields])
    print(fields[0].split(b"=")[1].decode("utf-8"), b"caf\xe9".decode("latin-1"))
    try:
        b"caf\xe9".decode()
    except UnicodeDecodeError as e:
        print("UnicodeDecodeError:", e)
    seen = frozenset(fields[1].split(b"=")[1].split(b","))
    print(sorted(seen), b"a" in seen, {seen: "tags"}[frozenset([b"b", b"a"])])
    return b"".join([MAGIC[:4], b"!"])
