use crate::token::Span;

/// テンプレート型パラメータ（型変数とそのトレイト制約）。
///
/// 関数・クラス・trait の定義時に宣言する型変数を表す。
/// 構文: `T: Trait1 and Trait2`
///
/// # フィールド
/// - `name`        : 型変数の名前（例: `T`, `T1`）。
/// - `constraints` : この型変数が満たすべきトレイト名のリスト（`and` で結合された複数制約）。
#[derive(Debug, Clone)]
pub struct TemplateParam {
    /// 型変数の名前（例: `T`, `T1`）。
    pub name: String,
    /// 型変数が満たすべきトレイト名のリスト（`and` で複数結合可能）。
    pub constraints: Vec<String>,
}

/// ネイティブ typed 呼び出しのインラインキャッシュ（AST への関数ポインタ焼き込み）。
///
/// `Expr::Call` ノードごとに1つ持ち、呼び出し先が「不変バインディングの
/// ネイティブ関数 + typed ABI あり」と初回解決されたときに
/// `Arc<NativeFnRef>`（type-erased）を格納する。以後の実行はスコープ検索・
/// Value マッチを跳ばして直接 typed ディスパッチに入る。
///
/// - `Clone` は空キャッシュを返す（AST コピー・スレッド間 deep_clone ごとに再解決）
/// - 不変バインディングは再代入・再宣言ともに禁止のため、キャッシュは無効化不要
#[derive(Default)]
pub struct NativeCallCache(
    pub std::cell::RefCell<Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>>,
    /// Arrow 関数呼び先のグローバル slot キャッシュ（R4）。
    /// 呼び先が不変グローバル関数（`fn` 定義）と解決されたとき、`scopes[0]` 内の slot 番号を
    /// `(slot_epoch, index)` で焼き込む。以後の同一呼び出しは builtin 判定・名前引き・`name.clone()`
    /// を跳ばして `scopes[0].slot(idx)` から直接ディスパッチする。`Cell<u64>` なので Send 安全。
    pub SlotCache,
    /// メソッド呼び出し（`obj.method(args)`）のインラインキャッシュ（method IC）。
    /// 呼び先が「plain な非 mut-self・単一オーバーロードのインスタンスメソッド
    /// （gen/native/static/class_method でない）」と解決されたとき `class_id` を焼き込む
    /// （`AttrCache` の class_id パッキングを流用、slot/access は未使用）。以後の同一 `class_id`
    /// は gen_methods/native/static/class_method 判定と不変性フィルタを跳ばして直接ディスパッチする。
    /// 非 mut-self に限定するのでインスタンス可変性に依存しない。`Cell<u64>` なので Send 安全。
    pub AttrCache,
);

impl Clone for NativeCallCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for NativeCallCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeCallCache(..)")
    }
}

/// C ABI 準拠のプリミティブ型注釈を Arrow の基底型名に解決する。
///
/// C ABI 型は独立した実行時値型ではなく **storage 型**（クラスフィールドの格納幅・
/// 外部言語境界での変換幅を規定する注釈）。Arrow 内部の実行時値は常に
/// `int`(i64) / `float`(f64) のままで、型検査・codegen 上は基底型の別名として扱う。
/// 詳細は .claude/skills/c-abi-interop/SKILL.md を参照。
///
/// 戻り値: 基底型名（`"int"` / `"float"`）。C ABI 型でなければ `None`。
pub fn c_abi_base_type(ann: &str) -> Option<&'static str> {
    match ann {
        "int8" | "int16" | "int32" | "int64" | "uint8" | "uint16" | "uint32" | "uint64" => {
            Some("int")
        }
        "float32" | "float64" => Some("float"),
        _ => None,
    }
}

/// 変数スロットキャッシュ（代入文の AST 焼き込み）。
///
/// `Stmt::Assign` / `Stmt::CompoundAssign` の対象がグローバル可変変数と初回解決されたとき、
/// インタープリタの `global_slot_cells` レジストリへのインデックスを焼き込む。
/// 以後の実行はスコープ検索（ハッシュ + 多段プローブ）なしの直接 Vec アクセスになる。
///
/// パック形式: 上位 32bit = slot_epoch、下位 32bit = レジストリインデックス + 1。0 = 未解決。
/// `freeze` で対象変数が不変化されると epoch が進み、全キャッシュが自動失効する。
/// `Clone` は空キャッシュを返す（AST コピー・別インタープリタへの持ち出しごとに再解決）。
#[derive(Default)]
pub struct SlotCache(pub std::cell::Cell<u64>);

impl SlotCache {
    /// (epoch, index) をパックして格納する。
    #[inline]
    pub fn fill(&self, epoch: u32, index: u32) {
        self.0.set(((epoch as u64) << 32) | (index as u64 + 1));
    }

    /// キャッシュが `epoch` 世代で有効ならインデックスを返す。
    #[inline]
    pub fn get(&self, epoch: u32) -> Option<usize> {
        let packed = self.0.get();
        if packed != 0 && (packed >> 32) as u32 == epoch {
            Some((packed as u32 - 1) as usize)
        } else {
            None
        }
    }
}

impl Clone for SlotCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// 識別子参照（[`Expr::Ident`]）の**記憶域の解決結果**（Phase R）。
///
/// Phase R のリゾルバ（[`crate::interpreter::resolver`]）がトップレベル関数本体を走査して書き込む。
/// リゾルバは解釈経路の前処理として走るが、ネイティブ codegen も**同じ解決済み AST を消費する**
/// （#11 R2-a′: codegen は slot を採番せずリゾルバの割り当てを収穫する）。
///
/// 以前は `Expr::Ident` / `Expr::LocalRef` / `Expr::GlobalRef` の 3 変種で表していた。
/// 統合の要点は「**未解決かどうかは変種ではなくフィールドの問題**」で、
/// 3 変種のままだと全パスが同じ 3 アームを書く必要があった一方、
/// 名前だけ欲しい大多数のサイトはその区別を使っていなかった。
#[derive(Debug, Clone, Default)]
pub enum Resolution {
    /// 未解決。実行時にスコープチェーンを名前で引く。
    /// リゾルバの対象外（テンプレート本体・実行時合成 AST・入れ子定義）か、
    /// 解決を諦めた（シャドウの可能性がある）名前。
    #[default]
    Unresolved,
    /// 関数 base スコープ（実行時は `scopes[frame_floor]`）内の slot 索引（R1）。
    ///
    /// ローカル読み取りがスコープ遡り＋文字列ハッシュから配列 1 回に置き換わる。
    /// `Expr::Ident` の `name` はフォールバック（境界外時）と一致検証のために保持され続ける。
    Local(u32),
    /// プログラム最上位スコープで宣言された名前（R2-b）。
    ///
    /// `SlotCache` は `scopes[0]` 内の index を `(slot_epoch, index)` で焼く実行時キャッシュ。
    /// **解決結果を AST ノードに置く**ことが要点で、ツリーウォーク（従来はキャッシュ無し）と
    /// VM（従来は `Chunk` 側の別キャッシュ）が同じ問いを別々に解いていたのを同一ノードへ寄せる。
    ///
    /// `slot_epoch` による一括無効化をそのまま使うので、`freeze` による
    /// `SlotCell` → `Immutable` 降格でも安全（epoch が進みキャッシュが失効する）。
    /// `SlotCache::clone` が空を返すため、AST コピー（テンプレート実体化）では自動的に再解決される。
    Global(SlotCache),
}

