#![allow(dead_code)]

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

/// 関数呼び出しの1引数。位置引数またはキーワード引数のいずれかを表す。
///
/// # バリアント
/// - `Positional(Expr)`                     : 位置引数。式の値をそのまま渡す。
/// - `Keyword { name: String, value: Expr }`: キーワード引数。`name=value` の形式で渡す。
#[derive(Debug, Clone)]
pub enum CallArg {
    /// 位置引数: `f(expr)` の `expr` 部分。
    Positional(Expr),
    /// キーワード引数: `f(name=expr)` の形式。
    Keyword { name: String, value: Expr },
}

impl CallArg {
    /// 引数の種類（位置引数・キーワード引数）を問わず、内包する式への参照を返す。
    ///
    /// # 戻り値
    /// 引数として渡された式への参照。
    pub fn expr(&self) -> &Expr {
        match self {
            Self::Positional(e) | Self::Keyword { value: e, .. } => e,
        }
    }
}

/// 関数定義の仮引数（パラメータ）。
///
/// # フィールド
/// - `name`     : パラメータ名（例: `x`, `self`）。
/// - `mutable`  : `mut` 修飾子が付いているかどうか。`true` なら呼び出し先で変更可能。
/// - `type_ann` : 型アノテーション文字列（例: `"int"`, `"str"`）。`self` は省略可能。
/// - `default`  : デフォルト値の式。省略時は `None`（必須パラメータ）。
#[derive(Debug, Clone)]
pub struct Param {
    /// パラメータ名（例: `x`, `self`）。
    pub name: String,
    /// `mut` 修飾子の有無。可変パラメータなら `true`。
    pub mutable: bool,
    /// 型アノテーション文字列（`self` は `None` 可）。
    pub type_ann: Option<String>,
    /// デフォルト値の式。`None` は必須パラメータ。
    pub default: Option<Expr>,
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
    /// 等値比較 `==`
    Eq,
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
            BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
            BinOp::Div => "/", BinOp::FloorDiv => "//", BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==", BinOp::NotEq => "!=",
            BinOp::Lt => "<", BinOp::Gt => ">", BinOp::LtEq => "<=", BinOp::GtEq => ">=",
            BinOp::And => "and", BinOp::Or => "or",
            BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^",
            BinOp::LShift => "<<", BinOp::RShift => ">>",
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
    /// 文字列リテラル（シングル・ダブル・トリプルクォート対応）。
    Str(String),
    /// 真偽値リテラル (`True` / `False`)。
    Bool(bool),
    /// `None` リテラル。
    None,
    /// 変数参照。スコープチェーンからこの名前の値をルックアップする。
    Ident(String),
    /// リストリテラル `[a, b, c]`。要素の式を順に評価して `Value::List` を生成する。
    List(Vec<Expr>),
    /// 属性アクセス `object.attr`。インスタンスフィールドやクラス変数の読み取りに使用する。
    Attr { object: Box<Expr>, attr: String },
    /// トレイト修飾アクセス `object::Trait.attr`。特定のトレイト実装のメソッドを明示的に呼び出す。
    TraitAccess { object: Box<Expr>, trait_name: String, attr: String },
    /// 二項演算 `left op right`。`span` はエラー報告に使用する位置情報。
    BinOp { op: BinOp, left: Box<Expr>, right: Box<Expr>, span: Span },
    /// 単項演算 `op operand`（例: `-x`, `not x`, `~x`）。
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    /// 関数呼び出し `func(args)`。`func` が `TemplateInstantiate` の場合はテンプレート呼び出しになる。
    Call { func: Box<Expr>, args: Vec<CallArg> },
    /// テンプレート型引数適用: `expr[T1, T2]` — テンプレート値に具体的な型引数を与える。
    /// `Call` 式の `func` として使用する。単独の値としては無効。
    TemplateInstantiate { base: Box<Expr>, type_args: Vec<String> },
    /// 添字アクセス: `expr[index]` — 辞書やリストなどのインデックスルックアップ。
    Subscript { object: Box<Expr>, index: Box<Expr> },
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
    },
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
#[derive(Debug, Clone)]
pub enum Stmt {
    /// 式文: 副作用のために式を評価する（例: `print(x)`）。
    Expr(Expr),
    /// 不変変数宣言: `let x = expr`。宣言後の再代入はエラー。
    Let(String, Expr),
    /// 不変定数宣言: `const X = expr`。`let` と同様に不変だが定数であることを明示する。
    Const(String, Expr),
    /// 可変変数宣言: `mut x = expr`。宣言後に再代入可能。
    Mut(String, Expr),
    /// 静的可変変数宣言: `static mut x = expr`。外側関数の全呼び出しでセルを共有する。
    /// `span` はセルの一意キー（初回評価判定）として使用する。
    Static(String, Expr, Span),
    /// 変数への代入: `x = expr`。`span` は型検査・エラー報告に使用する位置情報。
    Assign { name: String, value: Expr, span: Span },
    /// 属性（フィールド）への代入: `obj.attr = expr`。
    AttrAssign { target: Expr, value: Expr },
    /// 属性への複合代入: `obj.attr += expr` など。`op` は複合代入の演算子。
    AttrCompoundAssign { target: Expr, op: BinOp, value: Expr },
    /// 変数への複合代入: `x += expr` など。`span` は型検査・エラー報告に使用する位置情報。
    CompoundAssign { name: String, op: BinOp, value: Expr, span: Span },
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
        /// ループ変数名。各要素がこの名前にバインドされる。
        target: String,
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
    BlockReturn(Expr),
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
    /// クラス本体内の型付きフィールド宣言。
    ///
    /// 構文: `[mut|let|const] name: Type [= default]`
    ///
    /// # フィールド
    /// - `name`     : フィールド名。
    /// - `kind`     : フィールドの種別（`mut` / `let` / `const`）。
    /// - `type_ann` : 型アノテーション文字列（必須）。
    /// - `default`  : デフォルト値の式。`const` フィールドは必須、`mut` / `let` は省略可能。
    /// - `access`   : アクセス可能性（`public` / `private` / `protected`、デフォルトは `public`）。
    Field {
        /// フィールド名。
        name: String,
        /// フィールドの種別（可変インスタンス変数・不変インスタンス変数・クラス変数）。
        kind: FieldKind,
        /// 型アノテーション文字列（例: `"int"`, `"str"`）。クラスフィールドでは必須。
        type_ann: String,
        /// デフォルト値の式。`const` フィールドは必須、`mut` / `let` は `None` 可。
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
        names: Vec<(String, Option<String>)>,
        body: Vec<Stmt>,
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
    fn default() -> Self { Self::Public }
}

/// クラスフィールド宣言の種別。
///
/// クラス本体での変数宣言は `mut` / `let` / `const` の3種類に限られる。
///
/// # バリアント
/// - `Mut`   : 可変インスタンス変数 (`mut name: Type [= default]`)。コンストラクタ以降も再代入可能。
/// - `Let`   : 不変インスタンス変数 (`let name: Type [= default]`)。`__init__` 内の初回代入のみ許可。
/// - `Const` : クラス変数 (`const name: Type = default`)。全インスタンスで共有。代入不可。
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// 可変インスタンス変数 (`mut name: Type [= default]`)。
    /// コンストラクタ以降も `self.name = value` で再代入できる。
    Mut,
    /// 不変インスタンス変数 (`let name: Type [= default]`)。
    /// `__init__` 内での初回代入のみ許可。それ以降の代入は実行時 `TypeError`。
    Let,
    /// クラス変数 (`const name: Type = default`)。必ず初期値が必要。
    /// すべてのインスタンスで共有される。インスタンス経由・クラス名経由どちらでもアクセス可能。
    /// 代入は実行時 `TypeError`。
    Const,
}
