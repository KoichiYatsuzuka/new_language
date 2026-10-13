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
| `len(x)` / `repr(x)` / `id(x)` / `next(it)` | 長さ・`repr`・同一性・イテレータの次。文字列の `repr` は CPython と同じ（下の「文字列の `repr`」） |
| `enumerate(it, start=0)` / `zip(*its)` | 番号付き・組の列（遅延のジェネレータも最後まで回す） |
| `open(path, FileOpenMode.read)` | ファイルを開く（Arrow の形。下の「`open`」） |
| `close(f)` / `getenv(name)` / `parse_ar(src[, path])` | ファイルを閉じる・環境変数・Arrow のソースを構文木の値にする |

### 文字列の `repr`

`repr(s)`・コンテナの中の文字列の表示（`print([s])`）・`KeyError` の文言・`ascii(s)` は、CPython 3.12 の `repr` と同じ形です:

```ar
print(repr("it's"))            # "it's"（' を含み " を含まなければ " で囲む）
print(["a\nb", "c\\d"])        # ['a\nb', 'c\\d']（\t / \n / \r / \ / 引用符をエスケープ）
print(repr("x" + chr(0xa0)))   # 'x\xa0'（表示できない文字は \xhh / \uhhhh / \Uhhhhhhhh）
print(repr("日本"))             # '日本'（表示できる非 ASCII はそのまま）
```

- 「表示できる」の判定は CPython の `str.isprintable()` と同じ表（`ops/unicode_printable.rs`。CPython 3.12.2 から生成）で、
  `s.isprintable()` も同じ表を使います。

### 文字列の長さと位置

文字列の長さと位置は**文字の単位**です（添字 `s[i]`・切り出し `s[i:j]`・`for c in s` と同じ・CPython と同じ・
python_builtins_plan.md のタスク 1-17）:

```ar
let s = "γάμμα-μ"
print(len(s), s.find("μ"), s.rfind("μ"), s.find("μ", -2))   # 7 2 6 6
print(s[:s.find("-")])                                      # γάμμα
print("-42".zfill(6), repr("ab\tc".expandtabs(4)))          # -00042 'ab  c'
```

- `find` / `rfind` / `index` / `rindex` / `count` / `startswith` / `endswith` の `start` / `end` は文字の番号（負は後ろから・
  `None` は省略と同じ）。`startswith` / `endswith` は文字列の組も受ける（どれかに当たれば真）
- `zfill` / `ljust` / `rjust` / `center` / `expandtabs` / `%5s` / `%.3s` / `format` の幅・精度も文字の数
- ⚠ 以前は `len` / `find` などがバイトの単位で（`len("γάμμα")` が 10）、文字の途中の位置を渡すと落ちていた
- バイトの単位は `bytes`（`s.encode()`）で扱う

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
- 失敗は OS のエラーの種類で `FileNotFoundError` / `PermissionError` / … に振り分ける（[07_exceptions.md](07_exceptions.md)）

### ファイルのメソッド

どのファイル（Arrow の形でも Python の形でも）にも次があります（python_builtins_plan.md のタスク 5-4）:

| 名前 | 意味 |
|---|---|
| `readline(size=-1)` | 1 行（`\n` を含む）。終わりでは `""`（誤りにしない）。`size` はテキストなら文字の数 |
| `readlines(hint=-1)` / `writelines(xs)` | 残りの行の list ／ 順に書く（改行は足さない） |
| `for line in f` | 取り出すたびに `readline()` を呼ぶ（途中で抜けて続きを読める）。`iter(f.readline, "")` も同じ |
| `close()` / `flush()` / `tell()` | 閉じる（何度でも）／閉じずにディスクへ書き戻す／位置（バイト） |
| `closed` / `name` / `mode` | 閉じたか／パス／モードの文字列（`mode` は Python の形で開いたときだけ） |
| `read_line()` / `read_letter()` / `write_line(s)` | Arrow の名前（終わりで `EOFError`・`backward=True` で逆向き） |

同じ名前で意味の違う `read()` / `write()` は**開き方で決まります**:

| | Arrow の形（`open(p, FileOpenMode.read)`） | Python の形（`open(p)` / `open(p, "w")`） |
|---|---|---|
| `read()` の終わり | `EOFError` | `""`（`read(n)` は n 文字・n バイト） |
| `write(s)` の戻り値 | `None` | 書いた数（文字・バイト） |
| バイト列（`b` / `ByteRecognizingMode.byte`） | `list[int]` | `bytes` |
| 閉じたファイル | `IOError: I/O operation on closed file` | `ValueError: I/O operation on closed file.` |
| 読めない／書けないモード | 読みはどのモードでも可 | `"w"` で読むと `OSError: not readable`、`"r"` で書くと `OSError: not writable` |