impl std::fmt::Debug for SlotCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlotCache(..)")
    }
}

/// 属性アクセスのインラインキャッシュ（Phase R / R3）。
///
/// `Expr::Attr`（`obj.attr`）が具象クラスのインスタンスフィールドに解決されたとき、
/// `(class_id, フィールド slot, アクセスレベル)` を焼き込む。次回同じ `class_id` の
/// インスタンスなら、`field_index` の辞書引き・アクセスキーの走査・`format!` 確保を
/// すべて省いて slot を直接読める（[eval/attrs.rs] `eval_attr`）。
/// 多相な呼び出し点（毎回別クラス）では `class_id` 不一致でミスし、その都度再解決＋更新する
/// （単相 IC）。
///
/// パック形式: 上位 32bit = class_id（1 始まり・0=未解決）、bit 30-31 = アクセスレベル
/// （0=Public / 1=Private / 2=Protected）、下位 30bit = slot インデックス。
/// `Clone` は空キャッシュを返す（AST コピーごとに再解決）。
#[derive(Default)]
pub struct AttrCache(pub std::cell::Cell<u64>);

impl AttrCache {
    /// アクセスレベル定数。
    pub const PUBLIC: u8 = 0;

    /// (class_id, slot, access) をパックして格納する。
    #[inline]
    pub fn fill(&self, class_id: u32, idx: usize, access: u8) {
        let packed =
            ((class_id as u64) << 32) | (((access & 0x3) as u64) << 30) | (idx as u64 & 0x3FFF_FFFF);
        self.0.set(packed);
    }

    /// `class_id` が一致すれば `(slot, access)` を返す。
    #[inline]
    pub fn get(&self, class_id: u32) -> Option<(usize, u8)> {
        let packed = self.0.get();
        if packed != 0 && (packed >> 32) as u32 == class_id {
            let access = ((packed >> 30) & 0x3) as u8;
            let idx = (packed & 0x3FFF_FFFF) as usize;
            Some((idx, access))
        } else {
            None
        }
    }
}

impl Clone for AttrCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for AttrCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AttrCache(..)")
    }
}

/// 関数呼び出しの1引数。位置引数・キーワード引数・可変長引数のいずれかを表す。
///
/// # バリアント
/// - `Positional(Expr)`                     : 位置引数。式の値をそのまま渡す。
/// - `Keyword { name: String, value: Expr }`: キーワード引数。`name=value` の形式で渡す。
/// - `Variadic(Vec<Expr>)`                  : 可変長引数。`... = A, B, C` の形式で渡す。
#[derive(Debug, Clone)]
pub enum CallArg {
    /// 位置引数: `f(expr)` の `expr` 部分。
    Positional(Expr),
    /// キーワード引数: `f(name=expr)` の形式。
    Keyword { name: String, value: Expr },
    /// 可変長引数: `f(... = A, B, C)` の形式。呼び出し引数の最後にのみ使用可能。
    Variadic(Vec<Expr>),
}

impl CallArg {
    /// 引数の種類を問わず、内包する式への参照を返す。
    /// `Variadic` の場合は最初の要素への参照を返す（要素が1つ以上あることを前提とする）。
    pub fn expr(&self) -> &Expr {
        match self {
            Self::Positional(e) | Self::Keyword { value: e, .. } => e,
            Self::Variadic(exprs) => exprs.first().expect("variadic must have at least one expression"),
        }
    }
}

/// 関数定義の仮引数（パラメータ）。
///
/// # フィールド
/// - `name`     : パラメータ名（例: `x`, `self`）。可変長パラメータは `"..."`。
/// - `mutable`  : `mut` 修飾子が付いているかどうか。`true` なら呼び出し先で変更可能。
/// - `type_ann` : 型アノテーション文字列（例: `"int"`, `"str"`）。`self` は省略可能。
/// - `default`  : デフォルト値の式。省略時は `None`（必須パラメータ）。
/// - `variadic` : `true` のとき可変長パラメータ（`let ...: T`）。`local::args` に格納される。
#[derive(Debug, Clone)]
pub struct Param {
    /// パラメータ名（例: `x`, `self`）。可変長パラメータは `"..."`。
    pub name: String,
    /// `mut` 修飾子の有無。可変パラメータなら `true`。
    pub mutable: bool,
    /// 型アノテーション文字列（`self` は `None` 可）。
    pub type_ann: Option<String>,
    /// デフォルト値の式。`None` は必須パラメータ。
    pub default: Option<Expr>,
    /// `true` のとき可変長パラメータ (`let ...: T`)。関数内で `local::args` として参照する。
    pub variadic: bool,
}

impl Param {
    /// 外部言語ブリッジ（`import[cpp-*]` / `import[cs-*]` / `import[rs]`）の
    /// 型検査スタブ用パラメータを構築する共通コンストラクタ。
    ///
    /// `writable_ref` は外部言語側の「書き込み可能参照」を表す:
    /// - C/C++ : 非 const ポインタ（`T*` / `VECTOR*`）
    /// - C#    : `ref` / `out`（ELEMENT_TYPE_BYREF）
    /// - Rust  : `&mut self`（`&mut T` 値引数は ABI 非対応で関数ごと除外される）
    ///
    /// `true` のとき Arrow の `mut` パラメータとして扱われ、型チェッカーの
    /// `CallMutParamWithImmutableArg` 検査（Arrow ネイティブ関数と同一規則）が
    /// 「不変（`let`）変数を書き込み参照へ渡す」誤りを全ブリッジで一様に
    /// 静的検出する（.claude/skills/c-abi-interop/SKILL.md P5 参照）。
    pub fn bridge(name: impl Into<String>, type_ann: Option<String>, writable_ref: bool) -> Param {
        Param {
            name: name.into(),
            mutable: writable_ref,
            type_ann,
            default: None,
            variadic: false,
        }
    }
}

