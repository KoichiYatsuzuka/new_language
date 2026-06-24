# git SHA: d4bdc21ea237938cb9213f731fd60a3fe6046b78
from __future__ import annotations
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Union as TypingUnion


@dataclass
class Span:
    file: str
    line: int  # 1-based; 0 means unknown
    col: int   # 1-based

    @staticmethod
    def unknown() -> "Span":
        return Span(file="", line=0, col=0)

    def __str__(self) -> str:
        if self.line == 0:
            return "<unknown>"
        if not self.file:
            return f"line {self.line}, col {self.col}"
        return f"{self.file}:{self.line}:{self.col}"


# ---------------------------------------------------------------------------
# Token kinds
# ---------------------------------------------------------------------------

class TokenKind(Enum):
    # Variable declaration
    LET          = auto()
    CONST        = auto()
    MUT          = auto()
    STATIC       = auto()
    FREEZE       = auto()

    # Value literals (keyword form)
    TRUE         = auto()
    FALSE        = auto()
    NONE         = auto()
    UNDEFINED    = auto()

    # Logical operators (keyword form)
    AND          = auto()
    OR           = auto()
    NOT          = auto()

    # Comparison keywords
    IN           = auto()
    NOT_IN       = auto()
    IS           = auto()
    IS_NOT       = auto()

    # Control flow
    IF           = auto()
    ELIF         = auto()
    ELSE         = auto()
    MATCH        = auto()
    CASE         = auto()
    FOR          = auto()
    WHILE        = auto()
    BREAK        = auto()
    CONTINUE     = auto()
    PASS         = auto()
    RETURN       = auto()
    YIELD        = auto()
    YIELD_FROM   = auto()
    BLOCK_RETURN = auto()
    LOOP_YIELD   = auto()
    BLOCK        = auto()

    # Exception handling
    TRY          = auto()
    EXCEPT       = auto()
    FINALLY      = auto()
    RAISE        = auto()

    # Definitions
    FN           = auto()
    GEN          = auto()
    CLASS        = auto()
    ENUM         = auto()
    TRAIT        = auto()
    PROTOCOL     = auto()
    LAMBDA       = auto()
    TEMPLATE     = auto()

    # Import
    IMPORT       = auto()
    FROM         = auto()
    AS           = auto()

    # Scope
    DEL          = auto()
    GLOBAL       = auto()
    NONLOCAL     = auto()

    # Context manager
    WITH         = auto()

    # Async
    ASYNC        = auto()
    AWAIT        = auto()

    # Assertion
    ASSERT       = auto()

    # Access modifiers (class body section headers)
    PUBLIC       = auto()
    PRIVATE      = auto()
    PROTECTED    = auto()

    # Method kind modifiers
    CLASS_METHOD = auto()  # class_method fn

    # Event handler keywords
    ON           = auto()  # on
    OFF          = auto()  # off
    ONCE         = auto()  # once

    # Special type keywords
    SELF_TYPE    = auto()  # Self
    NEW_TYPE     = auto()  # new_type
    ANY          = auto()  # Any
    UNION        = auto()  # Union
    OPTION       = auto()  # Option

    # Arithmetic operators
    PLUS         = auto()  # +
    MINUS        = auto()  # -
    STAR         = auto()  # *
    SLASH        = auto()  # /
    SLASH_SLASH  = auto()  # //
    PERCENT      = auto()  # %
    STAR_STAR    = auto()  # **
    AT           = auto()  # @

    # Comparison operators
    EQ_EQ        = auto()  # ==
    EQ_EQ_EQ     = auto()  # ===
    NOT_EQ       = auto()  # !=
    LT           = auto()  # <
    GT           = auto()  # >
    LT_EQ        = auto()  # <=
    GT_EQ        = auto()  # >=

    # Bitwise operators
    AMP          = auto()  # &
    PIPE         = auto()  # |
    CARET        = auto()  # ^
    TILDE        = auto()  # ~
    LT_LT        = auto()  # <<
    GT_GT        = auto()  # >>

    # Assignment operators
    EQ           = auto()  # =
    PLUS_EQ      = auto()  # +=
    MINUS_EQ     = auto()  # -=
    STAR_EQ      = auto()  # *=
    SLASH_EQ     = auto()  # /=
    SLASH_SLASH_EQ = auto()  # //=
    PERCENT_EQ   = auto()  # %=
    STAR_STAR_EQ = auto()  # **=
    AMP_EQ       = auto()  # &=
    PIPE_EQ      = auto()  # |=
    CARET_EQ     = auto()  # ^=
    LT_LT_EQ     = auto()  # <<=
    GT_GT_EQ     = auto()  # >>=
    AT_EQ        = auto()  # @=
    COLON_EQ     = auto()  # :=
    COLON_COLON  = auto()  # ::

    # Other punctuation
    ARROW        = auto()  # ->
    FAT_ARROW    = auto()  # =>
    COLON        = auto()  # :
    COMMA        = auto()  # ,
    SEMICOLON    = auto()  # ;
    DOT          = auto()  # .
    ELLIPSIS     = auto()  # ...

    # Delimiters
    LPAREN       = auto()  # (
    RPAREN       = auto()  # )
    LBRACKET     = auto()  # [
    RBRACKET     = auto()  # ]
    LBRACE       = auto()  # {
    RBRACE       = auto()  # }

    # Literals (kind only; value stored in Token.value)
    INT          = auto()
    FLOAT        = auto()
    IMAGINARY_FLOAT = auto()  # e.g. 2j → coefficient 2.0
    STR          = auto()

    # Identifier
    IDENT        = auto()

    # Indentation (Python-style)
    NEWLINE      = auto()
    INDENT       = auto()
    DEDENT       = auto()

    # Unknown character
    UNKNOWN      = auto()

    # End of file
    EOF          = auto()


