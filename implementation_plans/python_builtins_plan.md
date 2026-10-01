# Python 変換: CPython 組み込みの対応計画

`import[py]` で変換した Python のコードの中で、**CPython の組み込み（`builtins` モジュールの名前）の多くが使えない**。
本書は、使えない組み込みの一覧と、実用上の損害（pandas を読み込んだときに認識されない箇所の数）を測り、
対応の順序を決めるための計画書である。

- 起票: 2026-09-30（フェーズ10 の 10-18 の作業中に `isinstance` が使えないことに気づいたのが発端。`type_check_redesign.md` とは別件）
- 測定: Arrow `ff4afe1`・CPython 3.12.2・pandas 2.2.0
- 分類（4 節）: 2026-10-01・Arrow `a16e0a1`（release ビルドで実際に動かして確かめた）

## 0. 何が起きるか

```python
# animals.py
class Animal:
    pass

class Dog(Animal):
    pass

def check():
    return isinstance(Dog(), Animal)
```
```
import[py] animals as a
print(a.check())      # NameError: 'isinstance' is not defined（CPython なら True）
```

- 変換器は Python の呼び出しを**名前のまま** Arrow の呼び出しに写す（`python_converter/expressions.rs` の `Call`）。
  Arrow の実行時にその名前の組み込みが無いと `NameError` になる。
- ⚠⚠ **失敗が遅い**: `import` の時点でも型検査の時点でも止まらず、**その行を実行したときに初めて** `NameError` になる。
  実行しない枝にあれば気づかないまま残る。
- Arrow で `isinstance` に当たるのは `x is Dog`（`Expr::IsType`）。右辺は**型の名前**を書く構文で、値ではない。

## 1. 調べ方（再現できるように）

### 1.1 組み込みの対応状況

CPython 3.12 の `dir(builtins)` の 158 名から、モジュールの属性（`__name__` / `__doc__` / `__package__` /
`__loader__` / `__spec__`）の 5 名を除いた **153 名**を対象にした。名前ごとに、Python の関数の中で

- 値として参照する（`return N`）
- 呼び出す（`return N()`。`input` / `help` / `exit` / `quit` / `breakpoint` / `copyright` / `credits` / `license` は呼ばない）

を `import[py]` 経由で実行し、どちらかで `NameError: 'N' is not defined` にならなければ「名前が通る」とした。
名前が通るものは、典型的な引数で CPython と結果を突き合わせた（1.3 の「部分的に対応」はここで見つけた）。

### 1.2 pandas の損害

- 対象: CPython で `import pandas` した後の `sys.modules` のうち、名前が `pandas` で始まり `.py` から読んだモジュール
  （**254 モジュール**）。ほかに C 拡張（`.pyd`）が **42 モジュール**読み込まれる（`pandas._libs.*`）。
- 各 `.py` を構文木で読み、**スコープを解決して組み込みを指す参照だけ**を数えた（仮引数・代入・import・def / class・
  for / with / except / walrus / match の束縛・`global` / `nonlocal` で隠れた名前は除く）。
- 型注釈（引数・戻り値・変数の注釈）は数えない（pandas は `from __future__ import annotations` で実行時に評価されない。
  変換器も注釈を式として扱わない）。docstring・コメントの中も数えない。
- 「認識されない箇所」の数え方は組み込みの状態で変える:
  - 未対応 … すべての参照
  - 呼び出しの形だけ解決される（`range` など）… 値としての参照（`print` はキーワード引数つきの呼び出しも）
  - `dict` … 呼び出し ／ `open` … すべて ／ `super` … `super().m(...)` 以外の形 ／ `staticmethod` / `classmethod` … デコレータ以外
  - `int` … 2 引数・`base=` の呼び出し

⚠ **この数は「組み込みの穴の大きさ」の指標**であって、埋めれば pandas が動くという意味ではない。pandas は
C 拡張 42 モジュール（`pandas._libs.*`）と numpy に依存しており、`import[py]`（Python のソースを変換する）では
そもそも読み込めない。未対応の構文・標準ライブラリも別にある。

⚠ 測定に使ったスクリプト（名前ごとの試験・pandas の集計・4 節の呼び出しの形の集計）は、まだリポジトリに入れていない。タスクを進めて数え直すときは `scripts/` に置く（規約どおり `.ps1` から呼ぶ形にする）。

## 2. 対応状況の一覧

| 状態 | 数 |
|---|---|
| 対応（名前が通り、試した範囲で CPython と同じ） | 30 |
| 部分的に対応（2.2） | 11 |
| **未対応**（名前が無い・2.1） | **112** |
| 計 | 153 |

### 2.1 未対応の組み込み

「pandas」は 1.2 の数え方で数えた、pandas（`import pandas` で読み込まれる .py）の中で認識されない箇所の数。

#### 関数（39）