/// 二項演算子の種別。
///
/// 算術・比較・論理・ビット演算の全演算子を網羅する。
/// 優先順位はパーサー側で制御される。
///
/// # バリアント
/// - `Add`, `Sub`, `Mul`, `Div`, `FloorDiv`, `Mod`, `Pow` : 算術演算子
/// - `Eq`, `NotEq`, `Lt`, `Gt`, `LtEq`, `GtEq`           : 比較演算子
/// - `And`, `Or`                                           : 論理演算子（短絡評価）
/// - `BitAnd`, `BitOr`, `BitXor`, `LShift`, `RShift`      : ビット演算子
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    /// 加算 `+`
    Add,
    /// 減算 `-`
    Sub,
    /// 乗算 `*`
    Mul,
    /// 除算 `/`（浮動小数点）
    Div,
    /// 整数除算 `//`
    FloorDiv,
    /// 剰余 `%`
    Mod,
    /// べき乗 `**`（右結合）
    Pow,
    /// 等値比較 `==`（値またはフィールドの構造的等値）
    Eq,
    /// 参照等値比較 `===`（参照先が同一のときのみ真）
    RefEq,
    /// 非等値比較 `!=`
    NotEq,
    /// 未満比較 `<`
    Lt,
    /// 超過比較 `>`
    Gt,
    /// 以下比較 `<=`
    LtEq,
    /// 以上比較 `>=`
    GtEq,
    /// 論理積 `and`（短絡評価）
    And,
    /// 論理和 `or`（短絡評価）
    Or,
    /// ビット積 `&`
    BitAnd,
    /// ビット和 `|`
    BitOr,
    /// ビット排他的論理和 `^`
    BitXor,
    /// 左シフト `<<`
    LShift,
    /// 右シフト `>>`
    RShift,
    /// 包含検査 `in`
    In,
    /// 非包含検査 `not in`
    NotIn,
}

impl BinOp {
    /// この演算子に対応するソースコード上の記号文字列を返す。
    ///
    /// エラーメッセージや型検査の診断メッセージ生成に使用する。
    ///
    /// # 戻り値
    /// 演算子の文字列表現（例: `"+"`, `"=="`, `"and"` など）。
    pub fn as_str(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::FloorDiv => "//",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==",
            BinOp::RefEq => "===",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::LShift => "<<",
            BinOp::RShift => ">>",
            BinOp::In => "in",
            BinOp::NotIn => "not in",
        }
    }
}

/// 単項演算子の種別。
///
/// # バリアント
/// - `Neg`    : 算術符号反転 `-x`
/// - `Not`    : 論理否定 `not x`
/// - `BitNot` : ビット反転 `~x`
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    /// 算術符号反転 `-x`
    Neg,
    /// 論理否定 `not x`
    Not,
    /// ビット反転 `~x`
    BitNot,
}

