// interpreter.rs — 型定義・定数・Interpreter構造体・new()/初期化
//
// サブモジュール担当:
//   interpreter/scope.rs     — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)
//   interpreter/ops.rs       — 演算・比較・真偽値・表示 (is_truthy / type_name / display / apply_binop など)
//   interpreter/exec.rs      — 文の実行 (exec / exec_block / exec_scoped_block)
//   interpreter/eval.rs      — 式の評価・attr_assign (eval / attr_assign)
//   interpreter/functions.rs — 関数・ジェネレータ・オーバーロード実行
//   interpreter/classes.rs   — クラス・インスタンス管理
//   interpreter/exceptions.rs — 例外クラス構築・トレースバック
//   interpreter/templates.rs — テンプレート展開・AST置換
//
// 実行フロー:
//   Interpreter::new() でグローバルスコープと組み込み型・例外クラスを初期化し、
//   exec(stmt) / eval(expr) を通じてツリーウォーク実行を行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::{Param, Stmt};

#[path = "interpreter/scope.rs"]
mod scope;
#[path = "interpreter/ops.rs"]
mod ops;
#[path = "interpreter/exec.rs"]
mod exec;
#[path = "interpreter/eval.rs"]
mod eval;
#[path = "interpreter/functions.rs"]
mod functions;
#[path = "interpreter/classes.rs"]
mod classes;
#[path = "interpreter/exceptions.rs"]
mod exceptions;
#[path = "interpreter/templates.rs"]
mod templates;
#[path = "interpreter/py_interop.rs"]
pub(self) mod py_interop;

#[cfg(test)]
#[path = "interpreter/tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// Sentinel / thread-local (private to this module tree)
// ---------------------------------------------------------------------------

/// 例外が伝播中であることを示すセンチネル文字列。
/// `Err(RAISE_SENTINEL)` として返されたとき、呼び出し元は言語レベルの `raise` と
/// インタープリタ内部のバグエラーを区別するためにこの値を検査する。
pub(self) const RAISE_SENTINEL: &str = "\x00__raise__";

thread_local! {
    /// ジェネレータ本体の一括評価中に `yield` された値を収集するスレッドローカル変数。
    /// `None` の場合はジェネレータ実行コンテキスト外であることを意味する。
    /// `exec_generator` が開始時に `Some(Vec::new())` をセットし、終了時に `take()` で回収する。
    pub(self) static GENERATOR_YIELDS: RefCell<Option<Vec<Value>>> = RefCell::new(None);
}

// ---------------------------------------------------------------------------
// Exception / traceback types
// ---------------------------------------------------------------------------

/// エラーのトレースバックにおける1つのスタックフレーム（コールスタックの1階層）。
///
/// - `file`: ソースファイル名
/// - `line`: 行番号（1始まり）
/// - `col`: 列番号（1始まり）
/// - `fn_name`: raise または伝播が発生した関数名（`<module>` はトップレベル）
/// - `context`: `line` を中心とした最大5行のソースコンテキスト文字列。取得不可能な場合は空文字列
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub file: String,
    pub line: usize,
    pub col: usize,
    /// raise（または伝播）が発生した関数名。トップレベルは `<module>`。
    pub fn_name: String,
    /// `line` を中心とした最大5行のソースコンテキスト。取得不可能な場合は空文字列。
    pub context: String,
}

/// コールスタックを遡って伝播中の言語レベル例外。
///
/// - `exception`: 例外インスタンス。ユーザー raise では常に `Value::Instance`
/// - `frames`: 例外が伝播するにつれて収集されたスタックフレームのリスト。
///   インデックス 0 が raise 発生地点（最内部）、末尾が `<module>` 到達直前の最外部フレーム。
#[derive(Debug, Clone)]
pub struct RaisedError {
    /// 例外インスタンス（ユーザー raise では常に `Value::Instance`）。
    pub exception: Value,
    /// 例外伝播中に収集されたフレーム: インデックス 0 = raise 地点（最内部）、
    /// 末尾 = `<module>` に到達する直前の最外部フレーム。
    pub frames: Vec<StackFrame>,
}

// ---------------------------------------------------------------------------
// Function / Class / Instance value types
// ---------------------------------------------------------------------------