| 名前 | pandas |
|---|---|
| `isinstance` | 2766 |
| `getattr` | 369 |
| `hasattr` | 162 |
| `all` | 113 |
| `any` | 98 |
| `issubclass` | 69 |
| `setattr` | 64 |
| `callable` | 59 |
| `max` | 51 |
| `sorted` | 37 |
| `min` | 35 |
| `iter` | 33 |
| `abs` | 26 |
| `hash` | 18 |
| `sum` | 15 |
| `ord` | 9 |
| `divmod` | 8 |
| `globals` | 5 |
| `chr` | 3 |
| `locals` | 2 |
| `round` | 2 |
| `__import__` | 1 |
| `dir` | 1 |
| `hex` | 1 |
| `pow` | 1 |
| `vars` | 1 |
| `__build_class__` | — |
| `aiter` | — |
| `anext` | — |
| `ascii` | — |
| `bin` | — |
| `breakpoint` | — |
| `compile` | — |
| `delattr` | — |
| `eval` | — |
| `exec` | — |
| `format` | — |
| `input` | — |
| `oct` | — |

#### 型（12）

| 名前 | pandas |
|---|---|
| `type` | 868 |
| `property` | 586 |
| `list` | 524 |
| `object` | 306 |
| `tuple` | 304 |
| `bytes` | 46 |
| `frozenset` | 45 |
| `map` | 34 |
| `reversed` | 15 |
| `memoryview` | 3 |
| `bytearray` | 2 |
| `filter` | — |

#### 定数（3）

| 名前 | pandas |
|---|---|
| `NotImplemented` | 50 |
| `Ellipsis` | 15 |
| `__debug__` | — |

#### 例外（40）

| 名前 | pandas |
|---|---|
| `ImportError` | 35 |
| `SyntaxError` | 12 |
| `LookupError` | 4 |
| `UnicodeDecodeError` | 4 |
| `FloatingPointError` | 3 |
| `FileNotFoundError` | 2 |
| `ModuleNotFoundError` | 2 |
| `UnicodeError` | 2 |
| `BufferError` | 1 |
| `PermissionError` | 1 |
| `UnicodeEncodeError` | 1 |
| `BaseException` | — |
| `BaseExceptionGroup` | — |
| `BlockingIOError` | — |
| `BrokenPipeError` | — |
| `ChildProcessError` | — |
| `ConnectionAbortedError` | — |
| `ConnectionError` | — |
| `ConnectionRefusedError` | — |
| `ConnectionResetError` | — |
| `EOFError` | — |
| `EnvironmentError` | — |
| `ExceptionGroup` | — |
| `FileExistsError` | — |
| `IndentationError` | — |
| `InterruptedError` | — |
| `IsADirectoryError` | — |
| `KeyboardInterrupt` | — |
| `MemoryError` | — |
| `NotADirectoryError` | — |
| `ProcessLookupError` | — |
| `ReferenceError` | — |
| `StopAsyncIteration` | — |
| `SystemError` | — |
| `SystemExit` | — |
| `TabError` | — |
| `TimeoutError` | — |
| `UnboundLocalError` | — |
| `UnicodeTranslateError` | — |
| `WindowsError` | — |

#### 警告（12）

| 名前 | pandas |
|---|---|
| `FutureWarning` | 234 |
| `DeprecationWarning` | 29 |
| `RuntimeWarning` | 16 |
| `UserWarning` | 15 |
| `Warning` | 15 |
| `ResourceWarning` | 3 |
| `UnicodeWarning` | 2 |
| `EncodingWarning` | 1 |
| `BytesWarning` | — |
| `ImportWarning` | — |
| `PendingDeprecationWarning` | — |
| `SyntaxWarning` | — |

#### 対話用（site）（6）

| 名前 | pandas |
|---|---|
| `copyright` | — |
| `credits` | — |
| `exit` | — |
| `help` | — |
| `license` | — |
| `quit` | — |

### 2.2 部分的に対応

| 組み込み | 何が足りないか | pandas で数えた形 | pandas |
|---|---|---|---|
| `dict` | 名前は通るが**呼べない**（`dict(a=1)` / `dict(pairs)` が `TypeError: 'dict' object is not callable`）。辞書の表示 `{..}`・`isinstance(x, dict)` の型としての使用は動く | 呼び出し | 50 |
| `open` | Arrow の `open` が呼ばれ、Python の mode 文字列（`open(p, "w")`）を受けない（`expected enum_item_FileOpenMode instance`） | すべて | 6 |
| `super` | `super().m(...)` の形だけ（変換器が脱糖する）。`super(C, self)` や `super()` 単独は変換の誤り | `super().m(...)` 以外 | 3 |
| `staticmethod` / `classmethod` | デコレータ（`@staticmethod`）としてだけ対応（変換器がフラグにする）。関数として呼ぶ・値として使うのは `NameError` | デコレータ以外 | 0 |
| `range` / `zip` / `enumerate` / `next` / `repr` / `print` | 呼び出しの形だけ解決される。**値として渡すと `NameError`**（`map(repr, xs)`・`key=len` の形） | 値としての参照 | 16 |
| `print` | キーワード引数つき（`print(x, end="")`）の関数は**丸ごとバイトコードにできない**（`VmForceError`） | キーワード引数つきの呼び出し | 0 |
| `int` | `int("ff", 16)`（基数の引数）が無い（`int() takes at most 1 argument`） | 2 引数・`base=` | 0 |
| 例外クラス（`ValueError` など 17 個） | 作れて捕まえられるが、`str(e)` がメッセージでなくオブジェクト表示、`e.args` が無い | （静的に数えられない） | — |
| `complex` | 表示が違う（`complex(1, 2)` が `(1.0+2.0j)`。CPython は `(1+2j)`） | （表示だけ） | — |