# Mapping from keyword string to TokenKind
KEYWORDS: dict[str, TokenKind] = {
    "let":          TokenKind.LET,
    "const":        TokenKind.CONST,
    "mut":          TokenKind.MUT,
    "static":       TokenKind.STATIC,
    "freeze":       TokenKind.FREEZE,
    "True":         TokenKind.TRUE,
    "False":        TokenKind.FALSE,
    "None":         TokenKind.NONE,
    "Undefined":    TokenKind.UNDEFINED,
    "and":          TokenKind.AND,
    "or":           TokenKind.OR,
    "not":          TokenKind.NOT,
    "in":           TokenKind.IN,
    "not in":       TokenKind.NOT_IN,
    "is":           TokenKind.IS,
    "is not":       TokenKind.IS_NOT,
    "if":           TokenKind.IF,
    "elif":         TokenKind.ELIF,
    "else":         TokenKind.ELSE,
    "match":        TokenKind.MATCH,
    "case":         TokenKind.CASE,
    "for":          TokenKind.FOR,
    "while":        TokenKind.WHILE,
    "break":        TokenKind.BREAK,
    "continue":     TokenKind.CONTINUE,
    "pass":         TokenKind.PASS,
    "return":       TokenKind.RETURN,
    "yield":        TokenKind.YIELD,
    "yield from":   TokenKind.YIELD_FROM,
    "block_return": TokenKind.BLOCK_RETURN,
    "loop_yield":   TokenKind.LOOP_YIELD,
    "block":        TokenKind.BLOCK,
    "try":          TokenKind.TRY,
    "except":       TokenKind.EXCEPT,
    "finally":      TokenKind.FINALLY,
    "raise":        TokenKind.RAISE,
    "fn":           TokenKind.FN,
    "gen":          TokenKind.GEN,
    "class":        TokenKind.CLASS,
    "enum":         TokenKind.ENUM,
    "trait":        TokenKind.TRAIT,
    "protocol":     TokenKind.PROTOCOL,
    "lambda":       TokenKind.LAMBDA,
    "template":     TokenKind.TEMPLATE,
    "import":       TokenKind.IMPORT,
    "from":         TokenKind.FROM,
    "as":           TokenKind.AS,
    "del":          TokenKind.DEL,
    "global":       TokenKind.GLOBAL,
    "nonlocal":     TokenKind.NONLOCAL,
    "with":         TokenKind.WITH,
    "async":        TokenKind.ASYNC,
    "await":        TokenKind.AWAIT,
    "assert":       TokenKind.ASSERT,
    "public":       TokenKind.PUBLIC,
    "private":      TokenKind.PRIVATE,
    "protected":    TokenKind.PROTECTED,
    "class_method": TokenKind.CLASS_METHOD,
    "on":           TokenKind.ON,
    "off":          TokenKind.OFF,
    "once":         TokenKind.ONCE,
    "Self":         TokenKind.SELF_TYPE,
    "new_type":     TokenKind.NEW_TYPE,
    "Any":          TokenKind.ANY,
    "Union":        TokenKind.UNION,
    "Option":       TokenKind.OPTION,
}

