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
| ~~`complex`~~ | ✅ 直した（タスク 1-5・2026-10-05）。~~表示が違う（`complex(1, 2)` が `(1.0+2.0j)`。CPython は `(1+2j)`）~~ | （表示だけ） | — |

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
| `isinstance(x, C)`（第 2 引数が組み込みの型名・クラス名・その組） | 1,727（組み込みの型 766・pandas のクラス 961） | `x is C`。組は `or` | ⚠ `isinstance(True, int)` は CPython で `True`、Arrow の `True is int` は `False`。`int` は `x is int or x is bool` に写す。Python のクラスの**修飾名**（`b is m.Box`）・基底（`b is m.Animal`）も書ける（6 節で直した） |
| `getattr(o, "名前")` / `getattr(o, "名前", d)` | 188（2 / 186） | `o.名前` / `AttributeError` を捕まえる `try` のブロック式 | 無い属性の読みは捕まえられる `AttributeError` になる |
| `hasattr(o, "名前")` | 150 | 同じ `try` のブロック式で `True` / `False` | 同上 |
| `setattr(o, "名前", v)` | 32 | `o.名前 = v` | クラス本体で宣言していない属性は作れない（`'Box' has no field 'zzz'`）。`setattr` に限らない既存の差 |
| `callable(x)` | 59 | `x is function`（関数・`__call__` を持つインスタンス／クラスで真） | ⚠ `__call__` を持たない**クラス**で `False`（CPython は `True`）。クラスかどうかの判定（4.2）が要る |
| `all` / `any` | 113 / 98 | 前置きの関数（`for` で途中で返す） | — |
| `max` / `min` | 51 / 35（2 引数 57・反復可能 27・値として 2） | 2 引数は比較 1 回。ほかは前置きの関数（`key=` / `default=`） | ⚠ list / tuple 同士の `<` が無い（タスク 1-3）。組を比べる形は前置きに辞書式の比較を書く |
| `sorted`（`key=` 7・`reverse=` 3） | 37 | 前置きの安定な整列（マージソート） | 組の比較は同上。⚠ 書けるが遅い。`list.sort()` も無い（6 節）ので、**Rust で 1 つ作って両方から使う方が筋が良い**（その場合は 4.2 へ移る） |
| `sum` | 15 | 前置き（`start` つきの `+` の畳み込み） | CPython 3.12 の float の和は補償つき（Neumaier）。同じ手順で書けば一致する |
| `iter(x)` | 33（すべて 1 引数） | 前置き: `x.__iter__()`。辞書は `d.keys().__iter__()`、ジェネレータはそのまま返す | 辞書とジェネレータに `__iter__` が無い（タスク 1-4）ので、それまでは場合分けが要る。2 引数の `iter(f, 番兵)` はジェネレータ関数で書ける |
| `abs` | 26 | 符号で分ける。複素数は `.real()` / `.imag()` から | — |
| `ord` | 9 | `s.ord()`（既にある。`"é".ord()` → `233`） | — |
| `divmod` | 8 | `(a // b, a % b)` | ⚠ **演算子の側に差がある**: float の `//` / `%` が無い（タスク 1-2）。演算子を直すまで `divmod` も同じ差を持つ（割る数が負のときの int の差は直した） |
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
| `object.__setattr__` / `__getattribute__` / `__new__` / `__repr__` | 約 51 | 上書きした dunder を飛ばして素の属性操作・生成を呼ぶ手段（名前が変数の `setattr` と同根）。⚠ `object.__setattr__` / `object.__delattr__`（[known_bugs.md](known_bugs.md) の #10）・`object.__getattribute__` と `__getattr__`（#11）は入った。`__getattribute__` の上書きは #12 |
| `bytes` / `bytearray` / `memoryview` | 46 / 2 / 3 | バイト列の値の種類（今の `"é".encode()` は `list[int]` を返す） |
| `frozenset` | 45 | 不変でハッシュできる集合の値の種類 |
| `ImportError` / `ModuleNotFoundError` | 35 / 2 | pandas の使い方は `try: import x` / `except ImportError:`（任意の依存）。Python の `import` をブロックの中に書くこと自体が今は変換の誤り |
| `UnicodeDecodeError` / `UnicodeEncodeError` | 4 / 1 | バイト列と `decode` / `encode` の失敗（`bytes` と一緒に） |
| `globals` / `locals` / `dir` / `vars` | 5 / 2 / 1 / 1 | 実行時のリフレクション（メタ情報 `Value::Meta` は展開時だけ） |
| `__import__` / `eval` / `exec` / `compile` / `__build_class__`・3 引数の `type(name, bases, dict)` | 1 / — / — / — / —・1 | 実行時に Python のコードを変換・実行する、import する、クラスを作る（`parse_ar` は Arrow のソースだけ） |
| `format` | — | 書式指定の小言語。`%` の printf 形式はある（`.2f`・`5d`・`x`・`-5s` は動く）が、`,` は `unsupported format character`、`%e` は `1.234568e4`（CPython `1.234568e+04`）、`^` / `>` の詰め・`_`・`%`・`#x` は無い。f-string の `{x:spec}` も同じものが要る |
| `delattr` | — | ⚠ 「消す」は入った（Python の `del o.a` はフィールドを未代入に戻す・[known_bugs.md](known_bugs.md) の #2・本体は `del_attr_val`）。組み込みの `delattr` という名前はまだ無い |
| `aiter` / `anext` / `StopAsyncIteration` | — | 非同期の反復の手順（Arrow の非同期は `AsyncManager`） |
| `KeyboardInterrupt` / `MemoryError` | — | Ctrl+C を例外に変える手順／メモリ不足を捕まえる手順（Rust はメモリ不足で止まる） |
| `ExceptionGroup` / `BaseExceptionGroup` | — | `except*` の構文と意味 |

### 4.4 相手の型が読めるか次第（組み込みの問題ではない）

| 形 | pandas | メモ |
|---|---|---|
| `isinstance` / `issubclass` の第 2 引数が pandas の外の型（`np.ndarray`・`collections.abc` の型・`datetime`・`pandas._libs` の C 拡張の型など） | 627（595 / 32） | 相手の型が Arrow の側にあれば 4.1 の `is` で書ける。numpy と `pandas._libs.*` は `import[py]` では読めない。`collections.abc`（`Iterable` など）は構造で判定するので、別に手段が要る |

### 4.5 実装予定（5 節）への影響

- 3-2 / 3-3 の大半（`all` 〜 `pow`・`map` / `filter` / `iter` / `ord` / `hex` / `oct` / `bin` / `ascii`）は前置き 1 つにまとめられる。
  ただし `divmod` / `sorted` / `min` / `max` を CPython と揃えるには、先に演算子の穴（タスク 1-2 / 1-3）を埋める。`iter` は 1-4 が前提。
- 3-1 は名前で行き先が違う: `list` は書き換え（4.1）、`tuple` は薄い新規（4.2）、`frozenset` / `bytes` は厚い新規（4.3）。
- 2-1 は名前の形（1,727）が書き換えで済む一方、pandas の ABC（298）は `__instancecheck__` が無いと残る。
- 2-2 の `type(x)` は薄い（クラスの値を返すだけ）が、`__name__` と「値がクラスか」の判定を一緒に足す。

## 5. 実装予定

フェーズは次のように分けた。フェーズの中は pandas の損害が大きい順。

- フェーズ 1: 失敗を前倒しする・後のフェーズの前提になる演算子とメソッドの穴（どれもほかのタスクの間も効く）
- フェーズ 2〜5: 意味が CPython と同じにできるもの（型の判定 → コンテナと集計 → 例外・警告 → 呼び出しの形の穴）
- フェーズ 6: Arrow の言語に対応物が無く、写し方を決める必要があるもの（`property` など）

「前提」の欄が「なし」のタスクは、ほかのタスクと独立に着手できる。