### 2.3 対応

`ArithmeticError`, `AssertionError`, `AttributeError`, `Exception`, `False`, `GeneratorExit`, `IOError`, `IndexError`, `KeyError`, `NameError`, `None`, `NotImplementedError`, `OSError`, `OverflowError`, `RecursionError`, `RuntimeError`, `StopIteration`, `True`, `TypeError`, `ValueError`, `ZeroDivisionError`, `bool`, `complex`, `float`, `id`, `int`, `len`, `set`, `slice`, `str`

⚠ 試したのは典型的な引数だけ。例外クラスと `complex` の違いは 2.2 に挙げた。

## 3. pandas での損害（多い順）

- **認識されない箇所: 計 7,204 箇所**
- **認識されない箇所を 1 つ以上含むモジュール: 187 / 254**

| # | 組み込み | 状態 | 認識されない箇所 | 参照の総数 | うち呼び出し | モジュール数 |
|---|---|---|---|---|---|---|
| 1 | `isinstance` | 未対応 | 2,766 | 2,766 | 2,766 | 155 |
| 2 | `type` | 未対応 | 868 | 868 | 846 | 107 |
| 3 | `property` | 未対応 | 586 | 586 | 9 | 73 |
| 4 | `list` | 未対応 | 524 | 524 | 332 | 98 |
| 5 | `getattr` | 未対応 | 369 | 369 | 369 | 87 |
| 6 | `object` | 未対応 | 306 | 306 | 1 | 77 |
| 7 | `tuple` | 未対応 | 304 | 304 | 113 | 71 |
| 8 | `FutureWarning` | 未対応 | 234 | 234 | 0 | 58 |
| 9 | `hasattr` | 未対応 | 162 | 162 | 162 | 69 |
| 10 | `all` | 未対応 | 113 | 113 | 113 | 49 |
| 11 | `any` | 未対応 | 98 | 98 | 98 | 48 |
| 12 | `issubclass` | 未対応 | 69 | 69 | 69 | 26 |
| 13 | `setattr` | 未対応 | 64 | 64 | 64 | 17 |
| 14 | `callable` | 未対応 | 59 | 59 | 59 | 33 |
| 15 | `max` | 未対応 | 51 | 51 | 50 | 26 |
| 16 | `NotImplemented` | 未対応 | 50 | 50 | 0 | 14 |
| 17 | `dict` | 部分（呼べない） | 50 | 172 | 50 | 26 |
| 18 | `bytes` | 未対応 | 46 | 46 | 11 | 21 |
| 19 | `frozenset` | 未対応 | 45 | 45 | 43 | 20 |
| 20 | `sorted` | 未対応 | 37 | 37 | 37 | 23 |
| 21 | `ImportError` | 未対応 | 35 | 35 | 17 | 20 |
| 22 | `min` | 未対応 | 35 | 35 | 34 | 20 |
| 23 | `map` | 未対応 | 34 | 34 | 34 | 21 |
| 24 | `iter` | 未対応 | 33 | 33 | 33 | 25 |
| 25 | `DeprecationWarning` | 未対応 | 29 | 29 | 0 | 17 |
| 26 | `abs` | 未対応 | 26 | 26 | 26 | 13 |
| 27 | `hash` | 未対応 | 18 | 18 | 18 | 11 |
| 28 | `RuntimeWarning` | 未対応 | 16 | 16 | 0 | 10 |
| 29 | `Ellipsis` | 未対応 | 15 | 15 | 0 | 6 |
| 30 | `UserWarning` | 未対応 | 15 | 15 | 0 | 11 |
| 31 | `Warning` | 未対応 | 15 | 15 | 0 | 3 |
| 32 | `range` | 部分（値にできない） | 15 | 235 | 220 | 11 |
| 33 | `reversed` | 未対応 | 15 | 15 | 15 | 12 |
| 34 | `sum` | 未対応 | 15 | 15 | 15 | 9 |
| 35 | `SyntaxError` | 未対応 | 12 | 12 | 8 | 5 |
| 36 | `ord` | 未対応 | 9 | 9 | 9 | 4 |
| 37 | `divmod` | 未対応 | 8 | 8 | 1 | 8 |
| 38 | `open` | 部分（引数の形） | 6 | 6 | 6 | 4 |
| 39 | `globals` | 未対応 | 5 | 5 | 5 | 1 |
| 40 | `LookupError` | 未対応 | 4 | 4 | 0 | 3 |
| 41 | `UnicodeDecodeError` | 未対応 | 4 | 4 | 0 | 4 |
| 42 | `FloatingPointError` | 未対応 | 3 | 3 | 0 | 1 |
| 43 | `ResourceWarning` | 未対応 | 3 | 3 | 0 | 2 |
| 44 | `chr` | 未対応 | 3 | 3 | 3 | 1 |
| 45 | `memoryview` | 未対応 | 3 | 3 | 1 | 3 |
| 46 | `super` | 部分（形） | 3 | 307 | 307 | 3 |
| 47 | `FileNotFoundError` | 未対応 | 2 | 2 | 2 | 2 |
| 48 | `ModuleNotFoundError` | 未対応 | 2 | 2 | 0 | 2 |
| 49 | `UnicodeError` | 未対応 | 2 | 2 | 0 | 2 |
| 50 | `UnicodeWarning` | 未対応 | 2 | 2 | 0 | 2 |
| 51 | `bytearray` | 未対応 | 2 | 2 | 0 | 2 |
| 52 | `locals` | 未対応 | 2 | 2 | 2 | 1 |
| 53 | `round` | 未対応 | 2 | 2 | 2 | 2 |
| 54 | `BufferError` | 未対応 | 1 | 1 | 0 | 1 |
| 55 | `EncodingWarning` | 未対応 | 1 | 1 | 0 | 1 |
| 56 | `PermissionError` | 未対応 | 1 | 1 | 0 | 1 |
| 57 | `UnicodeEncodeError` | 未対応 | 1 | 1 | 0 | 1 |
| 58 | `__import__` | 未対応 | 1 | 1 | 1 | 1 |
| 59 | `dir` | 未対応 | 1 | 1 | 1 | 1 |
| 60 | `hex` | 未対応 | 1 | 1 | 1 | 1 |
| 61 | `pow` | 未対応 | 1 | 1 | 0 | 1 |
| 62 | `repr` | 部分（値にできない） | 1 | 100 | 99 | 1 |
| 63 | `vars` | 未対応 | 1 | 1 | 1 | 1 |