/// 式（Expression）の AST ノード。
///
/// インタープリタが評価すると `Value` を返す構文要素を表す。
///
/// # バリアント
/// - `Int(i64)`       : 整数リテラル。
/// - `Float(f64)`     : 浮動小数点リテラル。
/// - `Str(String)`    : 文字列リテラル。
/// - `Bool(bool)`     : 真偽値リテラル (`True` / `False`)。
/// - `None`           : `None` リテラル。
/// - `Ident(String)`  : 変数名・識別子。スコープから値をルックアップする。
/// - `List(Vec<Expr>)`: リストリテラル `[a, b, c]`。
/// - `Attr`           : 属性アクセス `obj.attr`。
/// - `TraitAccess`    : トレイト修飾アクセス `obj::Trait.attr`。
/// - `BinOp`          : 二項演算 `left op right`。位置情報 `span` を含む。
/// - `UnaryOp`        : 単項演算 `op operand`。
/// - `Call`           : 関数呼び出し `func(args)`。
/// - `TemplateInstantiate` : テンプレート型引数適用 `expr[T1, T2]`。`Call` の `func` として使用する。
/// - `Subscript`      : 添字アクセス `expr[index]`。辞書のキールックアップなどに使用する。
/// - `Dict`           : 辞書リテラル `{key: value, ...}`。評価結果は `dict[Any, Any]` 型。
/// - `Tuple`          : タプルリテラル `(val, val, ...)`。評価結果は `tuple[T1, T2, ...]` 型。
#[derive(Debug, Clone)]
pub enum Expr {
    /// 整数リテラル（10進・16進・8進・2進対応）。
    Int(i64),
    /// 浮動小数点リテラル。
    Float(f64),
    /// 虚数リテラル（例: `2j` → 係数 `2.0`）。評価結果は `Value::Complex(0.0, coeff)`。
    ImaginaryLit(f64),
    /// 文字列リテラル（シングル・ダブル・トリプルクォート対応）。
    Str(String),
    /// 真偽値リテラル (`True` / `False`)。
    Bool(bool),
    /// `None` リテラル。
    None,
    /// `Undefined` リテラル。外部ライブラリのメンバが未定義の場合に用いる特殊型。
    /// 変数への代入は静的型エラー。条件判定・型アノテーション・引数としてのみ使用可能。
    Undefined,
    /// 変数参照。スコープチェーンからこの名前の値をルックアップする。
    ///
    /// `node_id` は AST 型解決層のキー（タスク #15b）。パーサが per-program 採番し、
    /// **0 = 未採番**（実行時に合成した AST・テンプレート置換由来）を意味する。
    /// 型検査は参照サイトごとの型をここへ焼く。参照サイト単位なのが要点で、
    /// 型ガード絞り込み（`if x is int:` は分岐スコープで再 `declare` する実装）により
    /// **同じ変数でも参照位置で型が変わる**ため、変数単位の表では表現できない。
    /// `res` は Phase R のリゾルバが書き込む**記憶域の解決結果**（[`Resolution`]）。
    /// 以前は `Ident` / `LocalRef` / `GlobalRef` の 3 変種に分かれていたが、
    /// 「1 つの概念（識別子参照）に 3 変種」は各パスに同じ 3 アームを書かせるだけだったので
    /// 1 変種＋解決フィールドへ統合した。
    Ident { name: String, node_id: u32, res: Resolution },
    /// リストリテラル `[a, b, c]`。要素の式を順に評価して `Value::List` を生成する。
    List(Vec<Expr>),
    /// 属性アクセス `object.attr`。インスタンスフィールドやクラス変数の読み取りに使用する。
    /// `cache` はインスタンスフィールド解決のインラインキャッシュ（R3・初回解決時に焼き込み）。
    Attr {
        object: Box<Expr>,
        attr: String,
        span: Span,
        cache: AttrCache,
        /// AST 型解決層の node-id（タスク #16）。パーサが per-module 採番。0 = 未採番。
        node_id: u32,
    },
    /// トレイト修飾アクセス `object::Trait.attr`。特定のトレイト実装のメソッドを明示的に呼び出す。
    TraitAccess {
        object: Box<Expr>,
        trait_name: String,
        attr: String,
    },
    /// 二項演算 `left op right`。`span` はエラー報告に使用する位置情報。
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
        /// AST 型解決層の node-id（タスク #16）。パーサが per-module 採番。0 = 未採番。
        node_id: u32,
    },
    /// 単項演算 `op operand`（例: `-x`, `not x`, `~x`）。
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    /// 関数呼び出し `func(args)`。`func` が `TemplateInstantiate` の場合はテンプレート呼び出しになる。
    /// `cache` は typed ネイティブ呼び出しのインラインキャッシュ（初回解決時に焼き込み）。
    Call {
        func: Box<Expr>,
        args: Vec<CallArg>,
        span: Span,
        cache: NativeCallCache,
        /// AST 型解決層の node-id（タスク #16）。0 = 未採番。
        node_id: u32,
    },
    /// テンプレート型引数適用: `expr[T1, T2]` — テンプレート値に具体的な型引数を与える。
    /// `Call` 式の `func` として使用する。単独の値としては無効。
    TemplateInstantiate {
        base: Box<Expr>,
        type_args: Vec<String>,
    },
    /// 添字アクセス: `expr[index]` — 辞書やリストなどのインデックスルックアップ。
    Subscript { object: Box<Expr>, index: Box<Expr>, node_id: u32 },
    /// スライス式: `begin:end` または `begin:end:step`。
    /// 添字 `expr[begin:end:step]` の中でのみ生成される。
    /// begin/end は Optional[Index]、step は Optional[int]。
    Slice {
        begin: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },
    /// 辞書リテラル: `{key: value, ...}` — 評価結果は `dict[Any, Any]` 型の値になる。
    Dict(Vec<(Expr, Expr)>),
    /// タプルリテラル: `(val, val, ...)` — 評価結果は `tuple[T1, T2, ...]` 型の値になる。
    /// 空タプル `()` や単要素タプル `(val,)` も含む。`(expr)` はタプルではなくグループ式。
    Tuple(Vec<Expr>),
    /// セットリテラル: `{val, val, ...}` — 評価結果は `set` 型の値になる。
    /// 空セットは `set()` コンストラクタで生成する（`{}` は空辞書）。
    Set(Vec<Expr>),
    /// ブロック式: `block [->Type]: body`。
    ///
    /// `block_return value` で即座に終了してその値を返す。
    /// `block_yield value` は値を積みながら実行を継続し、ブロック終了時にリストを返す。
    /// どちらも使わない場合は `None` を返す。
    /// `return_type` が `Some` の場合は静的型検査で `block_return`/`block_yield` の型を照合する。
    Block {
        stmts: Vec<Stmt>,
        return_type: Option<String>,
    },
    /// if 式: `if cond [->Type]: body [elif cond: body]* [else: body]`。
    ///
    /// `->Type` アノテーション付きで式として使用する。各分岐の `block_return` が値を返す。
    IfExpr {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
        return_type: Option<String>,
    },
    /// for 式: `for target in iter [->Type]: body`。
    ///
    /// `->list[T]` アノテーションと `block_yield` でリストを構築する。
    /// `->T` アノテーションと `block_return` で単一値を返す。
    ForExpr {
        target: String,
        iter: Box<Expr>,
        body: Vec<Stmt>,
        return_type: Option<String>,
    },
    /// while 式: `while cond [->Type]: body`。
    ///
    /// ForExpr と同様に `block_yield` または `block_return` で値を返す。
    WhileExpr {
        cond: Box<Expr>,
        body: Vec<Stmt>,
        return_type: Option<String>,
    },
    /// match 式: `match subject [->Type]: arms`。
    ///
    /// 各アームの `block_return` が値を返す。
    MatchExpr {
        subject: Box<Expr>,
        arms: Vec<MatchArm>,
        return_type: Option<String>,
    },
    /// キャスト式: `expr => TypeName`。
    ///
    /// インスタンスが `__cast__[TypeName]` 特殊メソッドを持つ場合に呼び出す。
    /// `new_type` へのキャストはコンストラクタ呼び出しに自動変換される。
    /// `let` パラメータへの代入時に型が合わない場合も自動的に適用される。
    Cast {
        /// キャスト対象の式。
        object: Box<Expr>,
        /// キャスト先の型名文字列（例: `"int"`, `"dict[str, int]"`）。
        type_name: String,
        /// エラー報告に使用する位置情報。
        span: Span,
        /// AST 型解決層の node-id（タスク #16）。0 = 未採番。
        node_id: u32,
    },
    /// 型ガード式: `expr is TypeName` または `expr is not TypeName`。
    /// ランタイムでは `Bool` を返す。型検査器は直後の `if` 分岐内でオペランドの型を絞り込む。
    /// - `negated: false` → `is`  （真なら型が一致）
    /// - `negated: true`  → `is not`（真なら型が不一致）
    IsType {
        /// 型を検査する対象の式（通常は変数名 `Ident`）。
        expr: Box<Expr>,
        /// `true` なら `is not`、`false` なら `is`。
        negated: bool,
        /// 比較先の型名（`"int"`, `"MyClass"` など）。
        type_name: String,
        /// エラー報告に使用する位置情報。
        span: Span,
        /// AST 型解決層の node-id（タスク #16）。0 = 未採番。
        node_id: u32,
    },
    /// 動的型アサーション: `expr mustbe Type`。
    /// 実行時に型チェックを行い、一致しなければ `TypeError` を raise する。
    /// 静的型検査では式の型を `Type` として確定する。
    MustBe {
        /// アサーション対象の式。実行時に一度だけ評価される。
        expr: Box<Expr>,
        /// ガード型の完全な文字列（例: `"int"`, `"list[int]"`, `"function[int]->str"`）。
        guard_type: String,
        /// エラー報告に使用する位置情報。
        span: Span,
        /// AST 型解決層の node-id（タスク #16）。パーサが per-module 採番。0 = 未採番。
        /// 型検査が `annotations` へ型・検査指示を焼く際のキー。
        node_id: u32,
    },
    /// デバッガ名前空間アクセス: `dbg::name`。デバッガ REPL 内でのみ有効。
    DebugVar(String),
    /// ローカル名前空間アクセス: `local::name`。可変長引数を持つ関数内で `local::args` として有効。
    LocalVar(String),
}

