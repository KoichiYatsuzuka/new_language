// interpreter.rs — Interpreter 構造体・スコープ変数・初期化
//
// サブモジュール担当:
//   interpreter/value.rs      — 実行時値の型定義 (Value / FnValue / ClassValue / …)
//   interpreter/built_in_types.rs — 組み込み型・例外クラス・列挙型の初期化
//   interpreter/scope.rs      — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)
//   interpreter/ops.rs        — 演算・比較・真偽値・表示 (is_truthy / type_name / display / apply_binop など)
//   interpreter/exec.rs       — 文の実行 (exec / exec_block / exec_scoped_block)
//   interpreter/eval.rs       — 式の評価・attr_assign (eval / attr_assign)
//   interpreter/functions.rs  — 関数・ジェネレータ・オーバーロード実行
//   interpreter/classes.rs    — クラス・インスタンス管理
//   interpreter/exceptions.rs — 例外クラス構築・トレースバック
//   interpreter/templates.rs  — テンプレート展開・AST置換
//
// 実行フロー:
//   Interpreter::new() でグローバルスコープと組み込み型・例外クラスを初期化し、
//   exec(stmt) / eval(expr) を通じてツリーウォーク実行を行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::Accessibility;

#[path = "interpreter/async_mgr.rs"]
pub(crate) mod async_mgr;
#[path = "interpreter/event_loop.rs"]
pub(crate) mod event_loop;
#[path = "interpreter/classes/mod.rs"]
mod classes;
#[path = "interpreter/cpp_bridge/mod.rs"]
// ⚠ C/C++ ブリッジ（libloading）。`native` 限定（評価コア切り出し #2）。
#[cfg(feature = "native")]
pub(crate) mod cpp_bridge;
#[path = "interpreter/proc_bridge.rs"]
// ⚠ 名前付きパイプ（Windows API）。`native` 限定（評価コア切り出し #5）。
//   参照元は `cs_proc_runtime` / `js_proc_runtime` だけで、どちらもスタブに差し替わる。
#[cfg(feature = "native")]
pub(crate) mod proc_bridge;
#[path = "interpreter/cs_dll_runtime.rs"]
// ⚠ .NET ブリッジ（libloading）。評価コアビルドでは同名スタブへ差し替える
// （評価コア切り出し #2）。`Value::CsObject` の match アームが 4 ファイルに散っているので、
// **呼び出し側に `#[cfg]` を配らない**ためにモジュールごと差し替える。
#[cfg(feature = "native")]
pub(crate) mod cs_dll_runtime;
#[cfg(not(feature = "native"))]
#[path = "interpreter/cs_dll_runtime_stub.rs"]
pub(crate) mod cs_dll_runtime;
#[path = "interpreter/cs_proc_runtime.rs"]
// ⚠ .NET 別プロセスブリッジ。評価コアでは同名スタブへ差し替える（#5）。
#[cfg(feature = "native")]
pub(crate) mod cs_proc_runtime;
#[cfg(not(feature = "native"))]
#[path = "interpreter/cs_proc_runtime_stub.rs"]
pub(crate) mod cs_proc_runtime;
#[path = "interpreter/js_proc_runtime.rs"]
// ⚠ Node.js ブリッジ。評価コアでは同名スタブへ差し替える（#5）。
#[cfg(feature = "native")]
pub(crate) mod js_proc_runtime;
#[cfg(not(feature = "native"))]
#[path = "interpreter/js_proc_runtime_stub.rs"]
pub(crate) mod js_proc_runtime;

/// FFI 境界検査（#16）: 動的型付け言語から Arrow へ入る値をスタブ宣言型と突き合わせる。
// ⚠ **`#[path]` を明示する**。このファイルは `crates/arrow-frontend` から
//   `#[path = "../../../src/interpreter.rs"]` で取り込まれるため、素の `mod x;` は
//   **取り込み側のディレクトリ**を基準に探されて見つからない。兄弟モジュールが
//   すべて `#[path = "interpreter/..."]` を持っているのはこの理由（評価コア切り出し #6）。
#[path = "interpreter/ffi_boundary.rs"]
pub(crate) mod ffi_boundary;
#[path = "interpreter/debugger.rs"]
// `pub(crate)`: VM コンパイラが行テーブル構築で `stmt_span_of` を使う（#1）。
pub(crate) mod debugger;
#[path = "interpreter/eval/mod.rs"]
mod eval;
#[path = "interpreter/exceptions.rs"]
mod exceptions;
#[path = "interpreter/exec/mod.rs"]
mod exec;
pub(crate) use exec::{collect_referenced_names, fn_own_names};
#[path = "interpreter/functions/mod.rs"]
mod functions;
#[path = "interpreter/msvc_errors.rs"]
// ⚠ MSVC 診断の解析。参照元は `cpp_bridge` だけで、それが `native` 限定（#5）。
#[cfg(feature = "native")]
mod msvc_errors;
#[path = "interpreter/native_api/mod.rs"]
// ⚠ ネイティブ callback ABI（libloading）。評価コアビルドでは同名スタブへ差し替える
// （評価コア切り出し #2）。コアから参照されるのは `ErrSlot` /
// `lookup_native_method_ptr` / `try_dispatch_native_method` の 3 つだけで、
// いずれも**分岐の条件式**に埋まっているので `#[cfg]` を配ると条件が読めなくなる。
#[cfg(feature = "native")]
mod native_api;
#[cfg(not(feature = "native"))]
#[path = "interpreter/native_api_stub.rs"]
mod native_api;
// ⚠ `pub(crate)`: `vm::op` のテストが `primitive_ann_matches`（実行時の唯一の型表・
//    タスク 6.1）を引いて `TypeTag` とのずれを検査する。
#[path = "interpreter/ops/mod.rs"]
pub(crate) mod ops;
#[path = "interpreter/py_interop.rs"]
// ⚠ Python 相互運用（pyo3）。評価コアビルドでは同名スタブへ差し替える
// （評価コア切り出し #2）。`Value::PyObject` の match アームが 10 箇所以上に散っているので、
// **呼び出し側に `#[cfg]` を配らない**ためにモジュールごと差し替える。
#[cfg(feature = "native")]
mod py_interop;
#[cfg(not(feature = "native"))]
// ⚠ `#[path]` は**宣言元ファイルのディレクトリ基準**。`interpreter.rs` は `src/` にあるので
// `interpreter/` を前置する（`eval/mod.rs` 側は `mod.rs` なので相対のままでよい）。
#[path = "interpreter/py_interop_stub.rs"]
mod py_interop;
#[path = "interpreter/resolver.rs"]
pub(crate) mod resolver;
#[path = "interpreter/scope.rs"]
mod scope;
#[path = "interpreter/str_methods.rs"]
pub(super) mod str_methods;
#[path = "interpreter/templates.rs"]
mod templates;
/// 診断フック `AR_TW_STATS=1`（#10 のスコープ計測）。既定では完全に無効。
#[path = "interpreter/tw_stats.rs"]
pub(crate) mod tw_stats;
/// 最上位文の VM 実行経路（#10-b）。**`functions/execution.rs` へ移してはいけない**（同ファイル冒頭参照）。
#[path = "interpreter/vm_toplevel.rs"]
mod vm_toplevel;

#[cfg(test)]
#[path = "interpreter/tests/mod.rs"]
mod tests;

#[path = "interpreter/ast_value.rs"]
 pub(crate) mod ast_value;
#[path = "interpreter/built_in_types.rs"]
mod built_in_types;

#[path = "interpreter/value/mod.rs"]
pub mod value;
pub use value::*;

// ---------------------------------------------------------------------------
// Sentinel / thread-local (private to this module tree)
// ---------------------------------------------------------------------------

/// Sentinel string used to signal an in-flight language-level `raise` through the
/// `eval()` return channel (`Result<Value, String>`).
///
/// ## Dual error-channel design
///
/// The interpreter has two distinct error paths:
///
/// | Path | Type | Used by | Carries |
/// |------|------|---------|---------|
/// | `exec()` return | `Ok(ExecResult::Raise(e))` | statement execution | full `RaisedError` |
/// | `eval()` return | `Err(RAISE_SENTINEL)` | expression evaluation | only a sentinel; full error in `self.current_exception` |
///
/// The split exists because `eval()` returns `Result<Value, String>`, which cannot
/// carry a `RaisedError` directly.  When `eval()` returns `Err(RAISE_SENTINEL)`,
/// `self.current_exception` holds the `RaisedError`.
///
/// ## Invariants
///
/// * Every site that returns `Err(RAISE_SENTINEL)` **must** have set
///   `self.current_exception = Some(…)` immediately before.
/// * Every site that checks for `RAISE_SENTINEL` in an `Err` must propagate or
///   consume the error: either call `self.take_current_exception()` or re-return
///   `Err(RAISE_SENTINEL)` so the caller can do so.
/// * Internal bugs should return a plain, non-sentinel `Err(message)`.  A caller
///   that sees an `Err` string not equal to `RAISE_SENTINEL` knows it is an
///   interpreter bug rather than a user `raise`.
pub(crate) const RAISE_SENTINEL: &str = " __raise__";