- 「参照の総数」は組み込みを指す参照すべて（認識される形も含む）。「モジュール数」は認識されない箇所を含むモジュールの数。
- 上位 7 つ（`isinstance` / `type` / `property` / `list` / `getattr` / `object` / `tuple`）で全体の約 80% を占める。
- `property` の 576 箇所は `@property`（デコレータ）。変換器はこれを**変換の誤り**にするので、
  実行時の `NameError` より早く、**モジュールごと読み込めない**（Arrow にプロパティ構文が無い・`python_converter/decorators.rs`）。
- `@classmethod`（198）・`@staticmethod`（19）はデコレータとして変換器が扱うので数えていない。
- `super` は 307 箇所のうち 304 箇所が `super().m(...)` の形（対応済み）。

## 4. 既存の機能で書けるか（未対応 112 名の分類）

2.1 の 112 名を、**Arrow に今ある機能だけで書けるもの**と**新規実装が要るもの**に分けた。

- **既存の機能で書ける**（4.1）: 新しい実行時の仕組みが要らない。手段は 3 つ:
  1. 変換時に Arrow の式へ書き換える（`isinstance(x, C)` → `x is C`）
  2. Arrow か Python で書いた**前置きの関数**を置く（`meta_expand` の `PRELUDE` と同じ形）
  3. 既存の表に名前を足す（例外・警告クラス）
- **新規実装が要る**: Rust の側に呼び口・値・意味を足す。
  - **薄い**（4.2）: 中身は実行時に既にあり、Arrow から呼ぶ口が無いだけ
  - **厚い**（4.3）: 新しい値の種類か、新しい意味（手順）が要る
- `isinstance` / `getattr` / `hasattr` / `setattr` / `object` / `tuple` は**呼び出しの形で行き先が分かれる**。
  pandas の箇所は形ごとに数え直した（1.2 と同じ 254 モジュール。`isinstance` 2,766・`type` の呼び出し 846・
  `hasattr` 162・`setattr` 64・`issubclass` 69・`list` の呼び出し 332・`tuple` の呼び出し 113 が 3 節と一致した）。
  ⚠ `object` / `list` / `tuple` の呼び出し以外の内訳はスコープを解決せずに数えたので「約」。

### 4.0 まとめ

| 区分 | 名前 | pandas の箇所 |
|---|---|---|
| 既存の機能で書ける（4.1） | 64 | 約 3,960（55%） |
| 形で分かれる（`isinstance` / `getattr` / `hasattr` / `setattr` / `object` / `tuple`） | 6 | （形ごとに各区分へ振り分けた） |
| 新規・薄い（4.2） | 14 | 約 1,410（20%） |
| 新規・厚い（4.3） | 28 | 約 1,130（16%） |
| 相手の型が読めるか次第（4.4） | — | 627（9%） |
| 計 | 112 | 7,129（3 節の 7,204 から 2.2 の部分的な対応の 75 を除いた数） |

- 箇所で見ると**半分強は新しい仕組みなしで消せる**。大きいのは `isinstance` の名前の形（1,727）・`list(it)`（332）・
  `getattr` / `hasattr` の文字列リテラルの形（338）・警告クラス（315）。
- 新規の上位は `type(x)`（867・薄い）・`property`（586・厚い）・`isinstance` の pandas の ABC（298・厚い）・
  `getattr` / `hasattr` / `setattr` の名前が変数の形（225・薄い）。

### 4.1 既存の機能で書ける

#### 関数

