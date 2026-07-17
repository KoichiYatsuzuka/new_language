# 実装計画: C# から Arrow のイベントハンドラを発火させる仕組み

状態: **計画のみ（未実装）** / 作成日: 2026-07-17

## 背景と現状

`docs/grammar/11_events.md` の「外部イベント」節に記載の仕組みは、送り側だけが実装済みで受け側の登録経路が未配線:

| 部品 | 状態 |
|---|---|
| `ar_event_fire(handler_id, data_ptr, len)` C ABI (`src/interpreter/event_loop.rs:198`) | 実装済み・スレッドセーフ |
| グローバル外部キュー `GLOBAL_EXT_QUEUE` (`event_loop.rs:184`) | 実装済み (`Interpreter::new()` で設定) |
| `drain_external_events()` (`src/interpreter/exec/exceptions_async.rs:262`) | 実装済み (`EventLoop.run()` の各ティック冒頭で呼出) |
| `external_handler_registry: HashMap<u64, Rc<RefCell<SignalData>>>` (`src/interpreter.rs:319`) | **初期化(384行)と参照(exceptions_async.rs:268)のみ。挿入箇所がどこにもない** |
| C# 側から `ar_event_fire` を呼ぶ手段 | **なし** (シンボルは arrow.exe 内。DLL からは関数ポインタを受け取る必要がある) |

## 設計判断

1. **ID はシグナル単位とする。** ドキュメントは「ハンドラ登録時に発行された ID」と書いているが、`SignalData.next_id` は各シグナル内で 1 始まりのため跨いで衝突する。また `drain_external_events` は該当シグナルの**全ハンドラ**を呼ぶ設計なので、実態は「シグナル ID」。プロセス全体で単調増加する専用カウンタから発番する（ドキュメントの文言も合わせて修正）。
2. **API 形状は `sig.external_id` 読み取り専用プロパティ。** 初回アクセス時に発番+registry 登録し、以後は同じ値を返す。`EventLoop.register(sig)` 案もあるが、既存の `handler_count` と同じプロパティ形式が最小変更。
3. **関数ポインタ注入方式。** `ar_event_fire` は arrow.exe の実行ファイル内シンボルであり、Windows では exe のエクスポートテーブルに載る保証がないため、C# NativeAOT DLL 側に `arrow_bridge_set_event_fire(ptr)` エクスポートを用意し、ブリッジロード直後に Rust 側から `ar_event_fire as usize` を渡す（`import[rs]` ラッパーの `ar_init()`/ArCallbacks と同じパターン）。
4. **ペイロードは当面 UTF-8 文字列。** `drain_external_events` の既存実装 (`String::from_utf8_lossy` → `Value::Str`) をそのまま使う。MessagePack 復号は将来課題。
5. **cs-proc は対象外(将来課題)。** IPC サブプロセスは同期要求応答型パイプのため、非同期通知の側路(第2パイプ等)が別途必要。今回は cs-dll のみ。

## Phase 1 — Arrow 側の登録経路 (Rust のみ、単独で検証可能)

### 1-1. `src/interpreter/event_loop.rs`
`SignalData` に発番済み ID の記憶用フィールドを追加:
```rust
pub struct SignalData {
    pub handlers: Vec<HandlerEntry>,
    pub async_queue: VecDeque<Value>,
    pub next_id: u64,
    /// 外部発火用に発番されたプロセス全体で一意な ID (未発番なら None)
    pub external_id: Option<u64>,
}
```
`new()` に `external_id: None` を追加。

### 1-2. `src/interpreter.rs`
`Interpreter` にカウンタを追加 (`external_handler_registry` の隣、319行付近):
```rust
pub(self) next_external_signal_id: u64,   // new() で 1 に初期化
```

### 1-3. `src/interpreter/eval/attrs.rs` (188行付近の Signal アーム)
`handler_count` の隣に `external_id` を追加:
```rust
"external_id" => {
    let existing = sig_rc.borrow().external_id;
    if let Some(id) = existing { return Ok(Value::Int(id as i64)); }
    let id = self.next_external_signal_id;
    self.next_external_signal_id += 1;
    sig_rc.borrow_mut().external_id = Some(id);
    self.external_handler_registry.insert(id, sig_rc.clone());
    Ok(Value::Int(id as i64))
}
```
注意: この match アームの直前で `let sig = sig_rc.borrow();` している (186行) ため、
借用競合を避けるアーム構成に直すこと (`handler_count` アーム内で borrow するよう変更)。

### 1-4. 検証 (C# 不要)
Rust 統合テストを既存テスト構成 (`cargo test`) に追従して追加:
スクリプト「`external_id` 取得 → 別スレッドから `ar_event_fire(id, b"hello", 5)` →
`EventLoop.run(0.2)` → ハンドラが `"hello"` を受け取る」を実行して stdout を検証。
`GLOBAL_EXT_QUEUE` は `OnceLock` なので同一テストプロセス内の複数テストでキューが共有される点に注意
(テストは 1 本にまとめるか、ID を分けて干渉を防ぐ)。