thread_local! {
}

// ---------------------------------------------------------------------------
// Interpreter internals
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// FxHash — スコープ変数名用の高速ハッシュ
// ---------------------------------------------------------------------------

/// FxHash（rustc-hash 由来のアルゴリズム）。変数名のような短い文字列に対して
/// std デフォルトの SipHash より大幅に速い（~10ns → ~2ns）。
/// スコープのキーは攻撃者制御の入力ではないため DoS 耐性（SipHash の目的）は不要。
#[derive(Default, Clone, Copy)]
 struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            let v = u64::from_le_bytes(c.try_into().unwrap());
            self.hash = (self.hash.rotate_left(5) ^ v).wrapping_mul(FX_SEED);
        }
        for &b in chunks.remainder() {
            self.hash = (self.hash.rotate_left(5) ^ b as u64).wrapping_mul(FX_SEED);
        }
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `FxHasher` の BuildHasher。`ScopeMap::default()` で使用する。
#[derive(Default, Clone, Copy)]
 struct FxBuildHasher;

impl std::hash::BuildHasher for FxBuildHasher {
    type Hasher = FxHasher;
    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher::default()
    }
}

/// スコープ1段分の変数ストレージ（Phase R / R0）。
///
/// 従来の `HashMap<String, Var>` を **slot 配列（宣言順）** に置き換えたもの。
/// - `names` / `slots`: 平行配列。`slots[i]` が名前 `names[i]` の `Var`。
///   `Resolution::Local(slot)` は `slots[i]` を index 1回で読む（スコープ遡り・ハッシュなしの高速経路）。
/// - `index`: 名前 → slot の遅延ハッシュ索引。**大きいスコープ（=グローバル）でのみ**構築する。
///
/// 関数/ブロックのローカルスコープは通常ごく少数の変数しか持たないため、宣言は単純な `push`
/// （ハッシュ計算なし）、未解決名の引きは**線形走査**の方が `HashMap` より速い。
/// 変数数が `INDEX_THRESHOLD` を超えたスコープ（実質グローバルのみ）だけ索引を構築して O(1) 化する。
/// 宣言順（= slot 番号）は決定的なので、リゾルバが静的に付けた slot 番号と実行時の slot が一致する。
/// 既存呼び出し側との互換のため `HashMap` 互換の `get`/`get_mut`/`insert`/`contains_key`/`iter` を提供する。
#[derive(Default)]
 struct Scope {
    /// (名前, Var) を宣言順に持つ単一配列（allocation 1本）。`slots[i]` が slot i。
    slots: Vec<(String, Var)>,
    /// 大きいスコープでのみ構築される名前索引（`None` = 線形走査）。
    index: Option<HashMap<String, usize, FxBuildHasher>>,
}

/// このサイズを超えたスコープはハッシュ索引を構築する（グローバルスコープ想定）。
/// 関数/ブロックローカルは通常これ未満で、索引なしの線形走査で済む。
const INDEX_THRESHOLD: usize = 16;

impl Scope {
    /// 名前 → slot 番号を引く（索引があれば O(1)、なければ末尾からの線形走査）。
    #[inline]
    fn find(&self, name: &str) -> Option<usize> {
        if let Some(idx) = &self.index {
            idx.get(name).copied()
        } else {
            // 末尾（最後に宣言されたもの）から走査する。小さいスコープでは十分速い。
            self.slots.iter().rposition(|(n, _)| n == name)
        }
    }

    /// 名前で `Var` を引く。
    #[inline]
    pub(self) fn get(&self, name: &str) -> Option<&Var> {
        self.find(name).map(|i| &self.slots[i].1)
    }

    /// 名前で `Var` を可変参照で引く。
    #[inline]
    pub(self) fn get_mut(&mut self, name: &str) -> Option<&mut Var> {
        let i = self.find(name)?;
        Some(&mut self.slots[i].1)
    }

    /// 変数を宣言/上書きする。既存名は同じ slot を保持したまま値を差し替え、
    /// 新規名は配列末尾に slot を確保する。戻り値は上書き前の `Var`（新規なら `None`）。
    #[inline]
    pub(self) fn insert(&mut self, name: String, var: Var) -> Option<Var> {
        if let Some(i) = self.find(&name) {
            return Some(std::mem::replace(&mut self.slots[i].1, var));
        }
        let i = self.slots.len();
        if let Some(idx) = &mut self.index {
            idx.insert(name.clone(), i);
        }
        self.slots.push((name, var));
        // しきい値を超えたら索引を構築して以降 O(1) 化する（グローバル想定）。
        if self.index.is_none() && self.slots.len() > INDEX_THRESHOLD {
            let mut idx: HashMap<String, usize, FxBuildHasher> = Default::default();
            for (j, (n, _)) in self.slots.iter().enumerate() {
                idx.insert(n.clone(), j);
            }
            self.index = Some(idx);
        }
        None
    }

    /// 指定名が宣言済みか。
    #[inline]
    pub(self) fn contains_key(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// (名前, Var) を走査する（宣言順）。
    pub(self) fn iter(&self) -> impl Iterator<Item = (&String, &Var)> {
        self.slots.iter().map(|(n, v)| (n, v))
    }

    /// slot 番号で直接 `Var` を引く高速経路（`Resolution::Local` 用）。
    #[inline]
    pub(self) fn slot(&self, i: usize) -> Option<&Var> {
        self.slots.get(i).map(|(_, v)| v)
    }

    /// デバッグ検証用: 名前 → slot 番号。
    #[inline]
    pub(self) fn slot_of(&self, name: &str) -> Option<usize> {
        self.find(name)
    }
}

/// スコープ1段分の変数ストレージ（互換エイリアス）。
 type ScopeMap = Scope;

/// スコープ内の1つの変数エントリ。
///
/// - `Immutable(Value)`: 不変変数（`let` / `const`）
/// - `Mutable(Value)`: 可変変数（`mut`）。クロージャにキャプチャされるまでは値を直接保持する。
/// - `Cell(Rc<RefCell<Value>>)`: クロージャにキャプチャされた可変変数。外側スコープと共有セルを通じて読み書きする。
/// - `SlotCell(Rc<RefCell<Value>>)`: スロットキャッシュ（AST 焼き込み）に昇格したグローバル可変変数。
///   `freeze` されると `Immutable` に戻り `slot_epoch` が進む（キャッシュ一括無効化）。
 enum Var {
    Immutable(Value),
    Mutable(Value),
    Cell(Rc<RefCell<Value>>),
    SlotCell(Rc<RefCell<Value>>),
}

impl Var {
    /// 通常の変数エントリを作成する。
    pub(self) fn new(value: Value, mutable: bool) -> Self {
        if mutable {
            Var::Mutable(value)
        } else {
            Var::Immutable(value)
        }
    }

    /// クロージャ共有セルに基づく変数エントリを作成する（常に mutable）。
    pub(self) fn new_cell(cell: Rc<RefCell<Value>>) -> Self {
        Var::Cell(cell)
    }

    /// 変数の現在の値を返す。
    pub(self) fn get_value(&self) -> Value {
        match self {
            Var::Immutable(v) | Var::Mutable(v) => v.clone(),
            Var::Cell(rc) | Var::SlotCell(rc) => rc.borrow().clone(),
        }
    }

    /// 変数に新しい値をセットする。`Immutable` に対して呼ぶのは呼び出し元の責任。
    pub(self) fn set_value(&mut self, val: Value) {
        match self {
            Var::Immutable(v) | Var::Mutable(v) => *v = val,
            Var::Cell(rc) | Var::SlotCell(rc) => *rc.borrow_mut() = val,
        }
    }

    /// 変数が再代入可能かどうかを返す。
    pub(self) fn is_mutable(&self) -> bool {
        matches!(self, Var::Mutable(_) | Var::Cell(_) | Var::SlotCell(_))
    }

    /// クロージャ共有セルを返す。`Cell` / `SlotCell` でない場合は `None`。
    pub(self) fn cell(&self) -> Option<Rc<RefCell<Value>>> {
        match self {
            Var::Cell(rc) | Var::SlotCell(rc) => Some(rc.clone()),
            _ => None,
        }
    }

