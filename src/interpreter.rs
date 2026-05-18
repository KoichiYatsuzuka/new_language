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
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::{Accessibility, Expr, Param, Stmt};

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
#[path = "interpreter/native_api.rs"]
pub(self) mod native_api;

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

    /// `for`/`while` 式の評価中に `loop_yield` された値を収集するスレッドローカル変数。
    /// `None` の場合は for/while 式の外であることを意味する（loop_yield はここで実行時エラー）。
    /// ネストした for/while 式を正しく扱うため、外側の式の値を退避して評価後に復元する。
    pub(self) static BLOCK_YIELDS: RefCell<Option<Vec<Value>>> = RefCell::new(None);

    /// 現在の for/while ループ（文・式両形式）のネスト深さ。
    /// `break` はこれが 0 のときに実行時エラーを返す。
    pub(self) static LOOP_DEPTH: RefCell<usize> = RefCell::new(0);
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

// ---------------------------------------------------------------------------
// Closure support
// ---------------------------------------------------------------------------

/// クロージャがキャプチャした変数の表現。
///
/// - `Immutable(Value)`: 不変変数のディープコピー（定義時点の値を保持）
/// - `Mutable(Rc<RefCell<Value>>)`: 可変変数の共有セル（外側スコープと読み書きを共有）
#[derive(Debug, Clone)]
pub(self) enum CapturedVar {
    /// 不変変数: 定義時点の値をディープコピーして保持する
    Immutable(Value),
    /// 可変変数: 外側スコープと同じセルを共有する
    Mutable(Rc<RefCell<Value>>),
}