# Reverse mapping: TokenKind -> display string (keywords + operators + punctuation)
_TOKEN_STR: dict[TokenKind, str] = {
    **{v: k for k, v in KEYWORDS.items()},
    TokenKind.PLUS:           "+",
    TokenKind.MINUS:          "-",
    TokenKind.STAR:           "*",
    TokenKind.SLASH:          "/",
    TokenKind.SLASH_SLASH:    "//",
    TokenKind.PERCENT:        "%",
    TokenKind.STAR_STAR:      "**",
    TokenKind.AT:             "@",
    TokenKind.EQ_EQ:          "==",
    TokenKind.EQ_EQ_EQ:       "===",
    TokenKind.NOT_EQ:         "!=",
    TokenKind.LT:             "<",
    TokenKind.GT:             ">",
    TokenKind.LT_EQ:          "<=",
    TokenKind.GT_EQ:          ">=",
    TokenKind.AMP:            "&",
    TokenKind.PIPE:           "|",
    TokenKind.CARET:          "^",
    TokenKind.TILDE:          "~",
    TokenKind.LT_LT:          "<<",
    TokenKind.GT_GT:          ">>",
    TokenKind.EQ:             "=",
    TokenKind.PLUS_EQ:        "+=",
    TokenKind.MINUS_EQ:       "-=",
    TokenKind.STAR_EQ:        "*=",
    TokenKind.SLASH_EQ:       "/=",
    TokenKind.SLASH_SLASH_EQ: "//=",
    TokenKind.PERCENT_EQ:     "%=",
    TokenKind.STAR_STAR_EQ:   "**=",
    TokenKind.AMP_EQ:         "&=",
    TokenKind.PIPE_EQ:        "|=",
    TokenKind.CARET_EQ:       "^=",
    TokenKind.LT_LT_EQ:       "<<=",
    TokenKind.GT_GT_EQ:       ">>=",
    TokenKind.AT_EQ:          "@=",
    TokenKind.COLON_EQ:       ":=",
    TokenKind.COLON_COLON:    "::",
    TokenKind.ARROW:          "->",
    TokenKind.FAT_ARROW:      "=>",
    TokenKind.COLON:          ":",
    TokenKind.COMMA:          ",",
    TokenKind.SEMICOLON:      ";",
    TokenKind.DOT:            ".",
    TokenKind.ELLIPSIS:       "...",
    TokenKind.LPAREN:         "(",
    TokenKind.RPAREN:         ")",
    TokenKind.LBRACKET:       "[",
    TokenKind.RBRACKET:       "]",
    TokenKind.LBRACE:         "{",
    TokenKind.RBRACE:         "}",
    TokenKind.NEWLINE:        "NEWLINE",
    TokenKind.INDENT:         "INDENT",
    TokenKind.DEDENT:         "DEDENT",
    TokenKind.EOF:            "EOF",
}


@dataclass
class Token:
    kind: TokenKind
    # For INT / FLOAT / STR / IDENT / UNKNOWN: holds the parsed value.
    # None for all other kinds.
    value: int | float | str | None = field(default=None)

    def __str__(self) -> str:
        if self.kind == TokenKind.INT:
            return str(self.value)
        if self.kind == TokenKind.FLOAT:
            return str(self.value)
        if self.kind == TokenKind.STR:
            return repr(self.value)
        if self.kind == TokenKind.IDENT:
            return str(self.value)
        if self.kind == TokenKind.UNKNOWN:
            return f"?{self.value}"
        return _TOKEN_STR.get(self.kind, self.kind.name)


@dataclass
class Spanned:
    token: Token
    span: Span