/// `match` 文の1アームのパターン部分。
///
/// `case` と `is` の2種類があり、1つの `match` 文内での混在はパースエラー。
#[derive(Debug, Clone)]
pub enum MatchPattern {
    /// `case <expr>:` — 値比較パターン。`Expr::Ident("_")` はワイルドカード（常にマッチ）。
    Case(Expr),
    /// `is <TypeName>:` — 型チェックパターン（instanceof 検査）。
    IsType(String),
}

/// `match` 文の1アーム（パターン + ボディ）。
#[derive(Debug, Clone)]
pub struct MatchArm {
    /// パターン部分（`case <expr>` または `is <TypeName>`）。
    pub pattern: MatchPattern,
    /// このアームが選択されたときに実行される文リスト。
    pub body: Vec<Stmt>,
}

/// 文（Statement）の AST ノード。
///
/// インタープリタが実行する構文要素を表す。式文・宣言・制御構文・定義などすべての文種を含む。
///
/// # バリアント（主要なもの）
/// - `Expr(Expr)`             : 式文。副作用のために式を評価する（例: 関数呼び出し単体）。
/// - `Let` / `Const` / `Mut` : 変数宣言（それぞれ不変・不変定数・可変）。
/// - `Assign`                 : 変数への代入 `x = expr`。
/// - `AttrAssign`             : 属性への代入 `obj.attr = expr`。
/// - `AttrCompoundAssign`     : 属性への複合代入 `obj.attr += expr` など。
/// - `CompoundAssign`         : 変数への複合代入 `x += expr` など。
/// - `If`                     : `if` / `elif` / `else` 条件分岐。
/// - `While`                  : `while` ループ。
/// - `For`                    : `for target in iter:` イテレータループ。
/// - `Block`                  : `block:` 無名スコープ。
/// - `Return`                 : `return [expr]` 関数からの返却。
/// - `Break` / `Continue` / `Pass` : ループ制御・空文。
/// - `BlockReturn`            : `block_return expr` — `block:` スコープからの値返却。
/// - `LoopYield`              : `loop_yield expr` — `for`/`while` 式内での値産出（リスト蓄積）。
/// - `Yield`                  : `yield expr` — ジェネレータ関数内での値産出。
/// - `Freeze`                 : `freeze x` — `mut` 変数を `let`（不変）に降格する。
/// - `FnDef`                  : `fn` 関数定義。テンプレート対応。
/// - `GenDef`                 : `gen` ジェネレータ関数定義。
/// - `ClassDef`               : `class` クラス定義。テンプレート対応。
/// - `TraitDef`               : `trait` トレイト定義。
/// - `Field`                  : クラス本体内のフィールド宣言 `[mut|let|const] name: Type`。
/// - `NewTypeDef`             : `new_type NewName: OriginalType` — 新しい型エイリアスの定義。
/// - `Try`                    : `try` / `except` / `finally` 例外処理。
/// - `Raise`                  : `raise [expr]` 例外の送出（または再送出）。

/// タプルアンパック宣言 (`let x, mut y, _ = expr`) における1バインディングスロット。
///
/// パーサーが生成し、型検査器が各スロットの可変性を検証する。
/// `Wildcard` は末尾にのみ配置できる（残余要素をすべて破棄する）。
#[derive(Debug, Clone)]
pub enum TupleTarget {
    /// `let name` — immutable binding
    Let(String),
    /// `mut name` — mutable binding
    Mut(String),
    /// `_` — discard all remaining elements (must be last)
    Wildcard,
    /// `name` with no qualifier — accepted by the parser, rejected by the type checker
    Bare(String),
}

