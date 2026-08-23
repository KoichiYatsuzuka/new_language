// prof.rs — 実行時間分布の計測フック（**`--features prof` でのみコンパイルされる**）。
//
// 目的: 「非コンパイル（解釈実行）の .ar を走らせたとき、時間がどこに行くか」を実測する。
// 2 つの軸を同時に取る。
//
//   軸1: パイプラインの段（起動 / 字句 / 構文 / 型検査 / 解決 / 初期化 / 実行 / 後始末）
//        → `Instant` の直接計測。段は数回しか通らないので計測費用は無視できる。
//
//   軸2: 実行中の **op 別の時間**（VM ディスパッチループ）
//        → **統計サンプリング**。ディスパッチループが「今どの op か」を relaxed store で
//          共有変数へ置き、別スレッドが一定間隔で読む。
//          ⚠ **op ごとに時計を読む方式は採らない**（安い op ほど相対誤差が大きくなり、
//          「命令には値段の差がある」（#46）という肝心の量が歪む）。サンプリングなら
//          サンプル数がそのまま滞在時間に比例する。
//
// ⚠ 既定ビルドではこのモジュールごと消える。診断フックを実行経路に足すときのルール
// （#10-a: `cfg!(feature = ..)` を先に見て定数 false にする）に従うこと。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

// ───────────────────────────── 軸1: パイプラインの段 ─────────────────────────────

/// 計測する段。`PHASE_NAMES` と**同じ順序**。
#[derive(Clone, Copy)]
pub enum Phase {
    /// プロセス開始〜ソース読み込み完了（引数解析・ファイル読み）
    Startup = 0,
    Lex = 1,
    Parse = 2,
    TypeCheck = 3,
    Resolve = 4,
    /// `Interpreter::new()` ＋ 注釈注入・検索パス設定
    InterpInit = 5,
    /// 最上位文の実行（VM コンパイル・import・実処理を全部含む）
    Exec = 6,
    /// AST・インタープリタ・値の解放
    Teardown = 7,
}

pub const PHASE_NAMES: [&str; 8] = [
    "startup",
    "lex",
    "parse",
    "type_check",
    "resolve",
    "interp_init",
    "exec",
    "teardown",
];

static PHASE_NS: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// `Exec` の内訳のうち、**VM のバイトコード生成**に費やした時間（`Exec` の一部）。
static COMPILE_NS: AtomicU64 = AtomicU64::new(0);
/// コンパイルした Chunk の本数（1 本あたりの費用を出すため）。
static COMPILE_N: AtomicU64 = AtomicU64::new(0);

static START: Mutex<Option<Instant>> = Mutex::new(None);

/// プロセス開始時刻を記録する（`main` の先頭で 1 回）。
pub fn mark_start() {
    *START.lock().unwrap() = Some(Instant::now());
}

/// 起動処理（引数解析・ファイル読み）の終わり。ここまでを `Startup` に計上する。
pub fn mark_startup_done() {
    let el = START.lock().unwrap().map(|t| t.elapsed().as_nanos() as u64);
    if let Some(ns) = el {
        add(Phase::Startup, ns);
    }
}

pub fn add(phase: Phase, ns: u64) {
    PHASE_NS[phase as usize].fetch_add(ns, Ordering::Relaxed);
}

/// スコープを抜けるときに経過を段へ加算するガード。
pub struct Timer {
    phase: Phase,
    t0: Instant,
}