| # | 内容 | pandas | 前提 | 重さ |
|---|---|---|---|---|
| ~~**1-1**~~ | ✅ **済**（2026-10-06・**決定: 変換の誤りにする**。`python_converter/builtins_check.rs` が変換の後に構文木を 1 周し、CPython 3.12 の組み込み 153 名のうち Arrow の実行時が解決できない名前（`is_runtime_builtin_name` が偽・組み込みを足すと自動で外れる）の読みを、名前ごとに最初の行を並べて誤りにする。モジュールのどこかで束縛される名前・デコレータ・クラスの基底・`super().m()` の `super` は通す（スコープは解かず保守的に）。`from m import *` のあるモジュールは見ない。構文の誤り（ブロックの中の `import` など）を先に出す。2026-10-06 の時点で未対応は 44 名（`format` / `vars` / `exit` / `bytes` / `frozenset` / `object` の値としての使用 …）。Arrow のソースは以前から型検査が未定義の名前として静的に弾いている） 未対応の組み込みの参照を**変換時に見つけて知らせる**（失敗を前倒しする）。⚠ 要判断: 変換の誤りにするとその名前を 1 箇所でも含むモジュールが丸ごと読めなくなる（実行しない枝でも）。警告にとどめるか、誤りにするかを決める | 7,204（187 モジュール） | なし | 小 |
| ~~**1-2**~~ | ✅ **済**（2026-10-05）float の `//` / `%`（int と float の混在を含む）。CPython の `_float_div_mod` / `float_rem`（`fmod` で余りを出し、符号を割る数に合わせ、0 にも割る数の符号を付け、商を整数へ寄せる）を写す。ゼロ除算の文言も合わせる（`float floor division by zero` / `float modulo`）。⚠⚠ **ネイティブ codegen も同じ式に揃える**: 今の `floor(a/b)` / `a - floor(a/b)*b` は `0.1 % 0.01`（`0.0`、CPython `3.47e-18`）・`-1.0 % inf`（`nan`、CPython `inf`）・`6.0 % -3.0`（`0.0`、CPython `-0.0`）でずれる。int の `//` / `%`（`ops::py_floor_div`）と同じく共通の関数 1 つにする。型検査の `binop.rs` も float を通す。詳細は 6 節 | —（演算子） | なし | 小 |
| ~~**1-3**~~ | ✅ **済**（2026-10-05）list 同士・tuple 同士の大小比較（`<` / `<=` / `>` / `>=`）。CPython の `list_richcompare` / `tuplerichcompare` の 2 段構え（`==` で最初に違う要素を探す → その要素だけを求められた演算子で比べる → 違いが無ければ長さで決める）。⚠ list と tuple は比べない（`TypeError`）、同じオブジェクトは等しいとみなす（`[nan] == [nan]` が真）、比べられない要素に届いたときだけ `TypeError`（`(1, 'a') < (2, 3)` は真）。型検査は `tuple[..]` の要素ごとに比べられるかを見る。3-2 の `sorted` / `min` / `max` でタプルを鍵にする形の前提。詳細は 6 節 | —（演算子） | なし | 中 |
| ~~**1-4**~~ | ✅ **済**（2026-10-05）辞書とジェネレータの `__iter__` メソッド。辞書はキーを挿入順に返すイテレータ（CPython の `dict_iter`・反復中に大きさが変わったら `RuntimeError: dictionary changed size during iteration`）、ジェネレータは自分自身（`PyObject_SelfIter`）。⚠ `for k in d:` / `for x in g:` は今も動く。足りないのはメソッドとしての呼び出しと、それを使う `iter()`（3-3）。⚠ list の `__iter__` は呼んだ時点の写しを回すので、反復中の書き換えも CPython と違う（別に要判断）。詳細は 6 節 | —（メソッド） | なし | 小 |
| ~~**1-5**~~ | ✅ **済**（2026-10-05・complex の表示も CPython の `complex_repr` に揃えた）float の表示（`str` / `repr` / `print`）を CPython の `repr` に合わせる。今は指数表記を使わず、`1e20` が `100000000000000000000`（`.0` も付かない）、`1e-10` が `0.0000000001`、`0.1 % 0.01` が `0.000000000000000003469446951953614` と出る（CPython は `1e+20` / `1e-10` / `3.469446951953614e-18`）。CPython は最短の往復表現（`PyOS_double_to_string(x, 'r', 0, Py_DTSF_ADD_DOT_0)`）で、10 進の指数が `-4` 未満か `16` 以上なら指数表記（`e+20` / `e-05`・指数は 2 桁以上）、それ以外は小数表記に `.0` を足す。1-2 の作業中に見つけた（6 節） | —（表示） | なし | 小 |
| ~~**1-6**~~ | ✅ **済**（2026-10-05）Python のモジュールの**最上位**で、組み込みの名前（`id` / `str` / `dict` / `len` …）を束縛し直せるようにする。今は `id = 3` が `NameError: variable 'id' is already declared`（実測）。CPython はモジュールの大域が組み込みを隠す（`builtins` は名前の探索の最後の段）。関数の仮引数・局所変数の `type` / `dict` / `len` / `print` は今も動く（実測）。⚠ 組み込みを足すほど（2-x / 3-x）ぶつかる名前が増えるので、組み込みを足すタスクの前提。⚠ Arrow のソースの最上位の `let len = 3` も同じ誤り（静的に `already declared`）だが、Arrow の規則として残すかは別に要判断（ここでは `import[py]` のモジュールだけを直す） | —（名前の解決） | なし | 小 |
| ~~**1-7**~~ | ✅ **済**（2026-10-05・`list(it)` / `tuple(it)` もここで入れた＝3-1 の一部）組み込みの関数・型の名前を**値として**使えるようにする（5-1 の前半を前提として前に出した）。束縛されていない名前が組み込みの名前なら、名前の探索の最後の段で組み込みの値（`Value::Type(名前)`）を返す（CPython の `builtins` と同じ位置・束縛した名前が勝つので 1-6 の衝突は起きない）。大域へ束縛する形（`int` / `str` / `dict` は今そう）にすると、同名の最上位の宣言が `already declared` になるので使わない（実測）。これで `map(repr, xs)` / `key=len` / `isinstance(x, (list, tuple))`（2-1）/ `list(it)` / `tuple(it)`（3-1）が書ける。呼ぶと組み込み関数の表（`eval_builtin_evaled` / キーワードつきは `eval_builtin_evaled_named`）へ回す | 16（値としての参照）+ 2-1 / 3-1 の前提 | なし | 中 |
| ~~**1-8**~~ | ✅ **済**（2026-10-06・**決定: 受け手の不変は呼ぶ時点で直接呼びと同じ判定にかける。束縛すること自体は誤りにしない**（関数の値が不変であることと、その関数が受け手を書き換えることは両立する）。下の調査どおり `FnValue::bound`（`BoundMethod { receiver, methods }`）と `exec_fn_evaled` の入口の `call_bound_method` で作った。等値・ハッシュ・シグナルの `off` は `same_binding` / `identity_ptr`。表示は `<bound method C.m of <C object at 0x..>>`。型検査・gen メソッド・組み込みの値のメソッドは 1-10 で足した。前提として 1-9 を先に直した） 🔍 **可否を調べた（2026-10-06・可能）**: `Value` に腕を足さず、**関数の値（`FnValue`）に「束縛した受け手とメソッドの実体」を持たせる**形で作れる。属性の読み（`eval/attrs.rs` のインスタンスの腕・今は束縛しない `Value::Function` を返す）で、メソッドの実体（オーバーロードなら全部）を引いて、`FnValue` の新しい欄 `bound: Option<(受け手, 実体)>` を持つ薄い包みを返す。呼ぶと `exec_fn_evaled` の入口で実体へ回し、`self_val = 受け手`（`obj.m(..)` と同じ実行・コンパイル済みの本体もそのまま使う）。根拠: 関数の値の呼び出しはツリーウォークも VM も `call_value_evaled` → `exec_fn_evaled` の 1 本（`sorted(key=)` / `map` / シグナルも同じ）。`FnValue` の構築は 21 箇所。⚠ 要注意点: ① 等値・ハッシュ・`off` の照合は `Rc` の同一性なので、包みは「受け手が同じ・実体が同じ」で比べる（`hash_eq_identity` ゲート）② `let c` の不変は**静的にしか**守っていない（実行時の不変印は `freeze` だけ）ので、`let a = c.add`（`mut self` のメソッドを不変の束縛から取る）を型検査で止める（`MutatingMethodOnImmutable` と同じ規則）③ 型検査は今 `c.get` を型なしで通す（`g()` の引数・戻り値を見ない）。メソッドの署名から `self` を除いた関数型を付けられる④ アクセス制御は読む時点で見る（呼ぶ時点で見ると、クラスの中で `self._key` を渡したコールバックが外で呼ばれたときに止まる）⑤ gen メソッド（`gen_methods`）・組み込みの値のメソッド（`xs.append`）は別の包み（後回しでよい） ⚠ 要判断（表し方）: **束縛メソッド**。`f = obj.method` / `getattr(obj, "method")` / `key=obj.method` で取り出したメソッドを呼ぶと `TypeError: missing argument 'self'`（実測・2-3 の作業中に見つけた）。インスタンスからメソッドを値として読むと束縛しない関数（`Value::Function`）が返るため。CPython は `self` を束縛したメソッドを返す。意味は明確だが、値の種類を足す（`Value` に腕を足すと網羅の `match` が多数止まる）か、`self` を捕まえたクロージャで表すかを決める必要がある。3-2 / 3-3 のコールバック（`sorted(key=obj.m)` / `map(obj.m, xs)`）の前提 | —（pandas の `self.method` を値として渡す形） | なし | 中 |
| ~~**1-9**~~ | ✅ **済**（2026-10-06・1-8 の調査中に見つけた・**決定: `freeze x` は `let x = x` と同じ結果**）`freeze` と `let` の束縛を 1 つの実装にする。以前は 4 通りあった: ① `freeze` は**コピーせずその場で**不変にした（同じオブジェクトを持つ相手＝`mut` の仮引数を渡した呼び出し側まで不変になる）② `let` はコピーしたが、コレクションの中のインスタンスは不変にせず `__freeze__` も呼ばなかった（`freeze` は呼ぶ）③ 関数の中の `let x = <式>`（VM の `StoreLocalFreezeInstance`）はインスタンスのときしかコピーせず、`let item = xs[0]` が `xs[0]` と共有された（最上位は L2 で直っていた）④ **関数の中の `freeze` がすべて `NameError`**（VM の `FreezeVar` が名前でスコープを探し、slot の変数を見つけられなかった。例題が最上位でしか書いていなかった）。⇒ `Interpreter::frozen_copy`（深いコピー → インスタンスは `__freeze__` とフィールドの不変化・コレクションは中のインスタンスへ 1 段・fixed_list は切り詰め）を唯一の実装にし、ツリーウォークの `exec_let`・VM の最上位と slot の `let`・デバッガの `let`・`freeze` がみなこれを呼ぶ。関数の中の `freeze x` は `LoadLocal` + `StoreLocalCopyFreeze`（`let x = <mut 変数>` と同じ命令）、クロージャが捕まえた変数・`static` は明示の `TypeError` | —（言語） | なし | 中 |
| ~~**1-10**~~ | ✅ **済**（2026-10-07・1-8 の残り）束縛メソッドを広げ、型を付ける。① `gen` メソッド（`bag.each` を呼ぶとジェネレータ）② 組み込みの値のメソッド（`xs.append` / `s.upper` / `d.items` / `st.add` …。同じ値を書き換える）。読める名前は `type_check/names.rs` の `BUILTIN_VALUE_METHODS`（実行時が束縛するか・型検査の読みの存在検査が同じ表を読む。テスト `builtin_value_methods_are_dispatched` が表の名前を実際に呼ぶ）。③ 型検査: `obj.m` を値として読むと `self` を除いたシグネチャの関数型（多重定義・可変長は `function`）。引数・戻り値を検査する。④ **受け手の不変は呼ぶ時点**（1-8 の決定どおり）: 関数型に印 `mutates_immutable`（`InferredType::Function` の欄）を付け、呼ぶと `MutatingMethodOnImmutable`（`c.add(1)` と同じ誤り）。束縛すること自体は誤りにしない。⚠ list / dict / set には実行時の不変の印が無いので、型検査が追えない渡し方（関数の引数に渡して中で呼ぶ）では止まらない（インスタンスは実行時にも止まる）。⚠ `complex` の `real` / `imag` / `angle` は読めない（CPython では値の属性）。⚠ 異なる関数型どうしの `==`（`c.get == c.add`）は静的な誤り（Arrow の比較の規則） | —（言語） | 1-8 | 中 |
| ~~**1-11**~~ | ✅ **済**（2026-10-09・6-1 の前提）属性に入れた関数を `obj.f(x)` で呼ぶ。メソッドの無い名前は CPython と同じく「属性を読んで、その値を呼ぶ」（フィールド・クラス変数・`static mut`。読みは `get_attr_val` なのでアクセス制御も同じ）。型検査はフィールドの関数型で引数・戻り値を検査し、呼べない値（`int` など）は `not callable`。⚠ 以前は `has no method ` で、Python の `self.callback(x)` も動かなかった（property の試作で見つけた） | —（言語） | なし | 小 |
| ~~**1-12**~~ | ✅ **済**（2026-10-09・6-1 の前提）ディスクリプタのプロトコル。クラス側の値（`static mut` = Python のクラス属性・`const`）が `__get__` を持つインスタンスなら、CPython の `object.__getattribute__` / `__setattr__` と同じ順で働く: 読みは ① データディスクリプタ（`__set__` か `__delete__` も持つ）の `__get__(obj, C)` ② フィールド ③ 非データディスクリプタの `__get__` ④ 値そのもの ⑤ メソッド。インスタンス経由の代入はデータディスクリプタなら `__set__(obj, v)`（`__set__` が無ければ `AttributeError: __set__`）。クラスから読むと `__get__(None, C)`。入口は `eval/attrs.rs` の 3 つ（`get_attr_val` / `attr_assign_evaled` / ネイティブの `set_attr_val`）だけで、メソッド呼び出しは 1-11 の「属性を読んで呼ぶ」で同じ経路に乗る。型検査は読んだ値を `__get__` の戻り値、インスタンス経由の代入を `__set__` の値の型、クラス経由の代入（`C.x = d`）をメンバーの宣言型で照合する（`TypeChecker::class_descriptor`）。⚠ インスタンスのフィールドに入れたディスクリプタは働かない（CPython と同じ）。⚠ `del obj.x`（`__delete__`）は Arrow に属性を消す構文が無いので未対応。⚠ VM の `GetAttr` の速い経路はフィールドにしか張られないので、ディスクリプタの読みは毎回 `get_attr_val` を通る | —（言語） | 1-11 | 中 |
| ~~**1-13**~~ | ✅ **済**（2026-10-12・6-1 の作業中に見つけた）`str` の `repr` を CPython 3.12 の `unicode_repr` に合わせた（`ops::py_str_repr`。`display_repr` が呼ぶので `repr()`・コンテナの中の表示・`KeyError` の文言・`ascii()` が同じになる）。引用符は `'`（`'` を含み `"` を含まなければ `"`）、`\` と引用符・`\t` / `\n` / `\r` をエスケープ、表示できない文字は `\xhh` / `\uhhhh` / `\Uhhhhhhhh`。表示できる文字の判定は CPython 3.12.2（Unicode 15.0）の `str.isprintable()` から作った表（`ops/unicode_printable.rs`・712 範囲・作り直す手順はファイルの冒頭）で、`str.isprintable()` も同じ表にした（以前は `!is_control()` で、U+00A0 / U+200B / 未割り当てを表示できるとしていた）。⚠ 以前は `'` で囲むだけで、`'it's'` / 改行がそのまま / `\` が 1 つのまま、と出ていた（`'` を含む文言の入った list を出す既存の例題 `js_proc_async_test.ar` の表示も CPython と同じ `".."` になった）。例題は `basics/str_repr.ar`（CPython と同じ出力）。⚠ 内部で起きた `KeyError` の `repr(e)` は `args` が文言なので CPython と違う（4-2 の既知の差） | —（表示） | なし | 小 |
| ~~**1-14**~~ | ✅ **済**（2026-10-12）関数の値の `__name__` / `__qualname__` / `__doc__`（6-6 の作業中に見つけた）。実行時は `Interpreter::function_attr`（`eval/attrs.rs`）、型は `str` / `str` / `Option[str]`（`infer_attr`）。`__qualname__` は `FnValue::qualname` / `GeneratorFnValue::qualname`: メソッドはクラスを定義するときに `C.m`、関数の中の関数は外側の本体をコンパイルした後で `ChunkFnDef::qualname` に `outer.<locals>.inner` を入れ `Op::MakeFn` が写す（`Chunk::set_nested_qualnames`）。`__doc__` は本体の先頭の文字列の文（変換器は docstring を残している）。Python の関数は変換器の別名を戻す（property の getter は `x`・持ち上げたラムダは `<lambda>`・`python_fn_name`。別名の作り方 `property_method_name` と同じ場所）。束縛メソッドは呼び先のもの、組み込みの値のメソッドは `list.append`。**あわせて直した誤り**: Python の関数を呼んだときに作る入れ子の関数が Arrow の関数になり（`is_python` を`in_python_module` だけで決めていた）、Arrow の束縛の規則で呼ばれていた（`def inner(a, b=2, **kw)` を `inner(1)` で呼ぶと `TypeError`）。定義した本体の `Chunk::is_python` を引き継ぐ。例題は `basics/function_attrs.ar`（`_error` あり）と `interop/py_function_attrs.ar`（CPython と同じ出力）。⚠ ツリーウォークで走る本体の中の関数の `__qualname__` は名前だけ。⚠ 関数の値に任意の属性は付けられない（`functools.wraps` は対象外） | —（デコレータの `f.__name__`） | なし | 小 |
| ~~**1-15**~~ | ✅ **済**（2026-10-12）`str(x)` / `print(x)` / `f"{x}"` を CPython の規則にした（6-3 の作業中に見つけた）: ① インスタンスは `__str__`、無ければ `repr`（`__repr__`。CPython の `object.__str__`）② コンテナ（list / tuple / dict / set / frozenset）は要素を `repr` で並べる（要素の `__str__` は使わない）。以前は `print([t])` / `str([t])` / `print(t)`（`__repr__` だけを持つ）が `<T object at ..>` だった（`repr([t])` だけが `__repr__` を呼んだ）。実装は `display_str`（`ops/operators.rs`）を `print` と `str()` の両方が通る形（f-string は `str()` に脱糖される）。例外の文言（`str(ValueError(x))`）も同じ。⚠ 組み込みの例外の階層は `__str__` が無くても `message`。⚠ new_type の値の `str` も `repr` と同じ形（`Meters(5)`）になった。例題は `classes/str_falls_back_to_repr.ar`。⚠ `str.format` の `{}` は今も `display`（3-4 で書式の解釈を 1 つにするときに揃える） | —（表示） | なし | 小 |
| ~~**1-16**~~ | ✅ **済**（2026-10-12）Python の関数の `*args` をタプルで受ける（1-14 の作業中に見つけた）。`bind_args_relaxed` があふれた位置引数をタプルにする（1 個も来なくても空タプル）。以前はリストで、`def f(*rest): return rest` が `[2, 3]`、`rest + (1,)` / `fmt % values` が誤りだった。あわせて変換器が `*args` の注釈を `list[Any]` から `Any` にした（Arrow の可変長の注釈は**要素の型**なので、要素が list の可変長になっていた）。Arrow の関数の可変長（`let ...: T` は `list[T]`）は変えていない。例題は `interop/py_varargs_tuple.ar`（CPython と同じ出力） | —（`*args`） | なし | 小 |
| ~~**1-17**~~ | ✅ **済**（2026-10-13）文字列の長さと位置を文字の単位にする（5-4 の作業中に見つけた）。`len(str)` がバイトの数で `len("γάμμα")` が `10`（CPython は `5`）、添字・切り出し・`for` は文字の単位で食い違っていた。調べると `find` / `rfind` / `index` / `rindex` / `count` もバイトの位置を返し（`"γάμμα".find("μ")` が `4`）、**文字の途中の `start` を渡すと Rust のパニックで落ちた**。あわせて直した: 負の `start` / `end`（後ろから）・`None`、`startswith` / `endswith` の `start` / `end`（黙って捨てていた）と組、`index` の文言（`substring not found`）、`zfill`（バイトで比べ、符号の前に 0 を置いていた）、`expandtabs`（タブ位置で揃えずに `tabsize` 個の空白にしていた）、`%5s` / `%.3s`（バイトで詰め・切り、文字の途中で落ちた）。範囲は CPython の `ADJUST_INDICES` と同じ（`str_char_range`）。`len` の 3 つの実装（ツリーウォーク・評価済み・型の呼び出し）を全部直した（ネイティブは `len` を呼び戻すので同じ）。例題は `basics/str_char_positions.ar`（`_error` あり・CPython と同じ出力） | —（`len` / `find`） | なし | 小 |
| ~~**1-18**~~ | ➡ **[know_bugs.md](know_bugs.md) の #7 へ移した**（2026-10-22）。CPython の物どうしの `is`（`===`）が `==` を呼んでいる | — | なし | 小 |
| ~~**1-19**~~ | ✅ **不要になった**（2026-10-22・master への rebase で決定）`import[py-int]` で本体を 1 行に書いた関数（`def f(): return 1`）の戻り値の型が `None` になる（6-2 の検討で見つけた）。原因は `parser/py_stub_extract.rs` の `py_infer_body_ret` が `def` の次の行からしか本体を探さないことだったが、master の ffi_value_exchange_plan.md 7-27 / 7-30（**利用者の決定**: 注釈・スタブファイルで型が分かるものはその型、ほかは `__init__` の戻り値を除き `Any`）で、本体の `return` から戻り値の型を推し量る処理そのものが無くなった。1 行の関数も戻り値は `Any` で、使う前に `mustbe` で確かめる（以前の `FfiTypeError: .. declared to return None but returned int` は起きない）。本体を同じ行から読む実装は取り込まず、例題 `interop/py_int_one_line.ar` を `Any` の規則で書き直して、1 行の `def` のシグネチャが読めることだけを確かめる。pandas 2.2.0 では最上位の `def` 9,378 のうち 9 がこの形（注釈なし） | 9 | なし | 小 |
| ~~**2-1**~~ | ✅ **済**（2026-10-05）。⚠ 変換時の写しはやめ、実行時の組み込み `isinstance` 1 本にした（1-7 で `list` / `tuple` も値になったので、名前・`m.C`・変数・組を同じ経路で判定できる。クラスは `class_id` と修飾名の祖先で見るので `x is C` より正確。`eval/py_builtins.rs`）。以下は起票時の案: `isinstance(x, C)` の写し: 第 2 引数が名前（`C` / `m.C` / `int`）なら `x is C`、組なら `or` へ変換時に写す。変数（`isinstance(x, cls)`）は実行時の組み込みで受ける。⚠ `isinstance(True, int)` は CPython で `True`（`bool` は `int` の派生） | 2,766 | なし | 中 |
| ~~**2-2**~~ | ✅ **済**（2026-10-05・3 引数は `TypeError`。クラスの表示はモジュール名を付けないまま）`type(x)`（1 引数）と `type(x).__name__` / `type(x) is C`。⚠ 3 引数の `type(name, bases, dict)`（クラスを作る）は対象外の候補 | 868 | なし | 中 |
| ~~**2-3**~~ | ✅ **済**（2026-10-05・読み書きは `o.name` と同じ経路。束縛メソッドが無いので `getattr(o, "method")` は 1-8 待ち）`getattr` / `hasattr` / `setattr`（既定値つき `getattr(o, n, d)` を含む） | 595 | なし | 中 |
| ~~**2-4**~~ | ✅ **済**（2026-10-05）`issubclass` / `callable` | 128 | 2-1 | 小 |
| ~~**3-1**~~ | ✅ **済**（2026-10-12） ~~`list(it)` / `tuple(it)`~~（1-7 で済）/ ~~`dict(..)`（呼べるようにする）~~（✅ 2026-10-05・Python の辞書内包も `dict([(k, v) for ..])` へ変換するようにした）/ ~~`frozenset`~~（✅ 2026-10-12・`Value::FrozenSet` / `InferredType::FrozenSet(Of)` を足した。作る・表示（`frozenset({1, 2})`）・`len` / `in` / 反復・演算子 `|` `&` `-` `^`（set と組めて結果は左の種類）・大小比較は包含関係（set にも足した）・`frozenset({1}) == {1}` は真（式としての `==` の層だけ。値の同一性 `values_eq` は型厳密のまま）・辞書のキーと集合の要素・書き換えないメソッド（set に `isdisjoint` を足し、`union` などの引数を任意のイテラブルに広げた）・`add` などは CPython と同じ `AttributeError`。型は `frozenset[T]`（`set[T]` と互いに代入できない・素の `frozenset` は注釈に書けない）。あわせて `bool(x)` を `if x:` と同じ規則にした（以前は空の tuple / dict が真）。例題は `collections/frozenset.ar`（CPython と同じ出力）。⚠ 集合の並びは作った順（CPython はハッシュの順・set と同じ差）。⚠ `import[py-int]` の CPython の set / frozenset は今も Python オブジェクトのまま） / ~~`bytes`~~（✅ 2026-10-12・`Value::Bytes(Rc<[u8]>)` / `InferredType::Bytes` を足した。`bytes()` / `bytes(n)` / `bytes(iterable)` / `bytes(s, encoding[, errors])`・`s.encode(encoding, errors)` が bytes を返す（以前は int の list）・`b.decode(..)`（`ops::codecs`: utf-8 / ascii / latin-1 と別名・strict / ignore / replace・誤りの文言と位置は CPython と同じ。`UnicodeDecodeError` / `UnicodeEncodeError` を例外の表に足した）・添字は int・スライス / `+` / `*` は bytes・`in` は int か部分列・大小は辞書順・辞書のキー・表示 `b'..'`（`ops::py_bytes_repr`）・メソッド 25 個（`classes/bytes_methods.rs`）。変換器は Python の `b".."` を `bytes([..])` に写す（以前は変換の誤り）。`import[py-int]` の CPython の bytes とも行き来する。例題は `basics/bytes.ar`・`interop/py_bytes.ar`（CPython と同じ出力）。⚠ 無いもの: `bytearray` / `memoryview`・`bytes.fromhex`・`%` の書式・`partition` など残りのメソッド・Arrow のソースの `b".."` のリテラル（構文を足すことになる）・例外の 5 引数の構築子と `start` / `end` の属性。⚠ Python の形で開いたファイルの `"rb"` の読みは今も int の list（5-4 で決める）） | 969 | なし | 中 |
| ~~**3-2**~~ | ✅ **済**（2026-10-05・6 節の `list.sort()` も入れた。Rust で実装した・前置きではない）`all` / `any` / `min` / `max` / `sum` / `sorted`（`key=` / `reverse=`）/ `reversed` / `abs` / `round` / `divmod` / `pow` | 401 | 1-2（`divmod` の float）・1-3（組を鍵にする比較） | 中 |
| ~~**3-3**~~ | ✅ **済**（2026-10-05・`format` を除く＝ **3-4 へ分けた**。`map` / `filter` / `iter(f, s)` は Arrow の `gen` で書いた前置きで遅延のまま）`map` / `filter` / `iter` / `hash` / `ord` / `chr` / `hex` / `oct` / `bin` / `format` / `ascii` | 98 | 1-4（`iter` の辞書・ジェネレータ） | 小 |
| ~~**3-4**~~ | ✅ **済**（2026-10-12）`format(value, spec)` と書式指定の小言語（3-3 から分けた）。`[[fill]align][sign][z][#][0][width][grouping][.precision][type]` を CPython 3.12 の `formatter_unicode.c` と同じ順序・同じ文言で解釈する 1 つの実装（`interpreter/ops/format_spec.rs`）を、3 つの入口から呼ぶ: ① 組み込みの `format`（`Interpreter::format_value`・インスタンスの `__format__` を呼ぶ）② `str.format`（`py_str_format` に書き直した: 変換 `!r/!s/!a`・属性 `{0.x}`・添字 `{0[k]}`・書式の中の置換 `{:{w}}`・自動と手動の番号の混用の誤り）③ f-string（パーサーが `{式[=][!変換][:書式]}` を分けて `format(x, spec)` / `repr(x)` … に脱糖。書式の無い `{x}` も `format(x)`＝CPython の `format(x, '')`）。変換器も Python の f-string の書式指定・`!a` を同じ形に写す（以前は変換の誤り）。`%` の `%e` / `%g` も同じ綴り（`1.234568e+04`）。照合は CPython と 145 件の書式（値・誤りの文言）を突き合わせて一致。例題は `basics/format_spec.ar`（`_error` あり）・`interop/py_format.ar`（CPython と同じ出力）。**あわせて直した誤り**: f-string の `:spec` / `!r` / `=` を**黙って捨てていた**（`f"{x:.2f}"` が `3.14159`）。式の後ろに読めない字句が残れば構文の誤りにした。⚠ `complex` の書式指定は未対応（明示の誤り）。⚠ `%` の他の欠け（`,` は CPython も非対応・`%-5s` などの詰めの細部）は手を付けていない | —（4.3 の `format`） | なし | 中 |
| ~~**3-5**~~ | ✅ **済**（2026-10-13）`enumerate` / `zip` をジェネレーターにする（5-4 の作業中に見つけた）。以前は元を**先に全部取り出して**組の列を作っていたので、ジェネレーターと振る舞いが違った（作った時点で元を回す・`break` しても残りを取り出し済み・`zip` が長い方も回し切る・`enumerate(src)` の後の `next(src)` が止まる・無限の列で終わらない）。`map` / `filter`（3-3）と同じく prelude の `gen`（`__py_enumerate` / `__py_zip2` / `__py_zip`）にしたので、中身がジェネレーターそのもので全部CPython と同じ（`zip` はどれかが尽きたら後ろの列を進めない）。速さも上がった（100 万要素で `enumerate` 670→481 ms・`zip` 774→593 ms。以前は組を全部作って持っていた）。あわせて: `next(it, default)` を足した、尽きた `next(it)` の `EndOfIteration` を例外の表に `StopIteration` の子として載せた（`except StopIteration` / `except EndOfIteration` で捕まる・Arrow の仕様の名前はそのまま・利用者の決定で A 案）、`enumerate(xs, 1)`（位置の `start`）を受ける（3 つの入口の引数の解釈を `enumerate_args` 1 つにした）。例題は `basics/lazy_iterators.ar`（CPython と同じ出力）。⚠ `type(enumerate(xs)).__name__` は `generator`（CPython は `enumerate`）。⚠ リストを回すと回し始めた時点の写しを回すのは従来どおり（Arrow の `for` と同じ） | — | なし | 小 |
| ~~**3-6**~~ | ✅ **済**（2026-10-20）dict のメソッド（6-2 の検討で見つけた）。`get` / `pop` / `setdefault` / `update` / `popitem` / `clear` / `copy` を CPython と同じに足した（`classes/dict_methods.rs` に `Value::Dict` のメソッドを切り出し・誤りの文言も同じ）。キーを引くのは `d[k]` と同じ（`__hash__` / `__eq__` を尊重）、型つきの辞書へ入れる `update` / `setdefault` は `d[k] = v` と同じ型の確認（`dict_store_checked` に 1 本化）。`update` の引数の解釈は `dict(..)` と共有（`dict_update_from`）。`setdefault` は入れた後の値を返す（`groups.setdefault(k, []).append(w)` が辞書の中に届く）。静的な型は `get(k)` が `Option[V]`（既定値が `V` なら `V`）・`pop(k)` が `V`・`popitem()` が `tuple[K, V]`。書き換えるメソッドは `let` の辞書には静的な誤り（`MUTATING_COLLECTION_METHODS`）。例題は `collections/dict_methods.ar`・`_error.ar`・`interop/py_dict_methods.ar`（CPython と同じ出力）。⚠ `copy()` は深い写し（CPython は浅い）。⚠ `del d[k]` は 3-7 へ分けた | 未計測 | なし | 中 |
| ~~**3-7**~~ | ➡ **[know_bugs.md](know_bugs.md) の #3 へ移した**（2026-10-22）。⚠ 要判断: `del d[k]` | 未計測 | 3-6 | 小 |
| ~~**4-1**~~ | ✅ **済**（2026-10-05・表は `type_check/names.rs` の `BUILTIN_EXCEPTIONS` 1 つにし、実行時の登録・内部エラーの変換・型検査がみなそこを読む。4.1 の警告 12・例外 24 と 4.2 の `FileNotFoundError` などの 5 つを足し、祖先を `bases` に平らに並べた。`open` の失敗は `io::ErrorKind` で振り分ける。`GeneratorExit` は `except Exception` で捕まらなくなった（CPython と同じ）。⚠ `IOError` / `EnvironmentError` / `WindowsError` は `OSError` と互いに捕まえ合うが**クラスは別**（`raise IOError("x")` の表示は `IOError: x`・CPython は `OSError: x`）。型検査では循環を避けて `OSError` の子にとどめたので `let e: IOError = OSError(..)` は静的に通らない。⚠ 4.3 の `ImportError` / `UnicodeDecodeError` / `SystemExit` / `ExceptionGroup` などは足していない。⚠ 内部の `SyntaxError: 'break' outside ..` は例外へ変えない（CPython ではコンパイル時の誤り）） 警告クラス（`FutureWarning` / `DeprecationWarning` / `RuntimeWarning` / `UserWarning` / `Warning` …）と、足りない例外クラス（`ImportError` / `LookupError` / `SyntaxError` / `FileNotFoundError` …）。⚠ 階層（`LookupError` → `KeyError` / `IndexError`）も合わせる（10-19 の `ClassValue::is_a`） | 382 | なし | 中 |
| ~~**4-2**~~ | ✅ **済**（2026-10-05・組み込みの例外は `__init__` を持たず、作るときの位置引数を `instantiate_evaled` が `args` に、CPython の `str(e)` を `message` に入れる＝`BaseException.__new__` と同じ位置。`super().__init__(..)` / `ValueError.__init__(self, ..)` は `eval_class_method` が受ける。`print(e)` / `str(e)` / `repr(e)` も CPython と同じ。`raise ValueError()` のトレースバックは名前だけ。内部の `KeyError` の文言も `repr` にした（`KeyError: 'zz'`）。⚠ 内部で起きた例外の `args` は文言 1 つの組（CPython は `KeyError` ならキーそのもの・`OSError` なら `(errno, strerror)`）。⚠ `Error` を実装しただけの Arrow のクラスは `args` を持たず、`print(e)` も `<MyErr object at ..>` のまま） 例外の `str(e)`（メッセージ）と `e.args` | — | なし | 小 |
| ~~**4-3**~~ | ✅ **済**（2026-10-05・構文は `except a.b.Err:` の修飾名を受け、変換器は属性の連なりを同じ綴りへ写す。照合は `x is m.Err` と同じ `value_is_type`（別名を引いて同じクラスか・修飾名の基底か）。型検査は束縛 `e` の型をモジュールの修飾名へ引き直す）  修飾名の例外を捕まえる `except m.Err:`（Python の `except requests.HTTPError:` の形）。今は Arrow の構文が受けず（`except e.MyErr:` が `ParseError: expected ':', got '.'`・Arrow のモジュールでも同じ）、変換器も明示エラーにしている（`only a simple exception name is supported in except`）。構文・変換器に足し、照合は `is` と同じ経路（`value_is_type` の修飾名・基底の修飾名）に載せる。`import[py]` のクラスの修飾名（6 節で直した）が前提 | —（構文） | なし | 中 |
| ~~**4-4**~~ | ✅ **済**（2026-10-05・入れ子の組は変換の誤りのまま。例題は `interop/py_except_tuple.ar`） Python の `except (A, B):`（組で複数の型を捕まえる形）。変換器は今も明示エラー（`multiple exception types is not supported`）。**型ごとに同じ本体の節を並べる**（`except A as e: body` / `except B as e: body`）へ写す。CPython は前から順に当てて最初の節だけを走らせるので、並べても走るのは高々 1 つで意味は変わらない。⚠ 組の入れ子（`except (A, (B, C)):`）は CPython 3.12 も実行時の `TypeError`（`catching classes that do not inherit from BaseException`・実測）なので変換の誤りにする。4-3 の修飾名も要素に書ける。4-3 の作業中に起票 | —（構文） | 4-3 | 小 |
| ~~**4-5**~~ | ✅ **済**（2026-10-13）Arrow のファイルの `EOFError` / `BOFError` を捕まえられるようにした（5-4 の作業中に見つけた）。Arrow の形の `read()` / `read_line()` / `read_letter()` が終わりで出す `EOFError: EOF` は例外の表（`names::BUILTIN_EXCEPTIONS`）に無い名前で、`except EOFError` は静的な誤り、`except Exception` も素の `except:` も捕まえず、プログラムが止まっていた。表に `EOFError`（CPython と同じく `Exception` の子）と、Arrow 独自の `BOFError`（逆向きに読んで先頭に着いた・`Exception` の子。`EOFError` の子にはしない＝`except EOFError` が逆向きの終わりまで捕まえると読み違える）を足した（表 1 つを実行時の登録・内部エラーの変換・型検査が読むので、それだけで全部に効く）。`raise EOFError(..)` も書ける・Python のモジュールの `EOFError` も通る。例題は `exceptions/eof_error.ar` | — | なし | 小 |
| ~~**5-1**~~ | ✅ **済**（2026-10-05・`print` の `sep=` / `end=` / `flush=` を `eval_builtin_evaled_named` で受け、`VM_BUILTIN_KW_NAMES` に足した。CPython と同じく `None` は既定、文字列以外は `TypeError`。⚠ `file=` は `None` だけ。最上位の `print(.., sep=..)` も同じ `VmForceError` だった） ~~呼び出しの形だけ解決される組み込み（`range` / `repr` …）を**値として**使えるようにする。~~（**1-7 へ移した**）`print` のキーワード引数（`end=` / `sep=`）で関数がバイトコードにできない問題 | 16 | なし | 中 |
| ~~**5-2**~~ | ✅ **済**（2026-10-05・2 番目が文字列か省略なら Python の形とみて Arrow の列挙の値へ写す: r / w / x / a と b / t / +・`encoding=`（UTF-8 / UTF-8-SIG / ASCII）・`errors="strict"`・`newline=None / "" / "
"`・`buffering`。`"a"` は無ければ作る、`"x"` はあれば `FileExistsError`。文言は CPython と同じ（`invalid mode: 'q'` …）。⚠ `"r+"` は Arrow の `write` が位置へ挿入する（CPython は上書き）ので明示の `NotImplementedError`。⚠ 改行は変換しない（CPython は Windows の既定で書くときに CR LF にする）。⚠ 返るのは Arrow のファイル（5-4）） `open` を Python の形（`open(path, "r", encoding=..)`）で受ける | 6 | なし | 小 |
| ~~**5-3**~~ | ✅ **済**（2026-10-05・ブロック文は `compile_block_expr_inner` を直に呼び、`block_body_bails` を掛けない。囲むブロック式があればそちらが従来どおり断る。例題は `basics/block_stmt_return.ar`、`interop/py_open.ar` の `with ...: return f.read()`） 関数の中の `block:` **文**の中の `return` が `VmForceError`（Arrow の `block:` も、Python の `with ...: return ..` の変換先も）。ブロック**式**は演算子のスタックが積まれた途中なので `return` を `block_body_bails` で断っているが、ブロック**文**は入口でスタックが平衡（`stmt_base`）なので普通の文と同じに抜けられるはず。5-2 の作業中に見つけた（`with open(p) as f: return f.read()` が読めない） | —（VM） | なし | 小 |
| ~~**5-4**~~ | ✅ **済**（2026-10-13）Python のファイルのメソッド（5-2 の作業中に起票）。① どのファイル（Arrow の形でもPython の形でも）にも Python の名前を足した: `readline(size)` / `readlines(hint)` / `writelines(xs)` / `close()` / `flush()` / `tell()` / `for line in f`（取り出すたびに `readline()` を呼ぶ遅延のイテレータ＝途中で抜けて続きを読める・prelude の `__py_file_lines`）/ 属性 `closed` / `name` / `mode`。`readline()` は終わりで `""`（Arrow の `read_line()` は従来どおり `EOFError`）。② 同じ名前で意味の違う `read()` / `write()` は、Python の形で開いたファイル（`FileData::py_mode`）だけ CPython の意味にした（終わりの `read()` は `""`・`read(n)`・`write` は書いた数・`"rb"` は `bytes`・閉じたファイルは `ValueError: I/O operation on closed file.`・読めない／書けないモードは `OSError: not readable` / `not writable`）。Arrow の形のファイルは従来どおり。**あわせて直した誤り**: 変換器の `with` が破棄だけに頼っていたので、VM がブロックの変数を関数の終わりまで持つと、同じ関数の中で `with open(p, "w")` の後に `with open(p)` で読むと書き戻される前だった。`with` を `try` / `finally` で包み、抜けるときに `__py_with_exit(x)`（ファイルなら閉じる）を呼ぶ。例題は `interop/py_file_methods.ar`（CPython と同じ出力）・`basics/file_lines.ar`（`_error` あり）。⚠ `seek` と `"r+"` は対象外（Arrow の `write` は挿入）。⚠ CPython の `io.UnsupportedOperation` は `OSError` で出す（Arrow の例外の表は単一継承） | 6 | 5-2 | 中 |
| ~~**6-1**~~ | ✅ **済**（2026-10-09・**決定: CPython と同じ `property` クラスとディスクリプタのプロトコルで受ける**。getter をメソッド呼び出しへ写す案・名前マングリングで置き換える案は、変換時に受け手の型が分からない／`self._constructor(..)` の形が書き換えられないので採らなかった）`property`。① 組み込みのクラス `property`（`eval/py_builtins.rs` の `PY_PROPERTY_SRC`。Arrow で書いたクラスで、名前を初めて引いたときに自分の大域の枠で組み立てる）。`fget` / `fset` / `fdel` / `__doc__`・`__get__` / `__set__` / `__delete__`・`getter` / `setter` / `deleter`（名前ごと写す）・`__set_name__`。文言は CPython 3.12 と同じ（`property 'x' of 'C' object has no setter`・名前が無ければ `property of 'C' ..`）。`C.x is C.x`。② 変換器（`python_converter/classes.rs`）: いちばん外側の `@property` / `@x.setter` / `@x.getter` / `@x.deleter` / `@Base.x.getter` のメソッドを別名（`__property_get_x` など）にし、`x` をクラスの属性の置き場（`static mut x: Any = None`）にして、**クラスの定義の後で** `C.x = property(C.__property_get_x)` / `C.x.__set_name__(C, "x")` / `C.x = C.x.setter(..)` を並べる（クラス本体の名前もクラス自身もクラス属性の初期化式から見えない＝穴 2 の対処）。`self.x = v` はフィールドにしない。③ あわせて、`__init__` 以外のメソッドだけが代入するフィールド（setter の `self._c = v`）もフィールドにした（以前は `has no field` で、`__init__` が setter を通す普通の形が動かなかった）。④ クラス属性の `name` をクラスから読めるようにした（Arrow の `C.name`＝クラスの名前がメンバーより先で、`@property def name` や `name = ..` を隠していた）。前提 1-11（属性に入れた関数を呼ぶ・`self._constructor(..)`）・1-12（ディスクリプタ）。例題は `interop/py_property.ar`（CPython と同じ出力）・`interop/py_property_error.ar`。⚠ 残りは **6-3〜6-8 へ分けた**（`x = property(getx, setx)`・`del obj.x`・`@cached_property`・内側の `@property`・docstring・精密な型） | 586 | 1-11 / 1-12 | 大 |
| ~~**6-2**~~ | ✅ **済**（2026-10-21）`super(C, self)` の 2 引数の形（`super().m(..)` は対応済み）。変換器（`python_converter/expressions.rs` の `super_form`）が `super(C, x).m(args)` を、`C` が変換中のクラスなら `super().m(args)` と同じ `<第 1 基底>.m(x, args)` に書き換える（受け手は `x`・基底が `object` なら `__py_object_init`・6-10）。変換中のクラスの名前は `supers.rs` のスタックに足した（`current_super_class`）。`builtins_check` はこの形の `super` を未対応の組み込みにしない。定義中でないクラスを渡す形（MRO の途中から探す `super(Middle, self)`）と、メソッド呼び出し以外の `super(C, x)` は変換の誤り。例題は `interop/py_super_two_args.ar`（CPython と同じ出力）・`_error.ar`。⚠ `@classmethod` の中の `super()` は 2 引数でも対象外（受け手を `self` とする書き換えなので・従来の `super()` と同じ）。⚠ 2026-10-20 に `object` / `Ellipsis` / `NotImplemented` / `staticmethod` / `classmethod` を 6-10〜6-13 へ分けた | 3 | なし | 小 |
| ~~**6-3**~~ | ✅ **済**（2026-10-12）クラス本体の名前を参照するクラス属性（6-1 から分けた）。変換器（`python_converter/classes.rs` / `class_scope.rs`）は ① 初期化式がクラス本体で**先に**束縛した名前（メソッド・クラス属性・入れ子のクラス）を引いていれば、置き場を `None` にして**クラスの定義の後で** `C.x = <名前を C.名前 に書き換えた式>` を置く（`x = property(getx, setx)` / `DOUBLE = LIMIT * 2` / `{"get": getx}` / `[i for i in range(N)]`）② メソッドの別名（`alias = getx` / `__repr__ = __str__`）は同じメソッドをその名前でも定義する（以前は `__` で始まる名前のクラス属性を黙って捨てていた）③ `name = lambda self: ..` はメソッドにする（以前は束縛しない関数の `static mut` で `missing argument 'self'`）。Python の名前の規則に合わせ、ラムダの本体と内包表記の要素の式・後で束縛される名前は書き換えない（大域を引く）。例題は `interop/py_class_body_names.ar`（CPython と同じ出力）。⚠ `staticmethod(f)` / `classmethod(f)` を値として置く形は 6-2 | — | 6-1 | 中 |
| ~~**6-4**~~ | ➡ **[know_bugs.md](know_bugs.md) の #2 へ移した**（2026-10-22）。⚠ 要判断: `del obj.x` | — | 6-1 | 中 |
| ~~**6-5**~~ | ✅ **済**（2026-10-12）`@functools.cached_property`（6-1 から分けた）。組み込みのクラス `cached_property` を Arrow で書いた（`eval/py_builtins.rs` の `PY_CACHED_PROPERTY_SRC`・`property` と同じく名前を初めて引いたときに組み立てる＝`py_builtin_class`）。非データディスクリプタで、`__get__` は `func(obj)` をインスタンスの**同じ名前のフィールド**へ `setattr` し、読み直した値を返す（代入は値を写すので）。変換器は `@cached_property` を property と同じ形で扱い（メソッドは `__property_cached_x`・クラスの後で `C.x = cached_property(C.__property_cached_x)` と `__set_name__`）、CPython のインスタンスの `__dict__` の代わりに**同じ名前のインスタンスのフィールド `x: Any`**（初期値なし）を置き場の `static mut x` と並べて宣言する。実行時は、フィールドが入っていれば読みはフィールドが勝ち（`get_attr_val` は非データディスクリプタより先にフィールドを見る）、**代入は非データディスクリプタと同じ名前のフィールドがあればフィールドへ**（`attr_assign_evaled`。以前は `static mut` の共有セルを書き換えた）。型検査は置き場の型（getter の戻り値の注釈・6-8）を見る。`from functools import cached_property` は読み込まない（ほかの名前は残す）。例題は `interop/py_cached_property.ar`（CPython と同じ出力）。⚠ `del obj.x` は変換の誤り（6-4 の `del` と同じ）。⚠ `import functools` / 別名（`as cp`）は対象外 | — | 6-1 | 中 |
| ~~**6-6**~~ | ✅ **済**（2026-10-12）内側に書いた `@property`（6-1 から分けた）。`@logged @property def x` は CPython では `logged(property(f))`。変換器は property のデコレータを並びのどこにあっても受け、**内側**のデコレータはメソッドに、**外側**のデコレータは組み立てた property に当てる（クラスの定義の後に `C.x = logged(C.x)` を足す・`@x.setter` も同じ）。⚠ 名前（`__set_name__`）は包む前の property に付ける（CPython は包んだ後の値に付ける。包みが別の物を返すときだけ違う）。外側の `@staticmethod` / `@classmethod` / `@abstractmethod`・property のデコレータが 2 つ、は変換の誤り。例題は `interop/py_property.ar`（CPython と同じ出力）。`py_decorators_error` の ① は名前の違う setter に替えた | — | 6-1 | 小 |
| ~~**6-7**~~ | ✅ **済**（2026-10-12）property の `__doc__` に getter の docstring を引き継ぐ（6-1 から分けた）。変換器がメソッドの docstring（本体の最初の文字列）を読み、`@property` は `property(f, None, None, doc)`、`@x.getter` / `@Base.x.getter` は差し替えた後で `C.x.__doc__ = doc`（docstring が無ければ `None`）にする。setter / deleter の docstring は使わない（どれも CPython 3.12 の `property_copy` / `getter_doc` と同じ結果・実測）。例題は `interop/py_property.ar`。⚠ 関数の値そのものの `__doc__` は無い（1-14） | — | 6-1 | 小 |
| ~~**6-8**~~ | ✅ **済**（2026-10-12）property の型（6-1 から分けた）。変換器（`python_converter/classes.rs`）が置き場の型を getter の戻り値の注釈にする（`@property def celsius(self) -> float` ⇒ `static mut celsius: float`。後から `@x.getter` で置いた注釈も効く・無ければ `Any`）。Arrow から `t.celsius` を読むと `float` になり、そのまま演算できる（以前は `OperationOnAny`）。`property[T]` の型は足さなかった（置き場の宣言型をフィールドの型として読む既存の経路で足りる。実行時は `static mut` の初期値・代入の型を検査せず、モジュールの本体の型の誤りは捨てるので、置き場に property が入っても困らない）。例題は `interop/py_property_typed.ar`（`_error` あり）。⚠ Python の注釈は実行時に検査しない（getter が違う型を返すと食い違う）。⚠ クラスから読む `C.x` も同じ型として扱う（実行時は property）。⚠ `x = property(getx)`（6-3 の形）は `Any` のまま | — | 6-1 / 1-12 | 中 |
| ~~**6-9**~~ | ➡ **[know_bugs.md](know_bugs.md) の #1 へ移した**（2026-10-22）。⚠ 要判断: Python のクラスのインスタンスの共有（既定の `__eq__` をアドレスの比較にする決定の前提） | — | 6-10 | 大 |
| ~~**6-10**~~ | ✅ **済**（2026-10-20）`object`（6-2 から分けた）。決定どおり: ① すべての値が `object` の派生（`isinstance(x, object)` / `issubclass(C, object)` / `x is object` が真。`ClassValue::is_a`・`primitive_ann_matches`・`py_isinstance` の型の判定だけの特例。注釈の `object` は静的には `Any`） ② Python の `object()` は変換器が `__py_object()` にする。Arrow のソースの `object()` は静的な誤り（`ObjectCallInArrow`）、クラスの値を呼ぶのは実行時の誤り ③ `object()` の値は組み込みのクラス（`BuiltinClass::Object` の印を持つ `ClassValue`・名前を初めて引いたときに作る）のインスタンスで、フィールドを持たない。写す関数（`deep_copy_value` / `deep_copy_unfrozen`）は同じ物を返し、`==` / `hash` は同一性（`is` / `id` / 表示 `<object object at 0x..>` は元からアドレス）。印を受ける所は網羅の `match`。あわせて: 基底が `object` のクラス（基底を書かない・`class C(object)`）の `super().__init__(..)` を `__py_object_init(self, ..)` にした（余分な引数はCPython と同じ `TypeError`。以前は `NameError: 'object'`／変換の誤り）。`super()` の `*args` / `**kwargs` を普通の呼び出しと同じに渡す（以前は `**` が変換の誤り）。`o.x = 1` の文言を CPython と同じにした。例題は `interop/py_object.ar`（CPython と同じ出力）・`interop/py_object_error.ar`。`object()` と `__eq__` を持つ値の `==`（`_M == V()`）は 6-12 で CPython と同じになった（反対側の `__eq__` を試す）。⚠ スレッドへ送る複製（`async`）では別の物になる。⚠ `type(o)()`・`object` のほかのメソッド（`super().__setattr__` / `object.__new__`・4.3）は対象外 | 約 255 | なし | 中 |
| ~~**6-11**~~ | ✅ **済**（2026-10-20）`Ellipsis`（6-2 から分けた）。決定どおり、値の `...` を `None` ではなく `Ellipsis` にした（変換器の `convert_constant` が名前 `Ellipsis` にする。文の `...` は従来どおり `pass`）。`Ellipsis` は組み込みのクラス `ellipsis` の唯一のインスタンス（`BuiltinClass::Ellipsis` の印・名前を初めて引いたときに作る `builtin_singleton`）。表示は `Ellipsis`、写さずに共有し、`is` / `==` / `hash` は種類で決まる（スレッドへ送った複製も同じ物）。`type(...)()` は `Ellipsis`、引数があれば CPython と同じ `TypeError: EllipsisType takes no arguments`。Arrow のソースからも `Ellipsis` の名前で使える。例題は `interop/py_ellipsis_value.ar`（CPython と同じ出力）。既存の `interop/py_ellipsis.ar` の ⑤（値の `...`）も CPython と同じ `Ellipsis` になった | 15 | 6-10 | 小 |
| ~~**6-12**~~ | ✅ **済**（2026-10-20）`NotImplemented` と演算子の手順（6-2 から分けた）。決定どおり CPython に準拠: `NotImplemented` は組み込みのクラス `NotImplementedType` の唯一のインスタンス（`BuiltinClass::NotImplemented`・`Ellipsis` と同じ作り）。二項演算子の演算子メソッドは `apply_binop_dyn` の `binop_by_dunders` 1 か所で CPython の `binary_op1` / `do_richcompare` と同じ手順で探す: 右が左の真の派生で反対側のメソッドを持てば右が先 → 左の `__add__` → 無いか `NotImplemented` なら右の `__radd__`（比較は向きを入れ替えた `__gt__` など・`==` / `!=` は同じ名前・左と同じクラスなら呼ばない・左が組み込みの値でも試す）→ 全部駄目なら `==` / `!=` は同一性、ほかは CPython と同じ文言の `TypeError`（`unsupported operand type(s) for +: ..` / `'<' not supported between instances of ..` / 左が str・list・tuple の `+` `*` は列の文言）。`!=` は `__ne__` が無ければ `__eq__` の反対。`in` / `index` など（`values_eq_dyn`）も同じ手順で、答えは真偽で読む。`str % x` は従来どおり str の書式。⚠ どちらにもメソッドが無い `==` は従来どおり中身で比べる（6-9）。⚠ 静的な検査は左のメソッドの仮引数だけ（右の反対側のメソッドは見ない）。⚠ `__iadd__` など複合代入の演算子メソッドは無い（`+=` は `+` の手順）。例題は `interop/py_not_implemented.ar`（CPython と同じ出力）・`classes/reflected_operators.ar`・`classes/reflected_operators_error.ar`。⚠ 見つけたが対象外: `"%s" % x` が `x` の `__str__` / `__repr__` を呼ばない（`<C object at 0x..>`） | 50 | 6-10 | 大 |
| ~~**6-13**~~ | ✅ **済**（2026-10-20）`staticmethod` / `classmethod` を関数として使う形（6-2 から分けた）。変換器（`python_converter/classes.rs` の `wrapped_method`）がクラス本体の `f = staticmethod(f)` / `g = classmethod(f)` を、先に定義したメソッドに `@staticmethod` / `@classmethod` を付けたのと同じにする（名前が違えば `twice = staticmethod(double)` の名前の写しを足し、元のメソッドは残す・CPython と同じ）。`builtins_check` はこの形の `staticmethod` / `classmethod` を未対応の組み込みにしない。クラス本体の外で定義した関数を包む形（`bump = staticmethod(helper)`）・クラス本体の外の `staticmethod(..)` は変換の誤り。例題は `interop/py_static_class_funcs.ar`（CPython と同じ出力）・`_error.ar`。⚠ 見つけたが対象外（デコレータの形も同じ）: 静的メソッド・クラスメソッドをインスタンスから呼ぶ形（`self.helper(..)`）が Arrow の規則で実行時の `AttributeError`（CPython は呼べる） | 0 | なし | 小 |

