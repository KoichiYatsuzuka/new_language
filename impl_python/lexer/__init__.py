# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Lexer package (mirrors src/lexer/mod.rs).

Sub-modules:
  chars   — character access helpers
  keyword — keyword and identifier lexing
  literal — string/number literal parsing
  math    — LaTeX math notation (stub)
  scan    — main Lexer class with indentation handling
  symbol  — symbol/operator tokenization
"""
from .scan import Lexer
from ..token import Spanned


def lex(source: str, filename: str = "") -> list[Spanned]:
    """Tokenize *source* and return a list of Spanned tokens."""
    return Lexer(source, filename).tokenize()


__all__ = ["Lexer", "lex"]
