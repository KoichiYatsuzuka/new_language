# 組み込み関数

宣言なしで使える関数と型の値。どの名前が組み込みかの表は `src/type_check/names.rs` の
`RUNTIME_BUILTIN_NAMES`（型検査）と、実行時の表（`eval/builtins.rs` の `BUILTIN_VALUE_NAMES` /
`vm::compiler::VM_BUILTIN_NAMES`）。両者はテスト `checker_knows_every_runtime_builtin_name` が突き合わせる。
Python 互換の組み込みの本体は `src/interpreter/eval/py_builtins.rs`、計画と経緯は
[python_builtins_plan.md](../../implementation_plans/python_builtins_plan.md)。

---

## Arrow の組み込み

| 名前 | 意味 |
|---|---|
| `print(*values, sep=" ", end="\n", flush=False)` | 値を `str` にして `sep` でつなぎ、`end` を足して標準出力へ。`sep` / `end` は `None` か文字列（それ以外は `TypeError: end must be None or a string, not int`）。`file=` は `None` だけ |
| `range(stop)` / `range(start, stop[, step])` | 整数の列 |
| `len(x)` / `repr(x)` / `id(x)` / `next(it)` | 長さ・`repr`・同一性・イテレータの次 |
| `enumerate(it, start=0)` / `zip(*its)` | 番号付き・組の列（遅延のジェネレータも最後まで回す） |
| `open(path, FileOpenMode.read)` | ファイルを開く（Arrow の形。下の「`open`」） |
| `close(f)` / `getenv(name)` / `parse_ar(src[, path])` | ファイルを閉じる・環境変数・Arrow のソースを構文木の値にする |

### `open`

Arrow の形（`open(path, FileOpenMode.read, StartPoint.top, ByteRecognizingMode.text, Encoding.UTF_8)`）に加えて、
**Python の形**も受けます（2 番目が文字列か省略のとき）:

```ar
let w = open("out.txt", "w", encoding="utf-8")
let r = open("out.txt")              # mode は "r"
let b = open("out.txt", mode="rb")   # バイト列
```

- mode は CPython と同じ: `r` / `w`（作り直す）/ `x`（あれば `FileExistsError`）/ `a`（無ければ作って末尾へ）と
  `b` / `t` / `+`。誤りの文言も CPython と同じ（`invalid mode: 'q'` …）
- `encoding=` は UTF-8 / UTF-8-SIG / ASCII、`errors="strict"`、`newline=None / "" / "\n"`、`buffering` は受けて無視
- ⚠ `"r+"` は明示の `NotImplementedError`（Arrow の `write` は位置へ**挿入**する。CPython は上書き）
- ⚠ 改行は変換しない（CPython は Windows の既定で、書くときに改行を CR LF にする）
- ⚠ 返るのは Arrow のファイル（`read` / `read_line` / `write`）。`readline` / `readlines` / `close()` /
  `for line in f` はまだ無い（python_builtins_plan.md のタスク 5-4）
- 失敗は OS のエラーの種類で `FileNotFoundError` / `PermissionError` / … に振り分ける（[07_exceptions.md](07_exceptions.md)）

---

## 型の値（呼んで作る）

`int` / `uint` / `float` / `str` / `bool` / `complex` / `list` / `tuple` / `dict` / `set` / `type`

- `list(it)` / `tuple(it)` / `set(it)` は回せる値なら何でも受ける（遅延のジェネレータ・辞書のキーも）
- `dict()` / `dict(pairs)` / `dict(mapping)` / `dict(a=1)` を CPython と同じく受ける
- `type(x)` はクラスの値（`type(x).__name__` / `type(x) is C` / `type(x)(..)`）。3 引数の `type(name, bases, dict)` は `TypeError`

---

## Python 互換の組み込み

結果・誤りの文言は CPython 3.12 と同じです。

| 名前 | メモ |
|---|---|
| `isinstance(x, C)` / `issubclass(C, B)` | 第 2 引数は名前・`m.C`・変数・組。クラスは同一性と修飾名の祖先で見る。`isinstance(True, int)` は `True` |
| `getattr(o, name[, default])` / `hasattr` / `setattr` | 名前が変数の属性の読み書き（`o.name` と同じ経路）。メソッドは束縛メソッドが返る |
| `callable(x)` | 関数・クラス・束縛メソッド |
| `all` / `any` | |
| `min` / `max`（`key=` / `default=`） | 空で `default` が無ければ `ValueError: max() arg is an empty sequence` |
| `sum(it, start=0)` | float は CPython と同じ補正つきの和 |
| `sorted(it, key=, reverse=)` / `list.sort(key=, reverse=)` | 安定 |
| `reversed` / `abs` / `divmod` | |
| `round(x[, ndigits])` | 偶数への丸め（float は正確な 10 進の値で） |
| `pow(base, exp[, mod])` | `mod` つきは整数の冪剰余（負の指数は逆元） |
| `map(f, *its)` / `filter(f or None, it)` | **遅延**（取り出すたびに呼ぶ・無限の列にも使える） |
| `iter(x)` / `iter(f, sentinel)` | `for` と同じイテレータ／`f()` が `sentinel` になるまで |
| `hash` / `ord` / `chr` | `hash` は辞書・集合の鍵と同じ値 |
| `hex` / `oct` / `bin` / `ascii` | 接頭辞つき（`-0b101`）／非 ASCII を `\x..` に |

### 組み込みの名前を値として使う

束縛されていない組み込みの名前は値になります（名前の探索の最後の段・CPython の `builtins` と同じ位置）:

```ar
print(list(map(repr, [1, 2])))          # ['1', '2']
print(sorted(["bb", "a"], key=len))      # ['a', 'bb']
print(isinstance(x, (list, tuple)))
print(len)                               # <built-in function len>
```

---

## 未対応の組み込み

CPython 3.12 の組み込み 153 名のうち、2026-10-06 の時点で次の 44 名は Arrow にありません:

`format` `vars` `dir` `globals` `locals` `eval` `exec` `compile` `__import__` `__build_class__` `__debug__`
`input` `exit` `quit` `help` `breakpoint` `copyright` `credits` `license` `delattr` `aiter` `anext`
`bytes` `bytearray` `memoryview` `frozenset` `object` `property` `staticmethod` `classmethod` `super`
`NotImplemented` `Ellipsis` `BaseExceptionGroup` `ExceptionGroup` `EOFError` `ImportError`
`ModuleNotFoundError` `KeyboardInterrupt` `MemoryError` `StopAsyncIteration` `SystemExit`
`UnicodeDecodeError` `UnicodeEncodeError`

- **Arrow のソース**で参照すると、型検査が未定義の名前として静的に弾きます。
- **Python のモジュール**（`import[py]`）で参照すると、読み込む時点で変換の誤りになります。呼ばれない関数の中でも
  同様です（[09_imports.md](09_imports.md) の「Python モジュール」）。
- ⚠ `staticmethod` / `classmethod` はデコレータとして、`object` はクラスの基底として、`super` は `super().m(..)` の形でなら
  変換器が受けます。
