// value/callables.rs — 関数・クラス・インスタンス値型: CapturedVar / FnValue / Generator/Template 各種 / ClassValue(+impl)。

use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::rc::Rc,
    crate::ast::{Accessibility, Param, Stmt},
};
use super::*;


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
    /// 定義サイト共有のコンパイル済み本体（#30）。`Op::MakeFn` で作られたクロージャだけが持つ。
    ///
    /// `Some` なら `get_or_compile_chunk` は `Interpreter::vm_chunks`（`FnValue` アドレスが
    /// キー＝**実体ごとに再コンパイル**）を引かずにこちらを使う。`None`（ツリーウォークの
    /// `exec_fn_def` 由来・テンプレート実体化・`deep_clone` 由来）は従来どおり。
    /// ⚠ **`deep_clone` では必ず `None`**（スレッドへ `Rc` を持ち出さない・#15）。
    pub vm_chunk: Option<crate::vm::chunk::SharedFnChunk>,
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
    /// raw ブロックレイアウト記述子（全フィールドがプリミティブ + trait 継承なしのクラスのみ）。
    /// Some のときインスタンスは `INST_HAS_RAW_LAYOUT` で生成され、フィールドは
    /// `InstanceData.raw` の C ABI レイアウト領域に格納される。
    pub raw_layout: Option<Rc<RawLayout>>,
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
                            // ⚠ スレッドへ送る複製では定義サイトの `Rc` を持ち出さない（#15/#30）。
                            vm_chunk: None,
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
            raw_layout: self.raw_layout.clone(),
        }
    }
}
