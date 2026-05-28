# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Type utility helpers and compatibility checking mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import TYPE_CHECKING

from .types import (
    TyInt, TyFloat, TyStr, TyBool, TyNone, TyList, TyDict, TySet,
    TyTypeVal, TyTypeValOf, TyNamedInstance, TyAny,
    TyUnion, TyUnresolved, TyFunction, InferredType,
)

if TYPE_CHECKING:
    pass


def _type_from_guard_name(type_name: str) -> "InferredType":
    return {
        "int": TyInt(), "float": TyFloat(), "str": TyStr(), "bool": TyBool(),
        "None": TyNone(), "list": TyList(), "dict": TyDict(), "set": TySet(),
        "function": TyFunction(params=None, return_type=TyAny()),
    }.get(type_name, TyNamedInstance(type_name))


class _TypeCheckerUtils:
    """Mixin providing type compatibility checks for TypeChecker."""

    def _type_matches(self, arg_ty: "InferredType", expected: "InferredType") -> bool:
        if isinstance(arg_ty, TyUnresolved): return True
        if isinstance(expected, TyAny):      return True
        if arg_ty == expected:               return True
        if isinstance(expected, TyTypeVal):
            return isinstance(arg_ty, (TyTypeValOf, TyTypeVal))
        if isinstance(expected, TyTypeValOf):
            if isinstance(arg_ty, TyTypeVal):    return True
            if isinstance(arg_ty, TyTypeValOf):
                return self._type_val_compatible(arg_ty.inner, expected.inner)
            return False
        if isinstance(expected, TyUnion):
            return any(self._type_matches(arg_ty, t) for t in expected.types)
        # Allow instance types with __cast__[ExpectedType] methods
        if isinstance(arg_ty, TyNamedInstance):
            cast_key = f"__cast__[{expected}]"
            methods = self._class_method_sigs.get(arg_ty.name)
            if methods and cast_key in methods:
                return True
        return False

    def _type_val_compatible(self, arg_inner: "InferredType", expected_inner: "InferredType") -> bool:
        if arg_inner == expected_inner:
            return True
        if not isinstance(arg_inner, TyNamedInstance):
            return False
        expected_name = str(expected_inner)
        current = arg_inner.name
        seen: set[str] = set()
        while True:
            orig = self._new_type_originals.get(current)
            if orig is None or orig in seen:
                break
            seen.add(orig)
            if orig == expected_name:
                return True
            current = orig
        bases = self._class_bases.get(arg_inner.name)
        return expected_name in bases if bases else False
