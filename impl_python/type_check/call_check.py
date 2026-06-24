# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Call inference and argument checking mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import Optional, TYPE_CHECKING

from ..ast import (
    Expr, ExprIdent, ExprAttr,
    CallArg, CallArgPositional, CallArgKeyword, CallArgVariadic,
)
from .types import (
    TyAny, TyBool, TyNamedInstance, TyFunction, TyUnresolved, TyResult, FnTypeParam, InferredType,
)
from .errors import (
    ErrCallArgCountMismatch, ErrCallArgTypeMismatch, ErrUnknownKeywordArg,
    ErrNoMatchingOverload, ErrSelfTypeMismatch, ErrCallMutParamWithImmutableArg,
    ErrIsNotOnNonUnion,
)

if TYPE_CHECKING:
    pass


class _TypeCheckerCallCheck:
    """Mixin providing call inference and argument checking for TypeChecker."""

    def _infer_call(self, func: Expr, args: list[CallArg]) -> "InferredType":
        # Result[T, E].is_OK() / is_ERR() → TyBool
        if isinstance(func, ExprAttr) and func.attr in ("is_OK", "is_ERR") and not args:
            if isinstance(func.object, ExprIdent):
                obj_info = self._lookup(func.object.name)
                if obj_info is not None and isinstance(obj_info.ty, TyResult):
                    return TyBool()

        method_call_info: Optional[tuple[str, str]] = None
        if isinstance(func, ExprAttr) and isinstance(func.object, ExprIdent):
            obj_info = self._lookup(func.object.name)
            if obj_info and isinstance(obj_info.ty, TyNamedInstance):
                method_call_info = (obj_info.ty.name, func.attr)

        func_name: Optional[str] = func.name if isinstance(func, ExprIdent) else None
        func_type = self._infer(func)

        arg_data: list[tuple[Optional[str], "InferredType"]] = []
        for arg in args:
            if isinstance(arg, CallArgPositional):
                arg_data.append((None, self._infer(arg.expr)))
            elif isinstance(arg, CallArgVariadic):
                from .types import TyList
                for e in arg.exprs:
                    self._infer(e)  # type-check each element expression
                arg_data.append(("...", TyList()))
            else:
                arg_data.append((arg.name, self._infer(arg.value)))

        if isinstance(func_type, TyFunction):
            if func_type.params is not None:
                fname = func_name or "<function>"
                self._check_fn_type_call(fname, args, arg_data, list(func_type.params))
                return func_type.return_type
            return TyAny()

        if method_call_info is not None:
            self._check_self_type_params(method_call_info[0], method_call_info[1], arg_data)

        if func_name is not None:
            self._check_call_args(func_name, arg_data)

        if func_name and func_name in self._known_class_names:
            return TyNamedInstance(func_name)

        if func_name and func_name in self._fn_sigs:
            sigs = self._fn_sigs[func_name]
            n = sum(1 for k, _ in arg_data if k != "...")
            matching = [s for s in sigs if s.required_count <= n <= len(s.params)]
            if len(matching) == 1 and matching[0].return_type is not None:
                return matching[0].return_type

        return TyUnresolved()

    def _check_self_type_params(
        self,
        cls_name: str,
        method_name: str,
        arg_data: list[tuple[Optional[str], "InferredType"]],
    ) -> None:
        from .types import TySelfType
        sigs = (self._class_method_sigs.get(cls_name) or {}).get(method_name)
        if not sigs:
            return
        normal_args = [(k, t) for k, t in arg_data if k != "..."]
        effective = len(normal_args) + 1
        count_ok = [s for s in sigs if s.required_count <= effective <= len(s.params)]
        if len(count_ok) != 1:
            return
        sig = count_ok[0]
        for arg_idx, (_, arg_ty) in enumerate(normal_args):
            pidx = arg_idx + 1
            if pidx >= len(sig.params):
                break
            pname, pty = sig.params[pidx]
            if pty == TySelfType() and isinstance(arg_ty, TyNamedInstance):
                if arg_ty.name != cls_name:
                    self._report(ErrSelfTypeMismatch(
                        method=method_name, param_name=pname,
                        expected_class=cls_name, got_class=arg_ty.name,
                    ))

    def _check_call_args(
        self,
        fname: str,
        arg_data: list[tuple[Optional[str], "InferredType"]],
    ) -> None:
        sigs = self._fn_sigs.get(fname)
        if not sigs:
            return
        normal_args = [(k, t) for k, t in arg_data if k != "..."]
        n = len(normal_args)
        count_ok = [s for s in sigs if s.required_count <= n <= len(s.params)]

        if not count_ok:
            if len(sigs) == 1:
                self._report(ErrCallArgCountMismatch(
                    func_name=fname, expected_min=sigs[0].required_count,
                    expected_max=len(sigs[0].params), got=n,
                ))
            else:
                self._report(ErrNoMatchingOverload(
                    func_name=fname, got=n, available=[len(s.params) for s in sigs],
                ))
            return

        if len(count_ok) > 1:
            return

        sig = count_ok[0]
        positional_idx = 0
        for key, arg_ty in normal_args:
            if key is not None:
                pos = next((i for i, (n2, _) in enumerate(sig.params) if n2 == key), None)
                if pos is None:
                    self._report(ErrUnknownKeywordArg(func_name=fname, arg_name=key))
                else:
                    expected = sig.params[pos][1]
                    if expected and not self._type_matches(arg_ty, expected):
                        self._report(ErrCallArgTypeMismatch(
                            func_name=fname, param_index=pos, expected=expected, got=arg_ty,
                        ))
            else:
                if positional_idx < len(sig.params):
                    expected = sig.params[positional_idx][1]
                    if expected and not self._type_matches(arg_ty, expected):
                        self._report(ErrCallArgTypeMismatch(
                            func_name=fname, param_index=positional_idx,
                            expected=expected, got=arg_ty,
                        ))
                positional_idx += 1

    def _check_fn_type_call(
        self,
        func_name: str,
        args: list[CallArg],
        arg_data: list[tuple[Optional[str], "InferredType"]],
        params: list[FnTypeParam],
    ) -> None:
        if len(arg_data) != len(params):
            self._report(ErrCallArgCountMismatch(
                func_name=func_name, expected_min=len(params),
                expected_max=len(params), got=len(arg_data),
            ))
            return

        positional_idx = 0
        for i, (key, arg_ty) in enumerate(arg_data):
            arg_expr = args[i].get_expr()
            if key is not None:
                pos = next((j for j, p in enumerate(params) if p.name == key), None)
                if pos is None:
                    self._report(ErrUnknownKeywordArg(func_name=func_name, arg_name=key))
                else:
                    param = params[pos]
                    if param.ty != TyAny() and not self._type_matches(arg_ty, param.ty):
                        self._report(ErrCallArgTypeMismatch(
                            func_name=func_name, param_index=pos,
                            expected=param.ty, got=arg_ty,
                        ))
                    if param.mutable and not self._is_mutable_expr(arg_expr):
                        self._report(ErrCallMutParamWithImmutableArg(
                            func_name=func_name, param_name=param.name,
                        ))
            else:
                if positional_idx < len(params):
                    param = params[positional_idx]
                    if param.ty != TyAny() and not self._type_matches(arg_ty, param.ty):
                        self._report(ErrCallArgTypeMismatch(
                            func_name=func_name, param_index=positional_idx,
                            expected=param.ty, got=arg_ty,
                        ))
                    if param.mutable and not self._is_mutable_expr(arg_expr):
                        self._report(ErrCallMutParamWithImmutableArg(
                            func_name=func_name, param_name=param.name,
                        ))
                positional_idx += 1

    def _is_mutable_expr(self, expr: Expr) -> bool:
        if isinstance(expr, ExprIdent):
            info = self._lookup(expr.name)
            return info.mutable if info else False
        return False