| 名前 | pandas | 書き方 | CPython とずれる点・前提 |
|---|---|---|---|
| `isinstance(x, C)`（第 2 引数が組み込みの型名・クラス名・その組） | 1,727（組み込みの型 766・pandas のクラス 961） | `x is C`。組は `or` | ⚠ `isinstance(True, int)` は CPython で `True`、Arrow の `True is int` は `False`。`int` は `x is int or x is bool` に写す。⚠ Python のクラスの**修飾名**（`b is m.Box`）は今は静的な誤り（6 節） |
| `getattr(o, "名前")` / `getattr(o, "名前", d)` | 188（2 / 186） | `o.名前` / `AttributeError` を捕まえる `try` のブロック式 | 無い属性の読みは捕まえられる `AttributeError` になる |
| `hasattr(o, "名前")` | 150 | 同じ `try` のブロック式で `True` / `False` | 同上 |
| `setattr(o, "名前", v)` | 32 | `o.名前 = v` | クラス本体で宣言していない属性は作れない（`'Box' has no field 'zzz'`）。`setattr` に限らない既存の差 |
| `callable(x)` | 59 | `x is function`（関数・`__call__` を持つインスタンス／クラスで真） | ⚠ `__call__` を持たない**クラス**で `False`（CPython は `True`）。クラスかどうかの判定（4.2）が要る |
| `all` / `any` | 113 / 98 | 前置きの関数（`for` で途中で返す） | — |
| `max` / `min` | 51 / 35（2 引数 57・反復可能 27・値として 2） | 2 引数は比較 1 回。ほかは前置きの関数（`key=` / `default=`） | ⚠ list / tuple 同士の `<` が無い（6 節）。組を比べる形は前置きに辞書式の比較を書く |
| `sorted`（`key=` 7・`reverse=` 3） | 37 | 前置きの安定な整列（マージソート） | 組の比較は同上。⚠ 書けるが遅い。`list.sort()` も無い（6 節）ので、**Rust で 1 つ作って両方から使う方が筋が良い**（その場合は 4.2 へ移る） |
| `sum` | 15 | 前置き（`start` つきの `+` の畳み込み） | CPython 3.12 の float の和は補償つき（Neumaier）。同じ手順で書けば一致する |
| `iter(x)` | 33（すべて 1 引数） | 前置き: `x.__iter__()`。辞書は `d.keys().__iter__()`、ジェネレータはそのまま返す | 辞書とジェネレータに `__iter__` が無い（6 節）ので場合分けが要る。2 引数の `iter(f, 番兵)` はジェネレータ関数で書ける |
| `abs` | 26 | 符号で分ける。複素数は `.real()` / `.imag()` から | — |
| `ord` | 9 | `s.ord()`（既にある。`"é".ord()` → `233`） | — |
| `divmod` | 8 | `(a // b, a % b)` | ⚠ **演算子の側に差がある**: float の `//` / `%` が無い（6 節）。演算子を直すまで `divmod` も同じ差を持つ（割る数が負のときの int の差は直した） |
| `round(x)` | 2（すべて 1 引数） | 前置き（偶数への丸め。`int()` は 0 方向の切り捨て） | 2 引数の `round(x, n)` は `x * 10**n` 経由だと端で CPython（正確な 10 進の丸め）とずれる |
| `pow` | 1（値として渡す形） | 2 引数は `**`、3 引数は前置き（二乗を繰り返す） | 法が大きいと i64 の掛け算があふれる（i128 が要る） |
| `hex` / `oct` / `bin` | 1 / — / — | `"%x" % n` / `"%o" % n`（動く）。`bin` は桁を繰り返す | `0x` などの接頭辞と負数の `-0x` の形は自前で付ける |
| `ascii` | — | `repr` の結果を `chars()` / `ord()` で見て、非 ASCII を `\x..` / `\u....` / `\U........` にする | — |
| `breakpoint` | — | 何もしない（CPython の `PYTHONBREAKPOINT=0` と同じ） | 本当に止めるならデバッガとつなぐ（新規） |

#### 型

| 名前 | pandas | 書き方 | CPython とずれる点・前提 |
|---|---|---|---|
| `list` | 524（呼び出し 332・`isinstance` の中 144・ほか 48） | `list(it)` は `[v for v in it]`（変換器の `build_list_comprehension`）。`isinstance` の中は `is list` | 値として渡す形（`map(list, ..)` など）は前置きの関数が要る |
| `tuple`（呼び出し以外。大半が `isinstance` の中） | 約 191 | `is tuple` | 呼び出しの `tuple(it)` は 4.2 |
| `object`（値として。`dtype=object`・`== object` など） | 約 255 | 番兵（空のクラスの値）。`isinstance(x, object)` は `True` | 素の dunder の呼び出し（`object.__setattr__` など約 50）は 4.3 |
| `map` / `filter` | 34 / — | 前置きのジェネレータ関数（遅延のまま） | — |
| `reversed` | 15 | 列は `xs[::-1]` を回す。`__reversed__` を持つインスタンスはそれを呼ぶ | — |

#### 定数

| 名前 | pandas | 書き方 | CPython とずれる点・前提 |
|---|---|---|---|
| `Ellipsis` | 15（14 が `x is Ellipsis` の比較） | 番兵 | 変換器は今 `...` を `None` にしている（`convert_constant`）。同じ番兵に揃える |
| `__debug__` | — | `True` に書き換える | — |

