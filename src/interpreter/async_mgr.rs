// async_mgr.rs — AsyncStatus, AsyncManagerData, threading logic
//
// Each async task captures the caller's scope: mut variables are shared by Rc reference
// (allowing mutation propagation), let variables are deep-cloned.  The task runs in a
// dedicated OS thread via std::thread::spawn and sends its result back through an mpsc channel.  AsyncManagerData::try_schedule starts new
// threads whenever a slot is free (up to num_thread), and poll_completed
// harvests finished threads via try_recv.

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

use crate::ast::Stmt;

use super::{Interpreter, Value, Var};

// ---------------------------------------------------------------------------
// AsyncStatus
// ---------------------------------------------------------------------------

/// 非同期タスクの進行状態を表す列挙型。
#[derive(Debug, Clone, PartialEq)]
pub enum AsyncStatus {
    /// スレッドスロットが空くのを待機中。
    Waiting,
    /// スレッドが起動して実行中。
    Running,
    /// タスクが完了（成功またはエラー）。結果は `results` / `error_list` に格納済み。
    Done,
}

// ---------------------------------------------------------------------------
// Thread-boundary wrappers
//
// Value uses Rc<RefCell<...>> internally, which is not Send.  We deep-clone
// all captured values before crossing the thread boundary so that no Rc is
// shared between threads.  After that, wrapping in SendableEnv / SendableBody
// is safe: each thread owns its copy exclusively.
// ---------------------------------------------------------------------------

/// スレッド境界を越えてスコープ環境（変数名・値・可変フラグ）を送るためのラッパー。
struct SendableEnv(Vec<(String, Value, bool)>);
unsafe impl Send for SendableEnv {}

/// スレッド境界を越えてタスク本体の AST を送るためのラッパー。
struct SendableBody(Vec<Stmt>);
unsafe impl Send for SendableBody {}

// ---------------------------------------------------------------------------
// Thread task / result
// ---------------------------------------------------------------------------

/// ペンディング状態の非同期タスク（本体とキャプチャ済み環境を保持）。
struct AsyncTask {
    body: Vec<Stmt>,
    env: Vec<(String, Value, bool)>,
    /// submit 時点の親スレッドの `--vm` モード（#32）。worker はこれを引き継ぐ。
    vm_mode: crate::vm::VmMode,
}

/// スレッドから親スレッドへ返す実行結果（値またはエラー文字列）。
pub(super) struct ThreadResult {
    pub(super) value: Option<Value>,
    pub(super) error: Option<String>,
}
unsafe impl Send for ThreadResult {}

// ---------------------------------------------------------------------------
// Running slot (one per live thread)
// ---------------------------------------------------------------------------

/// 実行中スレッドのスロット（タスクインデックス・受信チャネル・JoinHandle を保持）。
struct RunningSlot {
    task_idx: usize,
    rx: mpsc::Receiver<ThreadResult>,
    _join: std::thread::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// AsyncManagerData
// ---------------------------------------------------------------------------

/// `AsyncManager` のランタイムデータ。スレッドプール・タスクキュー・進行状態を管理する。
pub struct AsyncManagerData {
    /// 同時実行するスレッドの最大数。
    pub num_thread: usize,
    /// `true` の場合、タスクが例外を送出したら即座に再送出する（デフォルト: 遅延収集）。
    pub raise_immediately: bool,

    // タスクごとの進行状態（サブミット順のインデックスで管理）
    /// 各タスクの進行状態（`Waiting` / `Running` / `Done`）のリスト。
    pub progress: Vec<AsyncStatus>,
    /// 各タスクの完了結果（`Done` になった時点で設定される）。実行中は `Value::None`。
    pub results: Vec<Value>,
    /// 各タスクのエラーメッセージ（タスクが例外を送出した場合に設定される）。
    pub error_list: Vec<Option<String>>,

