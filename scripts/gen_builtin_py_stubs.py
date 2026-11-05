"""`.py` の無い Python のモジュールの型スタブを、typeshed と CPython の内省で作る（pandas_import_plan.md の 1-4・1-6）。

`import[py] sys` の型は、そのモジュールの `.pyi` / `.py` から抽出する。C で書かれたモジュール（`sys`・`itertools`・
`_collections` などの built-in と、標準ライブラリの拡張モジュール `select.pyd` など）には読むファイルが無いので、
Arrow に同梱する（`src/py_stubs.rs`）。ここはその同梱のスタブ `stubs/generated/<モジュール>.pyi` と、表
`src/py_stubs_generated.rs` を作り直す。

型の出所（D-10）:
- typeshed に `.pyi` のあるモジュールは **typeshed**（型が書いてある）。`typeshed_flatten.py` で、生成した CPython の版と OS に
  当てはまる分岐だけを残し、抽出器で読める形に畳む。`--typeshed <stdlib の置き場>` で渡す（既定は Pylance が同梱する
  typeshed-fallback）。ライセンス（Apache-2.0）は `stubs/generated/LICENSE-typeshed.txt` に写す
- typeshed に無いモジュール（`_collections` など）は CPython の内省:
- 関数・メソッド: `__text_signature__`（`inspect.signature`）で引数の名前・数・既定値の有無。型は書かれていないので `Any`。
  読めないもの（`__text_signature__` の無いもの）は `(*args, **kwargs)`（引数を問わない）。
- 大域変数・クラスの属性: 作ったときの値の型（`int` / `float` / `str` / `bool` / `bytes`、要素が揃った list / dict）。
  ⚠ `None` の値や、それ以外の値は `Any`（後で別の型が入りうる）。
- クラス: 基底を含むメンバーを平らにして書く（基底が別のモジュールでも、`object` のもの以外を全部持たせる）。

⚠ 手で書いたスタブ（`stubs/math.pyi`・`stubs/time.pyi`）のあるモジュールは作らない（戻り値の型を書いてあるので、そちらが正しい）。
⚠ CPython の版と OS で中身が変わる（`nt` / `winreg` は Windows だけ）。Linux だけのモジュール（`posix` / `fcntl` など）は
  ここでは作れない。使うときは `.pyi` を置く（利用者のファイルは同梱のスタブより先に読む）。

使い方: リポジトリのルートで `./scripts/gen_builtin_py_stubs.ps1 [-Typeshed <dir>]`（中でこのファイルを走らせる）。
"""
import argparse
import glob
import inspect
import importlib
import keyword
import os
import shutil
import sys
import types

import typeshed_flatten

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(ROOT, "stubs", "generated")
TABLE = os.path.join(ROOT, "src", "py_stubs_generated.rs")

# 手で書いたスタブがある
HAND_WRITTEN = {"math", "time"}
# 試験用・実験用のモジュール（使う人がいない）
EXCLUDED = {
    "xxsubtype", "_xxinterpchannels", "_xxsubinterpreters", "_ctypes_test", "_testbuffer", "_testcapi", "_testclinic",
    "_testconsole", "_testimportmultiple", "_testinternalcapi", "_testmultiphase", "_testsinglephase", "xxlimited",
    "xxlimited_35",
}
# 実行時に Python が呼ぶ属性のフック（Arrow のクラスは持てない・language-differences.md）と、型に関係しない仕組みのメソッド
SKIPPED_MEMBERS = {
    "__new__", "__init__", "__getattribute__", "__getattr__", "__setattr__", "__delattr__", "__init_subclass__",
    "__subclasshook__", "__reduce__", "__reduce_ex__", "__getstate__", "__setstate__", "__sizeof__", "__class_getitem__",
    "__dir__", "__format__", "__doc__", "__module__", "__dict__", "__weakref__", "__slots__", "__text_signature__",
    "__qualname__", "__name__",
}


def module_names():
    """作るモジュールの名前（built-in と、標準ライブラリの拡張モジュール）。"""
    names = set(sys.builtin_module_names)
    import _socket  # 標準ライブラリの拡張モジュールの置き場所（Windows は `DLLs`）

    ext_dir = os.path.dirname(_socket.__file__)
    for f in os.listdir(ext_dir):
        if f.endswith(".pyd") or f.endswith(".so"):
            names.add(f.split(".")[0])
    return sorted(n for n in names if n not in HAND_WRITTEN and n not in EXCLUDED)


def value_type(v):
    """値から型の注釈（`Any` に倒すものは `Any`）。"""
    if isinstance(v, bool):
        return "bool"
    for t in (int, float, str, bytes):
        if type(v) is t:
            return t.__name__
    if type(v) is list and v:
        elems = {value_type(e) for e in v}
        if len(elems) == 1 and "Any" not in elems:
            return f"list[{elems.pop()}]"
        return "list[Any]"
    if type(v) is dict and v:
        keys = {value_type(k) for k in v}
        vals = {value_type(e) for e in v.values()}
        k = keys.pop() if len(keys) == 1 else "Any"
        e = vals.pop() if len(vals) == 1 else "Any"
        return f"dict[{k}, {e}]"
    return "Any"


