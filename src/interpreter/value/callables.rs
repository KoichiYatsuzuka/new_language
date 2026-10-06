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
    /// 定義したモジュールの大域（`Interpreter::global_scopes` の添字・0 がメイン）。
    /// 本体は**この大域で**名前を引く（[`FnValue::globals`] と同じ）。
    pub globals: u32,
    /// 定義したときに走っていたクラス（[`FnValue::owner_class`] と同じ）。
    pub owner_class: Option<Rc<ClassValue>>,
}


/// テンプレートジェネレータ関数（`gen f[T](...)`）の実行時の値。
///
/// ⚠ 本体は持たない。具体化は展開時に作り（D36・タスク 2-16）、定義された時点で
///   `(このテンプレート, 型引数)` の組で登録される（`register_mono_instance`）。
///   この値はその組の鍵と、制約の検査（`check_template_constraints`）に使うだけ。
#[derive(Debug)]
pub struct TemplateGenFnValue {
    pub name: String,
    pub template_params: Vec<crate::ast::TemplateParam>,
}


/// 中断しているジェネレータ本体（bug_fix.md B13）。
///
/// ⚠⚠ **共有スタックは使えない**。中断中もオペランドスタックを保持するので、
/// `buf` を自前で持つ（CPython が `gi_iframe` に記憶域を持つのと同じ理由）。
pub struct GenProducer {
    /// ジェネレータ関数名。トレースバックと**デバッガの呼び出し深さ**に使う。
    pub(crate) name: String,
    /// 本体のチャンク。
    pub(crate) chunk: std::rc::Rc<crate::vm::Chunk>,
    /// **自前の** ローカル＋オペランドスタック。
    pub(crate) buf: Vec<Value>,
    /// 再開位置・例外ハンドラ・セル表。
    pub(crate) frame: crate::vm::run::Frame,
    /// メソッドのときの所属クラス（再開のたび `current_class` を張り直す）。
    pub(crate) self_class: Option<std::rc::Rc<super::ClassValue>>,
    /// 定義したモジュールの大域（再開のたびに張り直す・[`FnValue::globals`]）。
    pub(crate) globals: u32,
}

// ⚠ `Chunk` は `Debug` を実装しないので手書きする。
impl std::fmt::Debug for GenProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenProducer")
            .field("ip", &self.frame.ip)
            .finish()
    }
}

/// インスタンス化済みジェネレータオブジェクトの実行時状態。
///
/// 実態は「**既に手元にある値の列** ＋ **追加を生成する関数**」（B13）。
///
/// - `values` / `index`: 実体化済みの値。`range` / `zip` / list のイテレータ等はこれだけ使う。
/// - `producer`: 中断可能な本体。`None` なら実体化済み。
///
/// ⚠ 取り出し済みの値は**保持しない**（一度限り・メモリ一定）。
#[derive(Debug)]
pub struct GeneratorState {
    pub values: Vec<Value>,
    pub index: usize,
    /// 中断可能な本体。**枝が尽きたら `None`**（枟渇をこれで表す）。
    pub(crate) producer: Option<Box<GenProducer>>,
    /// 本体を実行中か。
    ///
    /// ⚠⚠ `producer` を `take()` してから走らせる（借用を握ったまま再開すると
    /// `RefCell already mutably borrowed` で**プロセスごと落ちる**）ので、
    /// 「取り出し中」と「枟渇」を `producer` だけでは区別できない。
    /// この旗で再入を弾く（CPython の `generator already executing` 相当）。
    pub(crate) running: bool,
    /// **複製できなかった**中断中ジェネレータの複製か（bug_fix.md B13）。
    ///
    /// ⚠⚠ 実行中のフレームは複製できない（CPython も `deepcopy` を拒否する）。
    /// `deep_clone`（`async` 提出時の深いコピー等）は `Value` を返すだけでエラーを返せないので、
    /// **印を付けておいて最初の取り出しで落とす**。印が無いと「黙って枯渇したジェネレータ」に
    /// なり、🔴 サイレントに空の結果を返す。
    pub(crate) poisoned: bool,
}

impl GeneratorState {
    /// **実体化済み**のイテレータを作る（`range` / `zip` / list・set・str の走査等）。
    ///
    /// ⚠ フィールドを直接並べる形は 14 箇所あり、B13 だけで 3 回（`producer` /
    /// `running` / `poisoned`）全箇所を触った。**新しいフィールドはここだけ**で済ませる。
    pub(crate) fn materialized(values: Vec<Value>) -> Self {
        GeneratorState { values, index: 0, producer: None, running: false, poisoned: false }
    }
}