    /// クロージャに捕捉されたセル（`Cell`）かどうか。
    /// `SlotCell`（スロットキャッシュ昇格）は含まない — freeze 可能なため。
    pub(self) fn is_closure_cell(&self) -> bool {
        matches!(self, Var::Cell(_))
    }
}

/// ツリーウォークインタープリタ本体。
///
/// ソースファイルを字句解析・構文解析・型検査した後、AST を受け取り実行する。
/// 主要エントリポイント: `exec(stmt)` と `eval(expr)`。
///
/// - `scopes`: スコープスタック。インデックス 0 がグローバルスコープ、末尾がローカルスコープ
/// - `source_map`: ファイル名 → ソース行リスト のマップ（トレースバックのコンテキスト表示用）
/// - `call_stack`: 関数名のスタック（例外フレーム生成時に参照）
/// - `current_exception`: `except` ブロック内で処理中の例外（裸の `raise` 文で再 raise するため）
/// - `static_cells`: `static mut` 変数の共有セル。キーは (ファイル名, 行, 列)。
/// チャンク実行の入れ子の上限（bug_fix.md B10）。CPython の既定と同じ 1000。
///
/// ⚠⚠ **この値とスタックサイズは対**。`main.rs` の `INTERP_STACK_SIZE` を先に広げずに
/// ここだけ上げると、**上限に当たる前にスタックが満ちて abort** に戻る——
/// この仕組みが防ごうとしているものそのものになる。
///
/// 実測値（単純再帰・ローカル多数・メソッド・相互再帰のどれでもほ㝒一定だった）:
///
/// | スタック | スタックが満ちる深さ |
/// |---|---|
/// | 1MB（main スレッドの既定）| ~134 |
/// | 64MB（現行）| **~9000** |
///
/// ⇒ 1000 なら余裕約 9 倍。上げるときは上の表を**測り直してから**。
pub(crate) const MAX_CALL_DEPTH: u32 = 1000;

/// 展開器の 1 文（配置呼び出し・装飾子・`const` の初期化）で VM が実行してよい命令数（D5）。
///
/// ⚠ 実際のメタ関数で届く数ではない（コードを組み立てるだけの処理は数万命令で終わる）。
///   届いたら終わらないループとみなす。⚠ CLI とエディタで同じ値にすること（片方だけが
///   エラーを出すと、エディタと CLI の診断が食い違う）。
pub(crate) const META_OPS_BUDGET: u64 = 20_000_000;

pub struct Interpreter {
    pub(self) scopes: Vec<ScopeMap>,
    /// スロットキャッシュに昇格したグローバル変数のセルレジストリ（append-only、インデックス安定）。
    /// `Stmt::Assign` / `Stmt::CompoundAssign` の `SlotCache` がここへのインデックスを保持する。
    pub(self) global_slot_cells: Vec<Rc<RefCell<Value>>>,
    /// スロットキャッシュの世代番号。`freeze`（SlotCell → Immutable 降格）時にインクリメントされ、
    /// 全 AST スロットキャッシュを一括無効化する。
    pub(self) slot_epoch: u32,
    /// 実行中のチャンクの入れ子の深さ（bug_fix.md B10 系統）。
    ///
    /// ⚠⚠ これが無いと再帰が**プロセスごと abort** させる
    /// （`thread 'main' has overflowed its stack`・トレースバック無し・catch 不能）。
    ///
    /// ⚠ 増減は [`crate::vm::run`] の**1 箇所だけ**で行う。関数呼び出し側
    /// （`exec_fn_evaled`）ではなくチャンク実行側で数えるのは、
    /// **ジェネレータの本体は `exec_fn_evaled` を通らない**から（実測：
    /// 関数側だけを守ったとき、再帰するジェネレータが落ちたままだった）。
    pub(crate) call_depth: u32,
    /// AST 型解決層の注釈（タスク #16）。型検査（`check_program`）が生成し main.rs が注入する。
    /// メインプログラムの node-id 索引で型・検査指示・CallInfo を引ける。段階(b)/(c) の消費側が参照。
    /// 既定は空（`Interpreter::new` 直後は注釈なし＝挙動不変。注入されるまで消費側はフォールバック）。
    pub(crate) annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    /// 関数ごとのコンパイル済み Chunk キャッシュ。キー = `Rc::as_ptr(fn_val)`。
    /// 値 = `(Weak<FnValue>, Some(chunk)=VM 実行 / None=非対応)`。
    /// テンプレート実体化は呼び出しごとに一時的な `Rc<FnValue>` を作って破棄するため、
    /// 解放されたアドレスが後続の別 fn_val に再利用され得る（キー衝突）。`Weak` を保持し、
    /// ヒット時に `upgrade()` が失敗したら「アドレス再利用＝別関数」と判定して再コンパイルする
    /// （リークなし・古い Chunk の誤用を防ぐ, Phase V-D）。
    pub(self) vm_chunks: HashMap<usize, (std::rc::Weak<FnValue>, Option<Rc<crate::vm::Chunk>>)>,
    /// ジェネレータ関数本体の Chunk キャッシュ（タスク #8）。キー = `Rc::as_ptr(gen_fn)`。
    /// `vm_chunks` と同型だが `GeneratorFnValue` を指すため別テーブル。`Weak` でアドレス再利用を弾く。
    pub(self) vm_gen_chunks:
        HashMap<usize, (std::rc::Weak<GeneratorFnValue>, Option<Rc<crate::vm::Chunk>>)>,
    /// テンプレート関数/ジェネレータ関数の実体化メモ（タスク #7）。
    /// キー = `(Rc::as_ptr(template) as usize, 具体型引数リスト)`。値 = 置換済み具体 `FnValue`。
    /// 同一 `(テンプレート, 型引数)` の再実体化で **AST 置換（`subst_stmts` の clone-walk）を省略**し、
    /// かつ **安定した `Rc<FnValue>` アドレスにより `vm_chunks` の Chunk が再利用**される
    /// （従来は呼び出しごとに一時 fn_val を作って捨てるため毎回再コンパイルしていた, §2.2）。
    /// テンプレートは寿命が長い（グローバル束縛）ので実体化数は有限＝メモリは有界。
    pub(self) template_fn_cache: HashMap<(usize, Vec<String>), Rc<FnValue>>,
    /// テンプレートジェネレータ関数の実体化メモ（タスク #7）。`template_fn_cache` と同様。
    pub(self) template_gen_cache: HashMap<(usize, Vec<String>), Rc<GeneratorFnValue>>,
    /// テンプレート**クラス**の具体化の登録（Phase T・D3）。キーは他の 2 本と同形の
    /// `(テンプレートの Rc アドレス, 型引数)`。
    ///
    /// ⚠ 具体化は展開時に作り、その宣言が定義された時点でここへ入る（`register_mono_instance`・
    ///   D36・タスク 2-16）。同じ `Box[int]` はいつも同じクラスなので、`Value::Class` の等値
    ///   （class_id 比較）も属性アクセスの IC も成り立つ（以前の実行時の具体化〔削除済み〕は、
    ///   これが無いと実体化のたびに新しい class_id を発行していた）。
    pub(self) template_class_cache: HashMap<(usize, Vec<String>), Rc<ClassValue>>,
    /// VM の値スタックバッファ（per-call 確保を避けるため使い回す）。
    /// 実行中は `std::mem::take` で借り出し、復帰時に容量ごと戻す（Phase V）。
    pub(crate) vm_stack: Vec<Value>,
    /// 現在の関数フレームの base スコープの `scopes` 内インデックス（Phase R / R0）。
    ///
    /// 関数に入ると呼び出し前の `scopes.len()` を新しい floor として記録し、base スコープを push する。
    /// 名前引き（get_var/assign_var/…）は `scopes[0]`（グローバル）+ `scopes[frame_floor..]`
    /// （現関数のローカル）のみを走査し、**呼び出し元のローカルは走査しない**（レキシカル隔離）。
    /// これにより「呼び出しごとに外側スコープを drain/退避/復元する」Vec 確保を排除する。
    /// モジュールトップレベルでは 1（グローバルのみ可視）。
    pub(self) frame_floor: usize,
    /// **モジュールごとの大域スコープ**（名前空間の分離・2026-09-26）。添字 0 がメインのプログラム。
    ///
    /// ⚠⚠ **いま使っている大域は `scopes[0]` にある。** ここのその添字には空の置き物が入っていて、
    /// 大域を差し替えるときに入れ替える（[`Self::switch_globals`]）。関数値は定義したモジュールの
    /// 添字を持ち（`FnValue::globals`）、呼び出しの間だけその大域が `scopes[0]` に来る。
    /// ⚠ 以前は `import` のたびにモジュールの名前を呼び出し側の大域へ流し込んでいた（名前空間の侵食）。
    pub(self) global_scopes: Vec<ScopeMap>,
    /// いま `scopes[0]` にある大域の添字（[`Self::global_scopes`] の）。
    pub(self) cur_globals: u32,
    /// 組み込みの `EventLoop`（単一の値）。モジュールの大域にも同じものを置く。
    pub(self) event_loop_value: Value,
    /// 展開器のモジュールの枠（`meta_push_module_frame`）を閉じるときに戻す状態。
    pub(self) meta_module_frames: Vec<(u32, Vec<ScopeMap>, usize)>,
    /// 展開器の前口上（`gensym` など）の名前と値。モジュールの展開の枠の大域にも置く。
    pub(self) meta_prelude: Vec<(String, Value)>,
    /// 読み込んだモジュールの大域の添字（鍵は `module_cache` と同じ・タスク 2-16）。
    ///
    /// ⚠ 読み込み済みのモジュールをもう一度 `import` したとき、その `import` 文の本体に置かれた
    ///   具体化（展開器が置く）をモジュールの大域で定義するのに使う（`exec_module`）。
    pub(self) module_globals_ids: HashMap<(String, PathBuf), u32>,
    /// ファイル名 → ソース行リスト のマップ（トレースバックのコンテキスト抽出用）。
    pub(self) source_map: HashMap<String, Vec<String>>,
    /// 関数名のコールスタック。関数実行前後で push / pop される。
    pub(self) call_stack: Vec<String>,
    /// `call_stack` から pop した `String` バッファの再利用プール（#12）。
    ///
    /// 関数名の push は**呼び出しごとに 1 回のヒープ確保**になっていた（実測 ~43ns/call）。
    /// 名前は毎回同じものが並ぶので、pop したバッファを取っておいて
    /// `clear()` + `push_str()` で詰め直せば定常状態で確保が 0 になる。
    /// `call_stack` 自体の型と `len()` の意味は変えないので、深さを見ている
    /// デバッガ（`debugger.rs`）や例外フレーム生成には影響しない。
    pub(self) call_name_pool: Vec<String>,
    /// `except` ブロック内で処理中の例外（裸の `raise` で再 raise するために保持）。
    pub(self) current_exception: Option<RaisedError>,
    /// 生存している**中断可能な**ジェネレータ（bug_fix.md B13 段階 D）。
    ///
    /// プログラム終了時に残っているものを `close()` して `finally` を走らせるために持つ。
    /// ⚠⚠ **`Drop` からは閉じられない**（`&mut Interpreter` を持てず、エラーも返せず、
    /// VM が借用中に再入する危険がある）ので、明示的な掃除口が要る。
    /// ⚠ `Weak` なので、既に落ちたものは自然に無効になる。伸長のたびに死んだ参照を掃く。
    pub(self) live_generators: Vec<std::rc::Weak<RefCell<GeneratorState>>>,
    /// モジュールキャッシュ: (lang, 解決済みパス) → ロード状態。
    /// 循環 import 検出と重複ロード防止に使用する。
    pub(self) module_cache: HashMap<(String, PathBuf), ModuleState>,
    /// Python モジュール body を実行中かどうかを示すフラグ。
    /// このフラグが `true` のとき定義された `FnValue` は `is_python: true` になる。
    pub(self) in_python_module: bool,
    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリ一覧。
    /// **明示的に登録されたぶんだけ**（ソースのあるディレクトリ・テストの手動登録）。
    /// ⚠ `ar_config.json` 由来のぶんは [`Self::config_search_dirs`] に**遅延で**入る（#69）。
    /// 読むときは必ず [`Self::python_search_dirs()`] を通すこと（両方を順に返す）。
    pub(self) python_search_dirs: Vec<PathBuf>,
    /// `ar_config.json` の祖先ウォークを始める起点（＝ソースのあるディレクトリ）。#69。
    pub(self) config_base_dir: Option<PathBuf>,
    /// `ar_config.json` の `python.search_paths` 由来の検索パス（**初回参照時に遅延して読む**・#69）。
    ///
    /// ⚠⚠ **起動時に読んではいけない。** 消費者は cs-dll / cs-proc / js-proc の
    /// ブリッジ探索と `import[py-int]` **だけ**で、大多数のスクリプトは 1 回も読まない。
    /// それなのに #69 以前は `run_program` が**必ず**祖先を root までウォークしており、
    /// `interp_init` の **48〜53%**（0.19〜0.21ms）を占めていた（支配項は `exists()` の syscall 連打）。
    pub(self) config_search_dirs: std::cell::OnceCell<Vec<PathBuf>>,
    /// `static mut` 変数の永続セル。キーは宣言の (ファイル名, 行, 列)。
    /// 外側関数の全呼び出しで同じセルを共有する。
    pub(self) static_cells: HashMap<(String, usize, usize), Rc<RefCell<Value>>>,
    /// 現在実行中のメソッドが属するクラス（アクセス制御チェック用）。
    /// クラスメソッドの外では `None`。
    pub(self) current_class: Option<Rc<ClassValue>>,
    /// トレイト名 → (フィールド名 → アクセス可能性) のマップ（TraitDef 実行時に収集）。
    /// クラスが継承したトレイトフィールドのアクセス制御に使用する。
    pub(self) trait_field_access: HashMap<String, HashMap<String, Accessibility>>,
    /// トレイト名 → (フィールド名, 可変フラグ, **型注釈**) の宣言順リスト（TraitDef 実行時に収集）。
    /// exec_class_def で field_index を構築する際に trait フィールドの順序を決定する。
    ///
    /// ⚠ 型注釈を持つのは、`build_field_index` が `ClassValue::field_tags`
    /// （slot 順の実行時型判定タグ）を**同じ順序で**組み立てるため。これが無いと
    /// trait 由来のスロットだけ実行時検査が抜ける。
    pub(self) trait_field_order: HashMap<String, Vec<(String, bool, String)>>,
    /// ★**`import[py]` 限定**: Python クラス名 → (フィールド名, 可変フラグ) の**平坦化済み**宣言順リスト。
    ///
    /// Arrow の `class` は**継承できない**（基底に置けるのはトレイトだけ。ネイティブ `.ar` では
    /// パーサが `cannot inherit from ... (only traits are allowed as bases)` で弾く）。
    /// しかし Python ではクラス継承が普通なので、**Python モジュールを読み込んでいる間だけ**
    /// トレイト継承と同じ仕組みでクラス継承を成立させる。
    ///
    /// ⚠ `trait_field_order` と分けてあるのは、Python 側のクラス名がトレイト名（`Error` など）と
    /// 衝突してトレイト継承を壊さないようにするため。`build_field_index` は
    /// **トレイトを先に見て、無ければこちら**を見る。
    /// ⚠ 「平坦化済み」= 自分の基底のフィールドも含む。多段継承（A→B→C）でも 1 段の参照で足りる。
    pub(self) py_class_field_order: HashMap<String, Vec<(String, bool, String)>>,
    /// プロトコル名 → 必須メンバー名リスト（ProtocolDef 実行時に収集）。
    /// `is Protocol` 実行時チェックで使用する。
    pub(self) protocol_required_members: HashMap<String, Vec<String>>,
    /// ロード済みのネイティブ共有ライブラリ。キーは DLL の絶対パス。
    /// ライブラリはインタープリタの生存期間を通じて保持される（アンロードしない）。
    /// **コンパイル時展開の最中か**（設計書 §4.1 / タスク 3-4）。
    ///
    /// ⚠⚠ 展開時の `print` が **stdout に出ると意味論の網が壊れる**。
    /// `compare_python_impl` と `compare_outputs` は stdout を差分比較するゲートなので、
    /// 展開時の出力がプログラムの出力に混ざると**唯一の網が黙って無効になる**。
    /// ⇒ ここが真の間、`print` は stderr へ出す。
    ///
    /// ⚠ 既定は偽。展開器だけが立てる（`meta_expand`）。
    pub(self) meta_expanding: bool,
    /// 展開中に VM が実行してよい残りの命令数（D5・タスク 5-0）。
    ///
    /// ⚠⚠ メタ関数の本体に `while True:` を書くと展開が終わらない。CLI なら止めればよいが、
    ///   エディタは打つたびに展開するので**解析が固まる**。展開中だけ VM を命令数を数える
    ///   ループで回し（`vm::run` の `run_budgeted`）、使い切ったら止める。通常の実行には何も足さない。
    /// ⚠ 展開器が最上位の文ごとに張り直す（[`Self::meta_reset_budget`]）。CLI とエディタで同じ値。
    pub(crate) meta_ops_left: u64,
    /// **展開時に見えている宣言**（名前 → その宣言・タスク 4-0）。
    ///
    /// ⚠⚠ `^対象` はここから引く。展開器が**前から順に**埋めるので、
    /// メタ関数は自分より前の宣言しか見られない（設計書 §1.7）。
    /// ⚠ 展開器以外は空のまま。通常実行で `^` を書くことはできない（1-3 の位置制限）。
    pub(self) meta_decls: std::collections::HashMap<String, crate::interpreter::value::MetaDecl>,
    /// **展開済みの最上位の文**（タスク 4-4）。展開時型推論の前提になる。
    ///
    /// ⚠ 展開器と共有する（`Rc<RefCell<..>>`）。展開器が最上位の文を出すたびに足す。
    /// ⚠ メタ関数を 1 つも持たないプログラムでは**展開器は足さない**（推論が起きえない
    ///   ので、AST を複製する手間をかけない）。
    pub(self) meta_prefix: Rc<RefCell<Vec<crate::ast::Stmt>>>,
    pub(self) native_libs: HashMap<PathBuf, NativeLibWrapper>,
    /// デバッガの状態 2 本（#67 で `debugger::DebugState` へ束ねた）。
    pub(self) dbg: debugger::DebugState,
    /// イベントループの状態 4 本（#67 で `event_loop::EventState` へ束ねた）。
    ///
    /// ⚠ **`Interpreter` の全面分解はしていない**（起票時の判断）。`impl` が 33 ファイルに
    /// 散っているので、凝集が明らかで参照が少ないクラスタだけを畳んである。
    pub(self) events: event_loop::EventState,
    // ── #10-b で追加。既存フィールドのオフセットを動かさないよう末尾に置く。
    /// 最上位ループの Chunk キャッシュ（#10-b）。キー = `Stmt` のアドレス。
    ///
    /// AST は `run_program` / `exec_module` が実行中ずっと保持しているのでアドレスは安定
    /// （`vm_chunks` のような `Weak` による再利用検査は不要）。`None` = コンパイル不能と判明済み。
    ///
    /// ⚠⚠ **不変条件: このキャッシュに載せた `Stmt` は、インタプリタが生きている間
    /// 解放してはいけない。** 解放するとアロケータが同じアドレスを再利用し、
    /// **別の文が前の文の Chunk を引き当てる**（#36 で実際に踏んだ: REPL がブロックごとに
    /// AST を捨てていたため `let xs = …` が `let total = …` の Chunk を実行し
    /// `NameError: variable 'total' is already declared` になった）。
    /// ⇒ 新しい入口を足すときは **AST を保持し続けること**（REPL は `run_repl` が Vec に溜める）。
    pub(self) vm_toplevel_chunks: HashMap<usize, Option<Rc<crate::vm::Chunk>>>,
    /// 最上位から見て `scopes[0]` を確実に指す名前の集合（#10-b, `resolver::toplevel_declared_globals`）。
    /// 最上位ループ Chunk の**書き込み先**判定に使う。空 = 最上位 VM 化を行わない。
    pub(self) toplevel_globals: std::collections::HashMap<String, bool>,
}

/// [`Interpreter::wire_resolution`] のグローバル集合の入れ方（#88）。
///
/// ⚠ **消費側は exhaustive に match する。** 入れ方を足したら全入口が止まる。
pub(crate) enum GlobalsMode {
    /// 集合を**置き換える**（1 プログラム = 1 回の入口）。
    Replace,
    /// 集合へ**積み増す**（REPL。前のブロックで宣言した名前も `scopes[0]` と判断できないと、
    /// 後のブロックの代入が VM に載らない）。
    Extend,
}

impl Interpreter {
    /// インタープリタを初期化する。
    ///
    /// グローバルスコープに以下を事前登録する:
    /// - 組み込み型値: `int`, `str`, `float`, `bool`, `dict`（`Value::Type`）
    /// - 組み込み `Error` trait（`Value::Trait`）
    /// - 標準例外クラス: `Exception`, `ValueError`, `TypeError`, ... 等（`Value::Class`）
    ///
    /// 戻り値: 初期化済みの `Interpreter` インスタンス
    pub fn new() -> Self {
        // EventLoop シングルトンを生成してグローバルスコープに登録する。
        let el_data = Rc::new(RefCell::new(event_loop::EventLoopData::new()));
        // ⚠ 組み込みの並びは `builtin_global_scope` 1 か所（モジュールの大域も同じものから作る）。
        let global = Self::builtin_global_scope(&Value::EventLoop(el_data.clone()));

        Self {
            scopes: vec![global],
            global_slot_cells: Vec::new(),
            slot_epoch: 0,
            call_depth: 0,
            annotations: std::rc::Rc::new(crate::type_check::AstAnnotations::default()),
            vm_chunks: HashMap::new(),
            vm_gen_chunks: HashMap::new(),
            vm_toplevel_chunks: HashMap::new(),
            toplevel_globals: std::collections::HashMap::new(),
            template_fn_cache: HashMap::new(),
            template_gen_cache: HashMap::new(),
            template_class_cache: HashMap::new(),
            vm_stack: Vec::new(),
            frame_floor: 1,
            // ⚠ 添字 0（メイン）は今 `scopes[0]` にあるので、ここは置き物。
            global_scopes: vec![ScopeMap::default()],
            cur_globals: 0,
            event_loop_value: Value::EventLoop(el_data.clone()),
            meta_module_frames: Vec::new(),
            meta_prelude: Vec::new(),
            module_globals_ids: HashMap::new(),
            source_map: HashMap::new(),
            call_stack: Vec::new(),
            call_name_pool: Vec::new(),
            current_exception: None,
            live_generators: Vec::new(),
            module_cache: HashMap::new(),
            in_python_module: false,
            python_search_dirs: Vec::new(),
            config_base_dir: None,
            config_search_dirs: std::cell::OnceCell::new(),
            static_cells: HashMap::new(),
            current_class: None,
            trait_field_access: HashMap::new(),
            py_class_field_order: HashMap::new(),
            trait_field_order: {
                // Error trait のフィールド順序を登録: サブクラス定義時に build_field_index が参照する
                let mut m = HashMap::new();
                m.insert("Error".to_string(), vec![
                    ("message".to_string(), false, "str".to_string()),
                    ("code_context".to_string(), false, "str".to_string()),
                    ("file".to_string(), false, "str".to_string()),
                    ("line".to_string(), false, "int".to_string()),
                    ("col".to_string(), false, "int".to_string()),
                ]);
                m
            },
            protocol_required_members: HashMap::new(),
            meta_expanding: false,
            meta_ops_left: 0,
            meta_decls: std::collections::HashMap::new(),
            meta_prefix: Rc::new(RefCell::new(Vec::new())),
            native_libs: HashMap::new(),
            dbg: debugger::DebugState::default(),
            events: event_loop::EventState::new(el_data),
        }
    }

    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリを登録する。
    pub fn add_python_search_dir(&mut self, dir: PathBuf) {
        self.python_search_dirs.push(dir);
    }

