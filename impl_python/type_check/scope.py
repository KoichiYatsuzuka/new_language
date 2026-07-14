# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Scope management mixin for TypeChecker (mirrors src/type_check.rs)."""
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional, TYPE_CHECKING

from ..token import Span

if TYPE_CHECKING:
    from .types import InferredType
    from .errors import TypeErrorKind, StaticTypeError


@dataclass
class _VarInfo:
    ty: "InferredType"
    mutable: bool


@dataclass
class _FnSig:
    params: list[tuple[str, Optional["InferredType"]]]
    required_count: int
    return_type: Optional["InferredType"]


class _TypeCheckerScope:
    """Mixin providing scope management for TypeChecker."""

    def _push_scope(self) -> None:
        self._scope_stack.append({})

    def _pop_scope(self) -> None:
        if len(self._scope_stack) > 1:
            self._scope_stack.pop()

    def _declare(self, name: str, ty: "InferredType", mutable: bool) -> None:
        self._scope_stack[-1][name] = _VarInfo(ty=ty, mutable=mutable)

    def _lookup(self, name: str) -> Optional[_VarInfo]:
        for scope in reversed(self._scope_stack):
            if name in scope:
                return scope[name]
        return None

    def _report(self, kind: "TypeErrorKind", span: Optional[Span] = None) -> None:
        from .errors import StaticTypeError
        self.errors.append(StaticTypeError(kind=kind, span=span))
