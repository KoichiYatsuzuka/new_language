# Python 変換: CPython 組み込みの対応計画

`import[py]` で変換した Python のコードの中で、**CPython の組み込み（`builtins` モジュールの名前）の多くが使えない**。
本書は、使えない組み込みの一覧と、実用上の損害（pandas を読み込んだときに認識されない箇所の数）を測り、
対応の順序を決めるための計画書である。

- 起票: 2026-09-30（フェーズ10 の 10-18 の作業中に `isinstance` が使えないことに気づいたのが発端。`type_check_redesign.md` とは別件）
- 測定: Arrow `ff4afe1`・CPython 3.12.2・pandas 2.2.0

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

⚠ 測定に使ったスクリプト（名前ごとの試験・pandas の集計）は、まだリポジトリに入れていない。タスクを進めて数え直すときは `scripts/` に置く（規約どおり `.ps1` から呼ぶ形にする）。

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

## 4. 実装予定

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

## 5. 測定のついでに見つけたもの（組み込み以外）

- `list.sort()`（メソッド）が無い（`AttributeError: 'list' object has no method 'sort'`）。組み込み関数ではないので本書の数には入れていない。

