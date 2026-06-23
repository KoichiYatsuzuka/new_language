# git SHA: aea2e1fe6909a7aed9643a2e7184f19fd0195ccc
"""Binary operator type checking mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import TYPE_CHECKING

from ..token import Span
from ..ast import BinOp
from .types import (
    TyInt, TyFloat, TyStr, TySet, TyAny, TyUnion, TyUnresolved,
    TyBool, InferredType,
)
from .errors import ErrOperationOnAny, ErrOperationOnUnion, ErrIncompatibleComparison

if TYPE_CHECKING:
    pass


def _ordered_comparable(lt: "InferredType", rt: "InferredType") -> bool:
    if isinstance(lt, TyUnresolved) or isinstance(rt, TyUnresolved):
        return True
    return (
        (isinstance(lt, TyInt)   and isinstance(rt, TyInt))   or
        (isinstance(lt, TyFloat) and isinstance(rt, TyFloat)) or
        (isinstance(lt, TyInt)   and isinstance(rt, TyFloat)) or
        (isinstance(lt, TyFloat) and isinstance(rt, TyInt))   or
        (isinstance(lt, TyStr)   and isinstance(rt, TyStr))
    )


def _infer_binop_result(op: BinOp, lt: "InferredType", rt: "InferredType") -> "InferredType":
    if isinstance(lt, TyAny) or isinstance(rt, TyAny):           return TyUnresolved()
    if isinstance(lt, TyUnion) or isinstance(rt, TyUnion):       return TyUnresolved()

    if isinstance(lt, TySet) and isinstance(rt, TySet):
        if op in (BinOp.BIT_OR, BinOp.BIT_AND, BinOp.BIT_XOR, BinOp.SUB): return TySet()
        if op in (BinOp.EQ, BinOp.REF_EQ, BinOp.NOT_EQ):                    return TyBool()
        return TyUnresolved()

    if op in (BinOp.EQ, BinOp.REF_EQ, BinOp.NOT_EQ, BinOp.LT, BinOp.GT, BinOp.LT_EQ, BinOp.GT_EQ,
              BinOp.AND, BinOp.OR, BinOp.IN, BinOp.NOT_IN):
        return TyBool()

    if op == BinOp.ADD:
        if isinstance(lt, TyInt)   and isinstance(rt, TyInt):                     return TyInt()
        if isinstance(lt, TyStr)   and isinstance(rt, TyStr):                     return TyStr()
        if isinstance(lt, (TyInt, TyFloat)) and isinstance(rt, (TyInt, TyFloat)): return TyFloat()
        return TyUnresolved()

    if op in (BinOp.SUB, BinOp.MUL, BinOp.POW):
        if isinstance(lt, TyInt)   and isinstance(rt, TyInt):                     return TyInt()
        if isinstance(lt, (TyInt, TyFloat)) and isinstance(rt, (TyInt, TyFloat)): return TyFloat()
        return TyUnresolved()

    if op == BinOp.DIV:
        return TyFloat()

    if op in (BinOp.FLOOR_DIV, BinOp.MOD):
        return TyInt() if isinstance(lt, TyInt) and isinstance(rt, TyInt) else TyUnresolved()

    if op in (BinOp.BIT_AND, BinOp.BIT_OR, BinOp.BIT_XOR, BinOp.L_SHIFT, BinOp.R_SHIFT):
        return TyInt()

    return TyUnresolved()


class _TypeCheckerBinop:
    """Mixin providing binary operator type checking for TypeChecker."""

    def _check_binop(
        self, op: BinOp, lt: "InferredType", rt: "InferredType", span: Span
    ) -> None:
        if isinstance(lt, TyAny) or isinstance(rt, TyAny):
            self._report(ErrOperationOnAny(op=op.as_str()), span)
            return
        union_side = lt if isinstance(lt, TyUnion) else (rt if isinstance(rt, TyUnion) else None)
        if union_side is not None:
            self._report(ErrOperationOnUnion(union_type=str(union_side), op=op.as_str()), span)
            return
        if op in (BinOp.LT, BinOp.GT, BinOp.LT_EQ, BinOp.GT_EQ):
            if not _ordered_comparable(lt, rt):
                self._report(ErrIncompatibleComparison(lhs=lt, rhs=rt, op=op.as_str()), span)
