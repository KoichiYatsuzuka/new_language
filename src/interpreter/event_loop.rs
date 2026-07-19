// event_loop.rs — Signal[T], EventLoop, and cross-thread event queue
//
// Arrow ネイティブのイベントシステムを実装する。
//
// 担当:
//   SignalData       — ハンドラリストと emit_async キューを持つシグナルのランタイム状態
//   HandlerEntry     — 個々のハンドラ（関数値・is_once・is_async フラグ・ID）
//   EventLoopData    — emit_async で積まれたイベントと post() コールバックのキュー
//   ExternalEvent    — C#/Go ブリッジから ar_event_fire() で積まれた外部イベント
//   ExternalEventQueue — Arc<Mutex<VecDeque<ExternalEvent>>>: スレッドセーフキュー
//   global_ext_queue   — プロセス全体で共有するグローバルキューの取得（遅延生成）

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::Value;

// ---------------------------------------------------------------------------
// HandlerEntry
// ---------------------------------------------------------------------------

/// Signal に登録された 1 つのハンドラエントリ。
#[derive(Debug, Clone)]
pub struct HandlerEntry {
    /// ハンドラの一意 ID（subscribe 時に発行）。
    pub id: u64,
    /// ハンドラ関数値（`Value::Function` / `Value::OverloadedFn` など）。
    pub func: Value,
    /// `true` の場合、呼び出し後に自動解除される一回限りのハンドラ。
    pub is_once: bool,
    /// `true` の場合、EventLoop 内で別スレッドで実行される非同期ハンドラ。
    pub is_async: bool,
}

// ---------------------------------------------------------------------------
// SignalData
// ---------------------------------------------------------------------------

/// `Signal[T]` のランタイム状態。
///
/// - `handlers`    : 登録済みハンドラのリスト（同期・非同期・一回限りを含む）
/// - `next_id`     : 次のハンドラ ID（単調増加）
/// - `external_id` : 外部発火用に発番されたプロセス全体で一意な ID（未発番なら None）
#[derive(Debug)]
pub struct SignalData {
    pub handlers: Vec<HandlerEntry>,
    pub next_id: u64,
    /// `sig.external_id` 初回アクセス時に発番され、external_handler_registry に登録される。
    pub external_id: Option<u64>,
}

impl SignalData {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            next_id: 1,
            external_id: None,
        }
    }

    /// ハンドラを登録して割り当てた ID を返す。
    pub fn subscribe(&mut self, func: Value, is_once: bool, is_async: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.handlers.push(HandlerEntry {
            id,
            func,
            is_once,
            is_async,
        });
        id
    }

    /// `func` と Rc ポインタが一致するハンドラをすべて削除する。
    pub fn unsubscribe_by_value(&mut self, func: &Value) {
        match func {
            Value::Function(target) => {
                self.handlers.retain(|h| {
                    if let Value::Function(hf) = &h.func {
                        !Rc::ptr_eq(hf, target)
                    } else {
                        true
                    }
                });
            }
            Value::OverloadedFn(targets) => {
                self.handlers.retain(|h| {
                    if let Value::Function(hf) = &h.func {
                        !targets.iter().any(|t| Rc::ptr_eq(hf, t))
                    } else {
                        true
                    }
                });
            }
            _ => {}
        }
    }

    /// `emit(val)` の全ハンドラを `(func, is_async)` ペアとして返す。
    /// `is_once=true` のエントリはリストから除去される。
    pub fn collect_handlers_for_emit(&mut self) -> Vec<(Value, bool)> {
        let mut result = Vec::new();
        let mut to_remove = Vec::new();
        for h in &self.handlers {
            result.push((h.func.clone(), h.is_async));
            if h.is_once {
                to_remove.push(h.id);
            }
        }
        self.handlers.retain(|h| !to_remove.contains(&h.id));
        result
    }
}

// ---------------------------------------------------------------------------
// EventLoopData
// ---------------------------------------------------------------------------