/// ジェネレータ関数の定義（`gen` キーワードで宣言）。
/// 呼び出すと `Value::Generator` を返す。
///
/// - `params`: 仮引数リスト
/// - `body`: 関数本体の文リスト（`yield` 文を含む）
#[derive(Debug)]
pub struct GeneratorFnValue {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// 具体型が未確定のテンプレートジェネレータ関数定義。
/// `gen fn[T: Trait](...)` 構文でパースされ、型引数を渡して実体化される。
///
/// - `template_params`: 型変数とその trait 制約
/// - `params`: 仮引数リスト（型変数名を含む場合がある）
/// - `body`: 関数本体の文リスト
#[derive(Debug)]
pub struct TemplateGenFnValue {
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// インスタンス化済みジェネレータオブジェクトの実行時状態。
/// `exec_generator` によってジェネレータ本体を一括実行し、すべての `yield` 値を収集してから保持する。
///
/// - `values`: ジェネレータ本体から収集されたすべての yield 値（順序保証）
/// - `index`: 次回 `next()` 呼び出しで返す値のインデックス。`values.len()` 以上になると枯渇
#[derive(Debug)]
pub struct GeneratorState {
    pub values: Vec<Value>,
    pub index: usize,
}

/// 具体型が未確定のテンプレート関数定義（`fn f[T: Trait](...)` 構文）。
/// 型引数付きで呼び出されたとき（`f[ConcreteType](args)`）、型変数を具体型に置換して実行される。
///
/// - `template_params`: 型変数名とその trait 制約のリスト
/// - `params`: 仮引数リスト（型変数名を型アノテーションに含む場合がある）
/// - `body`: 関数本体の文リスト
#[derive(Debug)]
pub struct TemplateFnValue {
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

/// 具体型が未確定のテンプレートクラス定義（`class C[T: Trait]:` 構文）。
/// 型引数付きで実体化されたとき（`C[ConcreteType](args)`）、型変数を置換して `ClassValue` を構築する。
///
/// - `name`: クラス名
/// - `template_params`: 型変数名とその trait 制約のリスト
/// - `bases`: 基底クラス・trait 名のリスト
/// - `body`: クラス本体の文リスト（フィールド宣言・メソッド定義を含む）
#[derive(Debug)]
pub struct TemplateClassValue {
    pub name: String,
    pub template_params: Vec<crate::ast::TemplateParam>,
    pub bases: Vec<String>,
    pub body: Vec<Stmt>,
}

/// 通常の関数定義（`fn` キーワード）の実行時表現。
///
/// - `params`: 仮引数リスト（名前・可変フラグ・型アノテーションを含む）
/// - `body`: 関数本体の文リスト
#[derive(Debug)]
pub struct FnValue {
    pub(self) params: Vec<Param>,
    pub(self) body: Vec<Stmt>,
    /// Python モジュールから変換された関数かどうか。
    /// `true` のとき、引数リストに存在しないキーワード引数をエラーにせず
    /// `AdditionalParam` dict として関数スコープに注入する。
    pub(self) is_python: bool,
}

/// クラス定義の実行時表現。インスタンス化（`instantiate`）の雛形となる。
///
/// - `name`: クラス名（`new_type` では派生名に上書きされる）
/// - `bases`: 基底クラス・trait 名のリスト（trait 制約の検証に使用）
/// - `methods`: メソッド名 → オーバーロード候補リスト のマップ
/// - `gen_methods`: ジェネレータメソッド名 → `GeneratorFnValue` のマップ（`gen` 定義）
/// - `field_defaults`: 初期値付き `mut`/`let` フィールドの (名前, デフォルト値, 可変フラグ) リスト
/// - `class_vars`: `const` クラス変数のマップ（全インスタンスで共有・代入不可）
/// - `field_mutability`: フィールド名 → 可変フラグ のマップ（初期値なしフィールドの初回代入時に参照）
#[derive(Debug)]
pub struct ClassValue {
    pub(self) name: String,
    pub(self) bases: Vec<String>,
    /// メソッド名 → オーバーロード候補リスト のマップ。
    pub(self) methods: HashMap<String, Vec<Rc<FnValue>>>,
    /// `gen` 定義のジェネレータメソッド（例: `gen __iter__(self) -> T:`）。
    pub(self) gen_methods: HashMap<String, Rc<GeneratorFnValue>>,
    /// 初期値付き `mut`/`let` フィールドの (名前, デフォルト値, 可変フラグ) リスト。
    pub(self) field_defaults: Vec<(String, Value, bool)>,
    /// `const` クラス変数。全インスタンスで共有され、代入は不可。
    pub(self) class_vars: HashMap<String, Value>,
    /// フィールド名 → 可変フラグ のマップ。初期値なしフィールドを初回代入するときに参照する。
    pub(self) field_mutability: HashMap<String, bool>,
}

/// クラスインスタンスの実行時データ。`Rc<RefCell<InstanceData>>` で共有・可変参照する。
///
/// - `class`: このインスタンスが属するクラスの定義（メソッド解決などに使用）
/// - `fields`: フィールド名 → (値, 可変フラグ) のマップ。trait 名前空間付きキー (`"Trait::field"`) も格納される
/// - `immutable`: `let` バインドされた場合に `true`。すべてのフィールドが不変になり、`mut self` メソッド呼び出しが禁止される
#[derive(Debug)]
pub struct InstanceData {
    pub class: Rc<ClassValue>,
    /// フィールド名 → (値, 可変フラグ) のマップ。
    pub fields: HashMap<String, (Value, bool)>,
    /// `let` バインドされたとき `true`。全フィールドが不変になり、`mut self` メソッドは呼べない。
    pub immutable: bool,
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// タプル値の内部ストレージ。
/// 内部表現（並列 Vec）はプライベートであり、公開 API（`get` / `len` / `element_type` など）のみが安定。
/// 内部フィールドは将来自由に変更できる。
///
/// - `values`: 実値の順序付きリスト（実行時は任意の型）
/// - `types`: 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）
#[derive(Debug)]
#[allow(dead_code)]
pub struct TupleData {
    /// 要素値の順序付きリスト（実行時は任意の型）。
    pub(self) values: Vec<Value>,
    /// 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）。
    pub(self) types: Vec<String>,
}

#[allow(dead_code)]
impl TupleData {
    /// 実値リストと型名リストから新しい `TupleData` を構築する。
    ///
    /// - `values`: 要素値のリスト
    /// - `types`: 各要素のランタイム型名のリスト（`values` と同じ長さであること）
    pub fn new(values: Vec<Value>, types: Vec<String>) -> Self {
        Self { values, types }
    }