/// テンプレート関数（`fn f[T: Trait](...)`）の実行時の値。
///
/// ⚠ 本体は持たない。具体化は展開時に作り（D36・タスク 2-16）、定義された時点で
///   `(このテンプレート, 型引数)` の組で登録される（`register_mono_instance`）。
///   この値はその組の鍵と、制約の検査（`check_template_constraints`）に使うだけ。
#[derive(Debug)]
pub struct TemplateFnValue {
    pub name: String,
    pub template_params: Vec<crate::ast::TemplateParam>,
}


/// テンプレートクラス（`class C[T: Trait]:`）の実行時の値（[`TemplateFnValue`] と同じ役割）。
#[derive(Debug)]
pub struct TemplateClassValue {
    pub name: String,
    pub template_params: Vec<crate::ast::TemplateParam>,
}


/// [`FnValue::globals`] の特別な値: **呼び出し側の大域のまま**走らせる（名前空間の分離）。
///
/// 組み込みが合成する関数（例外クラスの `__init__` など）は本体が引数と `self` しか引かないので、
/// どのモジュールにも属さない。⚠ `switch_globals` は知らない添字を差し替えないので、これで足りる。
pub const GLOBALS_OF_CALLER: u32 = u32::MAX;

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
    pub body: std::rc::Rc<[Stmt]>,
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
    /// 定義したモジュールの大域（`Interpreter::global_scopes` の添字・0 がメイン）。
    ///
    /// ⚠⚠ 本体の自由な名前は**定義したモジュールの大域で**引く（Python の関数が持つ globals と同じ）。
    /// 呼び出しの間だけ `scopes[0]` をこの大域へ差し替える（`Interpreter::switch_globals`）。
    /// 以前は `import` のたびにモジュールの名前を**呼び出し側の大域へ流し込んで**引かせていたので、
    /// 呼び出し側の名前空間が侵されていた（同名の `const` が再宣言になる・同名の関数が
    /// 多重定義として合成されて呼び出し側の関数が乗っ取られる・実測）。
    pub globals: u32,
    /// **定義したときに走っていたクラス**（アクセス制御と `Self` の文脈・フェーズ10 10-6）。
    ///
    /// メソッドの本体の中で作った入れ子の関数（`fn` / `gen`）だけが持つ。呼び出すとこのクラスの
    /// 文脈で走るので、`private:` のメンバーに届く（型検査と同じ**字句の**規則）。
    /// ⚠ 以前は入れ子の関数を呼ぶと文脈が消え、型検査が通した `self.secret` が実行時に
    ///   `AccessError` になっていた（実測）。
    /// ⚠ クラスのメソッドは持たない（`None`）。インスタンスメソッドは `self` のクラス、
    ///   静的メソッド・クラスメソッドは呼び出したクラス（`Interpreter::class_call_ctx`）が文脈。
    ///   持たせるとクラス → メソッド → クラスの循環になる。
    pub owner_class: Option<Rc<ClassValue>>,
}