/// EventLoop のランタイム状態。
///
/// - `signal_queue` : `emit_async(val)` で積まれた `(Signal の Rc, 値)` ペア
/// - `post_queue`   : `EventLoop.post(fn)` で積まれたコールバック関数値
#[derive(Debug)]
pub struct EventLoopData {
    /// `emit_async()` で積まれた `(signal_rc, value)` ペア。
    pub signal_queue: VecDeque<(Rc<RefCell<SignalData>>, Value)>,
    /// `EventLoop.post(fn)` で積まれたコールバック。メインスレッドで実行される。
    pub post_queue: VecDeque<Value>,
}

impl EventLoopData {
    pub fn new() -> Self {
        Self {
            signal_queue: VecDeque::new(),
            post_queue: VecDeque::new(),
        }
    }

    /// キューに処理すべき項目があれば `true`。
    pub fn has_work(&self) -> bool {
        !self.signal_queue.is_empty() || !self.post_queue.is_empty()
    }
}

// ---------------------------------------------------------------------------
// External event queue  (for C# / Go bridge)
// ---------------------------------------------------------------------------

/// C#/Go ブリッジスレッドから Arrow メインスレッドへ送るイベントデータ。
///
/// - `handler_id` : `sig.external_id` で発番されたシグナル単位の ID
///   （external_handler_registry のキー。該当シグナルの全ハンドラが呼ばれる）
/// - `data`       : イベント引数バイト列（現状は UTF-8 文字列として復号される）
#[derive(Debug, Clone)]
pub struct ExternalEvent {
    pub handler_id: u64,
    pub data: Vec<u8>,
}

/// スレッドセーフな外部イベントキュー。C#/Go ブリッジが `ar_event_fire()` で書き込み、
/// Arrow メインスレッドが `EventLoop.run()` のティック内で読み出す。
pub type ExternalEventQueue = Arc<Mutex<VecDeque<ExternalEvent>>>;

/// ExternalEventQueue を新規生成する（global_ext_queue の遅延初期化用）。
fn new_external_queue() -> ExternalEventQueue {
    Arc::new(Mutex::new(VecDeque::new()))
}

// ---------------------------------------------------------------------------
// Global external queue singleton  (for ar_event_fire C ABI)
// ---------------------------------------------------------------------------

/// `ar_event_fire` が書き込む静的キュー。初回アクセス時に生成され、
/// 以降は不変（同じ Arc を使い続ける）。
static GLOBAL_EXT_QUEUE: std::sync::OnceLock<ExternalEventQueue> = std::sync::OnceLock::new();

/// グローバル外部キューを取得する（未生成なら生成する）。
/// `Interpreter::new()` はこれを自分の `external_event_queue` として保持するため、
/// プロセス内の全インタープリタが同一キューを共有する
/// （2 個目以降のインタープリタでも `ar_event_fire` の書き込みが届く）。
pub fn global_ext_queue() -> ExternalEventQueue {
    GLOBAL_EXT_QUEUE.get_or_init(new_external_queue).clone()
}

/// `ar_event_fire(handler_id, data_ptr, len)` C ABI 実装。
/// 外部スレッド（C#/Go ブリッジ）から呼ばれ、グローバルキューにイベントを積む。
///
/// # Safety
/// `data_ptr` は `len` バイト以上の有効なメモリを指していなければならない。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ar_event_fire(handler_id: u64, data_ptr: *const u8, len: usize) {
    let data = if data_ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data_ptr, len).to_vec() }
    };
    if let Some(q) = GLOBAL_EXT_QUEUE.get() {
        if let Ok(mut guard) = q.lock() {
            guard.push_back(ExternalEvent { handler_id, data });
        }
    }
}

/// C# チャネルへの送信（逆方向: Arrow → Go/C# チャネル）。
/// 現時点では stub のみ（実装は C# ブリッジ側に依存）。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ar_channel_send(
    _name_ptr: *const u8,
    _name_len: usize,
    _data_ptr: *const u8,
    _data_len: usize,
) {
    // TODO: look up the channel write callback registered by the Go bridge and call it
}