/// ジェネレータ関数の定義（`gen` キーワードで宣言）。
/// 呼び出すと `Value::Generator` を返す。
///
/// - `params`: 仮引数リスト
/// - `body`: 関数本体の文リスト（`yield` 文を含む）
/// - `captured_env`: キャプチャした外側スコープ変数のマップ
#[derive(Debug)]
pub struct GeneratorFnValue {
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub(self) captured_env: std::collections::HashMap<String, CapturedVar>,
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
/// - `captured_env`: キャプチャした外側スコープ変数のマップ（クロージャ）
#[derive(Debug)]
pub struct FnValue {
    pub(self) params: Vec<Param>,
    pub(self) body: Vec<Stmt>,
    /// Python モジュールから変換された関数かどうか。
    /// `true` のとき、引数リストに存在しないキーワード引数をエラーにせず
    /// `AdditionalParam` dict として関数スコープに注入する。
    pub(self) is_python: bool,
    /// キャプチャした外側スコープ変数（クロージャ環境）。
    /// 不変変数はディープコピー、可変変数は共有セルとして保持する。
    pub(self) captured_env: std::collections::HashMap<String, CapturedVar>,
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
    /// フィールド名 → アクセス可能性 のマップ。プライベート・保護フィールドのアクセス制御に使用する。
    pub(self) field_access: HashMap<String, Accessibility>,
    /// メソッド名 → アクセス可能性 のマップ。プライベート・保護メソッドのアクセス制御に使用する。
    pub(self) method_access: HashMap<String, Accessibility>,
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

// ---------------------------------------------------------------------------
// File I/O types
// ---------------------------------------------------------------------------

/// ファイルオープンモード（Rust 内部表現）。
/// tl 側の `FileOpenMode` 列挙型と整数値で対応する。
#[derive(Debug, Clone, PartialEq)]
pub(self) enum FileOpenModeRust {
    /// 既存ファイルを読み書きモードで開く（内容保持）
    Write,
    /// ファイルを空の状態から読み書きモードで開く（内容破棄）
    Rewrite,
    /// 既存ファイルを読み取り専用で開く
    Read,
    /// 新規ファイルを作成して読み書きモードで開く（既存時はエラー）
    MakeAndWrite,
}

/// バイト認識モード（Rust 内部表現）。
/// tl 側の `ByteRecognizingMode` 列挙型と整数値で対応する。
#[derive(Debug, Clone, PartialEq)]
pub(self) enum ByteModeRust {
    /// バイト列として扱う: read 系は `list[int]`、write 系は `list[int]` を受け取る
    Byte,
    /// UTF-8 テキストとして扱う: read 系は `str`、write 系は `str` を受け取る
    Text,
}

/// ファイルオブジェクトの実行時状態。`open()` 組み込み関数で生成される。
///
/// - `path`: ファイルパス文字列
/// - `mode`: オープンモード
/// - `byte_mode`: バイト/テキストモード
/// - `content`: ファイル内容のメモリバッファ（open 時に全読み込み）
/// - `pointer`: 現在の読み書き位置（バイトインデックス）
/// - `is_closed`: `close()` または Drop 時に `true` にセット
/// - `file_handle`: 排他ロック保持用のファイルハンドル（close 時に None にセット）
#[derive(Debug)]
pub struct FileData {
    pub(self) path: String,
    pub(self) mode: FileOpenModeRust,
    pub(self) byte_mode: ByteModeRust,
    /// ファイル内容のメモリバッファ。読み書きはこのバッファに対して行い、close 時にディスクへ書き戻す。
    pub(self) content: Vec<u8>,
    /// 現在の読み書き位置（バイトインデックス）。0 がファイル先頭、content.len() がEOF。
    pub(self) pointer: usize,
    pub(self) is_closed: bool,
    /// ファイルハンドル。書き込みモードでは排他ロックとして機能し、close 時に None にセット。
    pub(self) file_handle: Option<std::fs::File>,
}

impl FileData {
    /// バッファをディスクに書き戻してファイルハンドルを閉じる。
    /// 書き込みモード (`write` / `rewrite` / `make_and_write`) のみ実際に書き戻す。
    /// 既に close 済みの場合は何もしない。
    pub(self) fn close(&mut self) {
        if self.is_closed {
            return;
        }
        self.is_closed = true;
        if matches!(
            self.mode,
            FileOpenModeRust::Write | FileOpenModeRust::Rewrite | FileOpenModeRust::MakeAndWrite
        ) {
            if let Some(ref mut f) = self.file_handle {
                use std::io::{Seek, SeekFrom, Write};
                let _ = f.seek(SeekFrom::Start(0));
                let _ = f.write_all(&self.content);
                // ファイルサイズをバッファサイズに合わせてトリム（書き込みが元より短い場合）
                let _ = f.set_len(self.content.len() as u64);
                let _ = f.flush();
            }
        }
        self.file_handle = None;
    }
}

impl Drop for FileData {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Native function support
// ---------------------------------------------------------------------------

/// Reference to a native (natively compiled) function exported by a shared library.
///
/// The library is identified by its filesystem path; the `Interpreter` holds the
/// loaded `Library` in `native_libs` keyed by the same path.
#[derive(Debug, Clone)]
pub struct NativeFnRef {
    /// Absolute path of the `.dll` / `.so` / `.dylib` that exports this function.
    pub lib_path: PathBuf,
    /// Base name of the tl function (e.g. `"is_prime"`).
    /// The actual exported symbol is `"{fn_name}_tl"`.
    pub fn_name: String,
    /// Number of positional parameters (used to size the args array).
    pub n_params: usize,
}

/// Wrapper around `libloading::Library` that implements `Debug`.
pub(self) struct NativeLibWrapper(pub(self) libloading::Library);

impl fmt::Debug for NativeLibWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<NativeLib>")
    }
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
/// - `NativeFunction`: natively compiled function loaded from a shared library
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    List(Rc<RefCell<Vec<Value>>>),
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
    /// ファイルオブジェクト。`open()` 組み込み関数が返す。
    /// メソッド（read / read_line / read_letter / write / write_line）で読み書きを行う。
    FileObject(Rc<RefCell<FileData>>),
    /// Natively compiled function from a shared library.
    /// Call it via `libloading` using the `{fn_name}_tl` exported symbol.
    NativeFunction(Rc<NativeFnRef>),
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
/// - `BlockReturn(v)`: `block_return` 文が実行された（ブロック式を即座に終了して `v` を返す）
/// - `BlockYield(v)`: `block_yield` 文が実行された（実行を継続しつつ `v` を結果リストに積む）
/// - `Raise(e)`: `raise` 文が実行された（言語レベルの例外 `e` がコールスタックを遡る）
#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecResult {
    Normal,
    Break,
    Continue,
    Return(Value),
    BlockReturn(Value),
    BlockYield(Value),
    /// コールスタックを遡って伝播中の言語レベル例外。
    Raise(RaisedError),
}

// ---------------------------------------------------------------------------
// Interpreter internals
// ---------------------------------------------------------------------------

/// スコープ内の1つの変数エントリ。
///
/// - `value`: 変数の現在の値（`mutable_cell` が `None` のとき有効）
/// - `mutable`: `true` なら再代入可能（`mut` 宣言）、`false` なら不変（`let` / `const` 宣言）
/// - `mutable_cell`: クロージャにキャプチャされた可変変数の共有セル。
///   `Some` のとき読み書きはセル経由で行い、`value` フィールドは使用しない。
pub(self) struct Var {
    pub(self) value: Value,
    pub(self) mutable: bool,
    /// クロージャとの共有セル。`Some` のとき読み書きはセル経由。
    pub(self) mutable_cell: Option<Rc<RefCell<Value>>>,
}

impl Var {
    /// 通常の変数エントリを作成する（セルなし）。
    pub(self) fn new(value: Value, mutable: bool) -> Self {
        Self { value, mutable, mutable_cell: None }
    }

