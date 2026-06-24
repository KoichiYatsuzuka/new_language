# git SHA: d4bdc21ea237938cb9213f731fd60a3fe6046b78
"""Static type checker package (mirrors src/type_check.rs).

Sub-modules:
  types      — InferredType variants and inferred_type_from_ann()
  errors     — TypeErrorKind variants and StaticTypeError
  scope      — _VarInfo, _FnSig, scope management mixin
  type_utils — type compatibility helpers and _type_from_guard_name()
  binop      — binary operator checking mixin
  call_check — call inference and argument checking mixin
  decorator  — decorator checking mixin
  infer      — expression type inference mixin
  stmt       — statement checking and signature collection mixin
"""
from __future__ import annotations
from typing import Optional

from ..ast import Stmt
from .types import (
    TyInt, TyFloat, TyStr, TyBool, TyNone, TyList, TyDict, TySet,
    TyTypeVal, TyTypeValOf, TySelfType, TyNamedInstance, TyProtocol, TyAny,
    TyUnion, TyTuple, TyNamespace, TyUnresolved, FnTypeParam, TyFunction,
    InferredType, inferred_type_from_ann,
)
from .errors import (
    ErrIncompatibleComparison, ErrAssignToImmutable, ErrCallArgCountMismatch,
    ErrCallArgTypeMismatch, ErrMissingParamTypeAnn, ErrMissingReturnTypeAnn,
    ErrUnknownKeywordArg, ErrNoMatchingOverload, ErrSelfTypeMismatch,
    ErrOperationOnAny, ErrOperationOnUnion, ErrIsNotOnNonUnion,
    ErrCallMutParamWithImmutableArg, ErrInvalidDecorator,
    TypeErrorKind, StaticTypeError,
)
from .scope import _VarInfo, _FnSig, _TypeCheckerScope
from .type_utils import _TypeCheckerUtils, _type_from_guard_name
from .binop import _TypeCheckerBinop
from .call_check import _TypeCheckerCallCheck
from .decorator import _TypeCheckerDecorator
from .infer import _TypeCheckerInfer
from .stmt import _TypeCheckerStmts


class TypeChecker(
    _TypeCheckerScope,
    _TypeCheckerUtils,
    _TypeCheckerBinop,
    _TypeCheckerCallCheck,
    _TypeCheckerDecorator,
    _TypeCheckerInfer,
    _TypeCheckerStmts,
):
    """Static type checker for Arrow source code."""

    def __init__(self) -> None:
        global_scope: dict[str, _VarInfo] = {}

        for name, inner in [
            ("int",      TyInt()),
            ("float",    TyFloat()),
            ("str",      TyStr()),
            ("bool",     TyBool()),
            ("Any",      TyAny()),
            ("function", TyFunction(params=None, return_type=TyAny())),
        ]:
            global_scope[name] = _VarInfo(ty=TyTypeValOf(inner), mutable=False)

        self._scope_stack: list[dict[str, _VarInfo]] = [global_scope]
        self._fn_sigs: dict[str, list[_FnSig]] = {}
        self._class_method_sigs: dict[str, dict[str, list[_FnSig]]] = {}
        self._known_class_names: set[str] = set()
        self._known_protocols: dict[str, bool] = {}
        self._protocol_required_members: dict[str, list[str]] = {}
        self._new_type_originals: dict[str, str] = {}
        self._class_bases: dict[str, list[str]] = {}
        self.errors: list[StaticTypeError] = []

        for cls_name, prim in [("path", "str"), ("Index", "int"), ("Size", "int")]:
            self._known_class_names.add(cls_name)
            self._new_type_originals[cls_name] = prim
        self._known_class_names.add("slice")

        for name in ("begin", "last"):
            global_scope[name] = _VarInfo(ty=TyNamedInstance("Index"), mutable=False)

    # ------------------------------------------------------------------
    # Public entry point
    # ------------------------------------------------------------------

    @staticmethod
    def check(stmts: list[Stmt]) -> list[StaticTypeError]:
        tc = TypeChecker()
        tc._collect_fn_sigs(stmts)
        tc._check_stmts(stmts)
        return tc.errors


__all__ = [
    "TypeChecker", "StaticTypeError", "FnTypeParam", "InferredType",
    "TyInt", "TyFloat", "TyStr", "TyBool", "TyNone", "TyList", "TyDict", "TySet",
    "TyTypeVal", "TyTypeValOf", "TySelfType", "TyNamedInstance", "TyProtocol", "TyAny",
    "TyUnion", "TyTuple", "TyNamespace", "TyUnresolved", "TyFunction",
    "inferred_type_from_ann",
]
