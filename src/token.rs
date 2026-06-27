use std::sync::Arc;

/// ソースコード上の位置情報。
///
/// 字句解析器が各トークンに付与し、パーサー・型検査器・インタープリタが
/// エラーメッセージの位置表示に利用する。
/// `file` は `Arc<str>` で共有することで、大量のトークンを生成しても
/// ファイル名のメモリコピーを最小化している。
///
/// # フィールド
/// - `file` — ソースファイル名（空文字列の場合は「ファイル不明」扱い）
/// - `line` — 1 始まりの行番号（0 は「位置不明」を表す）
/// - `col`  — 1 始まりの列番号（文字単位）
#[derive(Debug, Clone)]
pub struct Span {
    pub file: Arc<str>,
    pub line: usize, // 1 始まり
    pub col: usize,  // 1 始まり（文字単位）
}

impl Span {
    /// 位置情報が不明なダミーの `Span` を返す。
    ///
    /// 自動生成 AST ノードなど、ソース位置が存在しない場面で使用する。
    /// `line == 0` を「位置不明」の判定条件として利用する。
    ///
    /// # 戻り値
    /// `line = 0`, `col = 0`, `file = ""` の `Span`
    pub fn unknown() -> Self {
        Self {
            file: "".into(),
            line: 0,
            col: 0,
        }
    }
}

impl std::fmt::Display for Span {
    /// 位置情報を人間が読める形式に変換する。
    ///
    /// - `line == 0` の場合は `"<unknown>"` を返す
    /// - `file` が空の場合は `"line N, col M"` 形式
    /// - それ以外は `"ファイル名:行:列"` 形式（エラーメッセージの標準形式）
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const Y: &str = "\x1b[33m";
        const X: &str = "\x1b[0m";
        if self.line == 0 {
            write!(f, "{Y}<unknown>{X}")
        } else if self.file.is_empty() {
            write!(f, "{Y}line {}, col {}{X}", self.line, self.col)
        } else {
            write!(f, "{Y}{}:{}:{}{X}", self.file, self.line, self.col)
        }
    }
}

/// トークンと位置情報のペア。
///
/// 字句解析器が生成するトークン列の各要素。
/// パーサーはこの型のスライスを受け取って構文解析を行う。
///
/// # フィールド
/// - `token` — トークンの種別と値
/// - `span`  — ソースコード上の位置（ファイル名・行・列）
#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub span: Span,
}

/// f-string (f"...{expr}...") の各セグメント。
///
/// - `Lit`  — 補間なしのリテラル文字列部分
/// - `Expr` — `{...}` で囲まれた式のソーステキスト
#[derive(Debug, Clone, PartialEq)]
pub enum FStrPart {
    Lit(String),
    Expr(String),
}