    /// `ar_config.json` の祖先ウォークの起点を覚える（#69）。**この時点では読まない**。
    /// 実際に読むのは [`Self::python_search_dirs()`] が初めて呼ばれたとき。
    pub fn set_config_base_dir(&mut self, dir: PathBuf) {
        self.config_base_dir = Some(dir);
    }

    /// Python / ブリッジ探索に使う検索ディレクトリを**順に**返す（#69）。
    ///
    /// 順序は「**明示登録ぶん → `ar_config.json` 由来**」で、#69 以前に
    /// `run_program` が `add_python_search_dir` を呼んでいた順とまったく同じ。
    ///
    /// ⚠ **初回呼び出しで祖先ウォークが走る**（以降は `OnceCell` が返す）。
    /// ⇒ cs-dll / cs-proc / js-proc / `import[py-int]` を使わないスクリプトは**一度も走らない**。
    pub(crate) fn python_search_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        let cfg = self.config_search_dirs.get_or_init(|| match &self.config_base_dir {
            // ⚠ `ar_config` は `native` 限定（評価コアには `.ar_config` を読む意味が無い・#6）。
            #[cfg(feature = "native")]
            Some(d) => crate::ar_config::load_python_search_paths(d),
            #[cfg(not(feature = "native"))]
            Some(_) => Vec::new(),
            None => Vec::new(),
        });
        self.python_search_dirs.iter().chain(cfg.iter())
    }

