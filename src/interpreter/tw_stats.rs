// interpreter/tw_stats.rs — 診断フック `AR_TW_STATS=1`（#10 のスコープ計測用）。
//
// **ツリーウォークが `--vm=auto` の実行中に実際に何をしているか**を数える。
// #3（強制バイトコード）へ残る距離は「top-level に何が書かれているか」ではなく
// 「実行時にツリーウォークへ落ちる文が何か」でしか測れないため、実測用に置く。
//
// 計上するもの:
//   - `Stmt` バリアント別の `exec()` ディスパッチ回数を **モジュール最上位 / 関数本体内**に分けて
//   - VM チャンクのコンパイル成否（関数／ジェネレータ）
//
// **既定ではコンパイルされない**。`cargo build --features tw_stats` + `AR_TW_STATS=1` で有効。
// feature を切る理由は `enabled()` のコメント参照（env 判定だけだと 11% 退行する）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::ast::Stmt;

/// 収集先。async のワーカースレッドからも計上されるため TLS ではなく共有ロックにする。
static COUNTS: OnceLock<Mutex<HashMap<(&'static str, String), u64>>> = OnceLock::new();

// ツリーウォークの関数本体に入っている深さ。0 ならモジュール最上位。
// 診断専用なので TLS で十分（スレッドごとに最上位から始まる）。
thread_local! {
    static TW_FN_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// 計測が有効か。
///
/// ⚠ **`cfg!` の判定を先に置くこと。** feature 無しビルドでは定数 `false` になり、
/// 呼び出し側の `if enabled() { ... }` ごと消える。ここを環境変数だけの判定にすると
/// `exec()` の 1 文ごとに `OnceLock` の atomic 読みが残り、**11% 退行する**
/// （`partial_call_overhead.ar` = 5000 万回の文ディスパッチで実測）。
#[inline(always)]
pub(crate) fn enabled() -> bool {
    if !cfg!(feature = "tw_stats") {
        return false;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AR_TW_STATS").is_ok_and(|v| !v.is_empty()))
}

fn bump(cat: &'static str, key: &str) {
    let m = COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut g) = m.lock() {
        *g.entry((cat, key.to_string())).or_insert(0) += 1;
    }
}

// ⚠ `FnBodyGuard`（ツリーウォークの関数本体を囲むガード）は #33 で削除した。
// 関数本体のツリーウォーク経路そのものが無くなったので、`in_fn` は**構造的に 0**
// （0 でない値が出たら計測側の配線ミス）。`TW_FN_DEPTH` は判定の形を保つために残す。

/// `exec()` のディスパッチを 1 件計上する。
///
/// 3 分類する（#10-d）: **メインプログラム最上位 / import モジュール本体 / 関数本体内**。
/// モジュール本体は `exec_module` が `push_scope` してから回すので `toplevel_vm_candidate`
/// （`scopes.len() == 1`）が偽になり、**現状まるごとツリーウォーク**。メイン最上位と
/// 混ぜて数えると「最上位に何が残っているか」を読み違える。
pub(crate) fn record_stmt(stmt: &Stmt) {
    let cat = if TW_FN_DEPTH.with(|d| d.get()) > 0 {
        "in_fn"
    } else if TW_MODULE_DEPTH.with(|d| d.get()) > 0 {
        "module_body"
    } else {
        "toplevel"
    };
    bump(cat, stmt_kind(stmt));
}

// import モジュール本体を実行中の深さ（#10-d）。
thread_local! {
    static TW_MODULE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// import モジュール本体の実行を囲むガード（#10-d）。
pub(crate) struct ModuleBodyGuard;

impl ModuleBodyGuard {
    pub(crate) fn new() -> Self {
        TW_MODULE_DEPTH.with(|d| d.set(d.get() + 1));
        ModuleBodyGuard
    }
}

impl Drop for ModuleBodyGuard {
    fn drop(&mut self) {
        TW_MODULE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// VM コンパイルが諦めた地点を計上する（`vm/compiler.rs` の bail サイトから呼ぶ）。
/// `label` は「どの bail サイトか」、`detail` は「どの構文で諦めたか」。
///
/// **今どちらのコンパイルの最中かで分類先を分ける**（#27）。関数本体（`vm_bail_fn`）と
/// 最上位文（`vm_bail_toplevel`）は残タスクが別物なので、混ぜると内訳が読めない。
pub(crate) fn record_bail(label: &str, detail: &str) {
    BAIL_COUNT.with(|c| c.set(c.get() + 1));
    if COMPILING_TOPLEVEL.with(|c| c.get()) {
        // 最上位は**どの文種別を落としたか**を前置する（#27-c）。
        let kind = TOPLEVEL_STMT_KIND.with(|k| k.get());
        // ⚠ キーに空白を入れないこと（集計スクリプトが `key=value` を空白で分割する）。
        bump("vm_bail_toplevel", &format!("{kind}/{label}:{detail}"));
    } else {
        bump("vm_bail_fn", &format!("{label}:{detail}"));
    }
}

// これまでに記録した bail 件数（`compile_fn` が「未帰属の失敗」を検出するのに使う）。
thread_local! {
    static BAIL_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    // 最上位文のコンパイル中か（bail の分類に使う）。
    static COMPILING_TOPLEVEL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 最上位文のコンパイル区間を囲むガード（#27）。bail の分類先を切り替える。
pub(crate) struct ToplevelCompileGuard(bool, &'static str);

impl ToplevelCompileGuard {
    /// `stmt` は**コンパイル対象の最上位文**。bail のキーに種別を前置するのに使う（#27-c）。
    ///
    /// bail 理由だけでは「どの文を落としたか」が分からない。#3 に効くのは
    /// **制御フローを含む文**（`If`/`For`/`While`/`Try`/`Match`/`Block`）を落としている bail だけなので、
    /// 種別で切れないと優先度を決められない。
    pub(crate) fn new(stmt: &Stmt) -> Self {
        let prev = COMPILING_TOPLEVEL.with(|c| c.replace(true));
        let kind = stmt_kind(stmt);
        let prev_kind = TOPLEVEL_STMT_KIND.with(|k| k.replace(kind));
        ToplevelCompileGuard(prev, prev_kind)
    }
}

impl Drop for ToplevelCompileGuard {
    fn drop(&mut self) {
        COMPILING_TOPLEVEL.with(|c| c.set(self.0));
        TOPLEVEL_STMT_KIND.with(|k| k.set(self.1));
    }
}

// コンパイル中の最上位文の種別（#27-c。bail のキーに前置する）。
thread_local! {
    static TOPLEVEL_STMT_KIND: std::cell::Cell<&'static str> = const { std::cell::Cell::new("-") };
}

/// これまでに記録した bail 件数を返す。
pub(crate) fn bail_count() -> u64 {
    BAIL_COUNT.with(|c| c.get())
}

/// `Stmt` バリアント名を公開する（VM コンパイラの bail 計上用）。
pub(crate) fn stmt_kind_of(stmt: &Stmt) -> &'static str {
    stmt_kind(stmt)
}

/// **VM に載せる前に弾かれた**関数呼び出しを計上する（#27）。
/// `vm_eligible` が偽だとコンパイルを試みないので bail 統計には現れない。
pub(crate) fn record_ineligible(why: &'static str) {
    bump("vm_ineligible", why);
}

/// クロージャ生成時のキャプチャ内訳を計上する（#27）。
pub(crate) fn record_capture(kind: &'static str) {
    bump("closure_capture", kind);
}

/// **ツリーウォークの制御フロー**（TLS / センチネルを使う経路）に入ったことを計上する（#3）。
///
/// #3（強制バイトコード）は「TLS とセンチネルの実削除」を掲げているが、削除できるのは
/// **通常実行で 1 度も通らない**ものだけ。ここで実測してから消す。
pub(crate) fn record_tls(site: &'static str) {
    // ⚠ ここは**ホットパス**（ツリーウォークのループ入口）なので `enabled()` を先に見る。
    // `enabled()` は `cfg!` で定数 false に畳まれるため既定ビルドではコードごと消える（#10-a）。
    if !enabled() {
        return;
    }
    bump("tw_control_flow", site);
}

/// VM チャンクのコンパイル成否を計上する。
pub(crate) fn record_compile(kind: &'static str, ok: bool) {
    if ok {
        bump("vm_compile", kind);
    } else {
        bump("vm_compile", &format!("{kind}_FAILED"));
    }
}

/// 収集結果を stderr へ出す（`run_program` の末尾から呼ぶ）。
pub(crate) fn dump() {
    let Some(m) = COUNTS.get() else { return };
    let Ok(g) = m.lock() else { return };
    let mut rows: Vec<_> = g
        .iter()
        .map(|((c, k), &v)| (*c, k.as_str(), v))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0).then(b.2.cmp(&a.2)));
    for cat in ["toplevel", "module_body", "in_fn", "vm_compile", "vm_ineligible", "closure_capture", "vm_bail_fn", "vm_bail_toplevel", "tw_control_flow"] {
        let sub: Vec<_> = rows.iter().filter(|r| r.0 == cat).collect();
        if sub.is_empty() {
            continue;
        }
        let total: u64 = sub.iter().map(|r| r.2).sum();
        let body = sub
            .iter()
            .map(|r| format!("{}={}", r.1, r.2))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("TwStats[{cat}] total={total} {body}");
    }
}

/// `Stmt` バリアント名（計上キー）。
fn stmt_kind(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Expr(_) => "Expr",
        Stmt::Let(..) => "Let",
        Stmt::Const(..) => "Const",
        Stmt::Mut(..) => "Mut",
        Stmt::LetTuple { .. } => "LetTuple",
        Stmt::Static(..) => "Static",
        Stmt::Assign { .. } => "Assign",
        Stmt::AttrAssign { .. } => "AttrAssign",
        Stmt::AttrCompoundAssign { .. } => "AttrCompoundAssign",
        Stmt::CompoundAssign { .. } => "CompoundAssign",
        Stmt::If { .. } => "If",
        Stmt::Match { .. } => "Match",
        Stmt::While { .. } => "While",
        Stmt::For { .. } => "For",
        Stmt::Block(_) => "Block",
        Stmt::Return(_) => "Return",
        Stmt::Break => "Break",
        Stmt::Continue => "Continue",
        Stmt::Pass => "Pass",
        Stmt::BlockReturn(..) => "BlockReturn",
        Stmt::LoopYield(_) => "LoopYield",
        Stmt::Yield(_) => "Yield",
        Stmt::Freeze(..) => "Freeze",
        Stmt::FnDef { .. } => "FnDef",
        Stmt::GenDef { .. } => "GenDef",
        Stmt::ClassDef { .. } => "ClassDef",
        Stmt::TraitDef { .. } => "TraitDef",
        Stmt::ProtocolDef { .. } => "ProtocolDef",
        Stmt::Field { .. } => "Field",
        Stmt::NewTypeDef { .. } => "NewTypeDef",
        Stmt::EnumDef { .. } => "EnumDef",
        Stmt::Try { .. } => "Try",
        Stmt::Raise { .. } => "Raise",
        Stmt::Import { .. } => "Import",
        Stmt::FromImport { .. } => "FromImport",
        Stmt::AsyncAssign { .. } => "AsyncAssign",
        Stmt::BreakPoint { .. } => "BreakPoint",
        Stmt::DebugLet { .. } => "DebugLet",
        Stmt::EventSubscribe { .. } => "EventSubscribe",
        Stmt::EventUnsubscribe { .. } => "EventUnsubscribe",
    }
}