#### 例外・警告（既存の例外クラスの表に足す）

| 名前 | pandas |
|---|---|
| 警告 12（`Warning` / `FutureWarning` / `DeprecationWarning` / `RuntimeWarning` / `UserWarning` / `ResourceWarning` / `UnicodeWarning` / `EncodingWarning` / `BytesWarning` / `ImportWarning` / `PendingDeprecationWarning` / `SyntaxWarning`） | 315 |
| 例外 24（`SyntaxError` / `IndentationError` / `TabError` / `LookupError` / `FloatingPointError` / `BufferError` / `ReferenceError` / `SystemError` / `UnboundLocalError` / `UnicodeError` / `UnicodeTranslateError` / `BaseException` / `EnvironmentError` / `WindowsError` / `ConnectionError` / `ConnectionAbortedError` / `ConnectionRefusedError` / `ConnectionResetError` / `BrokenPipeError` / `BlockingIOError` / `ChildProcessError` / `ProcessLookupError` / `InterruptedError` / `TimeoutError`） | 22 |

- どれも **Python のコードが自分で `raise` / `except` するだけ**で、Arrow の実行時が自分で投げる場面が無い。
  `BUILTIN_EXCEPTION_NAMES` と、それに揃える 2 つの表（`type_check` の `EXCEPTION_CLASS_NAMES`・`exceptions.rs` の `CATCHABLE`）に足せば済む。
- ⚠ 今の組み込みの例外には**階層が無い**（`make_error_class` が `bases` を空で作る。`except ArithmeticError` が
  `ZeroDivisionError` を捕まえない・実測）。`ClassValue::is_a` は `bases` の**直接の名前だけ**を見るので、
  祖先を平らに並べて入れれば仕組みはそのままで捕まる。`LookupError` を足すなら `KeyError` / `IndexError` の `bases` も直す。
- `EnvironmentError` / `WindowsError` は `OSError` の別名（同じクラスを指す名前）にする。
  `BaseException` は、`SystemExit` / `KeyboardInterrupt`（4.2 / 4.3）が無いうちは `Exception` と同じに扱ってよい。
- 警告は**クラスとして**足すだけ。`warnings.warn(..)` は標準ライブラリ（`warnings`）の問題で、本書の対象外。

#### 対話用

| 名前 | pandas | 書き方 | CPython とずれる点・前提 |
|---|---|---|---|
| `help` / `copyright` / `credits` / `license` | — | 決まった文を `print` | `help(obj)` は docstring を出せない |

### 4.2 新規実装（薄い: 中身は実行時にあり、呼び口が無いだけ）

| 名前 | pandas | 使える中身 | メモ |
|---|---|---|---|
| `type(x)` | 867（1 引数 845・値として 22） | クラスの値（組み込みの型も `print(int)` が `<class 'int'>`） | 使われ方は `type(x).__name__` 258・`type(x)(...)`（作り直し）234・`type(self)._simple_new(..)` などのクラスメソッド約 100・`type(x) is C` などの比較 35。クラスの値は呼べる（`let f = int` の後の `f("12")` が動く）ので、返せば後は既存で回る。`__name__` は足す（今は `Type` の `.name`）。値として（`isinstance(x, type)` など）は「値がクラスか」の判定 |
| `getattr` / `hasattr` / `setattr`（名前が変数） | 225（181 / 12 / 32） | 属性の読み書きは内部で名前の文字列で引いている（`eval/attrs.rs`） | `getattr` の 40 は既定値つき |
| `isinstance(x, 変数)` | 146 | `value_is_type` / `ClassValue::is_a` | `is` の右辺は型の**名前**しか書けない（`Expr::IsType` の `type_name: String`） |
| `issubclass` | 37（ほか 32 は相手が外部の型・4.4） | `ClassValue::is_a` | クラス同士を比べる構文が無い |
| `tuple(it)` | 113 | `TupleData::new` | 長さが実行時に決まる組を作る手段が無い（`tuple(xs)` は静的に `name 'tuple' is not defined`） |
| `hash` | 18 | `hash_value`（`ops/hash.rs`） | 辞書・集合の鍵と同じ値を返す |
| `chr` | 3 | （Rust の `char::from_u32`） | 文字コードから文字列を作る手段が無い |
| （`callable` のクラスの場合） | （4.1 の 59 に含む） | `Value::Class` | 「値がクラスか」の判定。`type` を値として使う形と共有する |
| `input` | — | — | 標準入力を読む手段が無い（デバッガの中だけ） |
| `exit` / `quit` | — | — | プロセスを終える手段が無い。`SystemExit` を投げ、捕まえられなければ終了コードで終わる |
| `FileNotFoundError` / `PermissionError` / `IsADirectoryError` / `NotADirectoryError` / `FileExistsError` | 3 | 例外クラスの表（4.1 と同じ） | 名前を足すだけでは足りない: 今は `open` の失敗が `IOError`（実測）。OS のエラーの種類で投げ分け、`bases` に `OSError` を入れる |
| `SystemExit` / `EOFError` | — | 同上 | `exit` / `input` と一緒に |