/// 文（Statement）の AST ノード。
///
/// インタープリタが実行するすべての構文要素を表す。式文・変数宣言・制御構文・
/// 関数/クラス/トレイト定義・インポート・非同期タスクなどすべての文種を含む。
#[derive(Debug, Clone)]
pub enum Stmt {
    /// 式文: 副作用のために式を評価する（例: `print(x)`）。
    Expr(Expr),
    /// 不変変数宣言: `let x [: Type] = expr`。宣言後の再代入はエラー。型アノテーションは省略可能。
    Let(String, Option<String>, Expr),
    /// 不変定数宣言: `const X [: Type] = expr`。`let` と同様に不変だが定数であることを明示する。
    Const(String, Option<String>, Expr),
    /// 可変変数宣言: `mut x [: Type] = expr`。宣言後に再代入可能。
    Mut(String, Option<String>, Expr),
    /// タプルアンパック宣言: `let x, mut y, _ = expr`。
    /// `_` は末尾に置いて残余要素をすべて破棄する。
    LetTuple {
        targets: Vec<TupleTarget>,
        value: Expr,
        span: Span,
    },
    /// 静的可変変数宣言: `static mut x = expr`。外側関数の全呼び出しでセルを共有する。
    /// `span` はセルの一意キー（初回評価判定）として使用する。
    Static(String, Expr, Span),
    /// 変数への代入: `x = expr`。`span` は型検査・エラー報告に使用する位置情報。
    /// `slot` はグローバル可変変数への直接アクセス用スロットキャッシュ（初回解決時に焼き込み）。
    Assign {
        name: String,
        value: Expr,
        span: Span,
        slot: SlotCache,
    },
    /// 属性（フィールド）への代入: `obj.attr = expr`。
    AttrAssign { target: Expr, value: Expr },
    /// 属性への複合代入: `obj.attr += expr` など。`op` は複合代入の演算子。
    AttrCompoundAssign {
        target: Expr,
        op: BinOp,
        value: Expr,
    },
    /// 変数への複合代入: `x += expr` など。`span` は型検査・エラー報告に使用する位置情報。
    /// `slot` はグローバル可変変数への直接アクセス用スロットキャッシュ（初回解決時に焼き込み）。
    CompoundAssign {
        name: String,
        op: BinOp,
        value: Expr,
        span: Span,
        slot: SlotCache,
    },
    /// `if` / `elif` / `else` 条件分岐。
    ///
    /// # フィールド
    /// - `branches`  : `(条件式, ボディ)` のペアのリスト（`if` + 0個以上の `elif`）。
    /// - `else_body` : `else` 節のボディ。`else` がなければ `None`。
    If {
        /// `if` および `elif` の各節。`(条件式, ボディ文リスト)` の順に格納される。
        branches: Vec<(Expr, Vec<Stmt>)>,
        /// `else` 節のボディ文リスト。`else` がない場合は `None`。
        else_body: Option<Vec<Stmt>>,
    },
    /// `match (expr):` パターンマッチ文。
    ///
    /// `case` アームは `==` で値を比較し、`is` アームは型を検査する。
    /// `case _:` はワイルドカード（常にマッチ）として扱われる。
    /// 1つの `match` 文内で `case` と `is` を混在させるとパースエラー。
    ///
    /// # フィールド
    /// - `subject` : 検査対象の式。
    /// - `arms`    : マッチアームのリスト。
    /// - `span`    : エラー報告に使用する位置情報。
    Match {
        /// 検査対象の式（`match (x):` の `x`）。
        subject: Expr,
        /// マッチアームのリスト（`case` または `is` のいずれか一種類のみ）。
        arms: Vec<MatchArm>,
        /// エラー報告に使用する位置情報。
        span: Span,
    },
    /// `while cond:` ループ。`break` / `continue` をサポートする。
    ///
    /// # フィールド
    /// - `cond` : ループ継続条件式。
    /// - `body` : ループ本体の文リスト。
    While {
        /// ループ継続条件式。偽になるとループを終了する。
        cond: Expr,
        /// ループ本体の文リスト。
        body: Vec<Stmt>,
    },
    /// `for target in iter:` イテレータループ。
    ///
    /// `iter` 式を評価してイテレータを取得し、`target` に各要素を束縛しながらループする。
    /// `break` / `continue` をサポートする。
    ///
    /// # フィールド
    /// - `target` : ループ変数名（各イテレーション要素が束縛される）。
    /// - `iter`   : イテラブルな値を返す式（リスト・ジェネレータ・カスタムイテラブルなど）。
    /// - `body`   : ループ本体の文リスト。
    For {
        /// ループ変数名のリスト。単一変数なら `vec!["x"]`、タプルアンパックなら `vec!["x", "y"]`。
        targets: Vec<String>,
        /// イテラブルを返す式（リスト・ジェネレータなど）。
        iter: Expr,
        /// ループ本体の文リスト。
        body: Vec<Stmt>,
    },
    /// `block:` 無名スコープ。ブロック内の変数はブロック外に漏れない。
    Block(Vec<Stmt>),
    /// `return [expr]` — 関数からの返却。`None` の場合は `return None` と等価。
    Return(Option<Expr>),
    /// `break` — 最も内側のループを脱出する。
    Break,
    /// `continue` — 最も内側のループの次のイテレーションへ進む。
    Continue,
    /// `pass` — 何もしない空文。構文上ボディが必要な箇所に使用する。
    Pass,
    /// `block_return expr` — `block:` スコープから値を返却して即座に抜ける。
    BlockReturn(Expr, Span),
    /// `loop_yield expr` — `for`/`while` 式内から値を産出してリストに蓄積する。for/while 式の外では実行時エラー。
    LoopYield(Expr),
    /// `yield expr` — ジェネレータ関数内での値産出。
    /// ジェネレータ関数（`gen` キーワードで定義）の本体内でのみ有効。
    Yield(Expr),
    /// `freeze x` — `mut` 変数を `let`（不変）に降格する。
    /// 値に `__freeze__` メソッドがあれば、降格前に呼び出す。
    Freeze(String, Span),
    /// `fn` 関数定義。
    ///
    /// # フィールド
    /// - `name`            : 関数名。
    /// - `template_params` : テンプレート型パラメータのリスト（非テンプレートは空）。
    /// - `params`          : 仮引数リスト。
    /// - `return_type`     : 戻り値の型アノテーション文字列。省略可能（`None` は型検査エラー）。
    /// - `body`            : 関数本体の文リスト。
    /// - `is_abstract`     : trait 内の抽象メソッドかどうか（本体が空/`pass` のみのもの）。
    FnDef {
        /// 関数名。
        name: String,
        /// テンプレート型パラメータのリスト（非テンプレート関数は空リスト）。
        template_params: Vec<TemplateParam>,
        /// 仮引数リスト（`self` を含む場合はリストの先頭に位置する）。
        params: Vec<Param>,
        /// 戻り値の型アノテーション文字列（`None` の場合は型検査で `MissingReturnTypeAnn` エラー）。
        return_type: Option<String>,
        /// 関数本体の文リスト。
        body: Vec<Stmt>,
        /// `true` の場合、trait 内の抽象メソッド宣言（本体が `pass` のみ）。
        is_abstract: bool,
        /// `true` の場合、`static` キーワードで修飾されたスタティックメソッド。`self` を受け取らない。
        is_static: bool,
        /// `true` の場合、`class_method` キーワードで修飾されたクラスメソッド。第1引数は `cls: type[Self]`。
        is_class_method: bool,
        /// `@decorator` 構文で付与されたデコレータ式のリスト（上から順、適用は逆順）。
        decorators: Vec<Expr>,
        /// クラス本体内でのアクセス可能性（デフォルトは `Public`）。クラス外の関数定義では無視される。
        access: Accessibility,
    },
    /// ジェネレータ関数定義: `gen name[T: Trait](params) -> YieldType:`。
    ///
    /// `yield` 文を含む関数を定義する。呼び出し時に `Value::Generator` を返す。
    ///
    /// # フィールド
    /// - `name`            : ジェネレータ関数名。
    /// - `template_params` : テンプレート型パラメータのリスト。
    /// - `params`          : 仮引数リスト。
    /// - `yield_type`      : 各 `yield` で産出される値の型（`Generator[T]` の `T`）。
    /// - `body`            : ジェネレータ本体の文リスト。
    GenDef {
        /// ジェネレータ関数名。
        name: String,
        /// テンプレート型パラメータのリスト（非テンプレートは空リスト）。
        template_params: Vec<TemplateParam>,
        /// 仮引数リスト。
        params: Vec<Param>,
        /// 各 `yield` で産出される値の型（`Generator[T]` の `T`）。省略可能。
        yield_type: Option<String>,
        /// ジェネレータ本体の文リスト。
        body: Vec<Stmt>,
        /// クラス本体内でのアクセス可能性（デフォルトは `Public`）。
        access: Accessibility,
    },
    /// `class` クラス定義。
    ///
    /// # フィールド
    /// - `name`            : クラス名。
    /// - `template_params` : テンプレート型パラメータのリスト（非テンプレートは空）。
    /// - `bases`           : 継承する基底クラス・トレイト名のリスト。
    /// - `body`            : クラス本体の文リスト（フィールド宣言・メソッド定義を含む）。
    ClassDef {
        /// クラス名。
        name: String,
        /// テンプレート型パラメータのリスト（非テンプレートクラスは空リスト）。
        template_params: Vec<TemplateParam>,
        /// 継承する基底クラス・トレイト名のリスト。
        bases: Vec<String>,
        /// `@decorator` 構文で付与されたデコレータ式のリスト（上から順、適用は逆順）。
        decorators: Vec<Expr>,
        /// クラス本体の文リスト（`Field` / `FnDef` / `GenDef` などを含む）。
        body: Vec<Stmt>,
    },
    /// `trait` トレイト定義。
    ///
    /// # フィールド
    /// - `name`            : トレイト名。
    /// - `template_params` : テンプレート型パラメータのリスト。
    /// - `body`            : トレイト本体の文リスト（抽象メソッド宣言・デフォルト実装を含む）。
    TraitDef {
        /// トレイト名。
        name: String,
        /// テンプレート型パラメータのリスト（非テンプレートは空リスト）。
        template_params: Vec<TemplateParam>,
        /// トレイト本体の文リスト（抽象メソッドやデフォルト実装を含む）。
        body: Vec<Stmt>,
    },
    /// `protocol` プロトコル定義。静的型検査のみに使用され、インスタンス化・継承は不可。
    ///
    /// フィールドとメソッドシグネチャ（`...` 本体）のみを宣言する。
    /// 型がプロトコルを満たすかは構造的型付けで検査される。
    ///
    /// # フィールド
    /// - `name` : プロトコル名。
    /// - `body` : プロトコル本体（`Field` と抽象 `FnDef` のみ）。
    ProtocolDef {
        /// プロトコル名。
        name: String,
        /// プロトコル本体（フィールド宣言と抽象メソッドシグネチャ）。
        body: Vec<Stmt>,
    },
    /// クラス本体内の型付きフィールド宣言。
    ///
    /// 構文: `[mut|let|const|static mut] name: Type [= default]`
    ///
    /// `= default` は `const` / `static mut` のみ許可。`mut` / `let` への初期値指定は静的型エラー。
    ///
    /// # フィールド
    /// - `name`     : フィールド名。
    /// - `kind`     : フィールドの種別（`mut` / `let` / `const` / `static mut`）。
    /// - `type_ann` : 型アノテーション文字列（必須）。
    /// - `default`  : デフォルト値の式。`const` は必須、`static mut` は任意、`mut` / `let` は不可（静的型エラー）。
    /// - `access`   : アクセス可能性（`public` / `private` / `protected`、デフォルトは `public`）。
    Field {
        /// フィールド名。
        name: String,
        /// フィールドの種別（可変インスタンス変数・不変インスタンス変数・クラス変数）。
        kind: FieldKind,
        /// 型アノテーション文字列（例: `"int"`, `"str"`）。クラスフィールドでは必須。
        type_ann: String,
        /// デフォルト値の式。`const` フィールドは必須、`mut` / `let` は `None` のみ許可（値があれば静的型エラー）。
        default: Option<Expr>,
        /// アクセス可能性（デフォルトは `Public`）。
        access: Accessibility,
    },
    /// `new_type NewName: OriginalType` — 既存の型と構造的に同一だが名前が異なる新しい型を定義する。
    ///
    /// バインドは常に `const`（再代入はパースエラー）。
    /// 元の型がクラスの場合はクラス定義をコピー、プリミティブの場合はラッパークラスを自動生成する。
    ///
    /// # フィールド
    /// - `name`     : 新しい型の名前。
    /// - `original` : 元の型の名前（クラス名・プリミティブ型名・または別の `new_type` 名）。
    NewTypeDef {
        /// 新しい型の名前。
        name: String,
        /// 元の型の名前（クラス名またはプリミティブ型名）。
        original: String,
    },
    /// `enum Name: variant [= expr] ...` — 整数値に対応する名前付き定数の列挙型定義。
    ///
    /// 各バリアントは `enum_item_Name` 型（`new_type enum_item_Name: int` 相当）のインスタンスとして
    /// クラス `Name` の const メンバーに格納される。値は 0 から始まる自動採番、または明示的に指定可能。
    ///
    /// # フィールド
    /// - `name`     : 列挙型の名前（クラス名としても登録される）。
    /// - `variants` : `(バリアント名, Option<値式>)` のリスト。`None` は自動採番。
    EnumDef {
        /// 列挙型の名前。
        name: String,
        /// バリアントのリスト（名前と省略可能な値式のペア）。
        variants: Vec<(String, Option<Expr>)>,
    },
    /// `try: ... except Type as name: ... finally: ...` 例外処理構文。
    ///
    /// # フィールド
    /// - `body`         : `try` 節の本体文リスト。
    /// - `handlers`     : `except` 節のリスト（複数の `except` 節を持てる）。
    /// - `finally_body` : `finally` 節の本体文リスト。省略可能。
    Try {
        /// `try` 節の本体文リスト。例外が発生するとハンドラに制御が移る。
        body: Vec<Stmt>,
        /// `except` 節のリスト。順に評価され、最初にマッチしたハンドラが実行される。
        handlers: Vec<ExceptHandler>,
        /// `finally` 節の本体文リスト。例外の有無に関わらず常に実行される。
        finally_body: Option<Vec<Stmt>>,
    },
    /// `raise [expr]` — 例外の送出または再送出。
    ///
    /// # フィールド
    /// - `exc`  : 送出する例外式。`None` の場合は現在の例外を再送出する（bare `raise`）。
    /// - `span` : エラー報告に使用する位置情報。
    Raise {
        /// 送出する例外式。`None` の場合は bare `raise`（現在の例外を再送出）。
        exc: Option<Expr>,
        /// エラー報告・スタックトレースに使用する位置情報。
        span: Span,
    },
    /// `import[lang] module.submod as alias` — 外部モジュールのインポート。
    ///
    /// パース時に対象ファイルを読み込み変換した tl AST を `body` に格納する。
    /// 型検査器と実行エンジンはどちらも `body` を参照する。
    ///
    /// # フィールド
    /// - `lang`   : 言語識別子（`"py"` など）
    /// - `module` : モジュールパスの各セグメント（`["os", "path"]` for `os.path`）
    /// - `alias`  : `as alias` で与えたバインド名。`None` の場合は最後のセグメント名を使用
    /// - `body`   : パース済みの tl AST（モジュールの内容）
    Import {
        lang: String,
        module: Vec<String>,
        /// `.h` / header file path for `import[cpp-dll]` and `import[cpp-lib]`.
        with_file: Option<String>,
        alias: Option<String>,
        body: Vec<Stmt>,
    },
    /// `from module import[lang] Name1, Name2 as N2` — 名前を直接スコープに導入するインポート。
    ///
    /// モジュール全体の `body` を保持する点は `Import` と同じ（型検査・キャッシュのため）。
    ///
    /// # フィールド
    /// - `lang`   : 言語識別子（`"py"` など）
    /// - `module` : モジュールパスの各セグメント
    /// - `names`  : `(元の名前, as エイリアス)` のリスト。エイリアスなしは `None`
    /// - `body`   : パース済みの tl AST（モジュールの内容）
    FromImport {
        lang: String,
        module: Vec<String>,
        /// `.h` / header file path for `from import[cpp-dll]` / `import[cpp-lib]`.
        with_file: Option<String>,
        names: Vec<(String, Option<String>)>,
        body: Vec<Stmt>,
    },
    /// `target <- async [->Type]: body` — 非同期タスクを AsyncManager に追加する。
    ///
    /// `target` は `AsyncManager` インスタンスを保持する変数名。
    /// `body` はスレッド上で実行されるブロック本体（`block_return` で値を返す）。
    /// 呼び出し時点でのスコープ変数をディープコピーしてスレッドに渡す。
    AsyncAssign {
        /// AsyncManager を保持する変数名。
        target: String,
        /// 戻り値の型アノテーション（省略可能）。
        return_type: Option<String>,
        /// スレッドで実行される文リスト。
        stmts: Vec<Stmt>,
    },
    /// `break_point` — 実行を一時停止してデバッガ REPL を起動する。
    BreakPoint { span: Span },
    /// `let dbg::name = expr` — デバッガ REPL 内限定の一時変数宣言。再開時に削除される。
    DebugLet(String, Expr),
    /// `source on handler` または `source once handler` — イベントハンドラを購読する。
    ///
    /// - `source`   : `Signal[T]`・`EventSource[T]`・`GoChannel[T]` を返す式。
    /// - `handler`  : ハンドラ関数式（`fn(x): body` または変数名）。
    /// - `is_once`  : `true` の場合 `once` 演算子。呼び出し後に自動解除される。
    /// - `is_async` : `true` の場合、EventLoop 内で別スレッドで実行される非同期ハンドラ。
    /// - `span`     : エラー報告に使用する位置情報。
    EventSubscribe {
        source: Expr,
        handler: Expr,
        is_once: bool,
        is_async: bool,
        span: Span,
    },
    /// `source off handler` — イベントハンドラの購読を解除する。
    ///
    /// - `source`  : `Signal[T]`・`EventSource[T]`・`GoChannel[T]` を返す式。
    /// - `handler` : 解除するハンドラ関数値（`fn(x): ...` の変数名または式）。
    /// - `span`    : エラー報告に使用する位置情報。
    EventUnsubscribe {
        source: Expr,
        handler: Expr,
        span: Span,
    },
}