⚠ 対象外の候補（実行時にコードを作る・環境を覗く・対話用）: `exec` / `eval` / `compile` / `globals` / `locals` / `vars` / `dir` /
`__import__` / `__build_class__` / `breakpoint` / `input` / `help` / `exit` / `quit` / `copyright` / `credits` / `license` / `memoryview` / `aiter` / `anext`。
対応しないなら 1-1 で「未対応」と知らせる。

## 6. 測定のついでに見つけたもの（組み込み以外）

- ~~遅延のジェネレータ（`gen` 関数）を `zip` / `enumerate` / `set` / `*` の展開に渡すと黙って空・`TypeError`~~ → **直した**（タスク 3-3）。
  列への展開を `drain_iterable`（`for` と同じ規則で最後まで回す）に 1 本化し、実体化済みの値しか見ない `collect_iterable` を消した。
- ~~`list.sort()`（メソッド）が無い~~ → **足した**（タスク 3-2・`sorted` と同じ安定な並べ替え `py_sort_values`）。
- 以下は 4 節の分類で見つけた（どれも実測）。4.1 の前置きで書くものの正しさに効く:
  - ~~整数の `//` / `%` が**割る数が負のとき** CPython と違う。`-7 // -2` が `4`（CPython `3`）、`7 % -2` が `1`（CPython `-1`）。~~
    → **直した**（2026-10-01）。`div_euclid` / `rem_euclid` をやめ、`ops::py_floor_div` / `ops::py_mod` に 1 本化した
    （`apply_binop` と VM の `int_binop_specialized` の 2 か所。ネイティブ codegen の `@_tl_idiv` / `@_tl_imod` は元から正しかった）。
    回帰の例題は `examples/basics/int_floor_div_mod.ar`。
  - ~~float の `//` / `%` が無い~~ → **直した**（タスク 1-2・2026-10-05）。`ops::py_float_div_mod` に CPython の手順を写し、
    ネイティブ codegen も `@_tl_ffloordiv` / `@_tl_fmod` で同じ手順に揃えた。例題は `examples/basics/float_floor_div_mod.ar` / `_error.ar`。
    - 再現: Python の `def fdiv(a, b): return a // b` を `fm.fdiv(7.5, 2.0)` で呼ぶと
      `TypeError: unsupported operand types for 'FloorDiv': float and float`。Arrow で書いた `a // 2.0`（`a: float`）は
      静的に `unsupported operand types for '//': 'float' and 'float'`。
    - CPython（3.12.2・実測）: `-7.5 // 2.0` → `-4.0`、`-7.5 % 2.0` → `0.5`、`7.5 % -2.0` → `-0.5`、`6.0 % -3.0` → `-0.0`、
      `0.1 % 0.01` → `3.469446951953614e-18`、`-1.0 % inf` → `inf`、`1.0 // 0.0` → `ZeroDivisionError: float floor division by zero`、
      `1.0 % 0.0` → `ZeroDivisionError: float modulo`。
    - CPython の実装（`Objects/floatobject.c`）: `//` は `float_floor_div` → `_float_div_mod`、`%` は `float_rem`。
      `mod = fmod(vx, wx)`（`a/b` を経由しないので余りが厳密）→ 余りが 0 でなく符号が割る数と違えば `mod += wx; div -= 1.0` →
      余りが 0 なら `copysign(0.0, wx)` → 商 `div = (vx - mod) / wx` を `floor` して 0.5 を超えるずれは繰り上げ、商が 0 なら
      `copysign(0.0, vx / wx)`。int と混ざったら先に double へ変換する（`CONVERT_TO_DOUBLE`）。Rust の `f64 % f64` は C の `fmod` と同じ。
  - ~~list 同士・tuple 同士の大小比較（`<` など）が無い~~ → **直した**（タスク 1-3・2026-10-05）。実行時は `apply_binop_dyn` の
    `seq_ordered_cmp`（要素の比較に演算子メソッドを使えるように `&mut self` 側）、静的は `ordered_comparable` が要素ごとに見る
    （tuple は短い方の長さまでの全位置・要素の `Union` は構成型ごと）。例題は `examples/collections/seq_ordering.ar` / `_error.ar`。
    - 再現: Python の `if r > best:`（`r` / `best` がタプル）が `TypeError: unsupported operand types for 'Gt': tuple and tuple`。
      Arrow で書いた `(1, 2) < (1, 3)` は静的に `cannot compare 'tuple[int, int]' and 'tuple[int, int]' with <`。
    - CPython（実測）: `(1, 2) < (1, 2, 0)` → `True`、`(1, 'a') < (2, 3)` → `True`、`(1, 'a') < (1, 3)` → `TypeError: '<' not supported
      between instances of 'str' and 'int'`、`[1] < (1,)` → `TypeError: ... 'list' and 'tuple'`、同じ NaN を入れた `[nan] == [nan]` → `True`。
    - CPython の実装（`Objects/listobject.c` の `list_richcompare`・`Objects/tupleobject.c` の `tuplerichcompare`）: 両方が同じ種類で
      なければ `NotImplemented`（→ `TypeError`）。`PyObject_RichCompareBool(.., Py_EQ)`（同じオブジェクトなら等しい）で最初に違う
      位置を探し、無ければ長さを比べ、あればその要素だけを求められた演算子で比べ直す。list だけ `==` / `!=` で長さが違えば先に返す。
  - ~~float の表示が CPython と違う（指数表記を使わない）~~ → **直した**（タスク 1-5・2026-10-05）。`ops::py_float_repr`。
    complex も CPython の `complex_repr` に揃え（`(1+2j)` / `2j`）、実数 − complex の虚部の符号（`1.5 - 0j` が `(1.5-0j)`）も直した。
    例題は `examples/basics/float_repr.ar`。
    - 再現: `print(1e20)` → `100000000000000000000`（CPython `1e+20`）、`print(1e-5)` → `0.00001`（CPython `1e-05`）、
      `print(1.5e-7)` → `0.00000015`（CPython `1.5e-07`）、`print(2.5e300)` → 301 桁の整数（CPython `2.5e+300`）。
  - ~~辞書とジェネレータに `__iter__` メソッドが無い（list・str・set にはある）~~ → **直した**（タスク 1-4・2026-10-05）。
    辞書はキーの写しを回すジェネレータ（`for` と同じ）、ジェネレータは自分自身を返す。結果の型は `generator[要素の型]`。
    例題は `examples/collections/dunder_iter.ar`。
    - 再現: Python の `next(d.__iter__())` が `AttributeError: 'dict' object has no method '__iter__'`、
      `next(nums().__iter__())`（`nums` はジェネレータ関数）が `AttributeError: Generator object has no method '__iter__'`。
      `for k in d:` / `for x in nums():` は動く。
    - CPython（実測）: `next(d.__iter__())` は挿入順で最初のキー、型は `dict_keyiterator`。`iter(g) is g` は `True`。
      反復中にキーを足すと `RuntimeError: dictionary changed size during iteration`。
    - CPython の実装: 辞書（`Objects/dictobject.c`）は `tp_iter = dict_iter` → `dictiter_new(dict, &PyDictIterKey_Type)`。
      イテレータは作った時の大きさ（`di_used`）と位置を持ち、`dictiter_iternextkey` が毎回 `di_used != ma_used` を見て止める
      （止めたら止まったまま）。挿入順の表を進み、消された枠は飛ばす。ジェネレータ（`Objects/genobject.c`）は
      `tp_iter = PyObject_SelfIter`（自分を返す）。`iter(x)` は `Objects/abstract.c` の `PyObject_GetIter`（`tp_iter` が無ければ
      `__getitem__` の列として回すか `'X' object is not iterable`、返り値がイテレータでなければ `TypeError`）。
  - ~~Python のクラスを**修飾名**で書いた型の判定（`import[py] mod as m` の後の `b is m.Box`）が静的に `unknown type 'm.Box' in type guard`。~~
    → **直した**（2026-10-01）。原因は型検査のレジストリが `import[py]` のモジュールのクラスを素の名前（`Box`）で登録していたこと
    （10-8 のモジュール名での修飾が Arrow のソースだけだった）。`builder::names_types_by_module` で `py` も修飾し、
    `z.Dog` / `zoo.Dog` が `is` / `mustbe` / 型注釈に書けるようにした。`z.Dog()` の呼び出しの検査もメインの同名クラスを見ていたので直した
    （`call_check.rs`・`PyNamespace` も修飾名で引く）。実行時は、Python のクラスの祖先（`bases`）に基底の修飾名も載せ、
    `p is z.Animal`（`class Dog(Animal)` の派生）も当たるようにした（以前はクラス自身しか見ず偽）。
    別のモジュールの同名クラスも分かれる（kennel の `Dog` と zoo の `Dog`）。例題は `examples/interop/py_module_types.ar` / `_error.ar`。
    ⚠ 修飾名の `except m.Err:` は構文・変換器の両方が受けないので別件（**タスク 4-3**）。