    /// 最上位ループの VM 化（#10-b）で「書き込み先はグローバル」と断定してよい名前を注入する。
    /// `resolver::toplevel_declared_globals`（**シャドウ減算なし**）の結果をそのまま渡すこと。
    /// ⚠ 減算版（`toplevel_visible_globals_with`）を渡してはいけない — 別の文の
    /// `for i in ...` のせいで `while i < N` の `i` まで解決できなくなる（#27-c の実測）。
    /// **実行入口が VM へ渡す解決情報をまとめて注入する**（#88）。
    ///
    /// ⚠⚠ 注釈と最上位グローバル集合は**対で要る**。片方だけ渡した入口では
    /// 正しいコードでも `VmForceError` になる（#3/#36。VM は「解決情報が揃っている」前提）。
    /// ⇒ **2 つの setter を別々に呼ばせない**。#88 まで入口ごとの手写しで、
    /// 呼ぶ関数も順序も割れていた。
    ///
    /// ⚠ AST 側（`resolve_program` ＋ 型検査）は
    /// [`resolver::resolve_and_annotate`](crate::interpreter::resolver::resolve_and_annotate) が担う。
    /// **こちらは「揃えた情報をインタプリタへ載せる」だけ**。
    ///
    /// ⚠⚠ **3 つの setter は private にしてある**（#88）。`pub` のままだと入口が
    /// `set_annotations` だけ呼んでグローバル集合を忘れられる ＝ **今まさに割れていた形**。
    /// ⇒ **この関数以外から個別に呼べない**のが強制の全部。
    ///
    /// ⚠ `mode` は**グローバル集合の入れ方**で、ここだけは入口ごとに本当に違う（#88 で実測）:
    /// REPL は [`GlobalsMode::Extend`]（ブロックを跨いで積み増さないと後のブロックの代入が
    /// VM に載らない）、それ以外は [`GlobalsMode::Replace`]。
    pub(crate) fn wire_resolution(
        &mut self,
        annotations: crate::type_check::AstAnnotations,
        globals: std::collections::HashMap<String, bool>,
        mode: GlobalsMode,
    ) {
        self.set_annotations(std::rc::Rc::new(annotations));
        match mode {
            GlobalsMode::Replace => self.set_toplevel_globals(globals),
            GlobalsMode::Extend => self.extend_toplevel_globals(globals),
        }
    }