/// `try` 文内の単一の `except` 節を表す。
///
/// # フィールド
/// - `exc_type` : キャッチする例外型の名前（例: `"ValueError"`）。`None` は bare `except:`（全捕捉）。
/// - `name`     : 捕捉した例外を束縛する変数名（`as e` の `e`）。省略可能。
/// - `body`     : このハンドラの本体文リスト。
#[derive(Debug, Clone)]
pub struct ExceptHandler {
    /// キャッチする例外型名（例: `"ValueError"`）。`None` は bare `except:`（すべての例外を捕捉）。
    pub exc_type: Option<String>,
    /// 捕捉した例外を束縛する変数名（`as e` の `e`）。省略した場合は `None`。
    pub name: Option<String>,
    /// このハンドラの本体文リスト。
    pub body: Vec<Stmt>,
}

/// クラス・トレイトメンバーのアクセス可能性。
///
/// クラス本体内で `public:` / `private:` / `protected:` セクションで指定する。
/// 指定がない場合のデフォルトは `Public`。
///
/// # バリアント
/// - `Public`    : どこからでもアクセス可能（デフォルト）。
/// - `Private`   : 同じクラスのメソッド内からのみアクセス可能。
/// - `Protected` : 同じクラスまたは継承クラス（トレイト）のメソッド内からアクセス可能。
#[derive(Debug, Clone, PartialEq)]
pub enum Accessibility {
    /// どこからでもアクセス可能（デフォルト）。
    Public,
    /// 同じクラスのメソッド内からのみアクセス可能。
    Private,
    /// 同じクラスまたは継承クラスのメソッド内からアクセス可能。
    Protected,
}