/// 字句解析器が生成する全トークン種別。
///
/// キーワード・演算子・リテラル・識別子・インデント制御トークンなどを網羅する。
/// 各バリアントのカテゴリはインラインコメントで示す。
///
/// # バリアント（カテゴリ別）
///
/// ## 変数宣言キーワード
/// - `Let`    — 不変変数宣言 (`let`)
/// - `Const`  — 定数宣言 (`const`)
/// - `Mut`    — 可変変数宣言 (`mut`)
/// - `Freeze` — 凍結宣言 (`freeze`、将来仕様）
///
/// ## 値リテラル（キーワード形式）
/// - `True` / `False` / `None` — 真偽値・ヌル値
///
/// ## 論理演算子（キーワード形式）
/// - `And` / `Or` / `Not` — 論理積・論理和・否定
///
/// ## 比較キーワード
/// - `In` / `NotIn` — 包含判定（`in` / `not in`）
/// - `Is` / `IsNot` — 同一性判定（`is` / `is not`）
///
/// ## 制御フロー
/// - `If` / `Elif` / `Else` — 条件分岐
/// - `Match` — パターンマッチ
/// - `For` / `While` — ループ
/// - `Break` / `Continue` / `Pass` — ループ制御・空文
/// - `Return` / `Yield` / `YieldFrom` — 関数からの値返却・ジェネレータ
/// - `BlockReturn` / `LoopYield` / `Block` — ブロック式制御
///
/// ## 例外処理
/// - `Try` / `Except` / `Finally` / `Raise` — 例外処理
///
/// ## 定義キーワード
/// - `Fn`       — 関数定義
/// - `Gen`      — ジェネレータ関数定義
/// - `Class`    — クラス定義
/// - `Trait`    — トレイト定義
/// - `Lambda`   — ラムダ式（未実装）
/// - `Template` — テンプレート定義
///
/// ## インポート
/// - `Import` / `From` / `As` — モジュールインポート（未実装）
///
/// ## スコープ制御
/// - `Del` / `Global` / `Nonlocal` — 変数スコープ操作
///
/// ## コンテキストマネージャ・非同期
/// - `With` / `Async` / `Await`
///
/// ## 特殊型キーワード
/// - `SelfType` — クラス・trait 内でのみ使用可能な `Self` 型
/// - `NewType`  — 新しい型名を定義する `new_type` キーワード
/// - `Any`      — 動的型エスケープ `Any`
/// - `Union`    — ユニオン型 `Union[T1, T2, ...]`
/// - `Option`   — オプション型 `Option[T]`
///
/// ## 演算子・区切り記号
/// 各バリアントのインラインコメントに記号を示す。
///
/// ## リテラル
/// - `Int(i64)`    — 整数リテラル値
/// - `Float(f64)`  — 浮動小数点リテラル値
/// - `Str(String)` — 文字列リテラル値
///
/// ## 識別子
/// - `Ident(String)` — 変数名・関数名・クラス名など
///
/// ## インデント制御（Python スタイル）
/// - `Newline` — 論理的な行末
/// - `Indent`  — インデントレベルの増加
/// - `Dedent`  — インデントレベルの減少
///
/// ## その他
/// - `Unknown(char)` — 未知の文字（エラー回復用）
/// - `Eof`           — 入力終端
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Variable declaration
    Let,
    Const,
    Mut,
    Static,
    Freeze,

    // Value literals (keyword form)
    True,
    False,
    None,
    /// `Undefined` — 外部ライブラリのメンバが未定義の状態を表す特殊リテラル型。
    /// 変数への代入は禁止（静的型エラー）。条件判定・型アノテーションのみで使用可能。
    Undefined,

    // Logical operators (keyword form)
    And,
    Or,
    Not,

    // Comparison keywords
    In,
    NotIn,
    Is,
    IsNot,

    // Control flow
    If,
    Elif,
    Else,
    Match,
    Case,
    For,
    While,
    Break,
    Continue,
    Pass,
    Return,
    Yield,
    YieldFrom,
    BlockReturn,
    LoopYield,
    Block,

    // Exception handling
    Try,
    Except,
    Finally,
    Raise,

    // Definitions
    Fn,
    Gen,
    Class,
    Enum,
    Trait,
    Protocol,
    Lambda,
    Template,

    // Import
    Import,
    From,
    As,

    // Scope
    Del,
    Global,
    Nonlocal,

    // Context manager
    With,

    // Async
    Async,
    Await,

    // Assertion
    Assert,

    // Debugger
    BreakPoint,

    // Access modifiers (class body section headers)
    Public,
    Private,
    Protected,

    // Method kind modifiers (class body only)
    ClassMethod, // class_method fn — first param must be cls: type[Self]

    // Self type keyword (valid only inside class/trait bodies)
    SelfType,

    // new_type declaration keyword
    NewType,

    // Any type keyword (dynamic escape hatch; requires explicit downcast to use)
    Any,

    // Union[T1, T2, ...] and Option[T] type keywords
    Union,
    Option,
    // Intersection[T1, T2, ...] type keyword
    Intersection,

    // Event handler keywords
    On,   // on
    Off,  // off
    Once, // once

    // Type assertion
    MustBe, // must be

    // Arithmetic operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    SlashSlash, // //
    Percent,    // %
    StarStar,   // **
    At,         // @

    // Comparison operators
    EqEq,    // ==
    EqEqEq,  // ===
    NotEq,   // !=
    Lt,      // <
    Gt,      // >
    LtEq,    // <=
    GtEq,    // >=

    // Bitwise operators
    Amp,   // &
    Pipe,  // |
    Caret, // ^
    Tilde, // ~
    LtLt,  // <<
    GtGt,  // >>

    // Assignment operators
    Eq,           // =
    PlusEq,       // +=
    MinusEq,      // -=
    StarEq,       // *=
    SlashEq,      // /=
    SlashSlashEq, // //=
    PercentEq,    // %=
    StarStarEq,   // **=
    AmpEq,        // &=
    PipeEq,       // |=
    CaretEq,      // ^=
    LtLtEq,       // <<=
    GtGtEq,       // >>=
    AtEq,         // @=
    ColonEq,      // :=
    ColonColon,   // ::

    // Other punctuation
    Arrow,     // ->
    LeftArrow, // <-
    FatArrow,  // =>
    Colon,     // :
    Comma,     // ,
    Semicolon, // ;
    Dot,       // .
    Ellipsis,  // ...

    // Delimiters
    LParen,   // (
    RParen,   // )
    LBracket, // [
    RBracket, // ]
    LBrace,   // {
    RBrace,   // }

    // Literals
    Int(i64),
    Float(f64),
    /// Imaginary-unit literal: the coefficient before `j` (e.g. `2j` → `ImaginaryFloat(2.0)`).
    ImaginaryFloat(f64),
    Str(String),
    /// f-string: セグメントのリスト（リテラル文字列 + 補間式ソーステキスト）
    FStr(Vec<FStrPart>),

    // Identifier
    Ident(String),

    // Indentation (Python-style)
    Newline,
    Indent,
    Dedent,

    // Unknown character
    Unknown(char),

    // End of file
    Eof,
}