    fn set_toplevel_globals(&mut self, names: std::collections::HashMap<String, bool>) {
        self.toplevel_globals = names;
    }

    /// 最上位グローバル名の集合を**追加**する（REPL 用・#36）。
    ///
    /// REPL はブロックを 1 つずつ実行するので、前のブロックで宣言した名前を
    /// 後のブロックからも「`scopes[0]` を指す」と判断できるように積み増す必要がある。
    fn extend_toplevel_globals(&mut self, names: std::collections::HashMap<String, bool>) {
        self.toplevel_globals.extend(names);
    }

    /// AST 型解決層の注釈（タスク #16）を注入する。`check_program` が生成したものを main.rs が渡す。
    /// メインプログラムの node-id 索引。段階(b)/(c) の消費側（VM コンパイラ/eval/codegen）が参照する。
    fn set_annotations(&mut self, annotations: std::rc::Rc<crate::type_check::AstAnnotations>) {
        self.annotations = annotations;
    }

    /// CLIパラメータをグローバルスコープの `args` dict として登録する。
    /// スクリプト内では `args["key"]` でアクセスできる。
    pub fn set_cli_args(&mut self, params: HashMap<String, String>) {
        let mut dict = DictData::new("str".to_string(), "str".to_string());
        for (k, v) in params {
            // キーは常に `str` なので `reject_key` を通らない（B1-a）。
            Self::dict_set_pure(&mut dict, Value::str(k), Value::str(v))
                .expect("CLI 引数の dict のキーは常に str");
        }
        self.scopes[0].insert(
            "args".to_string(),
            Var::new(Value::Dict(Rc::new(RefCell::new(dict))), false),
        );
    }

    /// フィールドの型注釈を**クラス定義時に 1 度だけ** `FieldCheck` へ分類する（A-4）。
    ///
    /// `TypeTag` は `int`/`float`/`str`/`bool` しか区別しないので、クラス型・trait 型・
    /// protocol 型のフィールドは `Other` に落ちて**素通り**していた。実行時にはクラス名の
    /// レジストリが無く、代入ごとにスコープを引くのは hot path には重いので、ここで畳む。
    ///
    /// ⚠ **判定できないものは必ず `None`（= 通す）にする。** 型変数（`T`）・`list[T]`・
    /// `Union[...]`・未定義名・**このクラスより後に定義されるクラス**はここに落ちる。
    /// 取りこぼす方へ倒す（`field_tags` と同じ方針）。
    fn classify_field(&self, type_ann: &str) -> crate::interpreter::value::FieldCheck {
        use crate::interpreter::value::FieldCheck;
        // プリミティブは `field_tags` の担当。ここで触らない。
        if !matches!(crate::vm::op::TypeTag::of(type_ann), crate::vm::op::TypeTag::Other) {
            return FieldCheck::None;
        }
        // 型引数つき（`list[int]` / `Union[...]` / `Box[int]`）は対象外。要素型まで見るのは
        // 代入ごとには重く、`Box[int]` は実体化後のクラス名が `Box[int]` とは別なので
        // 名前一致でも判定できない。
        if type_ann.contains('[') {
            return FieldCheck::None;
        }
        if let Some(req) = self.protocol_required_members.get(type_ann) {
            return FieldCheck::Protocol(req.clone().into());
        }
        if self.trait_field_order.contains_key(type_ann) {
            return FieldCheck::Trait(type_ann.into());
        }
        // クラスかどうかはスコープを引いて確かめる。引けなければ（型変数・未定義名・
        // 後から定義されるクラス）`None` に倒す。
        match self.get_val(type_ann) {
            Some(Value::Class(_)) => FieldCheck::Class(type_ann.into()),
            _ => FieldCheck::None,
        }
    }

    /// クラスのフィールド宣言からオフセットインデックスを構築する。
    ///
    /// - `own_fields`: クラス本体で宣言された instance フィールドの (name, is_mutable) リスト（宣言順）
    /// - `bases`: 基底トレイト名リスト
    ///
    /// 戻り値: `(field_index, field_mutability_vec, field_tags, field_checks, field_count)`
    /// - `field_index`: フィールド名 → Vec インデックス（own フィールド名 + trait 修飾名 + unqualified alias）
    /// - `field_mutability_vec`: スロットインデックス → 元の可変フラグ
    /// - `field_tags`: スロットインデックス → **実行時型判定タグ**（`store_field` が使う）
    /// - `field_checks`: スロットインデックス → **`Other` タグの追加判定**（`store_field` が使う）
    /// - `field_count`: スロット総数
    ///
    /// ⚠⚠ `field_tags` / `field_checks` は `field_mutability_vec` と**必ず同じ順序・同じ個数**で
    /// 積むこと。ずれると `store_field` が**別のフィールドの型**で検査する（黙って誤った値を
    /// 通す／正しい値を弾く）。だから 3 つを同じループで push している。
    pub(crate) fn build_field_index(
        &self,
        own_fields: &[(String, bool, String)],
        bases: &[String],
    ) -> (
        HashMap<String, usize>,
        Vec<bool>,
        Vec<crate::vm::op::TypeTag>,
        Vec<crate::interpreter::value::FieldCheck>,
        usize,
    ) {
        use crate::vm::op::TypeTag;
        let mut field_index: HashMap<String, usize> = HashMap::new();
        let mut field_mutability_vec: Vec<bool> = Vec::new();
        let mut field_tags: Vec<TypeTag> = Vec::new();
        let mut field_checks: Vec<crate::interpreter::value::FieldCheck> = Vec::new();
        let mut idx = 0usize;

        // C ABI 準拠レイアウト（.claude/skills/c-abi-interop/SKILL.md P0b）:
        // Step 1: 継承 trait のフィールドを継承順で先頭に配置する。
        // これにより「基底部分が先頭」という C/C++ の継承レイアウト慣行と一致する。
        for base in bases {
            // トレイトを先に見る。無ければ ★`import[py]` 限定のクラス継承（Python クラスの
            // 平坦化済みフィールド順）を見る。トレイト名との衝突でトレイト側を壊さない順序。
            let base_fields = self
                .trait_field_order
                .get(base)
                .or_else(|| self.py_class_field_order.get(base));
            if let Some(trait_fields) = base_fields {
                for (fname, is_mutable, type_ann) in trait_fields {
                    let qualified = format!("{}::{}", base, fname);
                    if let Some(&existing_idx) = field_index.get(fname.as_str()) {
                        // 複数 trait が同名フィールドを持つ場合は同一スロットを共有する
                        field_index.insert(qualified, existing_idx);
                    } else {
                        field_index.insert(qualified, idx);
                        field_index.insert(fname.clone(), idx);
                        field_mutability_vec.push(*is_mutable);
                        field_tags.push(TypeTag::of(type_ann));
                        field_checks.push(self.classify_field(type_ann));
                        idx += 1;
                    }
                }
            }
        }

        // Step 2: own フィールドを宣言順で後続に配置する。
        // trait フィールドを再宣言した場合は既存スロットを共有し、own 宣言の可変性を優先する。
        for (fname, is_mutable, type_ann) in own_fields {
            if let Some(&existing_idx) = field_index.get(fname.as_str()) {
                field_mutability_vec[existing_idx] = *is_mutable;
                field_tags[existing_idx] = TypeTag::of(type_ann);
                field_checks[existing_idx] = self.classify_field(type_ann);
                continue;
            }
            field_index.insert(fname.clone(), idx);
            field_mutability_vec.push(*is_mutable);
            field_tags.push(TypeTag::of(type_ann));
            field_checks.push(self.classify_field(type_ann));
            idx += 1;
        }

        (field_index, field_mutability_vec, field_tags, field_checks, idx)
    }

