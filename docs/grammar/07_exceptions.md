# 例外処理

---

## try / except / finally

```ar
try:
    let result = risky_operation()
    process(result)
except ValueError as e:
    print("ValueError:", e.message)
except TypeError:
    print("type error")
except:
    print("unknown error")
finally:
    cleanup()
```

`Stmt::Try { body, handlers: Vec<ExceptHandler>, finally_body: Option<Vec<Stmt>> }`

### ExceptHandler の構造

```rust
struct ExceptHandler {
    exc_type: Option<String>,   // 例外クラス名（`m.Err` の修飾名も）。None は bare except (全捕捉)
    name:     Option<String>,   // as で束縛する変数名
    body:     Vec<Stmt>,
}
```

**実行**（VM の `SetupTry` / `ExcMatch`）:
1. `try` ボディを実行
2. 例外が発生した場合 (`RAISE_SENTINEL` または `ExecResult::Raise`):
   - 各 `except` ハンドラを上から順に評価
   - `exc_matches(exception, handler.exc_type)` でマッチ判定
   - `bare except` は全例外にマッチ
   - マッチしたハンドラの `name` に例外インスタンスをバインドしてボディを実行
   - マッチするハンドラがなければ例外を再送出
3. `finally` ボディを例外有無に関わらず実行

### 例外マッチング (`exc_matches` / `vm_exc_matches`)

```ar
try: ...
except Exception:    # GeneratorExit 以外のすべての例外（組み込みも、Error を実装した利用者の例外も）
    ...
except LookupError:  # KeyError / IndexError も（組み込みの例外の階層・下の表）
    ...
except m.Err:        # モジュールを通した名前（import m）・Python の except requests.HTTPError: も同じ
    ...
```

- 照合は `ClassValue::is_a`: 例外のクラスの名前が `exc_type` か、`bases` に `exc_type` があれば当たる。
  組み込みの例外は**祖先を `bases` に平らに並べて**持つので、親で捕まえると子も捕まる。
- `Error` を実装しただけの利用者の例外（`class MyErr(Error)`）は `Exception` / `BaseException` の派生として扱う。
- 修飾名（`except m.Err:`）は `x is m.Err` と同じ経路（`value_is_type`）: モジュールの別名を引いて同じクラスか、
  修飾名の基底に載っているか（`import[py]` のクラスの派生も当たる）。
- `except Error` は静的な誤り（`except Exception` を書く）。知らない名前の `except` も静的な誤り（腕が永久に死ぬ）。

---

## raise 文

```ar
raise ValueError("invalid input")  # 例外を送出
raise                               # 現在の例外を再送出 (bare raise)
```

`Stmt::Raise { exc: Option<Expr>, span: Span }`

**実行**:
1. `exc` がある場合: 式を評価 → `Value::Instance` であることを確認
2. `exc` がない場合: `current_exception` を再送出
3. スタックフレームを構築して `RaisedError` を作成
4. `ExecResult::Raise(raised_error)` を返す

---

## 組み込み例外クラス

すべての組み込み例外クラスは `Error` trait を実装し、**CPython 3.12 と同じ階層**を持ちます（60 クラス）。
表は `src/type_check/names.rs` の `BUILTIN_EXCEPTIONS`（名前と親）1 つで、実行時の登録・内部エラーの変換・
型検査がみなここを読みます。

```
BaseException
 ├── GeneratorExit                ← Exception の派生ではない（except Exception で捕まらない）
 └── Exception
      ├── ArithmeticError ── FloatingPointError / OverflowError / ZeroDivisionError
      ├── LookupError ── IndexError / KeyError
      ├── NameError ── UnboundLocalError
      ├── OSError ── FileNotFoundError / FileExistsError / PermissionError / IsADirectoryError /
      │              NotADirectoryError / TimeoutError / ConnectionError (── BrokenPipeError …) / …
      │              （IOError / EnvironmentError / WindowsError は OSError と互いに捕まえ合う）
      ├── RuntimeError ── NotImplementedError / RecursionError
      ├── SyntaxError ── IndentationError ── TabError
      ├── ValueError ── UnicodeError ── UnicodeTranslateError
      ├── Warning ── UserWarning / DeprecationWarning / FutureWarning / RuntimeWarning / …（12）
      ├── AssertionError / AttributeError / BufferError / ReferenceError / StopIteration /
      │   SystemError / TypeError
      └── AccessError                ← Arrow 独自（private / protected の違反）
```

