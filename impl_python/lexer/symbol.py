# git SHA: aea2e1fe6909a7aed9643a2e7184f19fd0195ccc
"""Symbol and operator tokenization (mirrors src/lexer/symbol.rs)."""
from __future__ import annotations
from ..token import Token, TokenKind


class _LexerSymbol:
    """Mixin providing symbol/operator tokenization."""

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
                self._pos += 1
                if self._ch() == '=':
                    self._pos += 1; return Token(TokenKind.EQ_EQ_EQ)
                return Token(TokenKind.EQ_EQ)
            if self._ch() == '>':
                self._pos += 1; return Token(TokenKind.FAT_ARROW)
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
            # <- is not a single token; emit LT, leave '-' for next call
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