    /// ソーステキストをファイル名と対応付けて登録する。
    /// トレースバックがコンテキスト行を表示できるようにするために使用する。
    ///
    /// - `filename`: ソースファイルのパス（Span の file フィールドと一致させること）
    /// - `content`: ソースファイル全体のテキスト
    pub fn add_source_text(&mut self, filename: &str, content: &str) {
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        self.source_map.insert(filename.to_string(), lines);
    }

    /// 現在伝播中の例外をインタープリタから取り出す（トップレベルのエラーハンドリング用）。
    ///
    /// 戻り値: `Some(RaisedError)` — 例外あり、`None` — 例外なし
    /// 残っている中断中のジェネレータを閉じる（bug_fix.md B13 段階 D）。
    ///
    /// プログラムが正常終了する直前に 1 回だけ呼ぶ。`finally` を走らせるのが目的。
    /// ⚠ 新しい順に閉じる（Python の後始末順に近い）。
    /// ⚠ 閉じる途中のエラーは**握り潰さず stderr に出して続ける**。ここで止めると
    ///   他のジェネレータの `finally` が走らなくなる（CPython も同じ扱い）。
    pub fn close_all_generators(&mut self) {
        let pending: Vec<_> = std::mem::take(&mut self.live_generators);
        for weak in pending.into_iter().rev() {
            let Some(rc) = weak.upgrade() else { continue };
            // 既に枯渇しているものは `gen_close` が何もしない。
            if let Err(e) = self.gen_close(&rc) {
                eprintln!("Warning: error while closing a generator at exit: {e}");
            }
        }
    }

    /// 展開器専用: このインタプリタが**コンパイル時展開の最中**だと印を付ける（タスク 3-4）。
    ///
    /// ⚠ これを立てると `print` が stderr へ出る。詳しい理由は `meta_expanding` の doc。
    /// このインタプリタが展開の最中か（タスク 3-4）。
    ///
    /// ⚠ 真なら `print` は stderr へ出る。外側から「このインタプリタは出力先を
    /// 切り替えている」と確かめられるようにしてある。
    /// 展開器専用: 展開時に見える宣言を 1 つ登録する（タスク 4-0）。
    ///
    /// ⚠ **前から順に呼ぶこと。** 後ろの宣言を先に入れると、§1.7 の
    /// 「自分より前だけ見られる」が崩れる。
    pub(crate) fn add_meta_decl(&mut self, name: String, decl: crate::interpreter::value::MetaDecl) {
        self.meta_decls.insert(name, decl);
    }

    /// `^名前` を引く（タスク 4-0）。
    ///
    /// ⚠ 見つからないときのエラーは**呼び出し側で作る**（「まだ宣言されていない」のか
    /// 「メタ情報を取れない種類」なのかで文面を変えたいため）。
    pub(crate) fn meta_lookup(&self, name: &str) -> Option<Value> {
        let crate::interpreter::value::MetaDecl::Decl(decl) = self.meta_decls.get(name)? else {
            return None;
        };
        // ⚠ `^x` の `x` は**もう前の文に入っている**（前方のみ参照・§1.7）。
        //   ⇒ 推論の前提に自分の宣言を足さない（足すと再宣言になる）。
        self.make_meta_value(name, Rc::clone(decl), false)
    }

    /// 展開器専用: 展開済みの最上位の文を共有する（タスク 4-4）。
    pub(crate) fn meta_prefix_handle(&self) -> Rc<RefCell<Vec<crate::ast::Stmt>>> {
        Rc::clone(&self.meta_prefix)
    }

    /// 展開器専用: `^` の宣言表を差し替え、元の表を返す（タスク 2-12）。
    ///
    /// ⚠ モジュールは**それぞれ自分の宣言だけ**を見る。展開の枠を移るたびに入れ替える。
    pub(crate) fn meta_swap_decls(
        &mut self,
        decls: std::collections::HashMap<String, crate::interpreter::value::MetaDecl>,
    ) -> std::collections::HashMap<String, crate::interpreter::value::MetaDecl> {
        std::mem::replace(&mut self.meta_decls, decls)
    }

    /// 展開器専用: モジュールの展開の枠を開く（タスク 2-12）。
    ///
    /// ⚠ 実行時の `exec_module` と**同じ形**にする —— モジュールの本体は**そのモジュールの大域で**
    /// 走らせる（`enter_module_frame`・名前空間の分離）。この中で定義したメタ関数は大域の添字を
    /// 持つので、後で別のモジュールから呼ばれても**自分のモジュールの名前**を引く。
    pub(crate) fn meta_push_module_frame(&mut self) {
        let saved = self.enter_module_frame();
        self.meta_module_frames.push(saved);
        // ⚠ 前口上（`gensym` / `compile_error` / 配置メタ関数の入口 …）はどのモジュールからも使う。
        //   値（関数）だけを置く。関数はメインの大域で走るので、旗や `gensym` の数え上げは共有される。
        for (name, value) in self.meta_prelude.clone() {
            self.scopes[0].insert(name, Var::new(value, false));
        }
    }

    /// 展開器専用: 前口上の名前を控える（`meta_push_module_frame` がモジュールの枠にも置く）。
    ///
    /// ⚠ 呼ぶのは前口上を読み込んだ直後（メインの大域が今の大域のとき）。
    pub(crate) fn meta_set_prelude(&mut self, names: &[&str]) {
        self.meta_prelude = names
            .iter()
            .filter_map(|n| self.get_val(n).map(|v| (n.to_string(), v)))
            .collect();
    }

    /// 展開器専用: **メインの大域の**変数に代入する（配置メタ関数の見張りの旗・タスク 3-8）。
    ///
    /// ⚠ 旗を読む前口上の関数はメインの大域で走る。モジュールの展開の枠の中から配置メタ関数を
    ///   呼ぶときも、旗はメインの大域のものを上げ下げしなければならない。
    pub(crate) fn meta_assign_main_global(&mut self, name: &str, value: Value) -> Result<(), String> {
        let prev = self.switch_globals(0);
        let r = self.vm_assign_global(name, value);
        self.switch_globals(prev);
        r
    }

    /// 展開器専用: モジュールの展開の枠を閉じ、その枠で定義した名前と値を返す（タスク 2-12）。
    ///
    /// ⚠ 呼び出し側の大域へは**何も登録しない**（実行時の `exec_module` と同じ・名前空間の分離）。
    ///   以前は「まだ無い名前だけ」を大域へも登録して、後ろで定義した同じモジュールのメタ関数を
    ///   引かせていたが、それは呼び出し側の名前空間を侵す。今はメタ関数が自分の大域で名前を引く。
    /// ⚠ 組み込みと同じ名前を宣言し直したものは名前空間に出ない（展開時の値としては使わない）。
    pub(crate) fn meta_pop_module_frame(&mut self) -> std::collections::HashMap<String, Value> {
        let mut members = self.module_members(&std::collections::HashMap::new());
        for (name, _) in &self.meta_prelude {
            members.remove(name);
        }
        if let Some(saved) = self.meta_module_frames.pop() {
            self.leave_module_frame(saved);
        }
        members
    }

