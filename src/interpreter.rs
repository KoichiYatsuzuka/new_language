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
pub(crate) mod cpp_bridge;
#[path = "interpreter/proc_bridge.rs"]
pub(crate) mod proc_bridge;
#[path = "interpreter/cs_dll_runtime.rs"]
pub(crate) mod cs_dll_runtime;
#[path = "interpreter/cs_proc_runtime.rs"]
pub(crate) mod cs_proc_runtime;
#[path = "interpreter/js_proc_runtime.rs"]
pub(crate) mod js_proc_runtime;

/// FFI 境界検査（#16）: 動的型付け言語から Arrow へ入る値をスタブ宣言型と突き合わせる。
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
pub(crate) use exec::{collect_declared_names, collect_referenced_names};
#[path = "interpreter/functions/mod.rs"]
mod functions;
#[path = "interpreter/msvc_errors.rs"]
 mod msvc_errors;
#[path = "interpreter/native_api/mod.rs"]
 mod native_api;
#[path = "interpreter/ops/mod.rs"]
mod ops;
#[path = "interpreter/py_interop.rs"]
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
 mod ast_value;
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
 const RAISE_SENTINEL: &str = "\x00__raise__";

/// Sentinel error string used to propagate a `break` signal through `eval()` return channels.
/// Produced when `break` is executed inside a control-flow expression body (e.g., an `if` or
/// `block:` expression) and needs to bubble up to the enclosing `for`/`while` loop.
 const BREAK_SENTINEL: &str = "\x00__break__";

/// `continue` 版の同型センチネル（#34）。
///
/// ⚠ **以前は存在せず、これがツリーウォークのバグだった**。`eval_block_expr` は
/// `continue` を SyntaxError にし、`eval_capture_block_return` は**黙って握り潰して
/// `None` を返して**いた（`let v = 1 + if c ->int: continue` が TypeError になった）。
/// VM は `break` と同じジャンプで正しく扱い、参照実装（`impl_python`）も継続する。
/// 基準を参照実装に合わせ、`break` と同じ経路で外側ループへ届けるようにした。
 const CONTINUE_SENTINEL: &str = "\x00__continue__";

