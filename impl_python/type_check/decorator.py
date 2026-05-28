# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Decorator type checking mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import TYPE_CHECKING

from ..ast import Expr, ExprIdent
from .types import TyAny, TyTypeVal, TyTypeValOf, TyUnresolved, TyFunction, InferredType
from .errors import ErrInvalidDecorator

if TYPE_CHECKING:
    pass


class _TypeCheckerDecorator:
    """Mixin providing decorator type checking for TypeChecker."""

    def _check_decorator(self, dec: Expr, is_fn_target: bool, target_name: str) -> None:
        if not isinstance(dec, ExprIdent):
            return
        dec_name = dec.name
        expected_arg = "function" if is_fn_target else "type"
        expected_ret = "function" if is_fn_target else "type"

        sigs = self._fn_sigs.get(dec_name)
        cls_sigs = self._class_method_sigs.get(dec_name)

        def _is_ok_arg(ty: "InferredType") -> bool:
            return (isinstance(ty, TyFunction) if expected_arg == "function"
                    else isinstance(ty, (TyTypeVal, TyTypeValOf)))

        def _is_ok_ret(ty: "InferredType") -> bool:
            return (isinstance(ty, TyFunction) if expected_ret == "function"
                    else isinstance(ty, (TyTypeVal, TyTypeValOf)))

        if sigs:
            if len(sigs) != 1:
                return
            sig = sigs[0]
            if len(sig.params) != 1:
                self._report(ErrInvalidDecorator(
                    reason=f"'{dec_name}' must take exactly 1 argument, takes {len(sig.params)}"
                ))
                return
            pty = sig.params[0][1]
            if pty and not isinstance(pty, (TyUnresolved, TyAny)) and not _is_ok_arg(pty):
                self._report(ErrInvalidDecorator(
                    reason=f"'{dec_name}' parameter must be '{expected_arg}'"
                ))
            rty = sig.return_type
            if rty and not isinstance(rty, (TyUnresolved, TyAny)) and not _is_ok_ret(rty):
                self._report(ErrInvalidDecorator(
                    reason=f"'{dec_name}' must return '{expected_ret}'"
                ))

        elif cls_sigs:
            init_sigs = cls_sigs.get("__init__")
            call_sigs = cls_sigs.get("__call__")
            if init_sigs and len(init_sigs) == 1:
                params = init_sigs[0].params
                if len(params) >= 2:
                    pty = params[1][1]
                    if pty and not isinstance(pty, (TyUnresolved, TyAny)) and not _is_ok_arg(pty):
                        self._report(ErrInvalidDecorator(
                            reason=f"'{dec_name}.__init__' second param must be '{expected_arg}'"
                        ))
            if call_sigs and len(call_sigs) == 1:
                rty = call_sigs[0].return_type
                if rty and not isinstance(rty, (TyUnresolved, TyAny)) and not _is_ok_ret(rty):
                    self._report(ErrInvalidDecorator(
                        reason=f"'{dec_name}.__call__' must return '{expected_ret}'"
                    ))