### 4.3 新規実装（厚い: 新しい値・意味が要る）

| 名前 | pandas | 足りないもの |
|---|---|---|
| `property` | 586（576 が `@property`） | 属性を読んだときに getter を呼ぶ仕組み。`o.x` → `o.x()` の書き換えは、変換時に `o` の型が分からないので変換器ではできない（タスク 6-1） |
| `isinstance` の第 2 引数が pandas の ABC（`ABCSeries` など） | 298 | メタクラスの `__instancecheck__`。pandas の ABC は `create_pandas_abc_type` が作るクラスで、名前は静的だが `is` では判定できない |
| `NotImplemented` | 50（28 が `return NotImplemented`） | 値は番兵で済むが、意味は「二項演算子が `NotImplemented` を受けたら反対側（`__radd__` など）を試す」手順にある |
| `object.__setattr__` / `__getattribute__` / `__new__` / `__repr__` | 約 51 | 上書きした dunder を飛ばして素の属性操作・生成を呼ぶ手段（名前が変数の `setattr` と同根） |
| `bytes` / `bytearray` / `memoryview` | 46 / 2 / 3 | バイト列の値の種類（今の `"é".encode()` は `list[int]` を返す） |
| `frozenset` | 45 | 不変でハッシュできる集合の値の種類 |
| `ImportError` / `ModuleNotFoundError` | 35 / 2 | pandas の使い方は `try: import x` / `except ImportError:`（任意の依存）。Python の `import` をブロックの中に書くこと自体が今は変換の誤り |
| `UnicodeDecodeError` / `UnicodeEncodeError` | 4 / 1 | バイト列と `decode` / `encode` の失敗（`bytes` と一緒に） |
| `globals` / `locals` / `dir` / `vars` | 5 / 2 / 1 / 1 | 実行時のリフレクション（メタ情報 `Value::Meta` は展開時だけ） |
| `__import__` / `eval` / `exec` / `compile` / `__build_class__`・3 引数の `type(name, bases, dict)` | 1 / — / — / — / —・1 | 実行時に Python のコードを変換・実行する、import する、クラスを作る（`parse_ar` は Arrow のソースだけ） |
| `format` | — | 書式指定の小言語。`%` の printf 形式はある（`.2f`・`5d`・`x`・`-5s` は動く）が、`,` は `unsupported format character`、`%e` は `1.234568e4`（CPython `1.234568e+04`）、`^` / `>` の詰め・`_`・`%`・`#x` は無い。f-string の `{x:spec}` も同じものが要る |
| `delattr` | — | 属性はクラス本体で固定なので「消す」概念が無い |
| `aiter` / `anext` / `StopAsyncIteration` | — | 非同期の反復の手順（Arrow の非同期は `AsyncManager`） |
| `KeyboardInterrupt` / `MemoryError` | — | Ctrl+C を例外に変える手順／メモリ不足を捕まえる手順（Rust はメモリ不足で止まる） |
| `ExceptionGroup` / `BaseExceptionGroup` | — | `except*` の構文と意味 |

### 4.4 相手の型が読めるか次第（組み込みの問題ではない）

| 形 | pandas | メモ |
|---|---|---|
| `isinstance` / `issubclass` の第 2 引数が pandas の外の型（`np.ndarray`・`collections.abc` の型・`datetime`・`pandas._libs` の C 拡張の型など） | 627（595 / 32） | 相手の型が Arrow の側にあれば 4.1 の `is` で書ける。numpy と `pandas._libs.*` は `import[py]` では読めない。`collections.abc`（`Iterable` など）は構造で判定するので、別に手段が要る |

### 4.5 実装予定（5 節）への影響

- 3-2 / 3-3 の大半（`all` 〜 `pow`・`map` / `filter` / `iter` / `ord` / `hex` / `oct` / `bin` / `ascii`）は前置き 1 つにまとめられる。
  ただし `divmod` / `sorted` / `min` / `max` を CPython と揃えるには、先に演算子の穴（6 節）を埋める。
- 3-1 は名前で行き先が違う: `list` は書き換え（4.1）、`tuple` は薄い新規（4.2）、`frozenset` / `bytes` は厚い新規（4.3）。
- 2-1 は名前の形（1,727）が書き換えで済む一方、pandas の ABC（298）は `__instancecheck__` が無いと残る。
- 2-2 の `type(x)` は薄い（クラスの値を返すだけ）が、`__name__` と「値がクラスか」の判定を一緒に足す。

## 5. 実装予定

フェーズは次のように分けた。フェーズの中は pandas の損害が大きい順。

- フェーズ 1: 失敗を前倒しする（ほかのタスクの間も効く）
- フェーズ 2〜5: 意味が CPython と同じにできるもの（型の判定 → コンテナと集計 → 例外・警告 → 呼び出しの形の穴）
- フェーズ 6: Arrow の言語に対応物が無く、写し方を決める必要があるもの（`property` など）

「前提」の欄が「なし」のタスクは、ほかのタスクと独立に着手できる。

