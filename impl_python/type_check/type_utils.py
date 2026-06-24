# git SHA: b614502cff33c6ad5e49427ca347db8ad90c31a5
"""Type utility helpers and compatibility checking mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import TYPE_CHECKING

from .types import (
    TyInt, TyFloat, TyStr, TyBool, TyNone, TyUndefined, TyList, TyDict, TySet,
    TyTypeVal, TyTypeValOf, TyNamedInstance, TyProtocol, TyAny,
    TyUnion, TyIntersection, TyUnresolved, TyFunction, InferredType,
)

if TYPE_CHECKING:
    pass


def _type_from_guard_name(type_name: str) -> "InferredType":
    return {
        "int": TyInt(), "float": TyFloat(), "str": TyStr(), "bool": TyBool(),
        "None": TyNone(), "Undefined": TyUndefined(), "list": TyList(), "dict": TyDict(), "set": TySet(),
        "function": TyFunction(params=None, return_type=TyAny()),
    }.get(type_name, TyNamedInstance(type_name))


class _TypeCheckerUtils:
    """Mixin providing type compatibility checks for TypeChecker."""

    def _type_matches(self, arg_ty: "InferredType", expected: "InferredType") -> bool:
        if isinstance(arg_ty, TyUnresolved): return True
        if isinstance(expected, TyAny):      return True
        if arg_ty == expected:               return True
        # Protocol typed param: accept any NamedInstance or Protocol (conformance checked separately)
        if isinstance(expected, TyProtocol):
            return isinstance(arg_ty, (TyNamedInstance, TyProtocol, TyAny))
        if isinstance(expected, TyTypeVal):
            return isinstance(arg_ty, (TyTypeValOf, TyTypeVal))
        if isinstance(expected, TyTypeValOf):
            if isinstance(arg_ty, TyTypeVal):    return True
            if isinstance(arg_ty, TyTypeValOf):
                return self._type_val_compatible(arg_ty.inner, expected.inner)
            return False
        if isinstance(expected, TyUnion):
            return any(self._type_matches(arg_ty, t) for t in expected.types)
        # Intersection: arg must match ALL constituent types
        if isinstance(expected, TyIntersection):
            return all(self._type_matches(arg_ty, t) for t in expected.types)
        # arg_ty is Intersection: any constituent type matching expected is enough
        if isinstance(arg_ty, TyIntersection):
            return any(self._type_matches(t, expected) for t in arg_ty.types)
        # Allow instance types with __cast__[ExpectedType] methods or class inheritance
        if isinstance(arg_ty, TyNamedInstance):
            cast_key = f"__cast__[{expected}]"
            methods = self._class_method_sigs.get(arg_ty.name)
            if methods and cast_key in methods:
                return True
            # Check class/trait inheritance: Duck(Flyable) satisfies Flyable
            if isinstance(expected, TyNamedInstance):
                if self._class_implements_trait_py(arg_ty.name, expected.name):
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