def params_text(fn, drop_first):
    """`inspect.signature` から引数の並び（型は `Any`・既定値は `...`）。読めなければ `None`。"""
    try:
        sig = inspect.signature(fn)
    except (ValueError, TypeError):
        return None
    out = []
    params = list(sig.parameters.values())
    if drop_first and params and params[0].kind in (params[0].POSITIONAL_ONLY, params[0].POSITIONAL_OR_KEYWORD):
        params = params[1:]
    # ⚠ `*args` の後の名前つき引数（`print(*args, sep=..)`）は、Arrow のスタブの変換器が読めない。`*args` を持つ関数は型検査が
    #   引数を問わない（`collect_module_types`）ので、それらを `**kwargs` にまとめても検査は変わらない
    if any(p.kind == p.VAR_POSITIONAL for p in params) and any(p.kind == p.KEYWORD_ONLY for p in params):
        params = [p for p in params if p.kind != p.KEYWORD_ONLY]
        if not any(p.kind == p.VAR_KEYWORD for p in params):
            params.append(inspect.Parameter("kwargs", inspect.Parameter.VAR_KEYWORD))
    saw_star = False
    for i, p in enumerate(params):
        if p.kind == p.VAR_POSITIONAL:
            out.append(f"*{p.name}: Any")
            saw_star = True
            continue
        if p.kind == p.VAR_KEYWORD:
            out.append(f"**{p.name}: Any")
            continue
        if p.kind == p.KEYWORD_ONLY and not saw_star:
            out.append("*")
            saw_star = True
        text = f"{p.name}: Any" + (" = ..." if p.default is not p.empty else "")
        out.append(text)
        nxt = params[i + 1] if i + 1 < len(params) else None
        if p.kind == p.POSITIONAL_ONLY and (nxt is None or nxt.kind != p.POSITIONAL_ONLY):
            out.append("/")
    return ", ".join(out)


def def_line(name, fn, indent, is_method, static=False):
    """1 つの関数・メソッドの `def` の行。"""
    text = params_text(fn, drop_first=is_method and not static)
    if text is None:
        text = "*args: Any, **kwargs: Any"
    if is_method and not static:
        text = "self" + (", " + text if text else "")
    return f"{indent}def {name}({text}) -> Any: ..."


def is_public(name):
    return not name.startswith("_") and name.isidentifier() and not keyword.iskeyword(name)


def is_dunder(name):
    return name.startswith("__") and name.endswith("__")


CALLABLE_DESCRIPTORS = (
    types.BuiltinFunctionType, types.FunctionType, types.MethodDescriptorType, types.WrapperDescriptorType,
    types.ClassMethodDescriptorType,
)


def class_lines(name, cls):
    """クラス 1 つ（基底のメンバーも平らにして持たせる）。"""
    lines = [f"class {name}:"]
    # コンストラクタ: クラスの `__text_signature__`（`itertools.count(start=0, step=1)`）
    init_text = params_text(cls, drop_first=False)
    if init_text is None:
        init_text = "*args: Any, **kwargs: Any"
    lines.append(f"    def __init__(self{', ' + init_text if init_text else ''}) -> None: ...")
    seen = set()
    for klass in cls.__mro__:
        if klass is object:
            break
        for mname, attr in sorted(klass.__dict__.items()):
            if mname in seen or mname in SKIPPED_MEMBERS or keyword.iskeyword(mname) or not mname.isidentifier():
                continue
            if mname.startswith("_") and not is_dunder(mname):
                continue
            # `object` から受け継いだメソッドはどの値でも同じなので書かない（`__eq__` / `__repr__` など）
            if is_dunder(mname) and getattr(object, mname, None) is attr:
                continue
            seen.add(mname)
            if isinstance(attr, (staticmethod, types.BuiltinFunctionType)) or isinstance(attr, classmethod) \
                    or isinstance(attr, types.ClassMethodDescriptorType):
                # 静的メソッド・クラスメソッド（`dict.fromkeys`）: 型スタブでは受け手を書かない
                fn = getattr(cls, mname, attr)
                lines.append("    @staticmethod")
                lines.append(def_line(mname, fn, "    ", True, static=True))
            elif isinstance(attr, CALLABLE_DESCRIPTORS):
                lines.append(def_line(mname, attr, "    ", True))
            elif isinstance(attr, (types.GetSetDescriptorType, types.MemberDescriptorType, property)):
                lines.append(f"    {mname}: Any")
            elif isinstance(attr, type):
                # 入れ子のクラスは書かない（抽出器も読まない）
                continue
            elif not callable(attr):
                lines.append(f"    {mname}: {value_type(attr)}")
    return lines


