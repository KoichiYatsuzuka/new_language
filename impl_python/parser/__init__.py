# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Parser package (mirrors src/parser/mod.rs).

Sub-modules:
  stmts   — statement parsing
  exprs   — expression parsing
  classes — class and trait parsing
  imports — import statement parsing
  types   — type annotation and parameter parsing
"""
from __future__ import annotations
from pathlib import Path
from typing import Optional

from ..token import Token, TokenKind, Span, Spanned
from ..ast import Stmt, TemplateParam, FieldKind
from .stmts import _ParserStmts
from .exprs import _ParserExprs
from .classes import _ParserClasses
from .imports import _ParserImports
from .types import _ParserTypes


class ParseError(Exception):
    pass


class Parser(_ParserStmts, _ParserExprs, _ParserClasses, _ParserImports, _ParserTypes):
    """Recursive descent parser for test_lang source code."""

    def __init__(self, tokens: list[Spanned], source_dir: Optional[Path] = None) -> None:
        self._tokens = tokens
        self._pos = 0
        self._known_traits: dict[str, tuple[list[TemplateParam], list, list[str]]] = {}
        # Pre-register built-in Error trait
        self._known_traits["Error"] = (
            [],
            [
                ("message",      FieldKind.LET, "str",  False),
                ("code_context", FieldKind.MUT, "str",  True),
                ("file",         FieldKind.MUT, "str",  True),
                ("line",         FieldKind.MUT, "int",  True),
                ("col",          FieldKind.MUT, "int",  True),
            ],
            [],
        )
        self._class_or_trait_depth = 0
        self._known_new_types: set[str] = set()
        resolved = source_dir if source_dir is not None else Path(".")
        self._source_dir = resolved
        self._root_dir = resolved
        self._module_cache: dict[tuple[str, Path], list[Stmt]] = {}
        self._loading: set[Path] = set()

    # ------------------------------------------------------------------
    # Token access helpers
    # ------------------------------------------------------------------

    def _current(self) -> Token:
        if self._pos < len(self._tokens):
            return self._tokens[self._pos].token
        return Token(TokenKind.EOF)

    def _current_kind(self) -> TokenKind:
        return self._current().kind

    def _peek1(self) -> Token:
        p = self._pos + 1
        if p < len(self._tokens):
            return self._tokens[p].token
        return Token(TokenKind.EOF)

    def _peek1_kind(self) -> TokenKind:
        return self._peek1().kind

    def _at(self, offset: int) -> Token:
        p = self._pos + offset
        if p < len(self._tokens):
            return self._tokens[p].token
        return Token(TokenKind.EOF)

    def _current_span(self) -> Span:
        if self._pos < len(self._tokens):
            return self._tokens[self._pos].span
        return Span.unknown()

    def _advance(self) -> None:
        if self._pos < len(self._tokens):
            self._pos += 1

    def _eat(self, expected: TokenKind) -> None:
        if self._current_kind() == expected:
            self._advance()
        else:
            cur = self._current()
            raise ParseError(
                f"expected `{expected.name}`, got `{cur.kind.name}`"
            )

    def _error(self, msg: str) -> ParseError:
        return ParseError(msg)

    def _skip_newlines(self) -> None:
        while self._current_kind() in (
            TokenKind.NEWLINE, TokenKind.INDENT, TokenKind.DEDENT, TokenKind.SEMICOLON
        ):
            self._advance()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def parse_program(self) -> list[Stmt]:
        stmts: list[Stmt] = []
        self._skip_newlines()
        while self._current_kind() != TokenKind.EOF:
            stmts.append(self._parse_stmt())
            self._skip_newlines()
        return stmts


def parse(source: str, filename: str = "", source_dir: Optional[Path] = None) -> list[Stmt]:
    """Tokenize and parse *source*, returning a list of top-level statements."""
    from ..lexer import lex
    tokens = lex(source, filename)
    sd = source_dir or (Path(filename).parent if filename else Path("."))
    return Parser(tokens, sd).parse_program()


__all__ = ["Parser", "ParseError", "parse"]
