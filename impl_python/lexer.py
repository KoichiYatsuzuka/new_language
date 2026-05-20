from __future__ import annotations
from typing import Optional

from .token import Span, Token, TokenKind, Spanned, KEYWORDS


class Lexer:
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

    # ------------------------------------------------------------------
    # String literals
    # ------------------------------------------------------------------

    def _lex_string(self) -> Token:
        quote = self._bump()
        assert quote is not None
        triple = self._ch() == quote and self._ch1() == quote
        if triple:
            self._pos += 2
        s: list[str] = []

        while True:
            c = self._ch()
            if c is None:
                break
            if c == '\\':
                self._pos += 1
                esc = self._bump()
                if esc is None:
                    break
                s.append({
                    'n': '\n', 't': '\t', 'r': '\r',
                    '\\': '\\', "'": "'", '"': '"', '0': '\0',
                }.get(esc, '\\' + esc))
            elif c == quote:
                if triple:
                    if self._ch1() == quote and self._ch2() == quote:
                        self._pos += 3
                        break
                    else:
                        s.append(c)
                        self._pos += 1
                else:
                    self._pos += 1
                    break
            else:
                s.append(c)
                self._pos += 1

        return Token(TokenKind.STR, value=''.join(s))

    # ------------------------------------------------------------------
    # Number literals
    # ------------------------------------------------------------------

    def _lex_number(self) -> Token:
        start = self._pos
        if self._ch() == '0':
            n = self._ch1()
            if n in ('x', 'X'):
                return self._lex_radix_int(start, 16, lambda c: c in '0123456789abcdefABCDEF')
            if n in ('o', 'O'):
                return self._lex_radix_int(start, 8, lambda c: c in '01234567')
            if n in ('b', 'B'):
                return self._lex_radix_int(start, 2, lambda c: c in '01')
        return self._lex_decimal_number(start)

    def _lex_radix_int(self, start: int, base: int, is_digit) -> Token:
        self._pos += 2  # skip prefix (0x / 0o / 0b)
        while self._ch() is not None and (is_digit(self._ch()) or self._ch() == '_'):
            self._pos += 1
        raw = ''.join(self._chars[start:self._pos])
        clean = raw.replace('_', '')
        try:
            value = int(clean[2:], base)
        except ValueError:
            value = 0
        return Token(TokenKind.INT, value=value)

    def _lex_decimal_number(self, start: int) -> Token:
        while self._ch() is not None and (self._ch().isdigit() or self._ch() == '_'):
            self._pos += 1

        is_float = False

        if self._ch() == '.' and self._ch1() is not None and self._ch1().isdigit():
            is_float = True
            self._pos += 1
            while self._ch() is not None and (self._ch().isdigit() or self._ch() == '_'):
                self._pos += 1

        if self._ch() in ('e', 'E'):
            is_float = True
            self._pos += 1
            if self._ch() in ('+', '-'):
                self._pos += 1
            while self._ch() is not None and self._ch().isdigit():
                self._pos += 1

        raw = ''.join(self._chars[start:self._pos])
        clean = raw.replace('_', '')
        if is_float:
            try:
                return Token(TokenKind.FLOAT, value=float(clean))
            except ValueError:
                return Token(TokenKind.FLOAT, value=0.0)
        else:
            try:
                return Token(TokenKind.INT, value=int(clean))
            except ValueError:
                return Token(TokenKind.INT, value=0)

    # ------------------------------------------------------------------
    # Identifiers and keywords
    # ------------------------------------------------------------------

    def _lex_word(self) -> Token:
        start = self._pos
        while self._ch() is not None and (self._ch().isalnum() or self._ch() == '_'):
            self._pos += 1
        word = ''.join(self._chars[start:self._pos])

        # Compound keywords: not in, is not, yield from
        if word == 'not':
            return self._maybe_two_word('in', TokenKind.NOT_IN, TokenKind.NOT)
        if word == 'is':
            return self._maybe_two_word('not', TokenKind.IS_NOT, TokenKind.IS)
        if word == 'yield':
            return self._maybe_two_word('from', TokenKind.YIELD_FROM, TokenKind.YIELD)

        kind = KEYWORDS.get(word)
        if kind is not None:
            return Token(kind)
        return Token(TokenKind.IDENT, value=word)

    def _maybe_two_word(self, second: str, combined: TokenKind, single: TokenKind) -> Token:
        saved = self._pos
        while self._ch() in (' ', '\t'):
            self._pos += 1
        word_start = self._pos
        while self._ch() is not None and (self._ch().isalnum() or self._ch() == '_'):
            self._pos += 1
        word = ''.join(self._chars[word_start:self._pos])
        if word == second:
            return Token(combined)
        self._pos = saved
        return Token(single)

    # ------------------------------------------------------------------
    # Symbols / operators
    # ------------------------------------------------------------------

    def _lex_symbol(self) -> Token:
        c = self._bump()
        assert c is not None

        if c == '(':
            self._bracket_depth += 1
            return Token(TokenKind.LPAREN)
        if c == ')':
            if self._bracket_depth > 0:
                self._bracket_depth -= 1
            return Token(TokenKind.RPAREN)
        if c == '[':
            self._bracket_depth += 1
            return Token(TokenKind.LBRACKET)
        if c == ']':
            if self._bracket_depth > 0:
                self._bracket_depth -= 1
            return Token(TokenKind.RBRACKET)
        if c == '{':
            self._bracket_depth += 1
            return Token(TokenKind.LBRACE)
        if c == '}':
            if self._bracket_depth > 0:
                self._bracket_depth -= 1
            return Token(TokenKind.RBRACE)

        if c == '+':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.PLUS_EQ)
            return Token(TokenKind.PLUS)

        if c == '-':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.MINUS_EQ)
            if self._ch() == '>':
                self._pos += 1; return Token(TokenKind.ARROW)
            return Token(TokenKind.MINUS)

        if c == '*':
            if self._ch() == '*':
                self._pos += 1
                if self._ch() == '=':
                    self._pos += 1; return Token(TokenKind.STAR_STAR_EQ)
                return Token(TokenKind.STAR_STAR)
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.STAR_EQ)
            return Token(TokenKind.STAR)

        if c == '/':
            if self._ch() == '/':
                self._pos += 1
                if self._ch() == '=':
                    self._pos += 1; return Token(TokenKind.SLASH_SLASH_EQ)
                return Token(TokenKind.SLASH_SLASH)
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.SLASH_EQ)
            return Token(TokenKind.SLASH)

        if c == '%':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.PERCENT_EQ)
            return Token(TokenKind.PERCENT)

        if c == '@':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.AT_EQ)
            return Token(TokenKind.AT)

        if c == '=':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.EQ_EQ)
            return Token(TokenKind.EQ)

        if c == '!':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.NOT_EQ)
            return Token(TokenKind.UNKNOWN, value='!')

        if c == '<':
            if self._ch() == '<':
                self._pos += 1
                if self._ch() == '=':
                    self._pos += 1; return Token(TokenKind.LT_LT_EQ)
                return Token(TokenKind.LT_LT)
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.LT_EQ)
            # <-  is not a token in token.py; treat as LT (do NOT consume '-')
            return Token(TokenKind.LT)

        if c == '>':
            if self._ch() == '>':
                self._pos += 1
                if self._ch() == '=':
                    self._pos += 1; return Token(TokenKind.GT_GT_EQ)
                return Token(TokenKind.GT_GT)
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.GT_EQ)
            return Token(TokenKind.GT)

        if c == '&':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.AMP_EQ)
            return Token(TokenKind.AMP)

        if c == '|':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.PIPE_EQ)
            return Token(TokenKind.PIPE)

        if c == '^':
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.CARET_EQ)
            return Token(TokenKind.CARET)

        if c == '~':
            return Token(TokenKind.TILDE)

        if c == ':':
            if self._ch() == ':':
                self._pos += 1; return Token(TokenKind.COLON_COLON)
            if self._ch() == '=':
                self._pos += 1; return Token(TokenKind.COLON_EQ)
            return Token(TokenKind.COLON)

        if c == ',':
            return Token(TokenKind.COMMA)
        if c == ';':
            return Token(TokenKind.SEMICOLON)

        if c == '.':
            if self._ch() == '.' and self._ch1() == '.':
                self._pos += 2; return Token(TokenKind.ELLIPSIS)
            return Token(TokenKind.DOT)

        return Token(TokenKind.UNKNOWN, value=c)

    # ------------------------------------------------------------------
    # Character access helpers
    # ------------------------------------------------------------------

    def _ch(self) -> Optional[str]:
        return self._chars[self._pos] if self._pos < len(self._chars) else None

    def _ch1(self) -> Optional[str]:
        p = self._pos + 1
        return self._chars[p] if p < len(self._chars) else None

    def _ch2(self) -> Optional[str]:
        p = self._pos + 2
        return self._chars[p] if p < len(self._chars) else None

    def _bump(self) -> Optional[str]:
        c = self._chars[self._pos] if self._pos < len(self._chars) else None
        if c is not None:
            self._pos += 1
        return c


# ---------------------------------------------------------------------------
# Helper: pre-compute (line, col) for every character index
# ---------------------------------------------------------------------------

def _compute_positions(chars: list[str]) -> list[tuple[int, int]]:
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


# ---------------------------------------------------------------------------
# Convenience function
# ---------------------------------------------------------------------------

def lex(source: str, filename: str = "") -> list[Spanned]:
    return Lexer(source, filename).tokenize()