impl Token {
    /// キーワードトークンの正規文字列表現を返す。キーワードでない場合は `None`。
    ///
    /// `Display` トレイトと `Lexer::lex_word` の両方から参照される
    /// キーワード ↔ 文字列の唯一の真実源（single source of truth）。
    /// 複合キーワード（`not in` / `is not` / `yield from`）もここで一括管理する。
    ///
    /// # 引数
    /// - `&self` — 文字列表現を取得したいトークン
    ///
    /// # 戻り値
    /// - `Some(&'static str)` — キーワードトークンに対応する文字列
    /// - `None`               — 演算子・リテラル・識別子など非キーワードトークン
    pub fn keyword_str(&self) -> Option<&'static str> {
        match self {
            Token::Let => Some("let"),
            Token::Const => Some("const"),
            Token::Mut => Some("mut"),
            Token::Static => Some("static"),
            Token::Freeze => Some("freeze"),
            Token::True => Some("True"),
            Token::False => Some("False"),
            Token::None => Some("None"),
            Token::Undefined => Some("Undefined"),
            Token::And => Some("and"),
            Token::Or => Some("or"),
            Token::Not => Some("not"),
            Token::In => Some("in"),
            Token::NotIn => Some("not in"),
            Token::Is => Some("is"),
            Token::IsNot => Some("is not"),
            Token::If => Some("if"),
            Token::Elif => Some("elif"),
            Token::Else => Some("else"),
            Token::Match => Some("match"),
            Token::Case => Some("case"),
            Token::For => Some("for"),
            Token::While => Some("while"),
            Token::Break => Some("break"),
            Token::Continue => Some("continue"),
            Token::Pass => Some("pass"),
            Token::Return => Some("return"),
            Token::Yield => Some("yield"),
            Token::YieldFrom => Some("yield from"),
            Token::BlockReturn => Some("block_return"),
            Token::LoopYield => Some("loop_yield"),
            Token::Block => Some("block"),
            Token::Try => Some("try"),
            Token::Except => Some("except"),
            Token::Finally => Some("finally"),
            Token::Raise => Some("raise"),
            Token::Fn => Some("fn"),
            Token::Gen => Some("gen"),
            Token::Class => Some("class"),
            Token::Enum => Some("enum"),
            Token::Trait => Some("trait"),
            Token::Lambda => Some("lambda"),
            Token::Template => Some("template"),
            Token::Import => Some("import"),
            Token::From => Some("from"),
            Token::As => Some("as"),
            Token::Del => Some("del"),
            Token::Global => Some("global"),
            Token::Nonlocal => Some("nonlocal"),
            Token::With => Some("with"),
            Token::Async => Some("async"),
            Token::Await => Some("await"),
            Token::Assert => Some("assert"),
            Token::BreakPoint => Some("break_point"),
            Token::Public => Some("public"),
            Token::Private => Some("private"),
            Token::Protected => Some("protected"),
            Token::ClassMethod => Some("class_method"),
            Token::SelfType => Some("Self"),
            Token::NewType => Some("new_type"),
            Token::Any => Some("Any"),
            Token::Union => Some("Union"),
            Token::Option => Some("Option"),
            Token::Intersection => Some("Intersection"),
            Token::On => Some("on"),
            Token::Off => Some("off"),
            Token::Once => Some("once"),
            Token::MustBe => Some("mustbe"),
            _ => None,
        }
    }
}

