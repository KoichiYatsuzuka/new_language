# イベントシステム (Signal / on / once / off / EventLoop)

Arrow ネイティブのイベントハンドラ機構。`Signal[T]` に関数を購読 (`on` / `once`) し、
`emit` / `emit_async` で発火します。遅延イベントは `EventLoop` が処理します。

実装: `src/interpreter/event_loop.rs` (ランタイム状態)、
`src/parser/stmts/assignment.rs` (パース)、
`src/interpreter/exec/exceptions_async.rs` (購読/解除の実行)、
`src/interpreter/classes/object_methods.rs` (`emit` / `EventLoop` メソッド)。

動作確認例: [examples/async/event_handler.ar](../../examples/async/event_handler.ar)

---

## キーワード

`on` / `off` / `once` は予約キーワードです (`src/lexer/keyword.rs`)。
識別子 (変数名・関数名) には使えません。

---

## Signal[T] の生成

```hv
let counter = Signal[int]()    # 型引数付き
let anything = Signal()        # 型引数なしも可
```

`Value::Signal(Rc<RefCell<SignalData>>)` を生成します。
型引数 `T` はランタイムでは無視されます (型注釈としてのみ使用)。

`SignalData` はハンドラリスト (`Vec<HandlerEntry>`)・`emit_async` 用キュー・
単調増加のハンドラ ID カウンタを持ちます。

---

## 購読 (on / once)

```hv
let counter = Signal[int]()

fn on_count(mut n: int) -> None:
    print("got:", n)

counter on on_count        # 継続購読
counter once on_count      # 1 回だけ購読 (呼び出し後に自動解除)
counter on async on_count  # 非同期購読 (EventLoop 経由で遅延実行)
```

**構文**:

```
source on   [async] handler
source once [async] handler
```

- `source` : `Signal` を返す任意の式
- `handler` : ハンドラ関数値の式 (関数名・関数を保持する変数など)

`Stmt::EventSubscribe { source, handler, is_once, is_async, span }`

**パース**: 文頭の式をパースした後、`on` / `once` / `off` が続く場合に
イベント文として扱われます (`try_parse_event_stmt`)。

**実行** (`exec_event_subscribe`):
1. `source` と `handler` を評価
2. `source` が `Signal` でなければ `TypeError`
3. `SignalData.subscribe(handler, is_once, is_async)` でハンドラ ID を発行して登録

同じ関数を複数回 `on` すると、その回数だけエントリが登録され、emit ごとに複数回呼ばれます。

---

## 解除 (off)

```hv
counter off on_count
```

**構文**: `source off handler`

`Stmt::EventUnsubscribe { source, handler, span }`

**実行** (`exec_event_unsubscribe`):
1. `source` と `handler` を評価
2. `source` が `Signal` でなければ `TypeError`
3. ハンドラ関数値と **Rc ポインタが一致する**エントリをすべて削除
   (`OverloadedFn` は構成関数のいずれかと一致すれば削除)

一致するエントリがない場合は何もしません (エラーになりません)。

---

## 発火 (emit / emit_async)

```hv
counter.emit(1)         # 同期発火: 同期ハンドラを即時呼び出し
counter.emit_async(2)   # 遅延発火: EventLoop のキューに積むだけ
```

どちらも引数は 0 個または 1 個 (0 個の場合はハンドラに `None` が渡されます)。
2 個以上は `TypeError`。

**`emit(val)`**:
1. 登録済みハンドラを全取得 (`is_once` のエントリはこの時点でリストから除去)
2. 同期ハンドラ → その場で `handler(val)` を呼び出し
3. 非同期ハンドラ (`on async`) → `(Signal, val)` ペアを EventLoop の
   `signal_queue` に積む (実際の呼び出しは `EventLoop.run()` が行う)

**`emit_async(val)`**: ハンドラを一切呼ばず、`(Signal, val)` ペアを
`signal_queue` に積むだけ。

---

## プロパティ

| プロパティ | 型 | 内容 |
|---|---|---|
| `sig.handler_count` | `int` | 現在登録されているハンドラ数 |

---

## EventLoop

グローバルに 1 つだけ存在する組み込みオブジェクトです (`Interpreter::new()` で登録)。

```hv
events.emit_async(10)
EventLoop.run(1.0)          # 1.0 秒間キューを処理し続ける

EventLoop.post(callback)    # 引数なしコールバックをキューに積む
EventLoop.run()             # キューが空になったら即終了
```

| メソッド | 動作 |
|---|---|
| `EventLoop.run()` | キューが空になるまで処理して終了 |
| `EventLoop.run(timeout)` | `timeout` 秒 (float/int、`timeout=` キーワード可) 経過までキューを処理し続ける |
| `EventLoop.post(fn)` | 引数なしコールバックを `post_queue` に積む (メインスレッドで実行) |

**`run()` の 1 ティック**:
1. 外部イベントキュー (C#/Go ブリッジ) を全件処理 (`drain_external_events`)
2. `signal_queue` から 1 件取り出し、その Signal の全ハンドラを呼び出す
3. `post_queue` から 1 件取り出してコールバックを呼び出す
4. キューが空ならタイムアウト判定 (タイムアウトなしなら即終了)、1ms スリープして繰り返し

**実装上の注意**:
- 非同期ハンドラ (`on async`) も別スレッドではなく、`EventLoop.run()` 内で
  **メインスレッド上で同期的に**実行されます。
- `signal_queue` の処理では、emit 時点ではなく **`run()` 時点の**ハンドラリストが
  使われます (`is_once` エントリは処理時点で除去)。

---

## 外部イベント (C#/Go ブリッジ)

外部スレッドからは C ABI 関数 `ar_event_fire(handler_id, data_ptr, len)` で
スレッドセーフなグローバルキューにイベントを積めます。

- `handler_id` : Arrow 側でハンドラ登録時に発行された ID
- データは MessagePack バイト列を想定 (現時点では `str` としてハンドラに渡されます)

積まれたイベントは `EventLoop.run()` のティック冒頭で
`drain_external_events()` が取り出し、対応する Signal の全ハンドラを呼び出します。

---

## 静的型検査

現時点では `EventSubscribe` / `EventUnsubscribe` 文の型チェックは**スキップ**されます
(`src/type_check/stmt/check.rs`)。ハンドラの引数型と `Signal[T]` の `T` の整合性は
検証されず、不一致は実行時エラーになります。

---

## 制限事項 (現状の実装)

- `on` / `once` / `off` の対象 (`source`) として有効なのは **`Signal` のみ**。
  AST のコメントに挙がっている `EventSource[T]` / `GoChannel[T]` は将来予定で未実装。
- `Signal` のメソッドは `emit` / `emit_async` の 2 つのみ (それ以外は `AttributeError`)。
- ハンドラ内で発生した例外はそのまま呼び出し元 (emit / `EventLoop.run()`) に伝播します。
