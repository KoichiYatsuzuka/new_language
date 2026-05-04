use std::sync::Arc;

/// ソースコード上の位置情報。Arc<str> でファイル名を共有してクローンを軽量化。
#[derive(Debug, Clone)]
pub struct Span {
    pub file: Arc<str>,
    pub line: usize, // 1 始まり
    pub col: usize,  // 1 始まり（文字単位）
}

impl Span {
    pub fn unknown() -> Self {
        Self { file: "".into(), line: 0, col: 0 }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "<unknown>")
        } else if self.file.is_empty() {
            write!(f, "line {}, col {}", self.line, self.col)
        } else {
            write!(f, "{}:{}:{}", self.file, self.line, self.col)
        }
    }
}

/// トークンと位置情報のペア。
#[derive(Debug, Clone)]
pub struct Spanned {
    pub token: Token,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Variable declaration
    Let,
    Const,
    Mut,

    // Value literals (keyword form)
    True,
    False,
    None,

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
    For,
    While,
    Break,
    Continue,
    Pass,
    Return,
    Yield,
    YieldFrom,
    BlockReturn,
    BlockYield,
    Block,

    // Exception handling
    Try,
    Except,
    Finally,
    Raise,

    // Definitions
    Fn,
    Class,
    Trait,
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

    // Arithmetic operators
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    SlashSlash,   // //
    Percent,      // %
    StarStar,     // **
    At,           // @

    // Comparison operators
    EqEq,         // ==
    NotEq,        // !=
    Lt,           // <
    Gt,           // >
    LtEq,         // <=
    GtEq,         // >=

    // Bitwise operators
    Amp,          // &
    Pipe,         // |
    Caret,        // ^
    Tilde,        // ~
    LtLt,         // <<
    GtGt,         // >>

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
    Arrow,        // ->
    Colon,        // :
    Comma,        // ,
    Semicolon,    // ;
    Dot,          // .
    Ellipsis,     // ...

    // Delimiters
    LParen,       // (
    RParen,       // )
    LBracket,     // [
    RBracket,     // ]
    LBrace,       // {
    RBrace,       // }

    // Literals
    Int(i64),
    Float(f64),
    Str(String),

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

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Let => write!(f, "let"),
            Token::Const => write!(f, "const"),
            Token::Mut => write!(f, "mut"),
            Token::True => write!(f, "True"),
            Token::False => write!(f, "False"),
            Token::None => write!(f, "None"),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::In => write!(f, "in"),
            Token::NotIn => write!(f, "not in"),
            Token::Is => write!(f, "is"),
            Token::IsNot => write!(f, "is not"),
            Token::If => write!(f, "if"),
            Token::Elif => write!(f, "elif"),
            Token::Else => write!(f, "else"),
            Token::Match => write!(f, "match"),
            Token::For => write!(f, "for"),
            Token::While => write!(f, "while"),
            Token::Break => write!(f, "break"),
            Token::Continue => write!(f, "continue"),
            Token::Pass => write!(f, "pass"),
            Token::Return => write!(f, "return"),
            Token::Yield => write!(f, "yield"),
            Token::YieldFrom => write!(f, "yield from"),
            Token::BlockReturn => write!(f, "block_return"),
            Token::BlockYield => write!(f, "block_yield"),
            Token::Block => write!(f, "block"),
            Token::Try => write!(f, "try"),
            Token::Except => write!(f, "except"),
            Token::Finally => write!(f, "finally"),
            Token::Raise => write!(f, "raise"),
            Token::Fn => write!(f, "fn"),
            Token::Class => write!(f, "class"),
            Token::Trait => write!(f, "trait"),
            Token::Lambda => write!(f, "lambda"),
            Token::Template => write!(f, "template"),
            Token::Import => write!(f, "import"),
            Token::From => write!(f, "from"),
            Token::As => write!(f, "as"),
            Token::Del => write!(f, "del"),
            Token::Global => write!(f, "global"),
            Token::Nonlocal => write!(f, "nonlocal"),
            Token::With => write!(f, "with"),
            Token::Async => write!(f, "async"),
            Token::Await => write!(f, "await"),
            Token::Assert => write!(f, "assert"),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::SlashSlash => write!(f, "//"),
            Token::Percent => write!(f, "%"),
            Token::StarStar => write!(f, "**"),
            Token::At => write!(f, "@"),
            Token::EqEq => write!(f, "=="),
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
            Token::Str(s) => write!(f, "{s:?}"),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Newline => write!(f, "NEWLINE"),
            Token::Indent => write!(f, "INDENT"),
            Token::Dedent => write!(f, "DEDENT"),
            Token::Unknown(c) => write!(f, "?{c}"),
            Token::Eof => write!(f, "EOF"),
        }
    }
}
