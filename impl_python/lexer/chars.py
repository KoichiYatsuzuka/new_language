# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Character access helpers for the Lexer (mirrors src/lexer/chars.rs)."""
from __future__ import annotations
from typing import Optional


class _LexerChars:
    """Mixin providing ch/ch1/ch2/bump character access."""

    _chars: list[str]
    _pos: int

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
