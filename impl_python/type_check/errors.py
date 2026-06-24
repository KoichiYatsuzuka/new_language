# git SHA: b614502cff33c6ad5e49427ca347db8ad90c31a5
"""Static type error kinds and StaticTypeError (mirrors src/type_check.rs)."""
from __future__ import annotations
from dataclasses import dataclass
from typing import Optional, TYPE_CHECKING

from ..token import Span

if TYPE_CHECKING:
    from .types import InferredType


@dataclass
class ErrIncompatibleComparison:
    lhs: "InferredType"
    rhs: "InferredType"
    op: str

@dataclass
class ErrAssignToImmutable:
    name: str

@dataclass
class ErrCallArgCountMismatch:
    func_name: str
    expected_min: int
    expected_max: int
    got: int

@dataclass
class ErrCallArgTypeMismatch:
    func_name: str
    param_index: int
    expected: "InferredType"
    got: "InferredType"

@dataclass
class ErrMissingParamTypeAnn:
    func_name: str
    param_name: str

@dataclass
class ErrMissingReturnTypeAnn:
    func_name: str

@dataclass
class ErrUnknownKeywordArg:
    func_name: str
    arg_name: str

@dataclass
class ErrNoMatchingOverload:
    func_name: str
    got: int
    available: list[int]

@dataclass
class ErrSelfTypeMismatch:
    method: str
    param_name: str
    expected_class: str
    got_class: str

@dataclass
class ErrOperationOnAny:
    op: str

@dataclass
class ErrOperationOnUnion:
    union_type: str
    op: str

@dataclass
class ErrIsNotOnNonUnion:
    var_name: str
    var_type: "InferredType"

@dataclass
class ErrCallMutParamWithImmutableArg:
    func_name: str
    param_name: str

@dataclass
class ErrInvalidDecorator:
    reason: str

@dataclass
class ErrFieldDefaultNotAllowed:
    field_name: str
    kind: str

@dataclass
class ErrProtocolConformanceFailed:
    type_name: str
    protocol_name: str
    reason: str

@dataclass
class ErrAssignUndefined:
    pass

@dataclass
class ErrIntersectionMemberConflict:
    member_name: str
    type_a: str
    type_b: str
    reason: str

@dataclass
class ErrIntersectionGuardTypeFails:
    guard_type: str
    intersection_type: str
    reason: str

@dataclass
class ErrResultSameTypes:
    ok_type: "InferredType"
    err_type: "InferredType"


TypeErrorKind = (
    ErrIncompatibleComparison | ErrAssignToImmutable | ErrCallArgCountMismatch |
    ErrCallArgTypeMismatch | ErrMissingParamTypeAnn | ErrMissingReturnTypeAnn |
    ErrUnknownKeywordArg | ErrNoMatchingOverload | ErrSelfTypeMismatch |
    ErrOperationOnAny | ErrOperationOnUnion | ErrIsNotOnNonUnion |
    ErrCallMutParamWithImmutableArg | ErrInvalidDecorator | ErrFieldDefaultNotAllowed |
    ErrProtocolConformanceFailed | ErrAssignUndefined |
    ErrIntersectionMemberConflict | ErrIntersectionGuardTypeFails | ErrResultSameTypes
)


def _format_kind(kind: "TypeErrorKind") -> str:
    match kind:
        case ErrIncompatibleComparison(lhs=lhs, rhs=rhs, op=op):
            return f"StaticTypeError: cannot compare '{lhs}' and '{rhs}' with `{op}`"
        case ErrAssignToImmutable(name=name):
            return f"StaticTypeError: cannot assign to immutable variable '{name}'"
        case ErrCallArgCountMismatch(func_name=fn, expected_min=mn, expected_max=mx, got=got):
            if mn == mx:
                return f"StaticTypeError: '{fn}' takes {mn} argument(s) but {got} were given"
            return f"StaticTypeError: '{fn}' takes {mn} to {mx} argument(s) but {got} were given"
        case ErrCallArgTypeMismatch(func_name=fn, param_index=pi, expected=ex, got=got):
            return f"StaticTypeError: argument {pi} of '{fn}' expects '{ex}' but got '{got}'"
        case ErrMissingParamTypeAnn(func_name=fn, param_name=pn):
            return f"StaticTypeError: parameter '{pn}' of function '{fn}' is missing a type annotation"
        case ErrMissingReturnTypeAnn(func_name=fn):
            return f"StaticTypeError: function '{fn}' is missing a return type annotation"
        case ErrUnknownKeywordArg(func_name=fn, arg_name=an):
            return f"StaticTypeError: '{fn}' has no parameter named '{an}'"
        case ErrNoMatchingOverload(func_name=fn, got=got, available=av):
            avail = ", ".join(str(n) for n in av)
            return (f"StaticTypeError: no overload of '{fn}' takes {got} "
                    f"argument(s) (overloads take: {avail})")
        case ErrSelfTypeMismatch(method=m, param_name=pn, expected_class=ec, got_class=gc):
            return (f"StaticTypeError: parameter '{pn}' of '{m}' "
                    f"expects 'Self' = '{ec}' but got '{gc}'")
        case ErrOperationOnAny(op=op):
            return f"StaticTypeError: cannot apply '{op}' to 'Any' - explicit downcast required"
        case ErrOperationOnUnion(union_type=ut, op=op):
            return f"StaticTypeError: cannot apply '{op}' to '{ut}' - explicit downcast required"
        case ErrIsNotOnNonUnion(var_name=vn, var_type=vt):
            return (f"StaticTypeError: 'is not' type guard on '{vn}' requires "
                    f"a Union or Optional type, but got '{vt}'")
        case ErrCallMutParamWithImmutableArg(func_name=fn, param_name=pn):
            return (f"StaticTypeError: parameter '{pn}' of '{fn}' "
                    f"expects a mutable argument, but got an immutable value")
        case ErrInvalidDecorator(reason=r):
            return f"StaticTypeError: invalid decorator: {r}"
        case ErrFieldDefaultNotAllowed(field_name=fn, kind=k):
            return (f"StaticTypeError: `{k}` field '{fn}' cannot have a default value "
                    f"in the class declaration; only `const` fields may have defaults")
        case ErrProtocolConformanceFailed(type_name=tn, protocol_name=pn, reason=r):
            return f"StaticTypeError: type '{tn}' does not satisfy protocol '{pn}': {r}"
        case ErrAssignUndefined():
            return ("StaticTypeError: cannot assign `Undefined` to a variable; "
                    "`Undefined` can only be used in conditions and type annotations")
        case ErrIntersectionMemberConflict(member_name=mn, type_a=ta, type_b=tb, reason=r):
            return (f"StaticTypeError: intersection member '{mn}' from '{ta}' and '{tb}' conflict: {r}")
        case ErrIntersectionGuardTypeFails(guard_type=gt, intersection_type=it, reason=r):
            return (f"StaticTypeError: type '{gt}' used in type guard does not satisfy '{it}': {r}")
        case ErrResultSameTypes(ok_type=ok, err_type=err):
            return (f"StaticTypeError: Result['{ok}', '{err}']: Ok type and Err type must be different")
        case _:
            return f"StaticTypeError: <unknown error {kind!r}>"


@dataclass
class StaticTypeError:
    kind: "TypeErrorKind"
    span: Optional[Span]

    def __str__(self) -> str:
        prefix = str(self.span) if self.span else "<unknown>"
        return f"{prefix}: {_format_kind(self.kind)}"