| # | 内容 | pandas | 前提 | 重さ |
|---|---|---|---|---|
| **1-1** | 未対応の組み込みの参照を**変換時に見つけて知らせる**（失敗を前倒しする）。⚠ 要判断: 変換の誤りにするとその名前を 1 箇所でも含むモジュールが丸ごと読めなくなる（実行しない枝でも）。警告にとどめるか、誤りにするかを決める | 7,204（187 モジュール） | なし | 小 |
| **2-1** | `isinstance(x, C)` の写し: 第 2 引数が名前（`C` / `m.C` / `int`）なら `x is C`、組なら `or` へ変換時に写す。変数（`isinstance(x, cls)`）は実行時の組み込みで受ける。⚠ `isinstance(True, int)` は CPython で `True`（`bool` は `int` の派生） | 2,766 | なし | 中 |
| **2-2** | `type(x)`（1 引数）と `type(x).__name__` / `type(x) is C`。⚠ 3 引数の `type(name, bases, dict)`（クラスを作る）は対象外の候補 | 868 | なし | 中 |
| **2-3** | `getattr` / `hasattr` / `setattr`（既定値つき `getattr(o, n, d)` を含む） | 595 | なし | 中 |
| **2-4** | `issubclass` / `callable` | 128 | 2-1 | 小 |
| **3-1** | `list(it)` / `tuple(it)` / `dict(..)`（呼べるようにする）/ `frozenset` / `bytes` | 969 | なし | 中 |
| **3-2** | `all` / `any` / `min` / `max` / `sum` / `sorted`（`key=` / `reverse=`）/ `reversed` / `abs` / `round` / `divmod` / `pow` | 401 | なし | 中 |
| **3-3** | `map` / `filter` / `iter` / `hash` / `ord` / `chr` / `hex` / `oct` / `bin` / `format` / `ascii` | 98 | なし | 小 |
| **4-1** | 警告クラス（`FutureWarning` / `DeprecationWarning` / `RuntimeWarning` / `UserWarning` / `Warning` …）と、足りない例外クラス（`ImportError` / `LookupError` / `SyntaxError` / `FileNotFoundError` …）。⚠ 階層（`LookupError` → `KeyError` / `IndexError`）も合わせる（10-19 の `ClassValue::is_a`） | 382 | なし | 中 |
| **4-2** | 例外の `str(e)`（メッセージ）と `e.args` | — | なし | 小 |
| **5-1** | 呼び出しの形だけ解決される組み込み（`range` / `repr` …）を**値として**使えるようにする。`print` のキーワード引数（`end=` / `sep=`）で関数がバイトコードにできない問題 | 16 | なし | 中 |
| **5-2** | `open` を Python の形（`open(path, "r", encoding=..)`）で受ける | 6 | なし | 小 |
| **6-1** | ⚠ 要判断: `property`（Arrow にプロパティ構文が無い。getter をメソッド呼び出しへ写すか） | 586 | なし | 大 |
| **6-2** | ⚠ 要判断: `object`（`class C(object)` の基底・`dtype=object` などの値）/ `NotImplemented` / `Ellipsis` / `super(C, self)` の形 / `staticmethod` / `classmethod` の関数としての使用 | 374 | なし | 中 |

⚠ 対象外の候補（実行時にコードを作る・環境を覗く・対話用）: `exec` / `eval` / `compile` / `globals` / `locals` / `vars` / `dir` /
`__import__` / `__build_class__` / `breakpoint` / `input` / `help` / `exit` / `quit` / `copyright` / `credits` / `license` / `memoryview` / `aiter` / `anext`。
対応しないなら 1-1 で「未対応」と知らせる。

## 6. 測定のついでに見つけたもの（組み込み以外）

- `list.sort()`（メソッド）が無い（`AttributeError: 'list' object has no method 'sort'`）。組み込み関数ではないので本書の数には入れていない。
- 以下は 4 節の分類で見つけた（どれも実測）。4.1 の前置きで書くものの正しさに効く:
  - ~~整数の `//` / `%` が**割る数が負のとき** CPython と違う。`-7 // -2` が `4`（CPython `3`）、`7 % -2` が `1`（CPython `-1`）。~~
    → **直した**（2026-10-01）。`div_euclid` / `rem_euclid` をやめ、`ops::py_floor_div` / `ops::py_mod` に 1 本化した
    （`apply_binop` と VM の `int_binop_specialized` の 2 か所。ネイティブ codegen の `@_tl_idiv` / `@_tl_imod` は元から正しかった）。
    回帰の例題は `examples/basics/int_floor_div_mod.ar`。
  - float の `//` / `%` が無い（`-7.5 // 2.0` が静的に `unsupported operand types`。型検査を通らない Python 由来のコードでも実行時に `TypeError`）。
  - list 同士・tuple 同士の大小比較（`<` など）が無い（静的にも実行時にも `TypeError`）。`sorted` / `min` / `max` で組を比べる形に効く。
  - 辞書とジェネレータに `__iter__` メソッドが無い（list・str・set にはある）。
  - Python のクラスを**修飾名**で書いた型の判定（`import[py] mod as m` の後の `b is m.Box`）が静的に `unknown type 'm.Box' in type guard`。
    `from mod import[py] Box` の後の `b is Box` は通る。

