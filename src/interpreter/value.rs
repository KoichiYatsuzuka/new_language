// value.rs — インタープリタが扱う実行時値の型定義
//
// 担当範囲:
//   - StackFrame / RaisedError        — 例外トレースバック型
//   - CapturedVar                      — クロージャキャプチャ変数
//   - FnValue / GeneratorFnValue / TemplateFnValue / TemplateGenFnValue / GeneratorState
//   - TemplateClassValue / ClassValue / InstanceData
//   - SliceValue / TupleData / DictData / DictKey
//   - PyObjHandle / NamespaceData / ModuleState
//   - FileOpenModeRust / ByteModeRust / FileData
//   - PtrParam / NativeFnRef / NativeLibWrapper
//   - Value enum                        — すべての実行時値のユニオン型
//   - deep_clone helpers               — スレッド境界を越えるための完全独立コピー
//   - ExecResult                       — 文実行の制御フロー信号

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use indexmap::IndexMap;

use crate::ast::{Accessibility, Param, Stmt};

use super::async_mgr;

// ---------------------------------------------------------------------------
// InstanceData flags (u32 in InstanceData.flags)
// ---------------------------------------------------------------------------

/// `let` バインドされたインスタンス: 全フィールドが不変、`mut self` メソッド呼び出し禁止。
pub const INST_IMMUTABLE: u32 = 0x80000000;
/// `raw_fields: Vec<u8>` による int/float フラット バッファが有効。
pub const INST_HAS_RAW_LAYOUT: u32 = 0x40000000;
/// 例外クラスのインスタンス（高速例外型チェック用）。
pub const INST_IS_EXCEPTION: u32 = 0x20000000;
/// `new_type` ラッパーのインスタンス（高速 new_type 判定用）。
pub const INST_IS_NEW_TYPE: u32 = 0x10000000;
/// bits 23-0: `raw_fields` の初期化済みスロットを示すビットマップ（最大 24 スロット）。
pub const INST_FIELD_INIT_MASK: u32 = 0x00FF_FFFF;

// ---------------------------------------------------------------------------
// Class ID registry
// ---------------------------------------------------------------------------

/// 次に割り当てるクラス ID（グローバルアトミックカウンタ）。
static NEXT_CLASS_ID: AtomicU32 = AtomicU32::new(1); // 0 = 未割り当て

