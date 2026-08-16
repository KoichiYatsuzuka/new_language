// debugger.rs — break_point REPL and single-step machinery
//
// When the interpreter encounters `Stmt::BreakPoint`, it calls
// `Interpreter::exec_breakpoint()`, which:
//   1. Prints source context (current line ± 2).
//   2. Enters a REPL loop reading from stdin.
//   3. Handles commands: empty (step over), e (step into),
//      o (step out), q (resume).
//
// Step modes are communicated to `exec()` via thread-locals so that
// checking is cheap and requires no borrow of the interpreter.

use std::cell::RefCell;
use std::io::{self, Write};

use crate::ast::{Expr, Stmt};
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::token::Span;
use super::Var;

use super::{ExecResult, Interpreter, Value};

// ---------------------------------------------------------------------------
// Thread-local debugger state
// ---------------------------------------------------------------------------

/// デバッガのステップ実行モードを表す列挙型。
/// スレッドローカルの `DBG_MODE` に格納され、`exec()` の各呼び出しで参照される。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum DbgMode {
    /// Normal execution — no stepping.
    Inactive,
    /// Pause before the next `exec()` call at the same call-stack depth
    /// as when the breakpoint fired.
    StepOver,
    /// Pause at the first `exec()` call that is *deeper* than the entry depth
    /// (i.e., we just entered a function).  Falls back to StepOver if no
    /// function call is made.
    StepInto,
    /// Pause when the call-stack depth drops back to `target` (i.e., we have
    /// returned from the function we were debugging).
    StepOut { target: usize },
}

thread_local! {
    pub(super) static DBG_MODE: RefCell<DbgMode> = const { RefCell::new(DbgMode::Inactive) };
    /// The call-stack depth at the moment the debugger was last entered.
    pub(super) static DBG_ENTRY_DEPTH: RefCell<usize> = const { RefCell::new(0) };
    /// Number of same-depth exec() calls to skip before pausing in StepInto mode.
    /// Set to 1 when StepInto is activated so the statement-with-the-call runs first.
    pub(super) static DBG_STEP_INTO_SKIP: RefCell<usize> = const { RefCell::new(0) };
    /// Whether we are currently inside the REPL (to avoid re-entering).
    static IN_REPL: RefCell<bool> = const { RefCell::new(false) };
}

/// デバッガがアクティブ（ステップ実行中）かを返す。
///
/// VM のディスパッチループ（[`crate::vm::run`]）が**入口で 1 回だけ**これを見て、
/// 真ならステップ判定つきループ（`run_stepping`）へ入る（#1）。
/// 通常経路には停止判定を一切足さないための入口分岐。
pub(crate) fn dbg_active() -> bool {
    DBG_MODE.with(|m| *m.borrow() != DbgMode::Inactive)
}

// ---------------------------------------------------------------------------
// Source context helpers
// ---------------------------------------------------------------------------

const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

/// ソースマップから `target_line`（1 始まり）の前後 2 行を表示し、対象行を `>>` でハイライトする。
fn print_context(interp: &Interpreter, file: &str, target_line: usize) {
    if target_line == 0 {
        println!("  (source location unknown)");
        return;
    }
    let map = &interp.source_map;
    let lines = match map.get(file) {
        Some(l) => l,
        None => {
            println!("  (no source available for '{file}')");
            return;
        }
    };

    let total = lines.len();
    let first = target_line.saturating_sub(2);
    let last = (target_line + 2).min(total);

    for lineno in first..=last {
        if lineno == 0 || lineno > total {
            continue;
        }
        let text = &lines[lineno - 1];
        if lineno == target_line {
            println!("{YELLOW}{lineno:>4}{RESET} {CYAN}>>{RESET} {text}");
        } else {
            println!("{YELLOW}{lineno:>4}{RESET}    {text}");
        }
    }
}

/// 文の表示用スパンを返す（VM の行テーブル構築用・#1）。
///
/// `best_span_for` の**フォールバックを除いた部分**と同じ判定なので、
/// ここが `None` を返す文は VM 側では `STMT_NO_SPAN` として記録され、
/// 停止時に `best_span_for` の `dbg_last_span` フォールバックへ委ねられる。
/// ＝ ツリーウォークと同じ表示になる。
pub(crate) fn stmt_span_of(stmt: &Stmt) -> Option<Span> {
    stmt_location(stmt).map(|(file, line)| Span {
        file: file.into(),
        line,
        col: 1,
    })
}