    /// 展開器専用: 今の枠に名前を束縛する（`const` の値・import したモジュール・タスク 2-12）。
    ///
    /// ⚠ 見るのは**今の枠だけ**。外側に同じ名前があっても隠して束縛する —— 別のモジュールに
    /// 同じ名前の `const` があっても、各モジュールのメタ関数は自分のモジュールの値を読む。
    /// ⚠ 同じ枠に既にあれば束縛しない（先に束縛した方が残る。再宣言は型検査が弾く）。
    pub(crate) fn meta_bind(&mut self, name: &str, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            if !scope.contains_key(name) {
                scope.insert(name.to_string(), Var::new(value, false));
            }
        }
    }

    /// メタ情報の値を作る（タスク 4-0 / 4-4）。**作り方はここ 1 か所**。
    ///
    /// `decl_not_in_prefix` は「その宣言がまだ展開済みの文に入っていない」か。
    /// 装飾子の第一引数（これから置き換えられる宣言）は入っていないので真にする。
    ///
    /// ⚠⚠ 注釈の無い変数束縛は**ここで**推論する（`binding_type`）。射影を引く時点まで
    /// 遅らせると、その間に展開が進んで前提の文が変わる。
    pub(crate) fn make_meta_value(
        &self,
        name: &str,
        decl: Rc<crate::ast::Stmt>,
        decl_not_in_prefix: bool,
    ) -> Option<Value> {
        use crate::ast::Stmt;
        let kind = crate::interpreter::value::MetaValue::kind_of(&decl)?;
        let needs_inference = matches!(
            &*decl,
            Stmt::Let(_, None, _) | Stmt::Mut(_, None, _) | Stmt::Const(_, None, _) | Stmt::Static(..)
        );
        let binding_type = if needs_inference {
            let prefix = self.meta_prefix.borrow();
            let ty = if decl_not_in_prefix {
                let mut with_decl = prefix.clone();
                with_decl.push((*decl).clone());
                crate::type_check::TypeChecker::binding_type_after(&with_decl, name)
            } else {
                crate::type_check::TypeChecker::binding_type_after(&prefix, name)
            };
            ty.map(|t| t.to_string())
        } else {
            None
        };
        Some(Value::Meta(Rc::new(crate::interpreter::value::MetaValue {
            kind,
            name: name.to_string(),
            decl,
            binding_type,
        })))
    }

    /// `^名前` が引けなかったときの文面（タスク 4-0）。
    ///
    /// ⚠ 「まだ宣言されていない」と「メタ情報を取れない種類」を**言い分ける**。
    /// 同じ文面にすると、前方参照の間違いと対象違いの間違いが区別できない。
    pub(crate) fn meta_lookup_error(&self, name: &str) -> String {
        // ⚠⚠ **実行時に評価されたら「宣言されていない」ではない**（タスク 3-9）。
        //   `^` はパース時に「メタ関数の中か、呼び出しの実引数の中」まで絞ってある（1-3）が、
        //   呼び先がメタ関数かはパース時に分からないので、`print(^T)` は通る。展開器は
        //   それを評価しないので実行時にここへ来る —— 以前は宣言表が空なので
        //   「前に何も宣言されていない」と**誤報した**（実測）。
        if !self.meta_expanding {
            return format!(
                "MetaError: `^{name}` can only be evaluated while the compile-time expander runs \
                 — inside a metafunction, or as an argument to a metafunction call (here it is \
                 part of ordinary code, which runs at run time)"
            );
        }
        match self.meta_decls.get(name) {
            Some(crate::interpreter::value::MetaDecl::Opaque(what)) => format!(
                "MetaError: `^{name}` is not available — '{name}' is {what}, and meta information \
                 exists only for classes, traits, enums, functions, fields and variable bindings"
            ),
            // ⚠ 引けたのに値が作れなかった（種類の判定と値の作り方がずれた）。ここへは来ないはず。
            Some(crate::interpreter::value::MetaDecl::Decl(_)) => format!(
                "MetaError: internal — `^{name}` names a declaration whose meta information \
                 could not be built"
            ),
            None => format!(
                "MetaError: `^{name}` refers to nothing declared before this point \
                 (expansion only sees declarations above it)"
            ),
        }
    }

    /// 展開時に禁じるビルトインか（設計書 D26 / 参考N・タスク 3-7）。禁じるなら文面を返す。
    ///
    /// ⚠⚠ **判定基準は「決定的か」**（D26）。非決定的なもの（`id` はアドレス・`getenv` は環境）と
    /// I/O は禁じる。`--compile` の `.arc` キャッシュが不健全になるうえ、エディタが展開を
    /// 走らせるようになると（D5）**打鍵のたびにファイルを触る**ことになる。
    /// ⚠ 一覧は参考N の禁止表と**過不足なく一致**させてある（名前で振り分けられている
    /// ビルトインを全部洗って確かめた）。ビルトインを足したら、どちらかへ分類すること。
    pub(crate) fn meta_forbidden_builtin(&self, name: &str) -> Option<String> {
        const FORBIDDEN: &[&str] = &[
            "id",
            "getenv",
            "open",
            "close",
            "parse_ar",
            "create_flat_int_list",
            "flat_get_int",
            "flat_set_int",
        ];
        if !self.meta_expanding || !FORBIDDEN.contains(&name) {
            return None;
        }
        Some(format!(
            "`{name}` cannot be called while expanding — metafunctions may only use \
             deterministic, side-effect-free builtins (no I/O, no addresses, no environment)"
        ))
    }

    /// 展開時に禁じる受け手か（D26 / 参考N のメソッド表・タスク 3-7）。禁じるなら文面を返す。
    ///
    /// ⚠ ファイル・非同期・イベントループ・シグナルは**受け手の型で**止める。メソッド名で
    /// 並べると、後から足したメソッドが素通りする。`EventLoop` は大域の単一値なので、
    /// 生成を止めるだけでは足りない。
    pub(crate) fn meta_forbidden_receiver(&self, obj: &Value) -> Option<String> {
        if !self.meta_expanding {
            return None;
        }
        let what = match obj {
            Value::FileObject(_) => "files",
            Value::AsyncManager(_) => "async tasks",
            Value::EventLoop(_) => "the event loop",
            Value::Signal(_) => "signals",
            _ => return None,
        };
        Some(format!(
            "{what} cannot be used while expanding — metafunctions may only use \
             deterministic, side-effect-free operations"
        ))
    }

    pub(crate) fn is_meta_expanding(&self) -> bool {
        self.meta_expanding
    }

    pub(crate) fn set_meta_expanding(&mut self, on: bool) {
        self.meta_expanding = on;
    }

    /// 展開中の命令数の上限を張り直す（`meta_ops_left` の doc）。
    pub(crate) fn meta_reset_budget(&mut self) {
        self.meta_ops_left = META_OPS_BUDGET;
    }

    pub fn take_current_exception(&mut self) -> Option<RaisedError> {
        self.current_exception.take()
    }

    /// `RaisedError` を人間が読めるトレースバック文字列にフォーマットする。
    ///
    /// - `raised`: フォーマットする例外情報
    ///
    /// 戻り値: `"Traceback (most recent call last):\n  ..."` 形式のエラーレポート文字列
    pub fn format_error_report(raised: &RaisedError) -> String {
        let mut out = String::from("Traceback (most recent call last):\n");

        // frames[0] is innermost (raise site); display outermost first.
        for frame in raised.frames.iter().rev() {
            if frame.line == 0 {
                out.push_str(&format!(
                    "  File \"{}\", in {}\n",
                    frame.file, frame.fn_name
                ));
            } else {
                out.push_str(&format!(
                    "  File \"{}\", line {}, col {}, in {}\n",
                    frame.file, frame.line, frame.col, frame.fn_name
                ));
            }
            if !frame.context.is_empty() {
                for line in frame.context.lines() {
                    out.push_str(&format!("    {}\n", line));
                }
            }
        }

        // Exception class name and message.
        match &raised.exception {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let class_name = inst.class.name.clone();
                let message = inst.class.field_index.get("message").and_then(|&idx| {
                    inst.field_value(idx).map(|v| match v {
                        Value::Str(s) => s.to_string(),
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => "<value>".to_string(),
                    })
                }).unwrap_or_default();
                out.push_str(&format!("{}: {}", class_name, message));
            }
            Value::Str(s) => out.push_str(s),
            other => out.push_str(&format!("<exception: {:?}>", other)),
        }

        out
    }

    /// Execute one statement in REPL mode.
    ///
    /// When `is_last` is true and the statement is a bare expression (`Stmt::Expr`),
    /// the value is evaluated and returned as a display string (skipping `None`).
    /// All other statements are executed normally via `exec`.
    /// Errors are returned as formatted strings instead of terminating the process.
    pub fn exec_repl_stmt(
        &mut self,
        stmt: &crate::ast::Stmt,
        is_last: bool,
    ) -> Result<Option<String>, String> {
        if is_last {
            if let crate::ast::Stmt::Expr(expr) = stmt {
                return match self.eval(expr) {
                    Ok(val) => Ok(if matches!(val, Value::None) {
                        None
                    } else {
                        Some(self.display(&val))
                    }),
                    Err(e) if e == RAISE_SENTINEL => Err(self
                        .take_current_exception()
                        .map(|r| Self::format_error_report(&r))
                        .unwrap_or_else(|| "UnhandledException".to_string())),
                    Err(e) => Err(e),
                };
            }
        }
        match self.exec(stmt) {
            Ok(ExecResult::Raise(raised)) => Err(Self::format_error_report(&raised)),
            Ok(_) => Ok(None),
            Err(e) if e == RAISE_SENTINEL => Err(self
                .take_current_exception()
                .map(|r| Self::format_error_report(&r))
                .unwrap_or_else(|| "UnhandledException".to_string())),
            Err(e) => Err(e),
        }
    }
}