- ⚠ CPython の「読めない／書けない」は `io.UnsupportedOperation`（`OSError` と `ValueError` の両方を継承）。Arrow の例外の
  表は単一継承なので `OSError` にした（`except OSError` で捕まえられるのは同じ）。
- ⚠ `seek` は無い（`"r+"` も受けない。Arrow の `write` は位置へ**挿入**するので、位置を動かせると CPython の上書きと違う）。
- ⚠ Arrow の形の `read()` が出す `EOFError` は、Arrow の例外の表に無い名前なので `except` で捕まえられない（タスク 4-5）。

---

## 型の値（呼んで作る）

`int` / `uint` / `float` / `str` / `bytes` / `bool` / `complex` / `list` / `tuple` / `dict` / `set` / `frozenset` / `type`

- `list(it)` / `tuple(it)` / `set(it)` / `frozenset(it)` は回せる値なら何でも受ける（遅延のジェネレータ・辞書のキーも）
- `frozenset`（python_builtins_plan.md のタスク 3-1）は不変でハッシュできる集合。辞書のキー・集合の要素にできる。
  演算子 `|` `&` `-` `^` は set と組んでよく結果は**左の種類**、`frozenset({1}) == {1}` は真、`<=` / `<` は包含関係。
  メソッドは書き換えないものだけ（`union` / `intersection` / `difference` / `symmetric_difference` / `issubset` /
  `issuperset` / `isdisjoint` / `copy`）。`add` などは CPython と同じ `AttributeError`。表示は `frozenset({1, 2})`。
  ⚠ 集合の並びは作った順（CPython はハッシュの順。`set` と同じ差）
- `bool(x)` は `if x:` と同じ規則（`__bool__` も見る。空の tuple / dict / frozenset は偽）

### `bytes`

不変のバイト列（python_builtins_plan.md のタスク 3-1）。結果・誤りの文言は CPython 3.12 と同じです。

```ar
let b: bytes = "héllo".encode()          # b'h\xc3\xa9llo'（encode の既定は utf-8）
print(b[0], b[1:3], len(b))              # 104 b'\xc3\xa9' 6
print(b.decode(), bytes([65, 66]))       # héllo b'AB'
print(bytes(3), bytes("abc", "ascii"))   # b'\x00\x00\x00' b'abc'
```

- 作る: `bytes()` / `bytes(n)`（0 が n 個）/ `bytes(iterable)`（0〜255 の int）/ `bytes(s, encoding[, errors])` /
  `s.encode(encoding="utf-8", errors="strict")`。⚠ Arrow のソースに `b".."` のリテラルは無い（Python のモジュールの
  `b".."` は変換器が `bytes([..])` に写す）
- 符号化は `utf-8` / `ascii` / `latin-1`（別名も・誤りの文言まで CPython と同じ）、`utf-16-le` / `utf-16-be`
  （BOM は付けも読みもしない。`utf-16` はバイト順が決まらないので誤り）、`shift_jis`（`cp932`）とそのほかの WHATWG の名前
  （`euc-jp` / `gbk` …）。`errors` は `strict` / `ignore` / `replace`（どの符号化でも）。失敗は
  `UnicodeDecodeError` / `UnicodeEncodeError`（`'utf-8' codec can't decode byte 0xff in position 0: invalid start byte`）。
  知らない符号化は `LookupError: unknown encoding: ..`（本体は `src/interpreter/ops/codecs.rs`）
- 演算子: `+`（bytes どうし。`b + "x"` は静的な誤り）/ `*` int / `in`（int か部分列の bytes）/ 大小は辞書順 / `==`
- メソッド: `decode` / `hex` / `count` / `find` / `rfind` / `index` / `rindex` / `startswith` / `endswith` / `replace` /
  `split` / `join` / `strip` / `lstrip` / `rstrip` / `upper` / `lower` / `isdigit` / `isalpha` / `isalnum` / `isspace` /
  `isascii` / `isupper` / `islower`