    /// クロージャ共有セルに基づく変数エントリを作成する（常に mutable）。
    pub(self) fn new_cell(cell: Rc<RefCell<Value>>) -> Self {
        Self { value: Value::None, mutable: true, mutable_cell: Some(cell) }
    }

    /// 変数の現在の値を返す。セルがある場合はセルの値を返す。
    pub(self) fn get_value(&self) -> Value {
        if let Some(cell) = &self.mutable_cell {
            cell.borrow().clone()
        } else {
            self.value.clone()
        }
    }

    /// 変数に新しい値をセットする。セルがある場合はセルに書き込む。
    pub(self) fn set_value(&mut self, val: Value) {
        if let Some(cell) = &self.mutable_cell {
            *cell.borrow_mut() = val;
        } else {
            self.value = val;
        }
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
    /// `static mut` 変数の永続セル。キーは宣言の (ファイル名, 行, 列)。
    /// 外側関数の全呼び出しで同じセルを共有する。
    pub(self) static_cells: HashMap<(String, usize, usize), Rc<RefCell<Value>>>,
    /// 現在実行中のメソッドが属するクラス（アクセス制御チェック用）。
    /// クラスメソッドの外では `None`。
    pub(self) current_class: Option<Rc<ClassValue>>,
    /// トレイト名 → (フィールド名 → アクセス可能性) のマップ（TraitDef 実行時に収集）。
    /// クラスが継承したトレイトフィールドのアクセス制御に使用する。
    pub(self) trait_field_access: HashMap<String, HashMap<String, Accessibility>>,
    /// ロード済みのネイティブ共有ライブラリ。キーは DLL の絶対パス。
    /// ライブラリはインタープリタの生存期間を通じて保持される（アンロードしない）。
    pub(self) native_libs: HashMap<PathBuf, NativeLibWrapper>,
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

        // 組み込み型値を事前定義: `int`, `str`, `float`, `bool`, `dict`, `function` を型式として使えるようにする
        // `len` も `Value::Type` として登録しておく — ネイティブコードが cb_get_global("len") で取得して
        // call_value_with_args 経由で呼べるようにするため。
        for name in ["int", "str", "float", "bool", "dict", "function", "len"] {
            global.insert(name.to_string(), Var::new(Value::Type(name.to_string()), false));
        }

        // 組み込み `Error` trait を事前登録（値としてアクセス可能にする）
        global.insert("Error".to_string(), Var::new(Value::Trait("Error".to_string()), false));

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
            global.insert(class_name.to_string(), Var::new(Value::Class(cls), false));
        }