def module_stub(modname):
    """モジュール 1 つのスタブ（行の並び）。import できなければ `None`。"""
    try:
        mod = importlib.import_module(modname)
    except Exception:
        return None
    body = []
    for name in sorted(vars(mod)):
        if not is_public(name):
            continue
        obj = getattr(mod, name)
        if isinstance(obj, types.ModuleType):
            continue
        if isinstance(obj, type):
            # ⚠ `builtins` のクラス（`int` / `str` / 例外）は Arrow の型と同じ名前なので書かない（Arrow が持っている）
            if modname == "builtins":
                continue
            body.extend(class_lines(name, obj))
        elif callable(obj):
            body.append(def_line(name, obj, "", False))
        elif modname != "builtins":
            body.append(f"{name}: {value_type(obj)}")
    head = [
        f"# {modname}.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。",
        f"# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython {sys.version.split()[0]} / {sys.platform}）。",
        "from typing import Any",
        "",
    ]
    return head + body


def default_typeshed():
    """Pylance が同梱する typeshed（`typeshed-fallback`）の置き場。いちばん新しい版のもの。"""
    found = sorted(glob.glob(os.path.expanduser("~/.vscode/extensions/ms-python.vscode-pylance-*/dist/typeshed-fallback")))
    return found[-1] if found else None


def typeshed_stub(root, commit, modname):
    """typeshed の `stdlib` にあるモジュールのスタブ（行の並び）。無ければ `None`。"""
    stdlib = os.path.join(root, "stdlib")
    for path in (os.path.join(stdlib, f"{modname}.pyi"), os.path.join(stdlib, modname, "__init__.pyi")):
        if os.path.exists(path):
            with open(path, encoding="utf-8") as fh:
                src = fh.read()
            # ⚠ `builtins` のクラス（`int` / `str` / 例外）は Arrow の型と同じ名前なので書かない（内省と同じ）
            body = typeshed_flatten.flatten(src, skip_classes=(modname == "builtins"))
            head = [
                f"# {modname}.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。",
                f"# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython {sys.version.split()[0]} / {sys.platform}）。",
                f"# typeshed（https://github.com/python/typeshed・commit {commit}）の stdlib/{os.path.relpath(path, stdlib).replace(os.sep, '/')} を、",
                "# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。",
            ]
            return head + [body]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--typeshed", default=None, help="typeshed の置き場（stdlib と LICENSE のあるディレクトリ）")
    opts = ap.parse_args()
    ts_root = opts.typeshed or default_typeshed()
    if ts_root is None or not os.path.isdir(os.path.join(ts_root, "stdlib")):
        sys.exit("typeshed が見つからない（--typeshed で stdlib のあるディレクトリを渡す）")
    commit_file = os.path.join(ts_root, "commit.txt")
    commit = open(commit_file, encoding="utf-8").read().strip() if os.path.exists(commit_file) else "unknown"
    os.makedirs(OUT_DIR, exist_ok=True)
    for f in os.listdir(OUT_DIR):
        if f.endswith(".pyi"):
            os.remove(os.path.join(OUT_DIR, f))
    shutil.copyfile(os.path.join(ts_root, "LICENSE"), os.path.join(OUT_DIR, "LICENSE-typeshed.txt"))
    written = []
    skipped = []
    from_typeshed = []
    for modname in module_names():
        lines = typeshed_stub(ts_root, commit, modname)
        if lines is not None:
            from_typeshed.append(modname)
        else:
            lines = module_stub(modname)
        if lines is None:
            skipped.append(modname)
            continue
        with open(os.path.join(OUT_DIR, f"{modname}.pyi"), "w", encoding="utf-8", newline="\n") as fh:
            fh.write("\n".join(lines) + "\n")
        written.append(modname)
    table = [
        "// py_stubs_generated.rs — 生成した同梱スタブの表（scripts/gen_builtin_py_stubs.ps1 が作る。手で書き換えない）。",
        "// 中身と使い方は `py_stubs.rs` の冒頭。",
        "",
        "/// `.py` の無い Python のモジュールの、typeshed と CPython の内省で作ったスタブ（モジュール名 → `.pyi` の中身・名前の順）。",
        "pub(crate) const GENERATED_STUBS: &[(&str, &str)] = &[",
    ]
    for m in written:
        table.append(f'    ("{m}", include_str!("../stubs/generated/{m}.pyi")),')
    table.append("];")
    with open(TABLE, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("\n".join(table) + "\n")
    print(f"wrote {len(written)} stubs to {OUT_DIR} ({len(from_typeshed)} from typeshed {commit[:8]}, the rest by introspection)")
    if skipped:
        print("could not import (skipped):", ", ".join(skipped))


if __name__ == "__main__":
    main()