- 表示は `b'..'`（`str(b)` も同じ。引用符の選び方・`\xhh` のエスケープは CPython と同じ）
- ⚠ 無いもの: `bytearray` / `memoryview`、`bytes.fromhex`、`%` の書式、`partition` などの残りのメソッド。
  ⚠ Python の形で開いたファイルの `"rb"` の読みは今も int の list（タスク 5-4）
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
| `property(fget=None, fset=None, fdel=None, doc=None)` | クラス。`getter` / `setter` / `deleter` は写した property を返す。クラス側に置くとディスクリプタとして働く（[06_classes_traits.md](06_classes_traits.md) の「ディスクリプタ」）。Python の `@property` / `@x.setter` は変換器がこれで組み立てる（[09_imports.md](09_imports.md)）。⚠ Arrow で書いたクラス（`eval/py_builtins.rs` の `PY_PROPERTY_SRC`）で、名前を初めて引いたときに組み立てる |
| `cached_property(func)` | クラス（`functools.cached_property`・タスク 6-5）。Python の `@cached_property` は変換器がこれで組み立てる |
| `format(value, format_spec='')` | 書式指定の小言語（下の節）。`str` を返す |

### `format` と書式指定の小言語

`format(v, spec)` / f-string の `{v:spec}` / `"{:spec}".format(v)` は同じ解釈です（python_builtins_plan.md のタスク 3-4・
実装は `interpreter/ops/format_spec.rs`）。CPython 3.12 と同じ規則・同じ誤りの文言:

```text
[[fill]align][sign][z][#][0][width][grouping][.precision][type]
align: < > ^ =    sign: + - (空白)    z: 負の 0 を 0 に    #: 0b/0o/0x・小数点を残す
grouping: , _（_ は 2/8/16 進なら 4 桁ごと）
type: int → b c d o x X n（e f g % は float にして書く）／float → e E f F g G n %（無し: repr、精度つきなら g の変形）／str → s
```

```ar
print(format(1234567, ","), format(255, "#010x"), format(3.14159, ".2f"))   # 1,234,567 0x000000ff 3.14
print(format(12345.678, "e"), format(0.25, ".1%"), format("abc", "*^7"))    # 1.234568e+04 25.0% **abc**
print("{0.x} {1[k]} {2!r:>6}".format(p, {"k": 1}, "s"))                     # 属性・添字・変換
```

- `bool` は空の指定なら `True` / `False`、それ以外は整数として書く（`format(True, ">3")` は `  1`）。
- クラスは `fn __format__(self, spec: str) -> str` で書式を決められる（無ければ空の指定のときだけ `str(x)`、ほかは
  `TypeError: unsupported format string passed to C.__format__`）。
- `str.format` は位置・キーワード・属性（`{0.name}`）・添字（`{0[1]}` / `{d[key]}`）・変換（`!r` / `!s` / `!a`）・
  書式の中の置換フィールド（`{:{w}}`）を受ける。自動と手動の番号は混ぜられない（CPython と同じ文言）。
- `%` 演算子の `%e` / `%g` も同じ綴り（`1.234568e+04`）。
- ⚠ `complex` の書式指定は未対応（`ValueError: format specifiers for complex numbers are not supported`）。

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

CPython 3.12 の組み込み 153 名のうち、2026-10-12 の時点で次の 38 名は Arrow にありません（`property` は 6-1、`format` は 3-4、`frozenset` /
`bytes` / `UnicodeDecodeError` / `UnicodeEncodeError` は 3-1 で足した）:

`vars` `dir` `globals` `locals` `eval` `exec` `compile` `__import__` `__build_class__` `__debug__`
`input` `exit` `quit` `help` `breakpoint` `copyright` `credits` `license` `delattr` `aiter` `anext`
`bytearray` `memoryview` `object` `staticmethod` `classmethod` `super`
`NotImplemented` `Ellipsis` `BaseExceptionGroup` `ExceptionGroup` `EOFError` `ImportError`
`ModuleNotFoundError` `KeyboardInterrupt` `MemoryError` `StopAsyncIteration` `SystemExit`

- **Arrow のソース**で参照すると、型検査が未定義の名前として静的に弾きます。
- **Python のモジュール**（`import[py]`）で参照すると、読み込む時点で変換の誤りになります。呼ばれない関数の中でも
  同様です（[09_imports.md](09_imports.md) の「Python モジュール」）。
- ⚠ `staticmethod` / `classmethod` はデコレータとして、`object` はクラスの基底として、`super` は `super().m(..)` の形でなら
  変換器が受けます。
