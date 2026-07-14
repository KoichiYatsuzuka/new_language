# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Main lexer scan implementation with indentation handling (mirrors src/lexer/scan.rs)."""
from __future__ import annotations
from typing import Optional

from ..token import Span, Token, TokenKind, Spanned
from .chars import _LexerChars
from .keyword import _LexerKeyword
from .literal import _LexerLiteral
from .symbol import _LexerSymbol


def _compute_positions(chars: list[str]) -> list[tuple[int, int]]:
    """Pre-compute (line, col) for every character index."""
    positions: list[tuple[int, int]] = []
    line, col = 1, 1
    for c in chars:
        positions.append((line, col))
        if c == '\n':
            line += 1
            col = 1
        else:
            col += 1
    positions.append((line, col))  # EOF position
    return positions


class Lexer(_LexerChars, _LexerKeyword, _LexerLiteral, _LexerSymbol):
    """Python-style indentation lexer producing Spanned token lists."""

    def __init__(self, source: str, filename: str = "") -> None:
        self._chars: list[str] = list(source)
        self._pos: int = 0
        self._positions: list[tuple[int, int]] = _compute_positions(self._chars)
        self._filename: str = filename
        self._indent_stack: list[int] = [0]
        self._pending: list[Spanned] = []
        self._at_line_start: bool = True
        self._bracket_depth: int = 0

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def tokenize(self) -> list[Spanned]:
        tokens: list[Spanned] = []
        while True:
            tok = self.next_token()
            tokens.append(tok)
            if tok.token.kind == TokenKind.EOF:
                break
        return tokens

    def next_token(self) -> Spanned:
        if self._pending:
            return self._pending.pop(0)

        if self._at_line_start and self._bracket_depth == 0:
            return self._handle_indent()

        self._skip_spaces()
        start = self._pos

        c = self._ch()
        if c is None:
            return self._emit_eof()

        if c in ('\n', '\r'):
            self._consume_newline()
            if self._bracket_depth > 0:
                return self.next_token()
            self._at_line_start = True
            return self._spanned(Token(TokenKind.NEWLINE), start)

        if c == '#':
            self._skip_comment()
            return self.next_token()

        if c in ('"', "'"):
            tok = self._lex_string()
            return self._spanned(tok, start)

        if c.isdigit():
            tok = self._lex_number()
            return self._spanned(tok, start)

        if c.isalpha() or c == '_':
            tok = self._lex_word()
            return self._spanned(tok, start)

        tok = self._lex_symbol()
        return self._spanned(tok, start)

    # ------------------------------------------------------------------
    # Position helpers
    # ------------------------------------------------------------------

    def _span_at(self, pos: int) -> Span:
        if pos < len(self._positions):
            line, col = self._positions[pos]
        else:
            line, col = self._positions[-1] if self._positions else (1, 1)
        return Span(file=self._filename, line=line, col=col)

    def _spanned(self, token: Token, start: int) -> Spanned:
        return Spanned(token=token, span=self._span_at(start))

    # ------------------------------------------------------------------
    # EOF handling
    # ------------------------------------------------------------------

    def _emit_eof(self) -> Spanned:
        span = self._span_at(self._pos)
        while len(self._indent_stack) > 1:
            self._indent_stack.pop()
            self._pending.append(Spanned(token=Token(TokenKind.DEDENT), span=span))
        if self._pending:
            return self._pending.pop(0)
        return Spanned(token=Token(TokenKind.EOF), span=span)

    # ------------------------------------------------------------------
    # Indentation handling
    # ------------------------------------------------------------------

    def _handle_indent(self) -> Spanned:
        self._at_line_start = False
        while True:
            level, char_count = self._measure_indent()
            after = self._pos + char_count
            next_ch = self._chars[after] if after < len(self._chars) else None

            if next_ch in ('\n', '\r'):
                self._pos = after
                self._consume_newline()
                continue

            if next_ch == '#':
                self._pos = after
                self._skip_comment()
                self._consume_newline()
                continue

            if next_ch is None:
                self._pos = after
                return self._emit_eof()

            current = self._indent_stack[-1]
            self._pos = after
            span = self._span_at(after)

            if level > current:
                self._indent_stack.append(level)
                return Spanned(token=Token(TokenKind.INDENT), span=span)
            elif level < current:
                while self._indent_stack[-1] > level:
                    self._indent_stack.pop()
                    self._pending.append(Spanned(token=Token(TokenKind.DEDENT), span=span))
                return self._pending.pop(0)
            else:
                return self.next_token()

    def _measure_indent(self) -> tuple[int, int]:
        level = 0
        count = 0
        i = self._pos
        while i < len(self._chars):
            c = self._chars[i]
            if c == ' ':
                level += 1
                count += 1
            elif c == '\t':
                level = (level // 8 + 1) * 8
                count += 1
            else:
                break
            i += 1
        return level, count

    def _consume_newline(self) -> None:
        if self._ch() == '\r' and self._ch1() == '\n':
            self._pos += 1
        if self._pos < len(self._chars):
            self._pos += 1

    def _skip_spaces(self) -> None:
        while self._ch() in (' ', '\t'):
            self._pos += 1

    def _skip_comment(self) -> None:
        while self._ch() is not None and self._ch() not in ('\n', '\r'):
            self._pos += 1