impl Timer {
    pub fn new(phase: Phase) -> Timer {
        Timer {
            phase,
            t0: Instant::now(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        add(self.phase, self.t0.elapsed().as_nanos() as u64);
    }
}

/// VM のコンパイル時間を加算する（`vm::compiler` の入口から呼ぶ）。
pub fn add_compile(ns: u64, chunks: u64) {
    COMPILE_NS.fetch_add(ns, Ordering::Relaxed);
    COMPILE_N.fetch_add(chunks, Ordering::Relaxed);
}

thread_local! {
    /// ネストしたコンパイル（`compile_fn` の中から更に別の Chunk）を二重計上しないための深さ。
    static COMPILE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `vm::compiler` の公開入口に置く計時ガード。**最外だけ**が計上する。
pub struct CompileTimer {
    t0: Instant,
    outermost: bool,
}

impl CompileTimer {
    pub fn new() -> CompileTimer {
        let outermost = COMPILE_DEPTH.with(|d| {
            let v = d.get();
            d.set(v + 1);
            v == 0
        });
        CompileTimer {
            t0: Instant::now(),
            outermost,
        }
    }
}

impl Default for CompileTimer {
    fn default() -> CompileTimer {
        CompileTimer::new()
    }
}

impl Drop for CompileTimer {
    fn drop(&mut self) {
        COMPILE_DEPTH.with(|d| d.set(d.get() - 1));
        if self.outermost {
            add_compile(self.t0.elapsed().as_nanos() as u64, 1);
        }
    }
}

// ───────────────── 軸1b: 段の**内訳**（`sum` には入れない補助計測） ─────────────────
//
// 段（`Phase`）は「合計が in_main になる」不変条件を持たせたいので、内訳はこちらへ分ける。
// ⚠ 内訳を `Phase` に足すと二重計上になり `PHASE sum` が壊れる。

/// `interp_init` の内訳（#58 の調査用）。`SUB_NAMES` と**同じ順序**。
#[derive(Clone, Copy)]
pub enum Sub {
    /// `Interpreter::new()`（組み込みグローバル登録・EventLoop 生成）
    InterpNew = 0,
    /// `set_toplevel_globals`（`toplevel_declared_globals` の AST 走査を含む）
    ToplevelGlobals = 1,
    /// `set_annotations` ＋ `add_source_text`
    AnnotSource = 2,
    /// `ar_config.json` 由来の検索パスの**登録**（起点を覚えるだけ）。
    ///
    /// ⚠⚠ **ここは常に ~0.000 ms でなければならない**（#69）。#69 以前は祖先ウォーク
    /// （`exists()` の syscall 連打 ＋ 読み込み ＋ JSON パース）を**起動時に必ず**行っており、
    /// `interp_init` の **48〜53%**（repo 外の深い階層では 55〜62%）を占めていた。
    /// 今は `Interpreter::python_search_dirs()` の初回参照まで遅延する。
    /// ⇒ **この値が 0 でなくなったら、eager なウォークが復活している。**
    CfgWalk = 3,
    /// `set_cli_args`
    CliArgs = 4,
}

pub const SUB_NAMES: [&str; 5] = [
    "interp_new",
    "toplevel_globals",
    "annot+source",
    "ar_config_setup",
    "cli_args",
];

static SUB_NS: [AtomicU64; 5] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub fn add_sub(sub: Sub, ns: u64) {
    SUB_NS[sub as usize].fetch_add(ns, Ordering::Relaxed);
}

/// `Timer` の内訳版。
pub struct SubTimer {
    sub: Sub,
    t0: Instant,
}

impl SubTimer {
    pub fn new(sub: Sub) -> SubTimer {
        SubTimer {
            sub,
            t0: Instant::now(),
        }
    }
}

impl Drop for SubTimer {
    fn drop(&mut self) {
        add_sub(self.sub, self.t0.elapsed().as_nanos() as u64);
    }
}

// ───────────────────────────── 軸2: op 別サンプリング ─────────────────────────────

/// 「いま実行中の op」。`0` = VM の外（ツリーウォーク／パース／後始末など）、
/// `n+1` = `op_prof::OP_NAMES[n]`。
static CUR: AtomicU32 = AtomicU32::new(0);
static SAMPLING: AtomicBool = AtomicBool::new(false);
static SAMPLER_RUN: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Option<Vec<u64>>> = Mutex::new(None);
static SAMPLER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

const NBUCKET: usize = 128;

/// ディスパッチループから毎命令呼ばれる。**relaxed store 1 本だけ**。
#[inline(always)]
pub fn note_op(op: &crate::vm::op::Op) {
    if SAMPLING.load(Ordering::Relaxed) {
        CUR.store(crate::vm::op_prof::op_index(op) as u32 + 1, Ordering::Relaxed);
    }
}

/// VM の外へ出たことを記録する（ツリーウォークの文ディスパッチ入口）。
#[inline(always)]
pub fn note_outside() {
    if SAMPLING.load(Ordering::Relaxed) {
        CUR.store(0, Ordering::Relaxed);
    }
}

/// native（FFI）の呼び先本体を表す専用バケット。`Op::Call` の**呼び出し機構**と
/// **呼び先が外で使った時間**を分けるために置く（分けないと FFI の時間が `Call` に紛れる）。
pub const NATIVE_BUCKET: u32 = 87;

/// `Op::Call` が native の呼び先へ落ちる直前に呼ぶ。
#[inline(always)]
pub fn note_native_callee() {
    if SAMPLING.load(Ordering::Relaxed) {
        CUR.store(NATIVE_BUCKET, Ordering::Relaxed);
    }
}

/// `vm::run::run` の入口に置き、**抜けるときに呼び出し元の op へ戻す**ガード。
///
/// ⚠ これが無いと、呼び先の Chunk が返った後の時間（呼び出し機構の後半・
/// VM の外の待ち時間）が **最後に実行した op**（多くは `ReturnNil`）へ張り付く。
/// 実際に `event_handler.ar` の 1 秒の待ちが丸ごと `ReturnNil` に化けていた。
pub struct CurGuard(u32);

impl CurGuard {
    #[inline(always)]
    pub fn new() -> CurGuard {
        CurGuard(CUR.load(Ordering::Relaxed))
    }
}

impl Default for CurGuard {
    fn default() -> CurGuard {
        CurGuard::new()
    }
}

impl Drop for CurGuard {
    #[inline(always)]
    fn drop(&mut self) {
        if SAMPLING.load(Ordering::Relaxed) {
            CUR.store(self.0, Ordering::Relaxed);
        }
    }
}

fn sample_interval_us() -> u64 {
    std::env::var("AR_PROF_US")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
}

/// サンプラースレッドを起動する。`AR_PROF=ops` のときだけ呼ばれる。
fn start_sampler() {
    let iv = std::time::Duration::from_micros(sample_interval_us());
    SAMPLER_RUN.store(true, Ordering::SeqCst);
    SAMPLING.store(true, Ordering::SeqCst);
    let h = std::thread::spawn(move || {
        // ⚠ `thread::sleep` は Windows では 1〜15ms 粒度なので使えない。
        // QPC を読みながらスピンする（1 コアを焼くが、計測中だけ）。
        let mut hist = vec![0u64; NBUCKET];
        let mut next = Instant::now() + iv;
        while SAMPLER_RUN.load(Ordering::Relaxed) {
            while Instant::now() < next {
                std::hint::spin_loop();
            }
            next += iv;
            let v = CUR.load(Ordering::Relaxed) as usize;
            if v < NBUCKET {
                hist[v] += 1;
            }
        }
        *RESULT.lock().unwrap() = Some(hist);
    });
    *SAMPLER.lock().unwrap() = Some(h);
}

fn stop_sampler() {
    if !SAMPLING.swap(false, Ordering::SeqCst) {
        return;
    }
    SAMPLER_RUN.store(false, Ordering::SeqCst);
    if let Some(h) = SAMPLER.lock().unwrap().take() {
        let _ = h.join();
    }
}

// ───────────────────────────── 起動・出力 ─────────────────────────────

fn mode() -> String {
    std::env::var("AR_PROF").unwrap_or_default()
}

/// 有効かどうか（`AR_PROF` が空でなければ段の計測を出す）。
pub fn enabled() -> bool {
    !mode().is_empty()
}

/// op サンプリングまで行うか。
pub fn ops_enabled() -> bool {
    mode() == "ops"
}

/// 実行直前に呼ぶ（サンプラーの起動）。
pub fn begin_exec() {
    if ops_enabled() {
        start_sampler();
    }
}

/// 実行直後に呼ぶ（サンプラーの停止）。
pub fn end_exec() {
    stop_sampler();
}

/// 結果を stderr へ出す。`AR_PROF_CSV=<path>` があれば CSV も書く。
pub fn dump() {
    if !enabled() {
        return;
    }
    // 2 回呼ばれても 1 回しか出さない（`run_program` の正常終了とエラー経路の両方から呼ぶ）。
    static DUMPED: AtomicBool = AtomicBool::new(false);
    if DUMPED.swap(true, Ordering::SeqCst) {
        return;
    }
    stop_sampler();
    let total_ns = START
        .lock()
        .unwrap()
        .map(|t| t.elapsed().as_nanos() as u64)
        .unwrap_or(0);
    let mut lines = Vec::new();
    lines.push(format!(
        "=== PROF phases (total_in_main = {:.3} ms) ===",
        total_ns as f64 / 1e6
    ));
    let mut sum = 0u64;
    for (i, name) in PHASE_NAMES.iter().enumerate() {
        let ns = PHASE_NS[i].load(Ordering::Relaxed);
        sum += ns;
        lines.push(format!(
            "PHASE {name:<12} {:>12.3} ms  {:>6.2}%",
            ns as f64 / 1e6,
            if total_ns > 0 {
                ns as f64 * 100.0 / total_ns as f64
            } else {
                0.0
            }
        ));
    }
    lines.push(format!(
        "PHASE {:<12} {:>12.3} ms  {:>6.2}%   (accounted)",
        "sum",
        sum as f64 / 1e6,
        if total_ns > 0 {
            sum as f64 * 100.0 / total_ns as f64
        } else {
            0.0
        }
    ));
    let cns = COMPILE_NS.load(Ordering::Relaxed);
    lines.push(format!(
        "PHASE {:<12} {:>12.3} ms  {:>6.2}%   ({} chunks, part of exec)",
        "vm_compile",
        cns as f64 / 1e6,
        if total_ns > 0 {
            cns as f64 * 100.0 / total_ns as f64
        } else {
            0.0
        },
        COMPILE_N.load(Ordering::Relaxed)
    ));

    // 段の内訳（`sum` には入っていない補助計測）。0 なら出さない。
    let sub_tot: u64 = SUB_NS.iter().map(|a| a.load(Ordering::Relaxed)).sum();
    if sub_tot > 0 {
        for (i, name) in SUB_NAMES.iter().enumerate() {
            let ns = SUB_NS[i].load(Ordering::Relaxed);
            lines.push(format!(
                "SUB   {name:<16} {:>12.3} ms  {:>6.2}% of interp_init",
                ns as f64 / 1e6,
                {
                    let init = PHASE_NS[Phase::InterpInit as usize].load(Ordering::Relaxed);
                    if init > 0 {
                        ns as f64 * 100.0 / init as f64
                    } else {
                        0.0
                    }
                }
            ));
        }
    }

    let hist = RESULT.lock().unwrap().take();
    if let Some(hist) = hist {
        let tot: u64 = hist.iter().sum();
        let iv = sample_interval_us();
        lines.push(String::new());
        lines.push(format!(
            "=== PROF ops (sampling: {} samples @ {}us = {:.3} ms of wall) ===",
            tot,
            iv,
            (tot * iv) as f64 / 1e3
        ));
        let mut rows: Vec<(u64, String)> = Vec::new();
        for (i, c) in hist.iter().enumerate() {
            if *c == 0 {
                continue;
            }
            let name = if i == 0 {
                "(outside_VM)".to_string()
            } else if i as u32 == NATIVE_BUCKET {
                "(native_callee)".to_string()
            } else {
                crate::vm::op_prof::OP_NAMES
                    .get(i - 1)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("?{i}"))
            };
            rows.push((*c, name));
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (c, name) in &rows {
            lines.push(format!(
                "OP {name:<22} {:>10}  {:>6.2}%  {:>10.3} ms",
                c,
                if tot > 0 {
                    *c as f64 * 100.0 / tot as f64
                } else {
                    0.0
                },
                (*c * iv) as f64 / 1e3
            ));
        }
        if let Ok(path) = std::env::var("AR_PROF_CSV") {
            let mut csv = String::from("kind,name,samples,pct,ms\n");
            for (c, name) in &rows {
                csv.push_str(&format!(
                    "op,{name},{c},{:.4},{:.4}\n",
                    if tot > 0 {
                        *c as f64 * 100.0 / tot as f64
                    } else {
                        0.0
                    },
                    (*c * iv) as f64 / 1e3
                ));
            }
            for (i, name) in PHASE_NAMES.iter().enumerate() {
                csv.push_str(&format!(
                    "phase,{name},0,0,{:.4}\n",
                    PHASE_NS[i].load(Ordering::Relaxed) as f64 / 1e6
                ));
            }
            csv.push_str(&format!("phase,vm_compile,0,0,{:.4}\n", cns as f64 / 1e6));
            csv.push_str(&format!(
                "phase,total_in_main,0,0,{:.4}\n",
                total_ns as f64 / 1e6
            ));
            let _ = std::fs::write(path, csv);
        }
    }
    eprintln!("{}", lines.join("\n"));
}