- `open` の失敗は OS のエラーの種類で `FileNotFoundError` / `PermissionError` / `FileExistsError` /
  `IsADirectoryError` / `NotADirectoryError` に振り分けます（それ以外は `IOError`）。
- ⚠ `IOError` などの別名は CPython では `OSError` そのものですが、Arrow ではクラスが別です
  （`raise IOError("x")` の表示は `IOError: x`。CPython は `OSError: x`）。
- ⚠ `SystemExit` / `KeyboardInterrupt` / `ImportError` / `UnicodeDecodeError` / `ExceptionGroup` などは
  まだありません（意味が要るもの）。

### 例外インスタンスのフィールド

| フィールド | 型 | 説明 |
|---|---|---|
| `message` | `str` | エラーメッセージ（CPython の `str(e)`） |
| `args` | `tuple` | 作るときの位置引数（組み込みの例外とその派生だけ） |
| `code_context` | `str` | 発生箇所のソースコンテキスト |
| `file` | `str` | ファイル名 |
| `line` | `int` | 行番号 |
| `col` | `int` | 列番号 |

`code_context`/`file`/`line`/`col` は `raise` 実行時にインタープリタが自動設定します。

### 組み込みの例外を作る・表示する

組み込みの例外は CPython と同じく**位置引数をいくつでも**受け、`args` に持ちます。

```ar
let a = ValueError()            # args = ()        str(a) = ''
let b = ValueError("bad")       # args = ('bad',)  str(b) = 'bad'
let c = ValueError("bad", 42)   # args = ('bad', 42)  str(c) = "('bad', 42)"
print(repr(c))                  # ValueError('bad', 42)
print(KeyError("k"))            # 'k'（KeyError は引数 1 つのとき repr）
```

- `message` は CPython の `str(e)`（引数なし `''`・1 つはその `str`・2 つ以上は `args` の `repr`）。
  `print(e)` / `str(e)` はこれを、`repr(e)` は `ValueError('bad', 42)` の形を出します。
- キーワード引数は受けません（`TypeError: ValueError() takes no keyword arguments`）。
- 組み込みの例外は `__init__` を持たず、作るときの位置引数を `instantiate_evaled` が `args` に入れます
  （CPython の `BaseException.__new__` と同じ位置）。`__init__` を書いた Python のクラスでも、`super().__init__` を
  呼ばなくても `args` は入り、`super().__init__(..)` を呼べば入れ直します。
- 内部で起きた例外（辞書の引き損ないなど）の `args` は文言 1 つの組です（CPython の `KeyError` ならキーそのもの）。
- ⚠ `Error` を実装しただけの Arrow のクラス（`class MyErr(Error)`）は `args` を持たず、`print(e)` は
  `<MyErr object at ..>` のままです。

---

## カスタム例外クラス

```ar
class NetworkError(Error):
    let url: str
    let status_code: int

    fn __init__(mut self, let url: str, let status_code: int) -> None:
        self.message = f"HTTP {status_code} at {url}"
        self.url = url
        self.status_code = status_code
```

`Error` trait を継承することで `try/except` で捕捉できます。

```ar
try:
    fetch(url)
except NetworkError as e:
    print(e.status_code, e.url)
```

---

## 例外の伝播

1. `raise` で `ExecResult::Raise(raised)` が生成される
2. `exec` の呼び出し側が `ExecResult::Raise` を受け取ると:
   - `try/except` の中であれば捕捉を試みる
   - そうでなければ上位に `ExecResult::Raise` を返す (コールスタックを遡る)
3. トップレベルに到達したら `Interpreter::format_error_report` でトレースバックを表示

`eval()` 内では例外を `RAISE_SENTINEL` で伝播させ、  
`current_exception` に `RaisedError` を格納します。

---

## assert 文

```ar
assert condition
assert condition, "エラーメッセージ"
```

条件が `False` のとき `AssertionError` を送出します。  
メッセージ付きの場合はそのメッセージを `AssertionError.message` に設定します。

---

## スタックトレース

例外が捕捉されずにトップレベルに到達した場合の出力例:

```
Traceback (most recent call last):
  File "script.ar", line 15, col 5, in main
    result = compute(data)
  File "script.ar", line 8, col 3, in compute
    return process(x)
ValueError: invalid input
```

`str(e)` が空の例外は名前だけを出します（`raise ValueError()` は `ValueError`。CPython と同じ）。

各フレームには `file`・`line`・`col`・`fn_name`・`context` (前後5行) が記録されます。
