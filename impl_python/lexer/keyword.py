# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Keyword and identifier lexing (mirrors src/lexer/keyword.rs)."""
from __future__ import annotations
from ..token import Token, TokenKind, KEYWORDS


class _LexerKeyword:
    """Mixin providing keyword/identifier lexing."""

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