impl Default for Accessibility {
    /// デフォルト値として `Public` を返す。
    ///
    /// クラス/トレイト本体でアクセス修飾子が指定されていない場合に使用される。
    fn default() -> Self {
        Self::Public
    }
}

/// クラスフィールド宣言の種別。
///
/// クラス本体での変数宣言は `mut` / `let` / `const` / `static mut` の4種類。
/// `mut` / `let` は初期値不可（静的型エラー）。`const` は初期値必須。`static mut` は初期値任意。
///
/// # バリアント
/// - `Mut`       : 可変インスタンス変数 (`mut name: Type`)。コンストラクタで初期化必須。
/// - `Let`       : 不変インスタンス変数 (`let name: Type`)。`__init__` 内の初回代入のみ許可。
/// - `Const`     : クラス変数 (`const name: Type = default`)。全インスタンスで共有。代入不可。
/// - `StaticMut` : 共有可変クラス変数 (`static mut name: Type [= default]`)。全インスタンスで共有。
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// 可変インスタンス変数 (`mut name: Type`)。初期値は持てない（静的型エラー）。
    /// コンストラクタ（`__init__`）で必ず初期化する。以降も `self.name = value` で再代入可能。
    Mut,
    /// 不変インスタンス変数 (`let name: Type`)。初期値は持てない（静的型エラー）。
    /// `__init__` 内での初回代入のみ許可。それ以降の代入は実行時 `TypeError`。
    Let,
    /// クラス変数 (`const name: Type = default`)。必ず初期値が必要。
    /// すべてのインスタンスで共有される。インスタンス経由・クラス名経由どちらでもアクセス可能。
    /// 代入は実行時 `TypeError`。
    Const,
    /// 静的可変クラス変数 (`static mut name: Type [= default]`)。
    /// すべてのインスタンスで共有される可変セル。インスタンス経由・クラス名経由どちらでもアクセス・代入可能。
    StaticMut,
}