impl std::fmt::Display for Token {
    /// トークンを人間が読める文字列に変換する。
    ///
    /// キーワードトークンは `keyword_str()` に委譲し、
    /// 演算子・区切り記号・リテラル・識別子は個別に記号文字列を返す。
    /// エラーメッセージやデバッグ出力で「期待したトークン」を表示する際に使う。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // キーワードトークンは keyword_str() で一括処理
        if let Some(s) = self.keyword_str() {
            return write!(f, "{s}");
        }
        match self {
            // keyword tokens are handled above via keyword_str()
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::SlashSlash => write!(f, "//"),
            Token::Percent => write!(f, "%"),
            Token::StarStar => write!(f, "**"),
            Token::At => write!(f, "@"),
            Token::EqEq => write!(f, "=="),
            Token::EqEqEq => write!(f, "==="),
            Token::NotEq => write!(f, "!="),
            Token::Lt => write!(f, "<"),
            Token::Gt => write!(f, ">"),
            Token::LtEq => write!(f, "<="),
            Token::GtEq => write!(f, ">="),
            Token::Amp => write!(f, "&"),
            Token::Pipe => write!(f, "|"),
            Token::Caret => write!(f, "^"),
            Token::Tilde => write!(f, "~"),
            Token::LtLt => write!(f, "<<"),
            Token::GtGt => write!(f, ">>"),
            Token::Eq => write!(f, "="),
            Token::PlusEq => write!(f, "+="),
            Token::MinusEq => write!(f, "-="),
            Token::StarEq => write!(f, "*="),
            Token::SlashEq => write!(f, "/="),
            Token::SlashSlashEq => write!(f, "//="),
            Token::PercentEq => write!(f, "%="),
            Token::StarStarEq => write!(f, "**="),
            Token::AmpEq => write!(f, "&="),
            Token::PipeEq => write!(f, "|="),
            Token::CaretEq => write!(f, "^="),
            Token::LtLtEq => write!(f, "<<="),
            Token::GtGtEq => write!(f, ">>="),
            Token::AtEq => write!(f, "@="),
            Token::ColonEq => write!(f, ":="),
            Token::ColonColon => write!(f, "::"),
            Token::Arrow => write!(f, "->"),
            Token::LeftArrow => write!(f, "<-"),
            Token::FatArrow => write!(f, "=>"),
            Token::Colon => write!(f, ":"),
            Token::Comma => write!(f, ","),
            Token::Semicolon => write!(f, ";"),
            Token::Dot => write!(f, "."),
            Token::Ellipsis => write!(f, "..."),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::Int(n) => write!(f, "{n}"),
            Token::Float(n) => write!(f, "{n}"),
            Token::ImaginaryFloat(n) => write!(f, "{n}j"),
            Token::Str(s) => write!(f, "{s:?}"),
            Token::FStr(_) => write!(f, "f-string"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Newline => write!(f, "NEWLINE"),
            Token::Indent => write!(f, "INDENT"),
            Token::Dedent => write!(f, "DEDENT"),
            Token::Unknown(c) => write!(f, "?{c}"),
            Token::Eof => write!(f, "EOF"),
            _ => unreachable!("keyword token should have been handled by keyword_str()"),
        }
    }
}
