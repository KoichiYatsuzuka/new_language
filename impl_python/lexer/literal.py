# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""String and number literal parsing (mirrors src/lexer/literal.rs)."""
from __future__ import annotations
from ..token import Token, TokenKind


class _LexerLiteral:
    """Mixin providing string and number literal lexing."""

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

        # imaginary suffix 'j'
        if self._ch() == 'j':
            next_ch = self._ch1()
            if next_ch is None or not (next_ch.isalnum() or next_ch == '_'):
                self._pos += 1  # consume 'j'
                try:
                    imag = float(clean) if is_float else float(int(clean))
                except ValueError:
                    imag = 0.0
                return Token(TokenKind.IMAGINARY_FLOAT, value=imag)

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