        // `path` 型: new_type path: str に相当するラッパークラス
        {
            let init_body = vec![Stmt::AttrAssign {
                target: Expr::Attr {
                    object: Box::new(Expr::Ident("self".to_string())),
                    attr: "value".to_string(),
                },
                value: Expr::Ident("value".to_string()),
            }];
            let init_fn = Rc::new(FnValue {
                params: vec![
                    Param { name: "self".to_string(), mutable: true, type_ann: None, default: None },
                    Param { name: "value".to_string(), mutable: false, type_ann: Some("str".to_string()), default: None },
                ],
                body: init_body,
                is_python: false,
                captured_env: HashMap::new(),
            });
            let mut methods = HashMap::new();
            methods.insert("__init__".to_string(), vec![init_fn]);
            let path_cls = Rc::new(ClassValue {
                name: "path".to_string(),
                bases: vec![],
                methods,
                gen_methods: HashMap::new(),
                field_defaults: vec![],
                class_vars: HashMap::new(),
                field_mutability: HashMap::from([("value".to_string(), true)]),
                field_access: HashMap::new(),
                method_access: HashMap::new(),
            });
            global.insert("path".to_string(), Var::new(Value::Class(path_cls), false));
        }

        // ファイル I/O 組み込み列挙型を登録する
        for (enum_name, variants) in [
            ("FileOpenMode", vec![("write", 0i64), ("rewrite", 1), ("read", 2), ("make_and_write", 3)]),
            ("StartPoint",   vec![("top", 0),     ("end", 1)]),
            ("ByteRecognizingMode", vec![("byte", 0), ("text", 1)]),
            ("Encoding",     vec![("ASCII", 0),   ("UTF_8", 1), ("UTF_8_with_BOM", 2), ("shift_JIS", 3)]),
        ] {
            let (item_name, item_cls, enum_cls) =
                Self::make_builtin_enum_class(enum_name, &variants);
            global.insert(item_name, Var::new(Value::Class(item_cls), false));
            global.insert(enum_name.to_string(), Var::new(Value::Class(enum_cls), false));
        }

        Self {
            scopes: vec![global],
            source_map: HashMap::new(),
            call_stack: Vec::new(),
            current_exception: None,
            module_cache: HashMap::new(),
            in_python_module: false,
            python_search_dirs: Vec::new(),
            static_cells: HashMap::new(),
            current_class: None,
            trait_field_access: HashMap::new(),
            native_libs: HashMap::new(),
        }
    }

    /// `import[py-int]` 時に Python の `sys.path` に追加するディレクトリを登録する。
    pub fn add_python_search_dir(&mut self, dir: PathBuf) {
        self.python_search_dirs.push(dir);
    }

    /// ビルトイン enum クラスのペア（item クラス + enum クラス）を Rust コードで生成する。
    ///
    /// - `name`: enum クラス名（例: `"FileOpenMode"`）
    /// - `variants`: (バリアント名, 整数値) のスライス
    ///
    /// 戻り値: `(item_cls_name, item_cls, enum_cls)` のタプル
    fn make_builtin_enum_class(
        name: &str,
        variants: &[(&str, i64)],
    ) -> (String, Rc<ClassValue>, Rc<ClassValue>) {
        let item_cls_name = format!("enum_item_{name}");
        // バリアントのインスタンス型（`enum_item_X`）: value フィールドを持つだけ
        let item_cls = Rc::new(ClassValue {
            name: item_cls_name.clone(),
            bases: vec![],
            methods: HashMap::new(),
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars: HashMap::new(),
            field_mutability: HashMap::from([("value".to_string(), true)]),
            field_access: HashMap::new(),
            method_access: HashMap::new(),
        });
        // 各バリアントをインスタンスとして生成し class_vars に登録
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        for (variant_name, int_val) in variants {
            let mut fields = HashMap::new();
            fields.insert("value".to_string(), (Value::Int(*int_val), true));
            let inst = Value::Instance(Rc::new(RefCell::new(InstanceData {
                class: item_cls.clone(),
                fields,
                immutable: false,
            })));
            class_vars.insert(variant_name.to_string(), inst);
        }
        // enum クラス本体（バリアントのみ保持、インスタンス化不可）
        let enum_cls = Rc::new(ClassValue {
            name: name.to_string(),
            bases: vec![],
            methods: HashMap::new(),
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars,
            field_mutability: HashMap::new(),
            field_access: HashMap::new(),
            method_access: HashMap::new(),
        });
        (item_cls_name, item_cls, enum_cls)
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