    /// タプルの要素数を返す。
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// タプルが空（要素数0）なら `true` を返す。
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 指定インデックスの要素値を返す。インデックスが範囲外なら `None`。
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// 指定インデックスの要素のランタイム型名を返す。インデックスが範囲外なら `None`。
    pub fn element_type(&self, index: usize) -> Option<&str> {
        self.types.get(index).map(|s| s.as_str())
    }

    /// すべての要素値をスライスとして返す。
    pub fn all_values(&self) -> &[Value] {
        &self.values
    }

    /// すべての要素型名をスライスとして返す。
    pub fn all_types(&self) -> &[String] {
        &self.types
    }
}

/// 辞書値の内部ストレージ。
/// キーと値を並列 Vec で保持する。アクセスには `get` / `set` メソッドを使用すること。
/// 内部表現（並列リスト）はプライベートであり、公開 API のみが安定。
///
/// - `key_type`: 有効なキーの型名。型なし辞書は `"Any"`
/// - `item_type`: 有効な値の型名。型なし辞書は `"Any"`
/// - `keys` / `items`: 並列 Vec でキーと値を格納（同一インデックスが対応する）
#[derive(Debug)]
pub struct DictData {
    /// 有効なキーの型名。型なし辞書は `"Any"`。
    pub key_type: String,
    /// 有効な値の型名。型なし辞書は `"Any"`。
    pub item_type: String,
    pub(self) keys: Vec<Value>,
    pub(self) items: Vec<Value>,
}

impl DictData {
    /// 空の型付き辞書を生成する。
    ///
    /// - `key_type`: キーの型名（型なしは `"Any"`）
    /// - `item_type`: 値の型名（型なしは `"Any"`）
    pub fn new(key_type: String, item_type: String) -> Self {
        Self { key_type, item_type, keys: vec![], items: vec![] }
    }

    /// 指定したキーに対応する値を返す。キーが存在しない場合は `None`。
    pub fn get(&self, key: &Value) -> Option<Value> {
        self.find_index(key).map(|i| self.items[i].clone())
    }

    /// キーと値を追加、またはキーが既に存在する場合は値を更新する。
    pub fn set(&mut self, key: Value, value: Value) {
        if let Some(i) = self.find_index(&key) {
            // 既存キーの値を更新
            self.items[i] = value;
        } else {
            // 新規キーと値を末尾に追加
            self.keys.push(key);
            self.items.push(value);
        }
    }