/// `TypeTag::Other` のフィールドに対する追加の実行時判定（A-4）。
///
/// `ClassValue::field_checks` の要素。`store_field` はインスタンスの値を入れる前にこれを見る。
///
/// ⚠⚠ **Arrow はクラス継承を許していない**（`class C(Base)` の `Base` は trait だけ。
/// 許さない旨は `parser/classes.rs` が
/// `cannot inherit from ... (only traits are allowed as bases)` で弾く）。
/// だから「クラス型フィールドはクラス名の**完全一致**」で判定でき、祖先を遡る必要が無い。
/// この前提が崩れたら（クラス継承を入れたら）`Class` の判定を継承チェーン探索へ変えること。
#[derive(Debug, Clone, PartialEq)]
pub enum FieldCheck {
    /// 判定しない（`int`/`float`/`str`/`bool` は `field_tags` が見る。
    /// 型変数・未定義名・`list[T]`・`Union[...]`・後から定義されるクラスもここ）。
    None,
    /// クラス型。インスタンスのクラス名が**完全一致**すること（継承が無いので一致のみ）。
    Class(std::rc::Rc<str>),
    /// trait 型。インスタンスの `bases` にこの trait を含むこと（または自身が同名）。
    Trait(std::rc::Rc<str>),
    /// protocol 型。**構造的**に満たすかを見る（必須メンバーが全て在ること）。
    Protocol(std::rc::Rc<[String]>),
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
    /// スロットインデックス → **実行時型判定タグ**（`build_field_index` が組む）。
    ///
    /// ⚠⚠ **これが無いと boxed レイアウトのインスタンスは型を一切検査しない。**
    /// `store_field` の raw レイアウト経路は「値がスロット形式に合うか」を見るが、
    /// boxed 経路は受け取った値をそのまま入れていた。raw レイアウトが付くのは
    /// 「trait 継承なし・全フィールドが int/float 系」のときだけなので、`str` が 1 つ
    /// 混ざる／trait を 1 つ実装する／**テンプレートクラスである**だけで検査が消え、
    /// 静的検査が届かない経路（FFI・ネイティブコールバック・`Any` 経由）から
    /// **黙って不整合な型が入っていた**。
    ///
    /// ⚠ `field_mutability_vec` と**同じ順序・同じ個数**（`build_field_index` が同じループで積む）。
    pub field_tags: Vec<crate::vm::op::TypeTag>,
    /// スロットインデックス → **`TypeTag::Other` のフィールドの追加判定**（`build_field_index` が組む）。
    ///
    /// `TypeTag` は `int`/`float`/`str`/`bool` しか区別できないので、クラス型・trait 型の
    /// フィールドは `Other` に落ちて**素通り**していた（A-3 時点の残り穴）。結果、
    ///
    /// ```arrow
    /// mut xs: list = [Cat(2), 3]   # 要素型なし ⇒ xs[0] は Unresolved
    /// h.pet = xs[0]                # pet: Dog なのに Cat が黙って入る
    /// ```
    ///
    /// ⚠ **判定は「クラス定義時に 1 度だけ」行う。** 実行時にはクラス名のレジストリが
    /// 無く、毎回スコープを引くのは代入の hot path には重い。定義時に注釈を
    /// protocol / trait / クラス / 判定不能へ分類して畳んでおく。
    ///
    /// ⚠ **判定できなければ通す**（`FieldCheck::None`）。型変数・未定義名・後から定義される
    /// クラスはここに落ちる。取りこぼす方へ倒す（`field_tags` と同じ方針）。
    ///
    /// ⚠ `field_mutability_vec` / `field_tags` と**同じ順序・同じ個数**。
    pub field_checks: Vec<FieldCheck>,
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
    /// 単相化したクラスのときの**具体化の名前**（`Stack[int]`）。通常のクラスは `None`（フェーズ10 10-10）。
    ///
    /// ⚠ `name` は単相化しても**テンプレートの名前**（`Stack`・タスク 2-8。表示と `Stack` 注釈との照合を
    ///   変えないため）なので、`x is Stack[int]` / `x mustbe Stack[int]` を判定する材料が無かった。
    ///   `value_is_type` がこちらと突き合わせる。綴りはパーサの正規形（空白なし）。
    pub instance_name: Option<String>,
    /// **定義したモジュールの名前**（`import tags` なら tags・入れ子は `a.b`）。メインのクラスは `None`
    /// （フェーズ10 10-8）。
    ///
    /// ⚠ 型検査はモジュールのクラスを `tags.Tag` の名前で扱う（メインや別のモジュールの同名クラスと
    ///   混ざらないように）。型検査が付けた実行時の検査（`CheckBefore`）・`is` / `mustbe` にも
    ///   `tags.Tag` が来るので、`value_is_type` が `name` と合わせて突き合わせる。表示名（`name`）は変えない。
    pub module_name: Option<Rc<str>>,
    /// **enum のメンバーのクラス**なら、その enum の名前（`enum Color` のメンバーなら `Color`）。
    /// 通常のクラスと、メンバーを持つ enum のクラス自身は `None`。
    ///
    /// ⚠⚠ 「enum のメンバーか」は**この印だけで判定する**（等値・ハッシュ・`open()` の引数）。
    ///   以前はクラス名の接頭辞 `enum_item_` で判定していたが、メンバーのクラス名を enum 名に
    ///   揃えると名前では見分けられない（`implementation_plans/enum_member_type_plan.md`）。
    /// ⚠ 等値（`values_eq_at`）とハッシュ（`hash_into`）は**同じ判定**を使うこと。
    ///   ずれると「入れたのに引けない辞書」になる。
    pub enum_of: Option<String>,
}