thread_local! {
    /// ジェネレータ本体の一括評価中に `yield` された値を収集するスレッドローカル変数。
    /// `None` の場合はジェネレータ実行コンテキスト外であることを意味する。
    /// `exec_generator` が開始時に `Some(Vec::new())` をセットし、終了時に `take()` で回収する。
    pub(self) static GENERATOR_YIELDS: RefCell<Option<Vec<Value>>> = const { RefCell::new(None) };

    /// `for`/`while` 式の評価中に `loop_yield` された値を収集するスレッドローカル変数。
    /// `None` の場合は for/while 式の外であることを意味する（loop_yield はここで実行時エラー）。
    /// ネストした for/while 式を正しく扱うため、外側の式の値を退避して評価後に復元する。
    pub(self) static BLOCK_YIELDS: RefCell<Option<Vec<Value>>> = const { RefCell::new(None) };

    /// 現在の for/while ループ（文・式両形式）のネスト深さ。
    /// `break` はこれが 0 のときに実行時エラーを返す。
    pub(self) static LOOP_DEPTH: RefCell<usize> = const { RefCell::new(0) };

    /// block_return / loop_yield のランタイム型チェック用。
    /// block:/if/for/while/match 式へ入るときに期待型アノテーション文字列を push し、
    /// 抜けるときに pop する。None は型注釈なし（任意の型を受け入れる）を意味する。
    pub(self) static BLOCK_RETURN_EXPECTED_TYPE: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
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
pub struct Interpreter {
    pub(self) scopes: Vec<ScopeMap>,
    /// スロットキャッシュに昇格したグローバル変数のセルレジストリ（append-only、インデックス安定）。
    /// `Stmt::Assign` / `Stmt::CompoundAssign` の `SlotCache` がここへのインデックスを保持する。
    pub(self) global_slot_cells: Vec<Rc<RefCell<Value>>>,
    /// スロットキャッシュの世代番号。`freeze`（SlotCell → Immutable 降格）時にインクリメントされ、
    /// 全 AST スロットキャッシュを一括無効化する。
    pub(self) slot_epoch: u32,
    /// バイトコード VM の実行モード（Off/Auto/Force）。CLI `--vm` で設定。既定 Auto（Phase V）。
    pub(crate) vm_mode: crate::vm::VmMode,
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
    /// モジュールキャッシュ: (lang, 解決済みパス) → ロード状態。
    /// 循環 import 検出と重複ロード防止に使用する。
    pub(self) module_cache: HashMap<(String, PathBuf), ModuleState>,
    /// Python モジュール body を実行中かどうかを示すフラグ。
    /// このフラグが `true` のとき定義された `FnValue` は `is_python: true` になる。
    pub(self) in_python_module: bool,
    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリ一覧。
    pub(self) python_search_dirs: Vec<PathBuf>,
    /// `static mut` 変数の永続セル。キーは宣言の (ファイル名, 行, 列)。
    /// 外側関数の全呼び出しで同じセルを共有する。
    pub(self) static_cells: HashMap<(String, usize, usize), Rc<RefCell<Value>>>,
    /// 現在実行中のメソッドが属するクラス（アクセス制御チェック用）。
    /// クラスメソッドの外では `None`。
    pub(self) current_class: Option<Rc<ClassValue>>,
    /// トレイト名 → (フィールド名 → アクセス可能性) のマップ（TraitDef 実行時に収集）。
    /// クラスが継承したトレイトフィールドのアクセス制御に使用する。
    pub(self) trait_field_access: HashMap<String, HashMap<String, Accessibility>>,
    /// トレイト名 → (フィールド名, 可変フラグ) の宣言順リスト（TraitDef 実行時に収集）。
    /// exec_class_def で field_index を構築する際に trait フィールドの順序を決定する。
    pub(self) trait_field_order: HashMap<String, Vec<(String, bool)>>,
    /// プロトコル名 → 必須メンバー名リスト（ProtocolDef 実行時に収集）。
    /// `is Protocol` 実行時チェックで使用する。
    pub(self) protocol_required_members: HashMap<String, Vec<String>>,
    /// ロード済みのネイティブ共有ライブラリ。キーは DLL の絶対パス。
    /// ライブラリはインタープリタの生存期間を通じて保持される（アンロードしない）。
    pub(self) native_libs: HashMap<PathBuf, NativeLibWrapper>,
    /// デバッガ REPL 内で `let dbg::name = expr` として宣言された一時変数。
    /// `q`（再開）または `break_point` のスコープ終了時にクリアされる。
    pub(self) dbg_vars: HashMap<String, Var>,
    /// Last span successfully extracted from a statement — used as fallback
    /// when the current statement has no extractable location (e.g. `Stmt::Mut`
    /// wrapping a bare `Expr::Call(Expr::Ident(...))`).
    pub(self) dbg_last_span: Option<crate::token::Span>,
    /// Arrow ネイティブの EventLoop シングルトン状態。
    /// `EventLoop.run()` が処理する非同期イベントキューと post コールバックキューを保持する。
    pub(self) event_loop_data: Rc<RefCell<event_loop::EventLoopData>>,
    /// C#/Go ブリッジが `ar_event_fire()` で書き込むスレッドセーフキュー。
    pub(self) external_event_queue: event_loop::ExternalEventQueue,
    /// 外部イベント handler_id → SignalData の逆引きマップ（C#/Go 連携時に使用）。
    pub(self) external_handler_registry: HashMap<u64, Rc<RefCell<event_loop::SignalData>>>,
    /// `sig.external_id` の発番カウンタ（プロセス内の Interpreter 単位で単調増加、1 始まり）。
    pub(self) next_external_signal_id: u64,
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
    /// 最上位から見て `scopes[0]` を確実に指す名前の集合（#10-b, `resolver::toplevel_visible_globals`）。
    /// 最上位ループ Chunk の**書き込み先**判定に使う。空 = 最上位 VM 化を行わない。
    pub(self) toplevel_globals: std::collections::HashSet<String>,
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
        let mut global: ScopeMap = ScopeMap::default();
        built_in_types::register_builtin_globals(&mut global);

        // Signal: テンプレート型コンストラクタ。Signal[T]() で Value::Signal を生成する。
        global.insert(
            "Signal".to_string(),
            Var::new(Value::Type("Signal".to_string()), false),
        );

        // EventLoop シングルトンを生成してグローバルスコープに登録する。
        let el_data = Rc::new(RefCell::new(event_loop::EventLoopData::new()));
        global.insert(
            "EventLoop".to_string(),
            Var::new(Value::EventLoop(el_data.clone()), false),
        );

        // グローバル外部イベントキューを共有する（ar_event_fire が書き込む先と同一）。
        let ext_q = event_loop::global_ext_queue();

        Self {
            scopes: vec![global],
            global_slot_cells: Vec::new(),
            slot_epoch: 0,
            vm_mode: crate::vm::VmMode::default(),
            annotations: std::rc::Rc::new(crate::type_check::AstAnnotations::default()),
            vm_chunks: HashMap::new(),
            vm_gen_chunks: HashMap::new(),
            vm_toplevel_chunks: HashMap::new(),
            toplevel_globals: std::collections::HashSet::new(),
            template_fn_cache: HashMap::new(),
            template_gen_cache: HashMap::new(),
            vm_stack: Vec::new(),
            frame_floor: 1,
            source_map: HashMap::new(),
            call_stack: Vec::new(),
            call_name_pool: Vec::new(),
            current_exception: None,
            module_cache: HashMap::new(),
            in_python_module: false,
            python_search_dirs: Vec::new(),
            static_cells: HashMap::new(),
            current_class: None,
            trait_field_access: HashMap::new(),
            trait_field_order: {
                // Error trait のフィールド順序を登録: サブクラス定義時に build_field_index が参照する
                let mut m = HashMap::new();
                m.insert("Error".to_string(), vec![
                    ("message".to_string(), false),
                    ("code_context".to_string(), false),
                    ("file".to_string(), false),
                    ("line".to_string(), false),
                    ("col".to_string(), false),
                ]);
                m
            },
            protocol_required_members: HashMap::new(),
            native_libs: HashMap::new(),
            dbg_vars: HashMap::new(),
            dbg_last_span: None,
            event_loop_data: el_data,
            external_event_queue: ext_q,
            external_handler_registry: HashMap::new(),
            next_external_signal_id: 1,
        }
    }

    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリを登録する。
    pub fn add_python_search_dir(&mut self, dir: PathBuf) {
        self.python_search_dirs.push(dir);
    }

    /// バイトコード VM の実行モードを設定する（CLI `--vm` から）。
    pub fn set_vm_mode(&mut self, mode: crate::vm::VmMode) {
        self.vm_mode = mode;
    }

    /// 最上位ループの VM 化（#10-b）で「書き込み先はグローバル」と断定してよい名前を注入する。
    /// `resolver::toplevel_visible_globals` の結果をそのまま渡すこと（判定を複製しない）。
    pub fn set_toplevel_globals(&mut self, names: std::collections::HashSet<String>) {
        self.toplevel_globals = names;
    }

    /// 最上位グローバル名の集合を**追加**する（REPL 用・#36）。
    ///
    /// REPL はブロックを 1 つずつ実行するので、前のブロックで宣言した名前を
    /// 後のブロックからも「`scopes[0]` を指す」と判断できるように積み増す必要がある。
    pub fn extend_toplevel_globals(&mut self, names: std::collections::HashSet<String>) {
        self.toplevel_globals.extend(names);
    }

    /// AST 型解決層の注釈（タスク #16）を注入する。`check_program` が生成したものを main.rs が渡す。
    /// メインプログラムの node-id 索引。段階(b)/(c) の消費側（VM コンパイラ/eval/codegen）が参照する。
    pub fn set_annotations(&mut self, annotations: std::rc::Rc<crate::type_check::AstAnnotations>) {
        self.annotations = annotations;
    }

    /// CLIパラメータをグローバルスコープの `args` dict として登録する。
    /// スクリプト内では `args["key"]` でアクセスできる。
    pub fn set_cli_args(&mut self, params: HashMap<String, String>) {
        let mut dict = DictData::new("str".to_string(), "str".to_string());
        for (k, v) in params {
            dict.set(Value::str(k), Value::str(v));
        }
        self.scopes[0].insert(
            "args".to_string(),
            Var::new(Value::Dict(Rc::new(RefCell::new(dict))), false),
        );
    }

    /// クラスのフィールド宣言からオフセットインデックスを構築する。
    ///
    /// - `own_fields`: クラス本体で宣言された instance フィールドの (name, is_mutable) リスト（宣言順）
    /// - `bases`: 基底トレイト名リスト
    ///
    /// 戻り値: `(field_index, field_mutability_vec, field_count)`
    /// - `field_index`: フィールド名 → Vec インデックス（own フィールド名 + trait 修飾名 + unqualified alias）
    /// - `field_mutability_vec`: スロットインデックス → 元の可変フラグ
    /// - `field_count`: スロット総数
    pub(crate) fn build_field_index(
        &self,
        own_fields: &[(String, bool)],
        bases: &[String],
    ) -> (HashMap<String, usize>, Vec<bool>, usize) {
        let mut field_index: HashMap<String, usize> = HashMap::new();
        let mut field_mutability_vec: Vec<bool> = Vec::new();
        let mut idx = 0usize;

        // C ABI 準拠レイアウト（.claude/skills/c-abi-interop/SKILL.md P0b）:
        // Step 1: 継承 trait のフィールドを継承順で先頭に配置する。
        // これにより「基底部分が先頭」という C/C++ の継承レイアウト慣行と一致する。
        for base in bases {
            if let Some(trait_fields) = self.trait_field_order.get(base) {
                for (fname, is_mutable) in trait_fields {
                    let qualified = format!("{}::{}", base, fname);
                    if let Some(&existing_idx) = field_index.get(fname.as_str()) {
                        // 複数 trait が同名フィールドを持つ場合は同一スロットを共有する
                        field_index.insert(qualified, existing_idx);
                    } else {
                        field_index.insert(qualified, idx);
                        field_index.insert(fname.clone(), idx);
                        field_mutability_vec.push(*is_mutable);
                        idx += 1;
                    }
                }
            }
        }

        // Step 2: own フィールドを宣言順で後続に配置する。
        // trait フィールドを再宣言した場合は既存スロットを共有し、own 宣言の可変性を優先する。
        for (fname, is_mutable) in own_fields {
            if let Some(&existing_idx) = field_index.get(fname.as_str()) {
                field_mutability_vec[existing_idx] = *is_mutable;
                continue;
            }
            field_index.insert(fname.clone(), idx);
            field_mutability_vec.push(*is_mutable);
            idx += 1;
        }

        (field_index, field_mutability_vec, idx)
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
