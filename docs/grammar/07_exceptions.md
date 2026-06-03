# 例外処理

---

## try / except / finally

```hv
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
    exc_type: Option<String>,   // 例外クラス名。None は bare except (全捕捉)
    name:     Option<String>,   // as で束縛する変数名
    body:     Vec<Stmt>,
}
```

**実行** (`exec_try`):
1. `try` ボディを実行
2. 例外が発生した場合 (`RAISE_SENTINEL` または `ExecResult::Raise`):
   - 各 `except` ハンドラを上から順に評価
   - `exc_matches(exception, handler.exc_type)` でマッチ判定
   - `bare except` は全例外にマッチ
   - マッチしたハンドラの `name` に例外インスタンスをバインドしてボディを実行
   - マッチするハンドラがなければ例外を再送出
3. `finally` ボディを例外有無に関わらず実行

### 例外マッチング (`exc_matches`)

```hv
try: ...
except Exception:   # Exception, ValueError, TypeError, ... のすべて
    ...
except ValueError:  # ValueError のみ
    ...
```

`exc_matches` はインスタンスのクラス名が `exc_type` またはその継承先 (`Error` トレイト含む)  
と一致するかを確認します。

---

## raise 文

```hv
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

すべての組み込み例外クラスは `Error` trait を実装しています。

| クラス名 | 説明 |
|---|---|
| `Exception` | 汎用例外の基底 |
| `ValueError` | 不正な値 |
| `TypeError` | 型の不一致 |
| `NameError` | 未定義変数の参照 |
| `AttributeError` | 存在しない属性へのアクセス |
| `IndexError` | インデックスが範囲外 |
| `KeyError` | 辞書に存在しないキー |
| `ZeroDivisionError` | ゼロ除算 |
| `RuntimeError` | 実行時エラー全般 |
| `StopIteration` | イテレータの終端 |
| `NotImplementedError` | 未実装メソッドの呼び出し |
| `OverflowError` | 数値オーバーフロー |
| `IOError` / `OSError` | I/O エラー |
| `AssertionError` | `assert` 失敗 |
| `ArithmeticError` | 算術演算エラー全般 |
| `AccessError` | `private`/`protected` アクセス違反 |

### 例外インスタンスのフィールド

| フィールド | 型 | 説明 |
|---|---|---|
| `message` | `str` | エラーメッセージ |
| `code_context` | `str` | 発生箇所のソースコンテキスト |
| `file` | `str` | ファイル名 |
| `line` | `int` | 行番号 |
| `col` | `int` | 列番号 |

`code_context`/`file`/`line`/`col` は `raise` 実行時にインタープリタが自動設定します。

---

## カスタム例外クラス

```hv
class NetworkError(Error):
    let url: str
    let status_code: int

    fn __init__(mut self, let url: str, let status_code: int) -> None:
        self.message = f"HTTP {status_code} at {url}"
        self.url = url
        self.status_code = status_code
```

`Error` trait を継承することで `try/except` で捕捉できます。

```hv
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

```hv
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
  File "script.hv", line 15, col 5, in main
    result = compute(data)
  File "script.hv", line 8, col 3, in compute
    return process(x)
ValueError: invalid input
```

各フレームには `file`・`line`・`col`・`fn_name`・`context` (前後5行) が記録されます。