    /// すべてのキーをクローンしてリストとして返す。
    pub fn all_keys(&self) -> Vec<Value> {
        self.keys.clone()
    }

    /// すべての値をクローンしてリストとして返す。
    pub fn all_items(&self) -> Vec<Value> {
        self.items.clone()
    }

    /// `keys` 内でキーが最初に見つかった位置（インデックス）を返す。存在しない場合は `None`。
    fn find_index(&self, key: &Value) -> Option<usize> {
        self.keys.iter().position(|k| Self::values_equal(k, key))
    }

    /// 辞書キー比較用の等値判定。プリミティブ型のみ対応（インスタンス等は常に `false`）。
    fn values_equal(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => x == y,
            (Value::Float(x), Value::Float(y)) => x == y,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::None, Value::None) => true,
            _ => false,
        }
    }
}

/// PyO3 を通じて Python オブジェクトへの参照を保持するハンドル。
/// GIL を保持せずにオブジェクトを所有でき、ドロップ時に Python 側の参照カウントを自動減少させる。
pub struct PyObjHandle {
    pub inner: pyo3::Py<pyo3::PyAny>,
}

impl std::fmt::Debug for PyObjHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<PyObject>")
    }
}

/// モジュールまたは名前空間の実行時データ。
/// `import[py] mod as m` で `m` にバインドされる。
/// `m.ClassName()` のように `.` でメンバにアクセスする。
#[derive(Debug, Clone)]
pub struct NamespaceData {
    /// モジュール名（エラーメッセージに使用）
    pub name: String,
    /// メンバ名 → 値のマップ
    pub members: HashMap<String, Value>,
}

/// モジュールキャッシュのエントリ状態。
#[derive(Debug, Clone)]
pub(self) enum ModuleState {
    /// 現在ロード中（循環 import 検出用）
    Loading,
    /// ロード済み
    Loaded(Rc<NamespaceData>),
}

/// インタープリタが扱う実行時値の列挙型。
///
/// 各バリアントの概要:
/// - `Int` / `Float` / `Str` / `Bool` / `None`: プリミティブ値
/// - `List`: 可変長リスト（各要素は任意の型）
/// - `Function`: 通常の関数値
/// - `OverloadedFn`: 同名関数のオーバーロード候補リスト（同スコープに複数定義されたとき）
/// - `Class`: クラス定義（コンストラクタとして呼び出し可能）
/// - `Instance`: クラスインスタンス（共有参照かつ内部可変）
/// - `Type`: 組み込み型名（`int`, `str`, `float`, `bool`）。ユーザー定義型は `Class` で表現
/// - `Trait`: 宣言されたトレイトの実行時表現
/// - `TemplateFn` / `TemplateClass` / `TemplateGenFn`: 未実体化のテンプレート関数・クラス・ジェネレータ
/// - `GeneratorFn`: ジェネレータ関数（呼び出すと `Generator` を返す）
/// - `Generator`: 実体化済みジェネレータ（yield 済み値を保持）
/// - `Dict`: 型付き or 型なし辞書（キー・値の並列 Vec で内部管理）
/// - `Tuple`: 不変・固定長のシーケンス（各要素の型情報を保持）
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    List(Vec<Value>),
    Function(Rc<FnValue>),
    /// 同スコープに同名で2つ以上のオーバーロードが定義された関数値。
    OverloadedFn(Vec<Rc<FnValue>>),
    Class(Rc<ClassValue>),
    Instance(Rc<RefCell<InstanceData>>),
    /// 組み込み型名を保持する型値（`int`, `str`, `float`, `bool`）。
    /// ユーザー定義クラス型は `Value::Class` で表現する。
    Type(String),
    /// 宣言された trait の実行時表現。
    Trait(String),
    /// 型変数でパラメータ化された未実体化のテンプレート関数。
    TemplateFn(Rc<TemplateFnValue>),
    /// 型変数でパラメータ化された未実体化のテンプレートクラス。
    TemplateClass(Rc<TemplateClassValue>),
    /// ジェネレータ関数（呼び出すと `Generator` を生成して返す）。
    GeneratorFn(Rc<GeneratorFnValue>),
    /// 型変数でパラメータ化された未実体化のテンプレートジェネレータ関数。
    TemplateGenFn(Rc<TemplateGenFnValue>),
    /// 実体化済みジェネレータ。すべての yield 値を一括収集して保持する。
    Generator(Rc<RefCell<GeneratorState>>),
    /// 型付きまたは型なし（Any）のキー・値を持つ辞書値。
    Dict(Rc<RefCell<DictData>>),
    /// 不変・固定長のシーケンス。各要素に型情報を保持する。
    Tuple(Rc<TupleData>),
    /// import されたモジュールまたは名前空間。`.` でメンバにアクセスする。
    Namespace(Rc<NamespaceData>),
    /// PyO3 経由で保持する Python オブジェクトへの参照。
    /// tl 側では不透明（opaque）な値として扱われる。
    PyObject(Rc<PyObjHandle>),
}