/// 文から代表的な（ファイル名, 行番号）を取り出す。スパンを持たない文は `None` を返す。
pub(super) fn stmt_location(stmt: &Stmt) -> Option<(String, usize)> {
    fn from_span(s: &Span) -> Option<(String, usize)> {
        if s.line == 0 {
            None
        } else {
            Some((s.file.to_string(), s.line))
        }
    }
    fn from_expr(e: &Expr) -> Option<(String, usize)> {
        match e {
            Expr::BinOp { span, .. } => from_span(span),
            Expr::Cast { span, .. } => from_span(span),
            Expr::IsType { span, .. } => from_span(span),
            Expr::Call { span, .. } => from_span(span),
            _ => None,
        }
    }
    match stmt {
        Stmt::Assign { span, .. }
        | Stmt::CompoundAssign { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Raise { span, .. }
        | Stmt::BreakPoint { span }
        | Stmt::Freeze(_, span)
        | Stmt::Static(_, _, span) => from_span(span),
        Stmt::LetTuple { span, .. } => from_span(span),
        Stmt::Expr(e)
        | Stmt::Let(_, _, e)
        | Stmt::Mut(_, _, e)
        | Stmt::Const(_, _, e)
        | Stmt::DebugLet(_, e) => from_expr(e),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// REPL command result
// ---------------------------------------------------------------------------

/// デバッガ REPL のユーザーコマンドを表す列挙型。
enum ReplCmd {
    StepOver,
    StepInto,
    StepOut,
    Resume,
}

// ---------------------------------------------------------------------------
// Debugger REPL
// ---------------------------------------------------------------------------

impl Interpreter {
    /// `Stmt::BreakPoint` または ステップ実行時に `exec()` から呼ばれるデバッガ REPL エントリポイント。
    pub(super) fn exec_breakpoint(&mut self, span: &Span) -> Result<ExecResult, String> {
        // Prevent re-entrancy (e.g. if the REPL itself triggers exec_breakpoint)
        let already_in = IN_REPL.with(|r| *r.borrow());
        if already_in {
            return Ok(ExecResult::Normal);
        }

        IN_REPL.with(|r| *r.borrow_mut() = true);

        // Record this span so statements without spans can fall back to it.
        self.record_dbg_span(span);

        // -- print header --
        let file = span.file.to_string();
        let line = span.line;
        println!("\n[break_point] Paused at {GREEN}{file}{RESET}:{line}");
        print_context(self, &file, line);
        println!("  Commands: <Enter> step | e step-into | o step-out | q resume");
        println!("  Debug vars: let dbg::x = expr  |  access: dbg::x");

        // Record entry depth for step commands
        let entry_depth = self.call_stack.len();
        DBG_ENTRY_DEPTH.with(|d| *d.borrow_mut() = entry_depth);

        let cmd = self.repl_loop();

        // Clear dbg vars when the debugger exits (resume or step)
        // For step commands we keep them until q is pressed.
        // We always clear on 'q' (resume).
        match &cmd {
            ReplCmd::Resume => {
                self.dbg_vars.clear();
                DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::Inactive);
            }
            ReplCmd::StepOver => {
                DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::StepOver);
            }
            ReplCmd::StepInto => {
                DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::StepInto);
                // Skip the immediate next same-depth exec() call so the
                // statement containing the function call gets to execute first.
                DBG_STEP_INTO_SKIP.with(|s| *s.borrow_mut() = 1);
            }
            ReplCmd::StepOut => {
                if entry_depth == 0 {
                    println!("  [dbg] Not inside a function — cannot step out.");
                    // Stay in the REPL
                    IN_REPL.with(|r| *r.borrow_mut() = false);
                    return self.exec_breakpoint(span);
                }
                DBG_MODE.with(|m| {
                    *m.borrow_mut() = DbgMode::StepOut {
                        target: entry_depth - 1,
                    }
                });
            }
        }

        IN_REPL.with(|r| *r.borrow_mut() = false);
        Ok(ExecResult::Normal)
    }

    /// デバッガ REPL の内部ループ。コマンドが入力されるまでラインを読み続ける。
    fn repl_loop(&mut self) -> ReplCmd {
        loop {
            print!("(dbg) ");
            let _ = io::stdout().flush();

            let mut raw = String::new();
            if io::stdin().read_line(&mut raw).is_err() {
                // EOF / broken pipe — resume
                return ReplCmd::Resume;
            }
            let line = raw
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();

            match line.trim() {
                "" => return ReplCmd::StepOver,
                "e" => return ReplCmd::StepInto,
                "o" => return ReplCmd::StepOut,
                "q" => return ReplCmd::Resume,
                code => {
                    if let Err(e) = self.exec_debug_input(code) {
                        eprintln!("  [dbg] Error: {e}");
                    }
                }
            }
        }
    }

    /// デバッガ入力の 1 行をパースして実行する。非デバッガ変数への代入は拒否する。
    fn exec_debug_input(&mut self, code: &str) -> Result<(), String> {
        let tokens = Lexer::new(code, "<debugger>").tokenize();
        let mut parser = Parser::new(tokens, None);
        let stmts = parser.parse_program().map_err(|e| e.to_string())?;
        let stmt = stmts
            .into_iter()
            .next()
            .ok_or_else(|| "empty input".to_string())?;

        // Reject mutations of non-debugger variables
        match &stmt {
            Stmt::Assign { name, .. } => {
                return Err(format!(
                    "assignment to '{name}' is not allowed in the debugger \
                     (use 'let dbg::name = expr' for temporary variables)"
                ));
            }
            Stmt::Mut(name, _, _) => {
                return Err(format!(
                    "mutation '{name}' is not allowed in the debugger \
                     (use 'let dbg::name = expr' for temporary variables)"
                ));
            }
            Stmt::CompoundAssign { name, .. } => {
                return Err(format!(
                    "compound assignment to '{name}' is not allowed in the debugger"
                ));
            }
            Stmt::AttrAssign { .. } | Stmt::AttrCompoundAssign { .. } => {
                return Err("attribute assignment is not allowed in the debugger".to_string());
            }
            _ => {}
        }

        // バイトコード経路（V-E）: 停止スコープの視点でコンパイル・実行する。
        // 式は値を返して表示、`let dbg::x` は宣言のみ。コンパイル不能な構文（メソッド呼び出し・
        // 添字・制御フロー等）はツリーウォークへフォールバックする。
        let value: Value = if let Some(chunk) = crate::vm::compile_debug(&stmt) {
            self.run_debug_chunk(&chunk)?
        } else {
            match &stmt {
                // 式文はフォールバックでも値を取り出して表示する（バイトコード経路と一致）。
                Stmt::Expr(e) => self.eval(e)?,
                _ => {
                    self.exec(&stmt)?;
                    Value::None
                }
            }
        };
        if !matches!(value, Value::None) {
            println!("{}", self.display(&value));
        }
        Ok(())
    }

    /// デバッグ用 Chunk を停止スコープ上で実行する（名前引きアクセス。共有バッファを使い回す）。
    /// `LoadName`/`DeclareName` は現在の `scopes`（停止フレーム）に対して名前解決・宣言する。
    fn run_debug_chunk(&mut self, chunk: &crate::vm::Chunk) -> Result<Value, String> {
        let mut buf = std::mem::take(&mut self.vm_stack);
        let base = buf.len();
        buf.resize(base + chunk.n_locals, Value::None);
        let result = crate::vm::run(self, chunk, &mut buf, base, None);
        buf.truncate(base);
        self.vm_stack = buf;
        result
    }

    /// 文の実行前に一時停止すべきかを判定する。`exec()` 冒頭で毎回呼ばれ、停止する場合は表示スパンを返す。
    pub(super) fn should_pause_at(&self, stmt: &Stmt) -> Option<Span> {
        if self.should_pause_now() {
            Some(self.best_span_for(stmt))
        } else {
            None
        }
    }

    /// **停止すべきか**を「モード × 呼び出し深さ」だけで判定する（表示スパンの決定は呼び出し側）。
    ///
    /// ツリーウォーク（`should_pause_at`）と VM の文境界判定（`vm_should_pause`）が
    /// **同じ判断を 2 箇所に持たない**ようにするための共通部。片方だけ直すと
    /// `--vm=off` と `--vm=auto` でステップ位置がずれる（[compare_debug_modes.ps1] が検出する）。
    ///
    /// 副作用: StepInto / StepOut は停止時にモードを StepOver へ遷移させる（元実装のまま）。
    fn should_pause_now(&self) -> bool {
        let mode = DBG_MODE.with(|m| m.borrow().clone());
        match mode {
            DbgMode::Inactive => false,
            DbgMode::StepOver => {
                // Only pause at the same depth as when the breakpoint fired.
                self.call_stack.len() == DBG_ENTRY_DEPTH.with(|d| *d.borrow())
            }
            DbgMode::StepInto => {
                let entry = DBG_ENTRY_DEPTH.with(|d| *d.borrow());
                let depth = self.call_stack.len();
                if depth > entry {
                    // We entered a function — switch to StepOver and pause here.
                    DBG_ENTRY_DEPTH.with(|d| *d.borrow_mut() = depth);
                    DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::StepOver);
                    DBG_STEP_INTO_SKIP.with(|s| *s.borrow_mut() = 0);
                    true
                } else if depth == entry {
                    // Same depth: check if we still need to let one statement pass.
                    let skip = DBG_STEP_INTO_SKIP.with(|s| {
                        let v = *s.borrow();
                        if v > 0 {
                            *s.borrow_mut() = v - 1;
                        }
                        v
                    });
                    if skip > 0 {
                        // Let this statement execute; the function call inside it
                        // will trigger the depth > entry branch above.
                        false
                    } else {
                        // The statement that was supposed to call a function has
                        // already run (or had no call). Fall back to step-over.
                        DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::StepOver);
                        true
                    }
                } else {
                    false
                }
            }
            DbgMode::StepOut { target } => {
                if self.call_stack.len() <= target {
                    // We have returned to (or past) the target depth.
                    DBG_MODE.with(|m| *m.borrow_mut() = DbgMode::StepOver);
                    DBG_ENTRY_DEPTH.with(|d| *d.borrow_mut() = self.call_stack.len());
                    true
                } else {
                    false
                }
            }
        }
    }

    /// VM の**文境界**から呼ばれる停止判定（#1）。`Stmt` の代わりに行テーブルの span を受け取る。
    ///
    /// `span_idx` は `Chunk::stmt_spans` の値（`spans` への index か `STMT_NO_SPAN`）。
    /// `STMT_NO_SPAN` は「位置情報を持たない文」で、ツリーウォークの `best_span_for` と同じく
    /// `dbg_last_span` へフォールバックする（そうしないと transcript が食い違う）。
    pub(crate) fn vm_should_pause(&mut self, chunk: &crate::vm::Chunk, span_idx: u32) -> Option<Span> {
        if !self.should_pause_now() {
            return None;
        }
        let span = chunk
            .spans
            .get(span_idx as usize)
            .cloned()
            .or_else(|| self.dbg_last_span.clone())
            .unwrap_or(Span {
                file: self.source_map.keys().next().cloned().unwrap_or_default().into(),
                line: 0,
                col: 0,
            });
        Some(span)
    }

    /// VM フレームで停止し、デバッガ REPL へ入る（#1）。
    ///
    /// VM 適格関数のローカルは **flat buffer（`buf[base..]`）にあり `scopes` に存在しない**ので、
    /// そのまま REPL へ入ると「呼び出し元のローカルが見えてしまう」。
    /// そこで停止中だけ `chunk.local_names`（slot → 変数名。V-E で用意され、ここが**最初の消費者**）
    /// を使って一時スコープを組み、`frame_floor` を進めて呼び出し元を隠す
    /// ＝ ツリーウォークで停止したときと同じ見え方にする。
    ///
    /// 値は**コピー**を見せる。REPL はプログラム変数への代入を拒否する仕様なので
    /// 書き戻しは不要（`let dbg::x` だけが書き込み可能で、それはこの一時スコープに入る）。
    pub(crate) fn vm_debug_pause(
        &mut self,
        chunk: &crate::vm::Chunk,
        buf: &[Value],
        base: usize,
        span: &Span,
        declared: &[bool],
    ) -> Result<(), String> {
        let saved_floor = self.frame_floor;
        let saved_len = self.scopes.len();
        self.push_scope();
        self.frame_floor = saved_len;
        for (slot, name) in chunk.local_names.iter().enumerate() {
            if name.is_empty() || name == "_" {
                continue; // temp slot（無名）と `_` は見せない
            }
            // ⚠ **まだ宣言文を実行していない slot は見せない**。flat buffer は全 slot を
            // `None` で初期化するので、見せるとツリーウォークでは NameError になる名前が
            // `None` として引けてしまう（off/auto 不一致）。
            if !declared.get(slot).copied().unwrap_or(false) {
                continue;
            }
            if let Some(v) = buf.get(base + slot) {
                self.declare_var(name.clone(), Var::new(v.clone(), false));
            }
        }
        let r = self.exec_breakpoint(span);
        // 例外で抜けても必ず戻す。
        self.scopes.truncate(saved_len);
        self.frame_floor = saved_floor;
        r.map(|_| ())
    }

    /// 文に使用可能な最良の `Span` を返す。スパンがなければ最後の既知スパン、最悪は行 0 を返す。
    fn best_span_for(&self, stmt: &Stmt) -> Span {
        if let Some((file, line)) = stmt_location(stmt) {
            return Span {
                file: file.into(),
                line,
                col: 1,
            };
        }
        // Fall back to last known good span (set in exec() after every successful pause).
        if let Some(ref s) = self.dbg_last_span {
            return s.clone();
        }
        let file = self.source_map.keys().next().cloned().unwrap_or_default();
        Span {
            file: file.into(),
            line: 0,
            col: 0,
        }
    }

    /// 直前に表示したスパンを記録し、次の位置不明文のフォールバックとして使えるようにする。
    pub(super) fn record_dbg_span(&mut self, span: &Span) {
        if span.line != 0 {
            self.dbg_last_span = Some(span.clone());
        }
    }
}