    /// 実行待ちタスクのキュー。スロットが空くたびに先頭から順番に取り出して実行する。
    pending: VecDeque<(usize, AsyncTask)>,
    /// 現在実行中のスレッドスロットのリスト。`num_thread` 個まで同時実行できる。
    running: Vec<RunningSlot>,
    /// すべてのスレッドに中断を要求するフラグ（`AsyncManager` がドロップされたとき設定される）。
    abort: Arc<AtomicBool>,
}

impl std::fmt::Debug for AsyncManagerData {
    /// `<AsyncManager num_thread=N tasks=M>` 形式の文字列として表示する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<AsyncManager num_thread={} tasks={}>",
            self.num_thread,
            self.progress.len()
        )
    }
}

impl AsyncManagerData {
    /// 指定スレッド数と `raise_immediately` フラグで新しい `AsyncManagerData` を生成する。
    pub fn new(num_thread: usize, raise_immediately: bool) -> Self {
        Self {
            num_thread,
            raise_immediately,
            progress: Vec::new(),
            results: Vec::new(),
            error_list: Vec::new(),
            pending: VecDeque::new(),
            running: Vec::new(),
            abort: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 新しいタスクを登録し、スレッドスロットが空いていれば即座に実行を開始する。
    pub fn add_task(
        &mut self,
        body: Vec<Stmt>,
        env: Vec<(String, Value, bool)>,
        vm_mode: crate::vm::VmMode,
    ) {
        if env.iter().any(|(_, v, _)| matches!(v, Value::PyObject(_))) {
            eprintln!(
                "Warning: async task captures Python objects; \
                 Python's GIL will serialize execution across tasks (no true parallelism)"
            );
        }
        let task_idx = self.progress.len();
        self.progress.push(AsyncStatus::Waiting);
        self.results.push(Value::None);
        self.error_list.push(None);
        self.pending.push_back((task_idx, AsyncTask { body, env, vm_mode }));
        self.try_schedule();
    }

    /// 空きスレッドスロットにペンディングタスクを順次割り当てて実行開始する。
    fn try_schedule(&mut self) {
        while self.running.len() < self.num_thread {
            let Some((task_idx, task)) = self.pending.pop_front() else {
                break;
            };

            let abort = self.abort.clone();
            let (tx, rx) = mpsc::channel::<ThreadResult>();

            let body = SendableBody(task.body);
            let env = SendableEnv(task.env);
            let vm_mode = task.vm_mode;

            let handle = std::thread::spawn(move || {
                // Rebind whole structs so the closure captures SendableBody/SendableEnv
                // (both declared Send), not the inner Vec which is not Send (Rust 2021
                // precise-field capture would otherwise bypass our unsafe impl Send).
                let body = body;
                let env = env;
                let result = run_task(body.0, env.0, abort, vm_mode);
                let _ = tx.send(result);
            });

            self.progress[task_idx] = AsyncStatus::Running;
            self.running.push(RunningSlot {
                task_idx,
                rx,
                _join: handle,
            });
        }
    }

    /// 実行中スレッドの完了結果をノンブロッキングでポーリングする。
    /// `raise_immediately` フラグにより中断が発生した場合は `true` を返す。
    pub fn poll_completed(&mut self) -> bool {
        let mut i = 0;
        let mut abort_triggered = false;
        while i < self.running.len() {
            match self.running[i].rx.try_recv() {
                Ok(result) => {
                    let idx = self.running[i].task_idx;
                    self.progress[idx] = AsyncStatus::Done;
                    if let Some(e) = &result.error {
                        self.error_list[idx] = Some(e.clone());
                        if self.raise_immediately && !self.abort.load(Ordering::Relaxed) {
                            self.abort.store(true, Ordering::Relaxed);
                            abort_triggered = true;
                        }
                    }
                    self.results[idx] = result.value.unwrap_or(Value::None);
                    self.running.swap_remove(i);
                    // don't advance i — swap_remove put the last element here
                }
                Err(mpsc::TryRecvError::Empty) => {
                    i += 1;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // thread panicked without sending — record as error
                    let idx = self.running[i].task_idx;
                    self.progress[idx] = AsyncStatus::Done;
                    self.error_list[idx] = Some("AsyncError: thread panicked".to_string());
                    if self.raise_immediately && !self.abort.load(Ordering::Relaxed) {
                        self.abort.store(true, Ordering::Relaxed);
                        abort_triggered = true;
                    }
                    self.running.swap_remove(i);
                }
            }
        }
        abort_triggered
    }

    /// `try_schedule` の公開エイリアス（`classes.rs` から呼ばれる）。
    pub fn try_schedule_pub(&mut self) {
        self.try_schedule();
    }

    /// すべてのタスクが完了し、ペンディングおよび実行中スロットが空かどうかを返す。
    pub fn all_done(&self) -> bool {
        self.progress.iter().all(|s| *s == AsyncStatus::Done)
            && self.pending.is_empty()
            && self.running.is_empty()
    }

    /// `error_list` 内の最初のエラー文字列を返す（`raise_immediately` 伝播用）。
    pub fn first_error(&self) -> Option<String> {
        self.error_list.iter().find_map(|e| e.clone())
    }

    /// ペンディング中（未開始）のタスクをすべてキャンセルしてエラー状態にする。
    pub fn cancel_pending(&mut self) {
        for (task_idx, _) in self.pending.drain(..) {
            self.progress[task_idx] = AsyncStatus::Done;
            self.error_list[task_idx] =
                Some("AsyncError: task cancelled (raise_immediately)".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Thread body
// ---------------------------------------------------------------------------

/// スレッド内でタスク本体を実行し、結果またはエラーを `ThreadResult` として返す。
fn run_task(
    body: Vec<Stmt>,
    env: Vec<(String, Value, bool)>,
    abort: Arc<AtomicBool>,
    // 親スレッドの `--vm` を引き継ぐ（#32）。以前は `Interpreter::new()` が既定の `Auto` に
    // 戻していたので、**`--vm=off` も `--vm=force` も worker には届いていなかった**
    // （＝`force_gate` が async 本体を一切検査できていなかった）。
    vm_mode: crate::vm::VmMode,
) -> ThreadResult {
    if abort.load(Ordering::Relaxed) {
        return ThreadResult {
            value: None,
            error: Some("AsyncError: task aborted".to_string()),
        };
    }

    let mut interp = Interpreter::new();
    interp.set_vm_mode(vm_mode);

    // ── VM 経路（#32）──────────────────────────────────────────────────────
    // タスク本体は「値を返すブロック式」なので `compile_async_body` で Chunk 化する。
    // これが無いと**本体の文は全部ツリーウォーク**で、本体に直接書いたループが
    // 同じループを関数へ出した場合の 3.53x 遅さになっていた（実測・#32）。
    //
    // ⚠ worker は `Interpreter::new()` なので**型注釈を引き継げない**
    // （`AstAnnotations` は `Rc` ベースで `Send` でない）。空の注釈でコンパイルするので
    // 型特化 op は乗らないが、注釈は意味論の根拠ではない（#15e）ので結果は変わらない。
    let capture_names: Vec<String> = env.iter().map(|(n, _, _)| n.clone()).collect();
    let chunk = if vm_mode == crate::vm::VmMode::Off {
        None
    } else {
        crate::vm::compile_async_body(&body, interp.annotations.clone(), &capture_names)
    };
    if let Some(chunk) = chunk {
        let mut buf: Vec<Value> = vec![Value::None; chunk.n_locals];
        for (name, slot) in &chunk.captured_slots {
            if let Some((_, v, _)) = env.iter().find(|(n, _, _)| n == name) {
                if let Some(cell) = buf.get_mut(*slot as usize) {
                    *cell = v.clone();
                }
            }
        }
        // 捕捉値のうち slot に載らなかったもの（本体が同名を宣言している等）は
        // 従来どおりスコープにも置く。`Op::LoadName` の落ち先になる。
        interp.push_scope();
        for (name, value, is_mutable) in env {
            interp.declare_var(name, Var::new(value, is_mutable));
        }
        let result = crate::vm::run(&mut interp, &chunk, &mut buf, 0);
        return finish_task(&mut interp, result);
    }
    // #25 と同じ規約: `--vm=force` はフォールバック禁止。ゲートの穴を塞ぐ（#32）。
    if vm_mode == crate::vm::VmMode::Force {
        return ThreadResult {
            value: None,
            error: Some("VmForceError: cannot compile async task body to bytecode".to_string()),
        };
    }

    // ── ツリーウォーク経路（従来）────────────────────────────────────────
    interp.push_scope();
    for (name, value, is_mutable) in env {
        interp.declare_var(name, Var::new(value, is_mutable));
    }

    let result = interp.eval_block_expr(&body);
    finish_task(&mut interp, result)
}

/// タスクの実行結果を `ThreadResult` へ変換する（VM 経路・ツリーウォーク経路で共有・#32）。
/// `raise` はスレッド内の例外をこのスレッドのインタプリタから取り出して文字列化する。
fn finish_task(
    interp: &mut Interpreter,
    result: Result<Value, String>,
) -> ThreadResult {
    match result {
        Ok(value) => ThreadResult {
            value: Some(value),
            error: None,
        },
        Err(e) if e == super::RAISE_SENTINEL => {
            // raise inside the thread: extract and format the exception from
            // the thread-local interpreter so the main thread gets a plain string.
            let msg = interp
                .take_current_exception()
                .map(|r| super::Interpreter::format_error_report(&r))
                .unwrap_or_else(|| "UnhandledException: (no details available)".to_string());
            ThreadResult {
                value: None,
                error: Some(msg),
            }
        }
        Err(e) => ThreadResult {
            value: None,
            error: Some(e),
        },
    }
}

// ---------------------------------------------------------------------------
// Expose AsyncStatus as Value helper (used in ops.rs / eval.rs)
// ---------------------------------------------------------------------------

impl AsyncStatus {
    /// `"Async.Waiting"` / `"Async.Running"` / `"Async.Done"` の表示文字列を返す。
    pub fn display_str(&self) -> &'static str {
        match self {
            AsyncStatus::Waiting => "Async.Waiting",
            AsyncStatus::Running => "Async.Running",
            AsyncStatus::Done => "Async.Done",
        }
    }
}

// ---------------------------------------------------------------------------
// Collect captured environment from interpreter scopes (deep-cloned)
// ---------------------------------------------------------------------------

/// インタープリタのスコープスタックから非同期タスク用の環境スナップショットを取得する。
/// `mut` 変数は Rc クローン（変更が伝播）、`let` 変数はディープクローン（独立コピー）となる。
pub(super) fn capture_env(interp: &Interpreter) -> Vec<(String, Value, bool)> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut env: Vec<(String, Value, bool)> = Vec::new();
    // 可視スコープ = 現関数のローカル（frame_floor..、内側優先）+ グローバル（0）。
    // 呼び出し元のローカルは隔離されているため対象外。
    let floor = interp.frame_floor;
    let visible = interp.scopes[floor..]
        .iter()
        .rev()
        .chain(std::iter::once(&interp.scopes[0]));
    for scope in visible {
        for (name, var) in scope.iter() {
            if seen.insert(name.clone()) {
                let value = if var.is_mutable() {
                    var.get_value().clone()
                } else {
                    var.get_value().deep_clone()
                };
                env.push((name.clone(), value, var.is_mutable()));
            }
        }
    }
    env
}

// ---------------------------------------------------------------------------
// Value::deep_clone — helper referenced from interpreter.rs
// ---------------------------------------------------------------------------
// Implemented as an inherent method on Value in interpreter.rs (see below).
// This module just re-exports the capture helper.
