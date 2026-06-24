# git SHA: b614502cff33c6ad5e49427ca347db8ad90c31a5
"""Expression type inference mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import TYPE_CHECKING

from ..ast import (
    BinOp, UnaryOp,
    Expr, ExprInt, ExprFloat, ExprStr, ExprBool, ExprNone, ExprUndefined,
    ExprIdent, ExprLocalVar, ExprList, ExprAttr, ExprTraitAccess, ExprBinOp, ExprUnaryOp,
    ExprCall, ExprTemplateInstantiate, ExprSubscript, ExprSlice,
    ExprDict, ExprTuple, ExprSet, ExprBlock, ExprIfExpr,
    ExprForExpr, ExprWhileExpr, ExprMatchExpr, ExprIsType, ExprCast,
    MatchPatternCase,
)
from .types import (
    TyInt, TyFloat, TyStr, TyBool, TyNone, TyUndefined, TyList, TyDict, TySet, TyTuple,
    TyAny, TyUnion, TyResult, TyIntersection, TyUnresolved, TyNamedInstance,
    InferredType, inferred_type_from_ann,
)
from .errors import ErrOperationOnAny, ErrOperationOnUnion
from .binop import _infer_binop_result

if TYPE_CHECKING:
    pass


class _TypeCheckerInfer:
    """Mixin providing expression type inference for TypeChecker."""

    def _infer(self, expr: Expr) -> "InferredType":  # noqa: C901
        match expr:
            case ExprInt():      return TyInt()
            case ExprFloat():    return TyFloat()
            case ExprStr():      return TyStr()
            case ExprBool():     return TyBool()
            case ExprNone():      return TyNone()
            case ExprUndefined(): return TyUndefined()
            case ExprList():     return TyList()
            case ExprSet():      return TySet()

            case ExprTuple(elements=elements):
                return TyTuple(tuple(self._infer(e) for e in elements))

            case ExprAttr(object=obj, attr=attr):
                obj_ty = self._infer(obj)
                if isinstance(obj_ty, TyAny):
                    self._report(ErrOperationOnAny(op="attribute access"))
                elif isinstance(obj_ty, TyResult):
                    # is_OK() / is_ERR() are valid methods on Result — no error
                    if attr not in ("is_OK", "is_ERR"):
                        self._report(ErrOperationOnUnion(union_type=str(obj_ty), op="attribute access"))
                elif isinstance(obj_ty, TyUnion):
                    self._report(ErrOperationOnUnion(union_type=str(obj_ty), op="attribute access"))
                elif isinstance(obj_ty, TyIntersection):
                    pass  # intersection member access is allowed without error
                return TyUnresolved()

            case ExprTraitAccess(object=obj):
                self._infer(obj)
                return TyUnresolved()

            case ExprCall(func=func, args=args):
                return self._infer_call(func, args)

            case ExprIdent(name=name):
                info = self._lookup(name)
                return info.ty if info else TyUnresolved()

            case ExprLocalVar(name=name):
                key = f"local::{name}"
                info = self._lookup(key)
                return info.ty if info else TyUnresolved()

            case ExprUnaryOp(op=op, operand=operand):
                ty = self._infer(operand)
                op_str = {UnaryOp.NEG: "-", UnaryOp.NOT: "not", UnaryOp.BIT_NOT: "~"}[op]
                if isinstance(ty, TyAny):
                    self._report(ErrOperationOnAny(op=op_str))
                    return TyUnresolved()
                if isinstance(ty, TyUnion):
                    self._report(ErrOperationOnUnion(union_type=str(ty), op=op_str))
                    return TyUnresolved()
                if op == UnaryOp.NOT:    return TyBool()
                if op == UnaryOp.NEG:
                    if isinstance(ty, TyInt):   return TyInt()
                    if isinstance(ty, TyFloat): return TyFloat()
                    return TyUnresolved()
                return TyInt()  # BIT_NOT

            case ExprBinOp(op=op, left=left, right=right, span=span):
                lt = self._infer(left)
                rt = self._infer(right)
                self._check_binop(op, lt, rt, span)
                return _infer_binop_result(op, lt, rt)

            case ExprTemplateInstantiate(base=base):
                self._infer(base)
                return TyUnresolved()

            case ExprDict():
                return TyDict()

            case ExprSubscript(object=obj, index=idx):
                self._infer(obj)
                self._infer(idx)
                return TyUnresolved()

            case ExprSlice(begin=begin, end=end, step=step):
                if begin is not None: self._infer(begin)
                if end   is not None: self._infer(end)
                if step  is not None: self._infer(step)
                return TyNamedInstance("slice")

            case ExprIsType(expr=e):
                self._infer(e)
                return TyBool()

            case ExprCast(object=obj, type_name=tname):
                self._infer(obj)
                ty = inferred_type_from_ann(tname)
                return ty if ty is not None else TyNamedInstance(tname)

            case ExprBlock(stmts=stmts, return_type=rt):
                self._push_scope()
                self._check_stmts(stmts)
                self._pop_scope()
                return (inferred_type_from_ann(rt) or TyUnresolved()) if rt else TyUnresolved()

            case ExprIfExpr(branches=branches, else_body=else_body, return_type=rt):
                for cond, body in branches:
                    self._infer(cond)
                    self._push_scope()
                    self._check_stmts(body)
                    self._pop_scope()
                if else_body is not None:
                    self._push_scope()
                    self._check_stmts(else_body)
                    self._pop_scope()
                return (inferred_type_from_ann(rt) or TyUnresolved()) if rt else TyUnresolved()

            case ExprForExpr(iter=iter_, body=body, return_type=rt):
                self._infer(iter_)
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()
                return (inferred_type_from_ann(rt) or TyUnresolved()) if rt else TyUnresolved()

            case ExprWhileExpr(cond=cond, body=body, return_type=rt):
                self._infer(cond)
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()
                return (inferred_type_from_ann(rt) or TyUnresolved()) if rt else TyUnresolved()

            case ExprMatchExpr(subject=subject, arms=arms, return_type=rt):
                self._infer(subject)
                for arm in arms:
                    if isinstance(arm.pattern, MatchPatternCase):
                        self._infer(arm.pattern.expr)
                    self._push_scope()
                    self._check_stmts(arm.body)
                    self._pop_scope()
                return (inferred_type_from_ann(rt) or TyUnresolved()) if rt else TyUnresolved()

            case _:
                return TyUnresolved()