// ---------------------------------------------------------------------------
// Control-flow signals
// ---------------------------------------------------------------------------

/// 文の実行結果を表す制御フロー信号。
///
/// - `Normal`: 通常終了（次の文へ進む）
/// - `Break`: `break` 文が実行された（ループを抜ける）
/// - `Continue`: `continue` 文が実行された（ループの次の反復へ進む）
/// - `Return(v)`: `return` 文が実行された（関数を抜けて値 `v` を返す）
/// - `BlockReturn(v)`: `block_return` 文が実行された（ブロック式の値として `v` を返す）
/// - `Raise(e)`: `raise` 文が実行された（言語レベルの例外 `e` がコールスタックを遡る）
#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecResult {
    Normal,
    Break,
    Continue,
    Return(Value),
    BlockReturn(Value),
    /// コールスタックを遡って伝播中の言語レベル例外。
    Raise(RaisedError),
}

// ---------------------------------------------------------------------------
// Interpreter internals
// ---------------------------------------------------------------------------

/// スコープ内の1つの変数エントリ。
///
/// - `value`: 変数の現在の値
/// - `mutable`: `true` なら再代入可能（`mut` 宣言）、`false` なら不変（`let` / `const` 宣言）
pub(self) struct Var {
    pub(self) value: Value,
    pub(self) mutable: bool,
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
pub struct Interpreter {
    pub(self) scopes: Vec<HashMap<String, Var>>,
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
        let mut global: HashMap<String, Var> = HashMap::new();

        // 組み込み型値を事前定義: `int`, `str`, `float`, `bool`, `dict` を型式として使えるようにする
        for name in ["int", "str", "float", "bool", "dict"] {
            global.insert(name.to_string(), Var { value: Value::Type(name.to_string()), mutable: false });
        }

        // 組み込み `Error` trait を事前登録（値としてアクセス可能にする）
        global.insert("Error".to_string(), Var { value: Value::Trait("Error".to_string()), mutable: false });

        // 標準例外クラスをすべて登録する。
        // 各クラスは `__init__(mut self, message: str)` を持ち、
        // code_context / file / line / col フィールドは raise 時にインタープリタが設定する。
        let exception_names = [
            "Exception", "ValueError", "TypeError", "NameError", "AttributeError",
            "IndexError", "KeyError", "ZeroDivisionError", "RuntimeError",
            "StopIteration", "NotImplementedError", "OverflowError", "IOError",
            "OSError", "AssertionError", "ArithmeticError",
        ];
        for class_name in exception_names {
            let cls = Self::make_error_class(class_name);
            global.insert(class_name.to_string(), Var { value: Value::Class(cls), mutable: false });
        }

        Self {
            scopes: vec![global],
            source_map: HashMap::new(),
            call_stack: Vec::new(),
            current_exception: None,
            module_cache: HashMap::new(),
            in_python_module: false,
            python_search_dirs: Vec::new(),
        }
    }

    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリを登録する。
    pub fn add_python_search_dir(&mut self, dir: PathBuf) {
        self.python_search_dirs.push(dir);
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
                out.push_str(&format!("  File \"{}\", in {}\n", frame.file, frame.fn_name));
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
        if let Value::Instance(inst_rc) = &raised.exception {
            let inst = inst_rc.borrow();
            let class_name = &inst.class.name;
            let message = inst.fields.get("message")
                .map(|(v, _)| match v {
                    Value::Str(s) => s.clone(),
                    Value::Int(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => "<value>".to_string(),
                })
                .unwrap_or_default();
            out.push_str(&format!("{}: {}", class_name, message));
        } else {
            out.push_str("<exception>");
        }

        out
    }
}