### 1-5. 型チェッカ確認
`src/type_check/` に Signal のプロパティ検査は現存しない (grep で `handler_count`/`Signal` ともにヒットなし
= 属性アクセスは検査対象外)。`sig.external_id` が型エラーにならないことを確認するだけでよい。
将来 Signal 属性の検査を導入する場合は `external_id: int` を追加。

## Phase 2 — C# (cs-dll) 側の発火経路

### 2-1. Rust: `src/interpreter/cs_dll_runtime.rs`
`load_bridge()` (68行) 内、`BridgeLib::load` 成功直後に任意シンボルを解決して注入:
```rust
if let Some(p) = bridge.sym_ptr("arrow_bridge_set_event_fire") {
    let setter: unsafe extern "C" fn(usize) = unsafe { std::mem::transmute(p) };
    unsafe { setter(crate::interpreter::event_loop::ar_event_fire as usize) };
}
```
シンボルが無い旧 DLL では何もしない (後方互換)。

### 2-2. C#: `examples/interop/cs_interop_test/EventSource.cs` (新規)
`ArrowExports.cs` の既存パターン (`[UnmanagedCallersOnly(EntryPoint = ...)]`、
int→i64 / float→i64 ビットパターン) に従う:
```csharp
public static unsafe class EventBridge
{
    private static delegate* unmanaged<ulong, byte*, nuint, void> _fire;

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_set_event_fire")]
    public static void SetEventFire(nint fnPtr)
        => _fire = (delegate* unmanaged<ulong, byte*, nuint, void>)fnPtr;

    internal static void Fire(ulong signalId, string payload)
    {
        if (_fire == null) return;
        var bytes = System.Text.Encoding.UTF8.GetBytes(payload);
        fixed (byte* p = bytes) _fire(signalId, p, (nuint)bytes.Length);
    }
}

public static class EventSource
{
    // 背景スレッドから count 回、intervalMs 間隔で発火するデモ
    public static void StartTimer(long signalId, int count, int intervalMs)
    {
        new Thread(() => {
            for (int i = 0; i < count; i++) {
                Thread.Sleep(intervalMs);
                EventBridge.Fire((ulong)signalId, $"tick {i + 1}");
            }
        }) { IsBackground = true }.Start();
    }
}
```
`ArrowExports` に `EventSource_StartTimer(long, long, long)` エクスポートを追加。

### 2-3. ビルドとスタブ再生成
1. NativeAOT 再発行 (csproj の既存構成に従う): `dotnet publish -c Release -r win-x64`
   → 生成物を `examples/interop/cs_interop_test/ArrowBridge_native.dll` の配置規約に合わせて更新
2. スタブ再生成: `cargo run -- --compile-cs examples/interop/cs_interop_test/ArrowBridge.dll`
   → `ArrowBridge.ars` に `EventSource` クラスが載ることを確認
3. 繰り返し実行するため上記 2 手順は `.ps1` 化する (regulations 準拠)

### 2-4. サンプル: `examples/interop/event_cs_fire.ar` (新規)
```
import[cs-dll] cs_interop_test.ArrowBridge as bridge

let ticks = Signal[str]()
fn on_tick(mut msg: str) -> None:
    print("[ar] received:", msg)
ticks on on_tick

let id = ticks.external_id            # 発番 + registry 登録
bridge.EventSource.StartTimer(id, 3, 50)
EventLoop.run(1.0)                    # C# スレッドからの発火を処理
print("done")
```
期待出力: `tick 1` / `tick 2` / `tick 3` の 3 行 + `done`。

## Phase 3 — 仕上げ

- `docs/grammar/11_events.md` の「外部イベント」節を更新:
  `handler_id` → シグナル単位 ID である旨、`sig.external_id` の使い方、cs-dll 注入方式
- `./generate-codebase-map.ps1` 再実行 (ファイル追加のため)
- 将来課題として明記: cs-proc 対応 / MessagePack 復号 / `external_id` の解除 API
  (registry は Rc を保持し続けるため、大量登録時はリーク相当になる)

## 注意点・落とし穴

1. **`EventLoop.run()` (タイムアウトなし) は外部イベントを待たない。**
   `has_work()` は signal_queue / post_queue しか見ないため、C# スレッドがまだ発火していない
   段階で即終了する。C# 発火を受けるサンプル・テストでは必ず `run(timeout)` を使う。
2. **ハンドラ内の例外は `run()` の呼び出し元へ伝播**し、残りのイベントはキューに残る (現行仕様)。
3. **`attrs.rs` の借用競合**: 既存コードは Signal アームの先頭で `borrow()` を保持している。
   `external_id` アームでは `borrow_mut()` と registry 挿入 (`&mut self`) が必要なため、
   アームごとに借用を取るよう書き換えること。
4. **NativeAOT リビルドが最重量ステップ** (dotnet SDK + C++ ツールチェーン必須、数分)。
   Phase 1 を先に完了・検証してから着手する。
5. テストでの `GLOBAL_EXT_QUEUE` 共有 (1-4 参照)。