impl ClassValue {
    /// このクラスが `type_name` か、その派生か（`except` の照合・`x is T`・フィールドの型検査）。
    ///
    /// ⚠⚠ **`Exception` はすべての例外の基底**（フェーズ10 10-19・Python と同じ）。組み込みの例外
    ///   （`ValueError` など）も、`Error` を実装した利用者の例外（`class MyErr(Error)`）も `Exception` の派生。
    ///   以前は組み込みの例外の基底が `Error`（trait）だけで、`Exception` はその**兄弟**だったため、
    ///   `except Exception` が `RuntimeError` も利用者の例外も捕まえなかった（まとめて捕まえる手段が無かった）。
    /// ⚠ 基底は `bases` を見るだけ（推移をたどらない）。Python のクラスの継承は定義時に祖先まで平坦化して
    ///   載せる（`exec_class_def`）。Arrow のクラスは trait しか継承できない。
    pub(crate) fn is_a(&self, type_name: &str) -> bool {
        self.name == type_name
            || self.bases.iter().any(|b| b == type_name)
            || (type_name == "Exception"
                && (self.is_exception || self.bases.iter().any(|b| b == "Error" || b == "Exception")))
    }

    /// 合成クラス（組み込み型・`enum` の実体型・`new_type` ラッパー等）の**土台**（#80）。
    ///
    /// `name` と `class_id` だけを受け取り、残りは「空」の既定値で埋める。
    /// 呼び出し側は `ClassValue { field_index, .., ..ClassValue::synthetic(name, id) }` の形で
    /// **既定と違うところだけ**を書く。
    ///
    /// ⚠⚠ **`Default` は実装しない。** `..Default::default()` を許すと
    /// 「フィールドを足したら各所で考える」強制が消えるため。
    /// `ClassValue` にフィールドを足すと**まずここがコンパイルエラーになる**（#59 と同じ仕掛け）。
    /// ⇒ 既定値を決める場所が 1 つに定まる。**`..` を書き足して黙らせないこと。**
    ///
    /// ⚠ exhaustive なリテラルは **src に 2 つだけ**で、答える問いが違う:
    /// ここは「**合成クラスの既定値は何か**」、[`Self::deep_clone`] は
    /// 「**そのフィールドはどう深いコピーを作るか**」。フィールドを足すと両方が止まる。
    ///
    /// ⚠ `class_id` を**引数で受ける**のは、`alloc_class_id()` の**呼ばれる順序**を
    /// 呼び出し側に残すため（#80 以前は各リテラルの中で採番していた。ここで採番すると
    /// 構造体更新記法の評価順が後ろになり、**採番の順序が変わる**）。
    pub fn synthetic(name: impl Into<String>, class_id: u32) -> ClassValue {
        ClassValue {
            name: name.into(),
            class_id,
            bases: vec![],
            methods: HashMap::new(),
            gen_methods: HashMap::new(),
            field_defaults: vec![],
            class_vars: HashMap::new(),
            field_mutability: HashMap::new(),
            field_index: HashMap::new(),
            field_count: 0,
            field_mutability_vec: vec![],
            field_tags: vec![],
            field_checks: vec![],
            field_access: HashMap::new(),
            method_access: HashMap::new(),
            static_method_names: HashSet::new(),
            class_method_names: HashSet::new(),
            static_vars: HashMap::new(),
            new_type_base: None,
            is_exception: false,
            raw_layout: None,
            instance_name: None,
            module_name: None,
            enum_of: None,
        }
    }

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
                            globals: rc.globals,
                            // クラスのメソッドは `owner_class` を持たない（`self` か呼び出したクラスが文脈）。
                            owner_class: None,
                            name: rc.name.clone(),
                            params: rc.params.clone(),
                            // ⚠ **`Rc` を clone してはいけない**（#45/#15）。`ClassValue::deep_clone`
                            // はスレッド送出経路で使われるので、中身を複製して独立させる。
                            body: std::rc::Rc::from(&rc.body[..]),
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
                        globals: rc.globals,
                        owner_class: None,
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
            field_tags: self.field_tags.clone(),
            field_checks: self.field_checks.clone(),
            field_access: self.field_access.clone(),
            method_access: self.method_access.clone(),
            static_method_names: self.static_method_names.clone(),
            class_method_names: self.class_method_names.clone(),
            static_vars,
            new_type_base: self.new_type_base.clone(),
            is_exception: self.is_exception,
            raw_layout: self.raw_layout.clone(),
            instance_name: self.instance_name.clone(),
            // ⚠ `Rc` をスレッドへ持ち出さない（#15）。
            module_name: self.module_name.as_deref().map(Rc::from),
            enum_of: self.enum_of.clone(),
        }
    }
}
