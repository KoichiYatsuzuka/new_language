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
#[path = "interpreter/classes.rs"]
mod classes;
#[path = "interpreter/cpp_bridge/mod.rs"]
pub(crate) mod cpp_bridge;
#[path = "interpreter/cs_dll_runtime.rs"]
pub(crate) mod cs_dll_runtime;
#[path = "interpreter/cs_proc_runtime.rs"]
pub(crate) mod cs_proc_runtime;
#[path = "interpreter/js_proc_runtime.rs"]
pub(crate) mod js_proc_runtime;
#[path = "interpreter/debugger.rs"]
pub(self) mod debugger;
#[path = "interpreter/eval.rs"]
mod eval;
#[path = "interpreter/exceptions.rs"]
mod exceptions;
#[path = "interpreter/exec.rs"]
mod exec;
#[path = "interpreter/functions.rs"]
mod functions;
#[path = "interpreter/msvc_errors.rs"]
pub(self) mod msvc_errors;
#[path = "interpreter/native_api.rs"]
pub(self) mod native_api;
#[path = "interpreter/ops.rs"]
mod ops;
#[path = "interpreter/py_interop.rs"]
pub(self) mod py_interop;
#[path = "interpreter/scope.rs"]
mod scope;
#[path = "interpreter/str_methods.rs"]
pub(super) mod str_methods;
#[path = "interpreter/templates.rs"]
mod templates;

#[cfg(test)]
#[path = "interpreter/tests.rs"]
mod tests;

#[path = "interpreter/ast_value.rs"]
pub(self) mod ast_value;
#[path = "interpreter/built_in_types.rs"]
mod built_in_types;

#[path = "interpreter/value.rs"]
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
pub(self) const RAISE_SENTINEL: &str = "\x00__raise__";

/// Sentinel error string used to propagate a `break` signal through `eval()` return channels.
/// Produced when `break` is executed inside a control-flow expression body (e.g., an `if` or
/// `block:` expression) and needs to bubble up to the enclosing `for`/`while` loop.
pub(self) const BREAK_SENTINEL: &str = "\x00__break__";

thread_local! {
    /// ジェネレータ本体の一括評価中に `yield` された値を収集するスレッドローカル変数。
    /// `None` の場合はジェネレータ実行コンテキスト外であることを意味する。
    /// `exec_generator` が開始時に `Some(Vec::new())` をセットし、終了時に `take()` で回収する。
    pub(self) static GENERATOR_YIELDS: RefCell<Option<Vec<Value>>> = RefCell::new(None);

    /// `for`/`while` 式の評価中に `loop_yield` された値を収集するスレッドローカル変数。
    /// `None` の場合は for/while 式の外であることを意味する（loop_yield はここで実行時エラー）。
    /// ネストした for/while 式を正しく扱うため、外側の式の値を退避して評価後に復元する。
    pub(self) static BLOCK_YIELDS: RefCell<Option<Vec<Value>>> = RefCell::new(None);

    /// 現在の for/while ループ（文・式両形式）のネスト深さ。
    /// `break` はこれが 0 のときに実行時エラーを返す。
    pub(self) static LOOP_DEPTH: RefCell<usize> = RefCell::new(0);

    /// block_return / loop_yield のランタイム型チェック用。
    /// block:/if/for/while/match 式へ入るときに期待型アノテーション文字列を push し、
    /// 抜けるときに pop する。None は型注釈なし（任意の型を受け入れる）を意味する。
    pub(self) static BLOCK_RETURN_EXPECTED_TYPE: RefCell<Vec<Option<String>>> = RefCell::new(Vec::new());
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
pub(self) struct FxHasher {
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
pub(self) struct FxBuildHasher;

impl std::hash::BuildHasher for FxBuildHasher {
    type Hasher = FxHasher;
    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher::default()
    }
}

/// スコープ1段分の変数マップ（FxHash キー）。
pub(self) type ScopeMap = HashMap<String, Var, FxBuildHasher>;

/// スコープ内の1つの変数エントリ。
///
/// - `Immutable(Value)`: 不変変数（`let` / `const`）
/// - `Mutable(Value)`: 可変変数（`mut`）。クロージャにキャプチャされるまでは値を直接保持する。
/// - `Cell(Rc<RefCell<Value>>)`: クロージャにキャプチャされた可変変数。外側スコープと共有セルを通じて読み書きする。
/// - `SlotCell(Rc<RefCell<Value>>)`: スロットキャッシュ（AST 焼き込み）に昇格したグローバル可変変数。
///   `freeze` されると `Immutable` に戻り `slot_epoch` が進む（キャッシュ一括無効化）。
pub(self) enum Var {
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
    /// ファイル名 → ソース行リスト のマップ（トレースバックのコンテキスト抽出用）。
    pub(self) source_map: HashMap<String, Vec<String>>,
    /// 関数名のコールスタック。関数実行前後で push / pop される。
    pub(self) call_stack: Vec<String>,
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
    /// Keeps inkwell JIT modules alive for the interpreter's lifetime.
    /// Each entry owns the `ExecutionEngine` whose function pointers are in use.
    #[allow(dead_code)]
    pub(self) jit_handles: Vec<Box<dyn std::any::Any>>,
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

        // 外部イベントキューを生成してグローバルキューにも登録する（ar_event_fire から利用）。
        let ext_q = event_loop::new_external_queue();
        event_loop::set_global_ext_queue(ext_q.clone());

        Self {
            scopes: vec![global],
            global_slot_cells: Vec::new(),
            slot_epoch: 0,
            source_map: HashMap::new(),
            call_stack: Vec::new(),
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
            jit_handles: Vec::new(),
            dbg_vars: HashMap::new(),
            dbg_last_span: None,
            event_loop_data: el_data,
            external_event_queue: ext_q,
            external_handler_registry: HashMap::new(),
        }
    }

    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリを登録する。
    pub fn add_python_search_dir(&mut self, dir: PathBuf) {
        self.python_search_dirs.push(dir);
    }

    /// CLIパラメータをグローバルスコープの `args` dict として登録する。
    /// スクリプト内では `args["key"]` でアクセスできる。
    pub fn set_cli_args(&mut self, params: HashMap<String, String>) {
        let mut dict = DictData::new("str".to_string(), "str".to_string());
        for (k, v) in params {
            dict.set(Value::Str(k), Value::Str(v));
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

        // C ABI 準拠レイアウト（for_claude/c_abi_interop.md P0b）:
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
                        Value::Str(s) => s,
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