/// 新しい一意なクラス ID を発行する。クラス定義時に一度だけ呼ぶ。
#[inline]
pub fn alloc_class_id() -> u32 {
    NEXT_CLASS_ID.fetch_add(1, Ordering::Relaxed)
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
// Closure support
// ---------------------------------------------------------------------------

/// クロージャがキャプチャした変数の表現。
///
/// - `Immutable(Value)`: 不変変数のディープコピー（定義時点の値を保持）
/// - `Mutable(Rc<RefCell<Value>>)`: 可変変数の共有セル（外側スコープと読み書きを共有）
#[derive(Debug, Clone)]
pub enum CapturedVar {
    /// 不変変数: 定義時点の値をディープコピーして保持する
    Immutable(Value),
    /// 可変変数: 外側スコープと同じセルを共有する
    Mutable(Rc<RefCell<Value>>),
}

// ---------------------------------------------------------------------------
// Function / Class / Instance value types
// ---------------------------------------------------------------------------

/// ジェネレータ関数の定義（`gen` キーワードで宣言）。
/// 呼び出すと `Value::Generator` を返す。
///
/// - `name`: ジェネレータ関数名（`__repr__` 等の表示に使用）
/// - `params`: 仮引数リスト
/// - `body`: 関数本体の文リスト（`yield` 文を含む）
/// - `captured_env`: キャプチャした外側スコープ変数のマップ
#[derive(Debug)]
pub struct GeneratorFnValue {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub captured_env: HashMap<String, CapturedVar>,
}

/// 具体型が未確定のテンプレートジェネレータ関数定義。
/// `gen fn[T: Trait](...)` 構文でパースされ、型引数を渡して実体化される。
///
/// - `name`: ジェネレータ関数名（実体化後の `GeneratorFnValue` に引き継がれる）
/// - `template_params`: 型変数とその trait 制約
/// - `params`: 仮引数リスト（型変数名を含む場合がある）
/// - `body`: 関数本体の文リスト
#[derive(Debug)]
pub struct TemplateGenFnValue {
    pub name: String,
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
/// - `name`: 関数名（実体化後の `FnValue` に引き継がれる）
/// - `template_params`: 型変数名とその trait 制約のリスト
/// - `params`: 仮引数リスト（型変数名を型アノテーションに含む場合がある）
/// - `body`: 関数本体の文リスト
#[derive(Debug)]
pub struct TemplateFnValue {
    pub name: String,
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
/// - `name`: 関数名（`__repr__` 等の表示に使用。匿名の場合は `"<anonymous>"`）
/// - `params`: 仮引数リスト（名前・可変フラグ・型アノテーションを含む）
/// - `body`: 関数本体の文リスト
/// - `captured_env`: キャプチャした外側スコープ変数のマップ（クロージャ）
#[derive(Debug)]
pub struct FnValue {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    /// Python モジュールから変換された関数かどうか。
    pub is_python: bool,
    /// キャプチャした外側スコープ変数（クロージャ環境）。
    pub captured_env: HashMap<String, CapturedVar>,
    /// 静的型アノテーションの戻り値型（文字列）。import[cs-dll] のブリッジ呼び出しで使用。
    pub return_type: Option<String>,
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
    pub name: String,
    /// クラスに割り当てられた一意な ID（`alloc_class_id()` で発行）。
    /// コンパイル済みコードからの class_id ベースのフィールド GEP に使用する。
    pub class_id: u32,
    pub bases: Vec<String>,
    /// メソッド名 → オーバーロード候補リスト のマップ。
    pub methods: HashMap<String, Vec<Rc<FnValue>>>,
    /// `gen` 定義のジェネレータメソッド（例: `gen __iter__(self) -> T:`）。
    pub gen_methods: HashMap<String, Rc<GeneratorFnValue>>,
    /// 初期値付き `mut`/`let` フィールドの (名前, デフォルト値, 可変フラグ) リスト。
    pub field_defaults: Vec<(String, Value, bool)>,
    /// `const` クラス変数。全インスタンスで共有され、代入は不可。
    pub class_vars: HashMap<String, Value>,
    /// フィールド名 → 可変フラグ のマップ。初期値なしフィールドを初回代入するときに参照する。
    pub field_mutability: HashMap<String, bool>,
    /// フィールド名 → Vec インデックス のマップ。own フィールド名・trait 修飾名（`"Trait::field"`）の両方を含む。
    /// `InstanceData.fields` Vec への O(1) アクセスに使用する。
    pub field_index: HashMap<String, usize>,
    /// `InstanceData.fields` Vec のスロット総数。
    pub field_count: usize,
    /// スロットインデックス → 元の可変フラグ（クラス定義時の宣言による）。`copy()` のフリーズ解除に使用。
    pub field_mutability_vec: Vec<bool>,
    /// フィールド名 → アクセス可能性 のマップ。プライベート・保護フィールドのアクセス制御に使用する。
    pub field_access: HashMap<String, Accessibility>,
    /// メソッド名 → アクセス可能性 のマップ。プライベート・保護メソッドのアクセス制御に使用する。
    pub method_access: HashMap<String, Accessibility>,
    /// `static fn` で定義されたスタティックメソッド名のセット。`self` を受け取らない。
    pub static_method_names: HashSet<String>,
    /// `class_method fn` で定義されたクラスメソッド名のセット。第1引数は `cls`（クラス自身）。
    pub class_method_names: HashSet<String>,
    /// `static mut` で定義されたクラス静的変数。全インスタンスで共有される可変セル。
    pub static_vars: HashMap<String, Rc<RefCell<Value>>>,
    /// `new_type Name: PrimType` で生成されたクラスの場合、元のプリミティブ型名を保持する。
    /// `repr()` でプリミティブ風の表示 (`Name(value)`) に使う。`None` は通常クラス。
    pub new_type_base: Option<String>,
    /// 例外クラスのとき `true`。インスタンス生成時に `INST_IS_EXCEPTION` フラグを立てる。
    pub is_exception: bool,
}

/// クラスインスタンスの実行時データ。`Rc<RefCell<InstanceData>>` で共有・可変参照する。
///
/// - `class_id`: クラスの一意 ID（コンパイル済みコードでの型判定・GEP に使用）
/// - `flags`: インスタンス状態フラグ（`INST_*` 定数を参照）
/// - `class`: このインスタンスが属するクラスの定義（メソッド解決などに使用）
/// - `fields`: フィールドスロットの Vec。`class.field_index[name]` でインデックスを引く。
///   `None` = 未初期化スロット（`__init__` で初回代入前）。`Some((val, mutable))` = 初期化済み。
#[derive(Debug)]
pub struct InstanceData {
    /// クラスの一意 ID。ヘッダ先頭 4 バイトとしてコンパイル済みコードから読める（Case C レイアウト）。
    pub class_id: u32,
    /// インスタンス状態フラグ。`INST_IMMUTABLE`・`INST_IS_EXCEPTION` 等のビット。
    pub flags: u32,
    pub class: Rc<ClassValue>,
    /// フィールドスロットの Vec。インデックスは `class.field_index` で解決する。
    /// `None` = 未初期化。`Some((値, 可変フラグ))` = 初期化済み。
    pub fields: Vec<Option<(Value, bool)>>,
}

// ---------------------------------------------------------------------------
// Value storage types
// ---------------------------------------------------------------------------

/// タプル値の内部ストレージ。
/// 内部表現（並列 Vec）はプライベートであり、公開 API（`get` / `len` / `element_type` など）のみが安定。
/// 内部フィールドは将来自由に変更できる。
///
/// スライス値: `begin:end:step` の内部表現。
/// `tuple[Optional[Index], Optional[Index], Optional[int]]` に相当する。
/// begin/end は `Index` インスタンスまたは `None`、step は `int` または `None`。
#[derive(Debug, Clone)]
pub struct SliceValue {
    pub begin: Option<Value>,
    pub end: Option<Value>,
    pub step: Option<Value>,
}

/// - `values`: 実値の順序付きリスト（実行時は任意の型）
/// - `types`: 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）
#[derive(Debug)]
#[allow(dead_code)]
pub struct TupleData {
    /// 要素値の順序付きリスト（実行時は任意の型）。
    pub values: Vec<Value>,
    /// 各要素のランタイム型名（例: `"int"`, `"str"`, `"MyClass"`）。
    pub types: Vec<String>,
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
/// `IndexMap` で挿入順を保持しつつ O(1) ルックアップを提供する。
/// アクセスには `get` / `set` メソッドを使用すること。
///
/// - `key_type`: 有効なキーの型名。型なし辞書は `"Any"`
/// - `item_type`: 有効な値の型名。型なし辞書は `"Any"`
#[derive(Debug)]
pub struct DictData {
    /// 有効なキーの型名。型なし辞書は `"Any"`。
    pub key_type: String,
    /// 有効な値の型名。型なし辞書は `"Any"`。
    pub item_type: String,
    map: IndexMap<DictKey, Value>,
}

/// `IndexMap` のキーとして使用するラッパー。`Value` のプリミティブ部分のみハッシュ可能。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum DictKey {
    Int(i64),
    Str(String),
    Bool(bool),
    None,
}

impl DictKey {
    /// `Value` を `DictKey` に変換する。ハッシュ不可能な型（リスト・インスタンス等）は `None` を返す。
    fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Int(n) => Some(DictKey::Int(*n)),
            Value::Float(f) => {
                // 整数値の float (e.g. 1.0) は Int キーとして扱う（Python 互換）
                if f.fract() == 0.0 && f.is_finite() {
                    Some(DictKey::Int(*f as i64))
                } else {
                    None
                }
            }
            Value::Str(s) => Some(DictKey::Str(s.clone())),
            Value::Bool(b) => Some(DictKey::Bool(*b)),
            Value::None => Some(DictKey::None),
            _ => None,
        }
    }
}

impl DictData {
    /// 空の型付き辞書を生成する。
    pub fn new(key_type: String, item_type: String) -> Self {
        Self {
            key_type,
            item_type,
            map: IndexMap::new(),
        }
    }

    /// 指定したキーに対応する値を返す。キーが存在しない場合は `None`。
    pub fn get(&self, key: &Value) -> Option<Value> {
        DictKey::from_value(key).and_then(|k| self.map.get(&k).cloned())
    }

    /// キーと値を追加、またはキーが既に存在する場合は値を更新する。
    pub fn set(&mut self, key: Value, value: Value) {
        if let Some(k) = DictKey::from_value(&key) {
            self.map.insert(k, value);
        }
        // unhashable key (e.g. instance) silently ignored — same as before
    }

    /// すべてのキーを `Value` リストとして返す（挿入順）。
    pub fn all_keys(&self) -> Vec<Value> {
        self.map
            .keys()
            .map(|k| match k {
                DictKey::Int(n) => Value::Int(*n),
                DictKey::Str(s) => Value::Str(s.clone()),
                DictKey::Bool(b) => Value::Bool(*b),
                DictKey::None => Value::None,
            })
            .collect()
    }

    /// すべての値をクローンしてリストとして返す（挿入順）。
    pub fn all_items(&self) -> Vec<Value> {
        self.map.values().cloned().collect()
    }

    /// キー・値のペアを挿入順で走査するイテレータ。
    pub(super) fn iter(&self) -> impl Iterator<Item = (&DictKey, &Value)> {
        self.map.iter()
    }

    /// 指定キーを辞書から削除する。存在しない場合は何もしない。
    // pub(super) fn remove(&mut self, key: &Value) {
    //     if let Some(k) = DictKey::from_value(key) {
    //         self.map.shift_remove(&k);
    //     }
    // }

    /// エントリ数を返す。
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// 辞書が空なら `true`。
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Module / Python interop types
// ---------------------------------------------------------------------------

/// PyO3 を通じて Python オブジェクトへの参照を保持するハンドル。
/// GIL を保持せずにオブジェクトを所有でき、ドロップ時に Python 側の参照カウントを自動減少させる。
pub struct PyObjHandle {
    pub inner: pyo3::Py<pyo3::PyAny>,
}

impl std::fmt::Debug for PyObjHandle {
    /// `PyObjHandle` のデバッグ表示。常に `"<PyObject>"` を出力する。
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
pub enum ModuleState {
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
pub enum FileOpenModeRust {
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
pub enum ByteModeRust {
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
    pub path: String,
    pub mode: FileOpenModeRust,
    pub byte_mode: ByteModeRust,
    /// ファイル内容のメモリバッファ。読み書きはこのバッファに対して行い、close 時にディスクへ書き戻す。
    pub content: Vec<u8>,
    /// 現在の読み書き位置（バイトインデックス）。0 がファイル先頭、content.len() がEOF。
    pub pointer: usize,
    pub is_closed: bool,
    /// ファイルハンドル。書き込みモードでは排他ロックとして機能し、close 時に None にセット。
    pub file_handle: Option<std::fs::File>,
}

impl FileData {
    /// バッファをディスクに書き戻してファイルハンドルを閉じる。
    /// 書き込みモード (`write` / `rewrite` / `make_and_write`) のみ実際に書き戻す。
    /// 既に close 済みの場合は何もしない。
    pub fn close(&mut self) {
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
    /// `FileData` がスコープを抜けるときに自動的に `close()` を呼び出す。
    /// 書き込みモードの場合、バッファをディスクに書き戻してからハンドルを解放する。
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Native function support
// ---------------------------------------------------------------------------

/// Describes how a C function parameter should be handled at the native boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PtrParam {
    /// Not a pointer — value passed by value.
    None,
    /// `const T*` — read-only pointer; any expression is accepted, no write-back.
    ConstPtr,
    /// `T*` — mutable pointer; requires a `mut` variable argument; value is written
    /// back to the variable after the call returns.
    MutPtr,
}

/// Reference to a native (natively compiled) function.
///
/// Two dispatch modes:
///   - `raw_fn_ptr != 0`: inkwell JIT — call the pointer directly (no libloading).
///   - `raw_fn_ptr == 0`: DLL via libloading — use `lib_path` to look up the library.
///
/// `cached_fn_ptr` is a lazily-populated cache for the cpp-dll case: set to the
/// resolved symbol address on first call so subsequent calls skip `GetProcAddress`.
#[derive(Debug)]
pub struct NativeFnRef {
    /// Absolute path of the `.dll` / `.so` / `.dylib`.  Empty for JIT functions.
    pub lib_path: PathBuf,
    /// Base name of the tl function (e.g. `"is_prime"`).
    /// The actual exported symbol is `"{fn_name}_tl"`.
    pub fn_name: String,
    /// Total number of positional parameters (used to size the args array).
    pub n_params: usize,
    /// Minimum number of required arguments.
    pub min_params: usize,
    /// Per-parameter mutability flags (`true` = `mut`, `false` = `let`).
    pub param_mutabilities: Vec<bool>,
    /// Per-parameter pointer kind (cpp-bridge only).
    pub ptr_params: Vec<PtrParam>,
    /// Non-zero for inkwell JIT functions: address of `fname_tl` in JIT memory.
    /// Cast to `unsafe extern "C" fn(*const i64, i32) -> i64` at call time.
    pub raw_fn_ptr: usize,
    /// Lazily cached raw function pointer for cpp-dll functions (raw_fn_ptr == 0).
    /// Written once on first call via ar_call_fn fast path; 0 = not yet resolved.
    pub cached_fn_ptr: std::sync::atomic::AtomicUsize,
}

impl Clone for NativeFnRef {
    fn clone(&self) -> Self {
        Self {
            lib_path: self.lib_path.clone(),
            fn_name: self.fn_name.clone(),
            n_params: self.n_params,
            min_params: self.min_params,
            param_mutabilities: self.param_mutabilities.clone(),
            ptr_params: self.ptr_params.clone(),
            raw_fn_ptr: self.raw_fn_ptr,
            cached_fn_ptr: std::sync::atomic::AtomicUsize::new(
                self.cached_fn_ptr.load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }
}

/// Wrapper around `libloading::Library` that implements `Debug`.
pub struct NativeLibWrapper(pub libloading::Library);

impl fmt::Debug for NativeLibWrapper {
    /// `NativeLibWrapper` のデバッグ表示。常に `"<NativeLib>"` を出力する。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<NativeLib>")
    }
}

// ---------------------------------------------------------------------------
// Flat-frozen list layout
// ---------------------------------------------------------------------------

/// フラットリストの各フィールドの型。
/// Int/Float はプリミティブ 8-byte フィールド。Struct は再帰的な SWD クラスフィールド。
#[derive(Debug, Clone)]
pub enum FlatFieldTy {
    Int,
    Float,
    /// 別の SWD クラスをインラインに展開したフィールド。
    Struct(Rc<FlatLayout>),
}

impl FlatFieldTy {
    pub fn stride(&self) -> usize {
        match self {
            FlatFieldTy::Int | FlatFieldTy::Float => 8,
            FlatFieldTy::Struct(sub) => sub.stride,
        }
    }
}

impl PartialEq for FlatFieldTy {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int, Self::Int) | (Self::Float, Self::Float) => true,
            (Self::Struct(a), Self::Struct(b)) => a.class_name == b.class_name,
            _ => false,
        }
    }
}

/// FrozenList の可変状態（バイト列・長さ・確保済みサイズ）。
/// `data` はフラット byte 列（stride バイト × allocated_size 要素分を確保、先頭 len 要素が有効）。
#[derive(Debug, Clone)]
pub struct FlatListData {
    /// フラット byte 列。`allocated_size * stride` バイトを確保し、先頭 `len * stride` が有効データ。
    pub data: Vec<u8>,
    /// 有効要素数（論理長）。
    pub len: usize,
    /// 確保済み要素数（容量）。`len <= allocated_size` が常に成立する。
    pub allocated_size: usize,
}

/// FrozenList の平坦メモリレイアウト記述。
/// `fields` はアルファベット順。全フィールドが SWD 型（int/float または別の SWD クラス）のみ。
#[derive(Debug, Clone)]
pub struct FlatLayout {
    pub class_name: String,
    /// (フィールド名, 型) のアルファベット順リスト。
    pub fields: Vec<(String, FlatFieldTy)>,
    /// 要素1つあたりのバイト数。各フィールドの stride() の合計。
    pub stride: usize,
    /// 再構成用クラス定義。
    pub class: Rc<ClassValue>,
}

impl FlatLayout {
    /// フラット配列インデックス `idx` の要素を `Value::Instance` として再構成する。
    pub fn reconstruct_item(&self, data: &[u8], idx: usize) -> Value {
        let base = idx * self.stride;
        self.reconstruct_at(data, base)
    }

    /// バイト列の `byte_base` 位置からこのレイアウトのインスタンスを再構成する。
    fn reconstruct_at(&self, data: &[u8], byte_base: usize) -> Value {
        let mut fields: Vec<Option<(Value, bool)>> = vec![None; self.class.field_count];
        let mut offset = byte_base;
        for (field_name, field_ty) in &self.fields {
            let val = match field_ty {
                FlatFieldTy::Float => {
                    let bytes: [u8; 8] = data[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
                    offset += 8;
                    Value::Float(f64::from_le_bytes(bytes))
                }
                FlatFieldTy::Int => {
                    let bytes: [u8; 8] = data[offset..offset + 8].try_into().unwrap_or([0u8; 8]);
                    offset += 8;
                    Value::Int(i64::from_le_bytes(bytes))
                }
                FlatFieldTy::Struct(sub) => {
                    let v = sub.reconstruct_at(data, offset);
                    offset += sub.stride;
                    v
                }
            };
            if let Some(&idx) = self.class.field_index.get(field_name.as_str()) {
                fields[idx] = Some((val, false));
            }
        }
        let flags = INST_IMMUTABLE
            | if self.class.is_exception { INST_IS_EXCEPTION } else { 0 }
            | if self.class.new_type_base.is_some() { INST_IS_NEW_TYPE } else { 0 };
        Value::Instance(Rc::new(RefCell::new(InstanceData {
            class_id: self.class.class_id,
            flags,
            class: self.class.clone(),
            fields,
        })))
    }
}

// ---------------------------------------------------------------------------
// Value enum
// ---------------------------------------------------------------------------

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
    /// 符号付き 64 ビット整数プリミティブ値。
    Int(i64),
    /// 符号なし 64 ビット整数値（ビット演算・ネイティブ ABI との連携用）。
    UInt(u64),
    /// 64 ビット浮動小数点プリミティブ値。
    Float(f64),
    /// 複素数プリミティブ値（実部・虚部はそれぞれ f64）。
    Complex(f64, f64),
    /// 文字列プリミティブ値（Unicode UTF-8）。
    Str(String),
    /// 真偽値プリミティブ値（`true` / `false`）。
    Bool(bool),
    /// `None` リテラル。Python の `None` に相当する。
    None,
    /// `Undefined` リテラル。外部ライブラリのメンバが未定義の状態を表す特殊値。
    /// 変数への代入は静的型エラー。条件判定（`x is Undefined`）と引数としてのみ使用可能。
    Undefined,
    /// 可変長リスト値。要素は任意の型を混在できる。`Rc<RefCell<...>>` により共有・可変参照する。
    List(Rc<RefCell<Vec<Value>>>),
    /// フラット固定長リスト。全要素が同一クラスかつ全フィールドが int/float。
    /// `state` は可変メタデータ（data/len/allocated_size）、`layout` はレイアウト記述（不変）。
    /// `mut` 変数として宣言すれば `append` で要素を追加でき、`freeze` で余剰確保を解放する。
    FrozenList { state: Rc<RefCell<FlatListData>>, layout: Rc<FlatLayout> },
    /// 通常の関数値（`fn` キーワードで定義された関数）。
    Function(Rc<FnValue>),
    /// 同スコープに同名で2つ以上のオーバーロードが定義された関数値。
    OverloadedFn(Vec<Rc<FnValue>>),
    /// クラス定義値。コンストラクタとして呼び出すとインスタンスを生成する。
    Class(Rc<ClassValue>),
    /// クラスインスタンス値。`Rc<RefCell<...>>` により共有・可変参照する。
    Instance(Rc<RefCell<InstanceData>>),
    /// 組み込み型名を保持する型値（`int`, `str`, `float`, `bool`）。
    /// ユーザー定義クラス型は `Value::Class` で表現する。
    Type(String),
    /// 宣言された trait の実行時表現。
    Trait(String),
    /// 宣言された protocol の実行時表現（型チェック専用; インスタンス化は不可）。
    Protocol(String),
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
    /// 重複なしの可変コレクション。要素の順序は保証しない。
    Set(Rc<RefCell<Vec<Value>>>),
    /// import されたモジュールまたは名前空間。`.` でメンバにアクセスする。
    Namespace(Rc<NamespaceData>),
    /// PyO3 経由で保持する Python オブジェクトへの参照。
    /// tl 側では不透明（opaque）な値として扱われる。
    PyObject(Arc<PyObjHandle>),
    /// ファイルオブジェクト。`open()` 組み込み関数が返す。
    /// メソッド（read / read_line / read_letter / write / write_line）で読み書きを行う。
    FileObject(Rc<RefCell<FileData>>),
    /// Natively compiled function from a shared library.
    /// Call it via `libloading` using the `{fn_name}_tl` exported symbol.
    NativeFunction(Arc<NativeFnRef>),
    /// スライス値: `obj[begin:end:step]` または `slice(begin, end, step)` で生成される。
    /// begin/end は Optional[Index]、step は Optional[int]。
    Slice(Rc<SliceValue>),
    /// AsyncManager インスタンス。`AsyncManager(num_thread=N)` で生成される。
    AsyncManager(Rc<RefCell<async_mgr::AsyncManagerData>>),
    /// Async 列挙型の値 (Async.Waiting / Async.Running / Async.Done)。
    AsyncStatusVal(async_mgr::AsyncStatus),
    /// `Signal[T]()` で生成される Arrow ネイティブのイベントソース。
    /// `.emit(val)` で全ハンドラを同期呼び出し、`.emit_async(val)` でキューに積む。
    Signal(Rc<RefCell<super::event_loop::SignalData>>),
    /// EventLoop シングルトン。`EventLoop.run()` / `EventLoop.post(fn)` で利用する。
    EventLoop(Rc<RefCell<super::event_loop::EventLoopData>>),
    /// import[cs-dll] で生成される C# オブジェクトのハンドル。
    CsObject(Rc<CsObjectData>),
    /// import[js-proc] で生成される JavaScript モジュール関数のランタイム表現。
    /// 呼び出し時に js_proc_runtime 経由で Node.js ブリッジに IPC 送信する。
    JsProcFn {
        /// ブリッジスクリプトのパス（ブリッジレジストリのキー）。
        bridge_key:  String,
        /// JS モジュール名（スラッシュ区切り、ブリッジに渡す）。
        module_name: String,
        /// 呼び出す関数名。
        fn_name:     String,
    },
    /// `Result[T, E]` 値。`Ok(value)` または `Err(error)` で生成される。
    /// `ok: true` → Ok 側の値、`ok: false` → Err 側の値。
    ResultVal { ok: bool, inner: Box<Value> },
}

/// import[cs-dll] / import[cs-proc] ブリッジが管理する C# オブジェクトのランタイム表現。
#[derive(Debug, Clone)]
pub struct CsObjectData {
    pub class_name: String,
    pub handle: i64,
    /// NativeAOT DLL パス (cs-dll) または proc exe パス (cs-proc)。
    pub bridge_path: std::path::PathBuf,
    /// 元の ClassValue stub（return type 解決に使用）。
    pub class: Rc<ClassValue>,
    /// true = cs-proc (IPC サブプロセス), false = cs-dll (DLL 直接呼び出し)
    pub is_proc: bool,
}

// ---------------------------------------------------------------------------
// deep_clone helpers (used when capturing the environment for async tasks)
// ---------------------------------------------------------------------------

/// Deep-clone a CapturedVar environment map so no Rc is shared across threads.
pub fn deep_clone_captured_env(
    env: &HashMap<String, CapturedVar>,
) -> HashMap<String, CapturedVar> {
    env.iter()
        .map(|(k, v)| {
            let cloned = match v {
                CapturedVar::Immutable(val) => CapturedVar::Immutable(val.deep_clone()),
                CapturedVar::Mutable(cell) => {
                    CapturedVar::Mutable(Rc::new(RefCell::new(cell.borrow().deep_clone())))
                }
            };
            (k.clone(), cloned)
        })
        .collect()
}

impl ClassValue {
    /// Create a fully independent deep copy of this ClassValue (no shared Rcs).
    pub fn deep_clone(&self) -> ClassValue {
        let methods = self
            .methods
            .iter()
            .map(|(k, overloads)| {
                let new_overloads = overloads
                    .iter()
                    .map(|rc| {
                        Rc::new(FnValue {
                            name: rc.name.clone(),
                            params: rc.params.clone(),
                            body: rc.body.clone(),
                            is_python: rc.is_python,
                            captured_env: deep_clone_captured_env(&rc.captured_env),
                            return_type: rc.return_type.clone(),
                        })
                    })
                    .collect();
                (k.clone(), new_overloads)
            })
            .collect();

        let gen_methods = self
            .gen_methods
            .iter()
            .map(|(k, rc)| {
                (
                    k.clone(),
                    Rc::new(GeneratorFnValue {
                        name: rc.name.clone(),
                        params: rc.params.clone(),
                        body: rc.body.clone(),
                        captured_env: deep_clone_captured_env(&rc.captured_env),
                    }),
                )
            })
            .collect();

        let class_vars = self
            .class_vars
            .iter()
            .map(|(k, v)| (k.clone(), v.deep_clone()))
            .collect();
        let static_vars = self
            .static_vars
            .iter()
            .map(|(k, rc)| (k.clone(), Rc::new(RefCell::new(rc.borrow().deep_clone()))))
            .collect();
        let field_defaults = self
            .field_defaults
            .iter()
            .map(|(n, v, m)| (n.clone(), v.deep_clone(), *m))
            .collect();

        ClassValue {
            name: self.name.clone(),
            class_id: self.class_id,
            bases: self.bases.clone(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability: self.field_mutability.clone(),
            field_index: self.field_index.clone(),
            field_count: self.field_count,
            field_mutability_vec: self.field_mutability_vec.clone(),
            field_access: self.field_access.clone(),
            method_access: self.method_access.clone(),
            static_method_names: self.static_method_names.clone(),
            class_method_names: self.class_method_names.clone(),
            static_vars,
            new_type_base: self.new_type_base.clone(),
            is_exception: self.is_exception,
        }
    }
}

impl Value {
    /// Create a fully independent deep copy with no shared Rc pointers.
    /// Used before sending values across thread boundaries for async tasks.
    pub fn deep_clone(&self) -> Value {
        match self {
            Value::Int(n) => Value::Int(*n),
            Value::UInt(n) => Value::UInt(*n),
            Value::Float(f) => Value::Float(*f),
            Value::Complex(re, im) => Value::Complex(*re, *im),
            Value::Str(s) => Value::Str(s.clone()),
            Value::Bool(b) => Value::Bool(*b),
            Value::None => Value::None,
            Value::Undefined => Value::Undefined,
            Value::List(rc) => {
                let v = rc.borrow().iter().map(|x| x.deep_clone()).collect();
                Value::List(Rc::new(RefCell::new(v)))
            }
            Value::FrozenList { state, layout } => {
                let old = state.borrow();
                Value::FrozenList {
                    state: Rc::new(RefCell::new(FlatListData {
                        data: old.data.clone(),
                        len: old.len,
                        allocated_size: old.allocated_size,
                    })),
                    layout: Rc::new(FlatLayout {
                        class_name: layout.class_name.clone(),
                        fields: layout.fields.clone(),
                        stride: layout.stride,
                        class: Rc::new(layout.class.deep_clone()),
                    }),
                }
            }
            Value::Set(rc) => {
                let v = rc.borrow().iter().map(|x| x.deep_clone()).collect();
                Value::Set(Rc::new(RefCell::new(v)))
            }
            Value::Dict(rc) => {
                let b = rc.borrow();
                let mut d = DictData::new(b.key_type.clone(), b.item_type.clone());
                for (k, v) in b.iter() {
                    let key_val = match k {
                        DictKey::Int(n) => Value::Int(*n),
                        DictKey::Str(s) => Value::Str(s.clone()),
                        DictKey::Bool(b) => Value::Bool(*b),
                        DictKey::None => Value::None,
                    };
                    d.set(key_val, v.deep_clone());
                }
                Value::Dict(Rc::new(RefCell::new(d)))
            }
            Value::Tuple(rc) => {
                let vals = rc.all_values().iter().map(|x| x.deep_clone()).collect();
                Value::Tuple(Rc::new(TupleData::new(vals, rc.all_types().to_vec())))
            }
            Value::Slice(s) => Value::Slice(Rc::new(SliceValue {
                begin: s.begin.as_ref().map(|v| v.deep_clone()),
                end: s.end.as_ref().map(|v| v.deep_clone()),
                step: s.step.as_ref().map(|v| v.deep_clone()),
            })),
            Value::Instance(rc) => {
                let b = rc.borrow();
                let fields: Vec<Option<(Value, bool)>> = b
                    .fields
                    .iter()
                    .map(|slot| slot.as_ref().map(|(v, m)| (v.deep_clone(), *m)))
                    .collect();
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    class_id: b.class_id,
                    flags: b.flags,
                    class: Rc::new(b.class.deep_clone()),
                    fields,
                })))
            }
            Value::Function(rc) => Value::Function(Rc::new(FnValue {
                name: rc.name.clone(),
                params: rc.params.clone(),
                body: rc.body.clone(),
                is_python: rc.is_python,
                captured_env: deep_clone_captured_env(&rc.captured_env),
            return_type: None,
            })),
            Value::OverloadedFn(fns) => Value::OverloadedFn(
                fns.iter()
                    .map(|rc| {
                        Rc::new(FnValue {
                            name: rc.name.clone(),
                            params: rc.params.clone(),
                            body: rc.body.clone(),
                            is_python: rc.is_python,
                            captured_env: deep_clone_captured_env(&rc.captured_env),
                            return_type: rc.return_type.clone(),
                        })
                    })
                    .collect(),
            ),
            Value::GeneratorFn(rc) => Value::GeneratorFn(Rc::new(GeneratorFnValue {
                name: rc.name.clone(),
                params: rc.params.clone(),
                body: rc.body.clone(),
                captured_env: deep_clone_captured_env(&rc.captured_env),
            })),
            Value::Generator(rc) => {
                let b = rc.borrow();
                let vals = b.values.iter().map(|v| v.deep_clone()).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: vals,
                    index: b.index,
                })))
            }
            Value::Class(rc) => Value::Class(Rc::new(rc.deep_clone())),
            Value::Namespace(rc) => {
                let members = rc
                    .members
                    .iter()
                    .map(|(k, v)| (k.clone(), v.deep_clone()))
                    .collect();
                Value::Namespace(Rc::new(NamespaceData {
                    name: rc.name.clone(),
                    members,
                }))
            }
            // TemplateFn / TemplateClass / TemplateGenFn contain only Clone data (no RefCell)
            Value::TemplateFn(rc) => Value::TemplateFn(rc.clone()),
            Value::TemplateClass(rc) => Value::TemplateClass(rc.clone()),
            Value::TemplateGenFn(rc) => Value::TemplateGenFn(rc.clone()),
            // Arc-wrapped types: atomic refcount, safe to share across threads
            Value::PyObject(arc) => Value::PyObject(Arc::clone(arc)),
            Value::NativeFunction(arc) => Value::NativeFunction(Arc::clone(arc)),
            Value::ResultVal { ok, inner } => Value::ResultVal {
                ok: *ok,
                inner: Box::new(inner.deep_clone()),
            },
            // Primitive type tags, async values — just clone
            other => other.clone(),
        }
    }
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
    /// 通常終了。次の文へ制御を移す。
    Normal,
    /// `break` 文が実行された。最も内側のループを抜ける。
    Break,
    /// `continue` 文が実行された。ループの次のイテレーションへ進む。
    Continue,
    /// `return expr` 文が実行された。現在の関数を即座に終了して値を返す。
    Return(Value),
    /// `block_return expr` 文が実行された。`block:` / `if` / `match` / `for` / `while` 式を即座に終了して値を返す。
    BlockReturn(Value),
    /// `loop_yield expr` 文が実行された。実行を継続しつつ値を結果リストに積む。`for`/`while` 式でのみ有効。
    BlockYield(Value),
    /// コールスタックを遡って伝播中の言語レベル例外。`try/except` で捕捉される。
    Raise(RaisedError),
}