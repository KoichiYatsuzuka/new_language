// async_mgr.rs — AsyncStatus, AsyncManagerData, threading logic
//
// Each async task captures the caller's scope as a deep-cloned environment,
// runs in a dedicated OS thread via std::thread::spawn, and sends its result
// back through an mpsc channel.  AsyncManagerData::try_schedule starts new
// threads whenever a slot is free (up to num_thread), and poll_completed
// harvests finished threads via try_recv.

use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use crate::ast::Stmt;

use super::{Interpreter, Value, Var};

// ---------------------------------------------------------------------------
// AsyncStatus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum AsyncStatus {
    Waiting,
    Running,
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

struct SendableEnv(Vec<(String, Value, bool)>);
unsafe impl Send for SendableEnv {}

struct SendableBody(Vec<Stmt>);
unsafe impl Send for SendableBody {}

// ---------------------------------------------------------------------------
// Thread task / result
// ---------------------------------------------------------------------------

struct AsyncTask {
    body: Vec<Stmt>,
    env: Vec<(String, Value, bool)>,
}

pub(super) struct ThreadResult {
    pub(super) value: Option<Value>,
    pub(super) error: Option<String>,
}
unsafe impl Send for ThreadResult {}

// ---------------------------------------------------------------------------
// Running slot (one per live thread)
// ---------------------------------------------------------------------------

struct RunningSlot {
    task_idx: usize,
    rx: mpsc::Receiver<ThreadResult>,
    _join: std::thread::JoinHandle<()>,
}

// ---------------------------------------------------------------------------
// AsyncManagerData
// ---------------------------------------------------------------------------

pub struct AsyncManagerData {
    pub num_thread: usize,
    pub raise_immediately: bool,

    // Per-task state (indexed by submission order)
    pub progress: Vec<AsyncStatus>,
    pub results: Vec<Value>,
    pub error_list: Vec<Option<String>>,

    pending: VecDeque<(usize, AsyncTask)>,
    running: Vec<RunningSlot>,
    abort: Arc<AtomicBool>,
}

impl std::fmt::Debug for AsyncManagerData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<AsyncManager num_thread={} tasks={}>", self.num_thread, self.progress.len())
    }
}

impl AsyncManagerData {
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

    /// Submit a new task and immediately start it if a thread slot is free.
    pub fn add_task(&mut self, body: Vec<Stmt>, env: Vec<(String, Value, bool)>) {
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
        self.pending.push_back((task_idx, AsyncTask { body, env }));
        self.try_schedule();
    }

    /// Start pending tasks in free thread slots.
    fn try_schedule(&mut self) {
        while self.running.len() < self.num_thread {
            let Some((task_idx, task)) = self.pending.pop_front() else { break };

            let abort = self.abort.clone();
            let (tx, rx) = mpsc::channel::<ThreadResult>();

            let body = SendableBody(task.body);
            let env = SendableEnv(task.env);

            let handle = std::thread::spawn(move || {
                // Rebind whole structs so the closure captures SendableBody/SendableEnv
                // (both declared Send), not the inner Vec which is not Send (Rust 2021
                // precise-field capture would otherwise bypass our unsafe impl Send).
                let body = body;
                let env = env;
                let result = run_task(body.0, env.0, abort);
                let _ = tx.send(result);
            });

            self.progress[task_idx] = AsyncStatus::Running;
            self.running.push(RunningSlot { task_idx, rx, _join: handle });
        }
    }

    /// Poll all running threads for completed results (non-blocking).
    /// Returns true if the abort flag was set due to raise_immediately.
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
                Err(mpsc::TryRecvError::Empty) => { i += 1; }
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

    /// Public alias for try_schedule (needed from classes.rs).
    pub fn try_schedule_pub(&mut self) {
        self.try_schedule();
    }

    pub fn all_done(&self) -> bool {
        self.progress.iter().all(|s| *s == AsyncStatus::Done)
            && self.pending.is_empty()
            && self.running.is_empty()
    }

    /// First error in error_list (if any), for raise_immediately propagation.
    pub fn first_error(&self) -> Option<String> {
        self.error_list.iter().find_map(|e| e.clone())
    }

    /// Cancel all pending (not-yet-started) tasks.
    pub fn cancel_pending(&mut self) {
        for (task_idx, _) in self.pending.drain(..) {
            self.progress[task_idx] = AsyncStatus::Done;
            self.error_list[task_idx] = Some("AsyncError: task cancelled (raise_immediately)".to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Thread body
// ---------------------------------------------------------------------------

fn run_task(body: Vec<Stmt>, env: Vec<(String, Value, bool)>, abort: Arc<AtomicBool>) -> ThreadResult {
    if abort.load(Ordering::Relaxed) {
        return ThreadResult {
            value: None,
            error: Some("AsyncError: task aborted".to_string()),
        };
    }

    let mut interp = Interpreter::new();
    interp.push_scope();
    for (name, value, is_mutable) in env {
        interp.declare_var(name, Var::new(value, is_mutable));
    }

    let result = interp.eval_block_expr(&body);
    match result {
        Ok(value) => ThreadResult { value: Some(value), error: None },
        Err(e) if e == super::RAISE_SENTINEL => {
            // raise inside the thread: extract and format the exception from
            // the thread-local interpreter so the main thread gets a plain string.
            let msg = interp.take_current_exception()
                .map(|r| super::Interpreter::format_error_report(&r))
                .unwrap_or_else(|| "UnhandledException: (no details available)".to_string());
            ThreadResult { value: None, error: Some(msg) }
        }
        Err(e) => ThreadResult { value: None, error: Some(e) },
    }
}

// ---------------------------------------------------------------------------
// Expose AsyncStatus as Value helper (used in ops.rs / eval.rs)
// ---------------------------------------------------------------------------

impl AsyncStatus {
    pub fn display_str(&self) -> &'static str {
        match self {
            AsyncStatus::Waiting => "Async.Waiting",
            AsyncStatus::Running => "Async.Running",
            AsyncStatus::Done    => "Async.Done",
        }
    }
}

// ---------------------------------------------------------------------------
// Collect captured environment from interpreter scopes (deep-cloned)
// ---------------------------------------------------------------------------

/// Snapshot the visible scope variables, deep-cloning each value.
/// The outermost (global) scope is included so built-in lookups work.
pub(super) fn capture_env(interp: &Interpreter) -> Vec<(String, Value, bool)> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut env: Vec<(String, Value, bool)> = Vec::new();
    for scope in interp.scopes.iter().rev() {
        for (name, var) in scope {
            if seen.insert(name.clone()) {
                env.push((name.clone(), var.get_value().deep_clone(), var.is_mutable()));
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
