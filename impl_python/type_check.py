from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional

from .token import Span
from .ast import (
    BinOp, UnaryOp, FieldKind,
    CallArg, CallArgPositional, CallArgKeyword,
    Expr, ExprInt, ExprFloat, ExprStr, ExprBool, ExprNone, ExprIdent,
    ExprList, ExprAttr, ExprTraitAccess, ExprBinOp, ExprUnaryOp,
    ExprCall, ExprTemplateInstantiate, ExprSubscript, ExprSlice,
    ExprDict, ExprTuple, ExprSet, ExprBlock, ExprIfExpr,
    ExprForExpr, ExprWhileExpr, ExprMatchExpr, ExprIsType,
    Stmt, StmtExpr, StmtLet, StmtConst, StmtMut, StmtStatic,
    StmtAssign, StmtAttrAssign, StmtAttrCompoundAssign, StmtCompoundAssign,
    StmtIf, StmtMatch, StmtWhile, StmtFor, StmtBlock,
    StmtReturn, StmtBreak, StmtContinue, StmtPass,
    StmtBlockReturn, StmtLoopYield, StmtYield, StmtFreeze,
    StmtFnDef, StmtGenDef, StmtClassDef, StmtTraitDef, StmtField,
    StmtNewTypeDef, StmtEnumDef, StmtTry, StmtRaise,
    StmtImport, StmtFromImport,
    MatchPatternCase, MatchPatternIsType,
)


# ---------------------------------------------------------------------------
# InferredType
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class TyInt:
    def __str__(self) -> str: return "int"

@dataclass(frozen=True)
class TyFloat:
    def __str__(self) -> str: return "float"

@dataclass(frozen=True)
class TyStr:
    def __str__(self) -> str: return "str"

@dataclass(frozen=True)
class TyBool:
    def __str__(self) -> str: return "bool"

@dataclass(frozen=True)
class TyNone:
    def __str__(self) -> str: return "None"

@dataclass(frozen=True)
class TyList:
    def __str__(self) -> str: return "list"

@dataclass(frozen=True)
class TyDict:
    def __str__(self) -> str: return "dict"

@dataclass(frozen=True)
class TySet:
    def __str__(self) -> str: return "set"

@dataclass(frozen=True)
class TyTypeVal:
    def __str__(self) -> str: return "type"

@dataclass(frozen=True)
class TyTypeValOf:
    inner: "InferredType"
    def __str__(self) -> str: return f"type[{self.inner}]"

@dataclass(frozen=True)
class TySelfType:
    def __str__(self) -> str: return "Self"

@dataclass(frozen=True)
class TyNamedInstance:
    name: str
    def __str__(self) -> str: return self.name

@dataclass(frozen=True)
class TyAny:
    def __str__(self) -> str: return "Any"

@dataclass(frozen=True)
class TyUnion:
    types: tuple["InferredType", ...]

    def __str__(self) -> str:
        if len(self.types) == 2 and self.types[1] == TyNone():
            return f"Option[{self.types[0]}]"
        return "Union[" + ", ".join(str(t) for t in self.types) + "]"

@dataclass(frozen=True)
class TyTuple:
    types: tuple["InferredType", ...]
    def __str__(self) -> str:
        return "tuple[" + ", ".join(str(t) for t in self.types) + "]"

@dataclass(frozen=True)
class TyNamespace:
    members: tuple[tuple[str, "InferredType"], ...]

    def as_dict(self) -> dict[str, "InferredType"]:
        return dict(self.members)

    def __str__(self) -> str:
        return f"<module({len(self.members)} members)>"

@dataclass(frozen=True)
class TyUnresolved:
    def __str__(self) -> str: return "unknown"

@dataclass(frozen=True)
class FnTypeParam:
    name: str
    mutable: bool
    ty: "InferredType"

@dataclass(frozen=True)
class TyFunction:
    params: Optional[tuple[FnTypeParam, ...]]
    return_type: "InferredType"

    def __str__(self) -> str:
        if self.params is None:
            result = "function"
        else:
            parts = [
                f"{'mut' if p.mutable else 'let'} {p.name}:{p.ty}"
                for p in self.params
            ]
            result = "function{" + ",".join(parts) + "}"
        if self.return_type != TyAny():
            result += f"->{self.return_type}"
        return result


InferredType = (
    TyInt | TyFloat | TyStr | TyBool | TyNone | TyList | TyDict | TySet |
    TyTypeVal | TyTypeValOf | TySelfType | TyNamedInstance | TyAny |
    TyUnion | TyTuple | TyNamespace | TyUnresolved | TyFunction
)


# ---------------------------------------------------------------------------
# Type annotation string -> InferredType
# ---------------------------------------------------------------------------

def _split_top_level_commas(s: str) -> list[str]:
    result: list[str] = []
    depth = 0
    start = 0
    for i, c in enumerate(s):
        if c == "[":
            depth += 1
        elif c == "]":
            if depth > 0:
                depth -= 1
        elif c == "," and depth == 0:
            result.append(s[start:i])
            start = i + 1
    result.append(s[start:])
    return result


def _split_top_level_commas_fn(s: str) -> list[str]:
    result: list[str] = []
    depth = 0
    start = 0
    for i, c in enumerate(s):
        if c in ("[", "{"):
            depth += 1
        elif c in ("]", "}"):
            if depth > 0:
                depth -= 1
        elif c == "," and depth == 0:
            result.append(s[start:i])
            start = i + 1
    result.append(s[start:])
    return result


def _find_closing_bracket(s: str, open_ch: str, close_ch: str) -> Optional[int]:
    depth = 0
    for i, c in enumerate(s):
        if c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return i
    return None


def _parse_fn_type_ann(rest: str) -> Optional["InferredType"]:
    params: Optional[list[FnTypeParam]]
    after: str

    if rest.startswith("["):
        close = _find_closing_bracket(rest, "[", "]")
        if close is None:
            return None
        inner = rest[1:close]
        after = rest[close + 1:]
        if not inner.strip():
            params = []
        else:
            params = []
            for idx, part in enumerate(_split_top_level_commas_fn(inner)):
                p = part.strip()
                if p.startswith("mut "):
                    mutable, type_str = True, p[4:].strip()
                elif p.startswith("let "):
                    mutable, type_str = False, p[4:].strip()
                else:
                    mutable, type_str = False, p
                colon = type_str.find(":")
                if colon != -1:
                    name = type_str[:colon].strip()
                    ty_s = type_str[colon + 1:].strip()
                else:
                    name = f"param{idx + 1}"
                    ty_s = type_str
                ty = inferred_type_from_ann(ty_s) or TyAny()
                params.append(FnTypeParam(name=name, mutable=mutable, ty=ty))
    elif rest.startswith("{"):
        close = _find_closing_bracket(rest, "{", "}")
        if close is None:
            return None
        inner = rest[1:close]
        after = rest[close + 1:]
        if not inner.strip():
            params = []
        else:
            params = []
            for part in _split_top_level_commas_fn(inner):
                p = part.strip()
                if p.startswith("mut "):
                    mutable, rest_p = True, p[4:].strip()
                elif p.startswith("let "):
                    mutable, rest_p = False, p[4:].strip()
                else:
                    mutable, rest_p = False, p
                colon = rest_p.find(":")
                if colon == -1:
                    return None
                name = rest_p[:colon].strip()
                ty_s = rest_p[colon + 1:].strip()
                ty = inferred_type_from_ann(ty_s) or TyAny()
                params.append(FnTypeParam(name=name, mutable=mutable, ty=ty))
    else:
        params = None
        after = rest

    if after.startswith("->"):
        return_type = inferred_type_from_ann(after[2:].strip()) or TyAny()
    else:
        return_type = TyAny()

    return TyFunction(
        params=tuple(params) if params is not None else None,
        return_type=return_type,
    )


def inferred_type_from_ann(ann: str) -> Optional["InferredType"]:
    if ann.startswith("Union[") and ann.endswith("]"):
        inner = ann[6:-1]
        parts = _split_top_level_commas(inner)
        resolved = [t for t in (inferred_type_from_ann(p.strip()) for p in parts) if t is not None]
        return TyUnion(tuple(resolved)) if len(resolved) >= 2 else None

    if ann.startswith("Option[") and ann.endswith("]"):
        t = inferred_type_from_ann(ann[7:-1].strip())
        return TyUnion((t, TyNone())) if t is not None else None

    if ann.startswith("list[") and ann.endswith("]"):
        return TyList()
    if ann.startswith("set[") and ann.endswith("]"):
        return TySet()
    if ann.startswith("dict[") and ann.endswith("]"):
        return TyDict()

    if ann.startswith("tuple[") and ann.endswith("]"):
        inner = ann[6:-1]
        parts = _split_top_level_commas(inner)
        resolved = [t for t in (inferred_type_from_ann(p.strip()) for p in parts) if t is not None]
        return TyTuple(tuple(resolved))

    if ann.startswith("type[") and ann.endswith("]"):
        inner = ann[5:-1].strip()
        inner_ty = inferred_type_from_ann(inner)
        if inner_ty is None and inner and all(c.isalnum() or c == "_" for c in inner) and inner[0].isalpha():
            inner_ty = TyNamedInstance(inner)
        return TyTypeValOf(inner_ty) if inner_ty is not None else None

    if ann.startswith("function"):
        return _parse_fn_type_ann(ann[8:])

    return {
        "int": TyInt(), "float": TyFloat(), "str": TyStr(), "bool": TyBool(),
        "None": TyNone(), "list": TyList(), "dict": TyDict(), "set": TySet(),
        "type": TyTypeVal(), "Self": TySelfType(), "Any": TyAny(),
    }.get(ann)


# ---------------------------------------------------------------------------
# Error types
# ---------------------------------------------------------------------------

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


TypeErrorKind = (
    ErrIncompatibleComparison | ErrAssignToImmutable | ErrCallArgCountMismatch |
    ErrCallArgTypeMismatch | ErrMissingParamTypeAnn | ErrMissingReturnTypeAnn |
    ErrUnknownKeywordArg | ErrNoMatchingOverload | ErrSelfTypeMismatch |
    ErrOperationOnAny | ErrOperationOnUnion | ErrIsNotOnNonUnion |
    ErrCallMutParamWithImmutableArg | ErrInvalidDecorator
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
        case _:
            return f"StaticTypeError: <unknown error {kind!r}>"


@dataclass
class StaticTypeError:
    kind: "TypeErrorKind"
    span: Optional[Span]

    def __str__(self) -> str:
        prefix = str(self.span) if self.span else "<unknown>"
        return f"{prefix}: {_format_kind(self.kind)}"


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

@dataclass
class _VarInfo:
    ty: "InferredType"
    mutable: bool


@dataclass
class _FnSig:
    params: list[tuple[str, Optional["InferredType"]]]
    required_count: int
    return_type: Optional["InferredType"]


# ---------------------------------------------------------------------------
# TypeChecker
# ---------------------------------------------------------------------------

class TypeChecker:
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

    # ------------------------------------------------------------------
    # Pre-pass: collect function / class signatures
    # ------------------------------------------------------------------

    def _collect_fn_sigs(self, stmts: list[Stmt]) -> None:
        for stmt in stmts:
            match stmt:
                case StmtFnDef(name=name, params=params, return_type=rt, body=body):
                    sig = _FnSig(
                        params=[(p.name, inferred_type_from_ann(p.type_ann) if p.type_ann else None)
                                for p in params],
                        required_count=sum(1 for p in params if p.default is None),
                        return_type=inferred_type_from_ann(rt) if rt else None,
                    )
                    self._fn_sigs.setdefault(name, []).append(sig)
                    self._collect_fn_sigs(body)

                case StmtClassDef(name=name, bases=bases, body=body):
                    self._known_class_names.add(name)
                    self._class_bases[name] = bases
                    cls_methods: dict[str, list[_FnSig]] = {}
                    for s in body:
                        if isinstance(s, StmtFnDef):
                            sig = _FnSig(
                                params=[(p.name, inferred_type_from_ann(p.type_ann) if p.type_ann else None)
                                        for p in s.params],
                                required_count=sum(1 for p in s.params if p.default is None),
                                return_type=inferred_type_from_ann(s.return_type) if s.return_type else None,
                            )
                            cls_methods.setdefault(s.name, []).append(sig)
                    self._class_method_sigs[name] = cls_methods
                    self._collect_fn_sigs(body)

                case StmtEnumDef(name=name):
                    self._known_class_names.add(name)
                    self._known_class_names.add(f"enum_item_{name}")

                case StmtTraitDef(body=body):
                    self._collect_fn_sigs(body)

                case StmtMatch(arms=arms):
                    for arm in arms:
                        self._collect_fn_sigs(arm.body)

                case StmtIf(branches=branches, else_body=else_body):
                    for _, body in branches:
                        self._collect_fn_sigs(body)
                    if else_body:
                        self._collect_fn_sigs(else_body)

                case StmtWhile(body=body) | StmtFor(body=body) | StmtBlock(stmts=body):
                    self._collect_fn_sigs(body)

        for stmt in stmts:
            if isinstance(stmt, StmtNewTypeDef):
                self._known_class_names.add(stmt.name)
                self._new_type_originals[stmt.name] = stmt.original
                if stmt.original in self._class_method_sigs:
                    self._class_method_sigs[stmt.name] = self._class_method_sigs[stmt.original]

    # ------------------------------------------------------------------
    # Scope helpers
    # ------------------------------------------------------------------

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
        self.errors.append(StaticTypeError(kind=kind, span=span))

    # ------------------------------------------------------------------
    # Statement checking
    # ------------------------------------------------------------------

    def _check_stmts(self, stmts: list[Stmt]) -> None:
        for stmt in stmts:
            self._check_stmt(stmt)

    def _check_stmt(self, stmt: Stmt) -> None:  # noqa: C901
        match stmt:
            case StmtLet(name=name, expr=expr):
                self._declare(name, self._infer(expr), False)

            case StmtConst(name=name, expr=expr):
                self._declare(name, self._infer(expr), False)

            case StmtMut(name=name, expr=expr):
                self._declare(name, self._infer(expr), True)

            case StmtStatic(name=name, expr=expr):
                self._declare(name, self._infer(expr), True)

            case StmtAssign(name=name, value=value, span=span):
                info = self._lookup(name)
                if info is not None and not info.mutable:
                    self._report(ErrAssignToImmutable(name), span)
                self._infer(value)

            case StmtCompoundAssign(name=name, value=value, span=span):
                info = self._lookup(name)
                if info is not None and not info.mutable:
                    self._report(ErrAssignToImmutable(name), span)
                self._infer(value)

            case StmtAttrAssign(target=target, value=value):
                self._infer(target)
                self._infer(value)

            case StmtAttrCompoundAssign(target=target, value=value):
                self._infer(target)
                self._infer(value)

            case StmtExpr(expr=expr):
                self._infer(expr)

            case StmtIf(branches=branches, else_body=else_body):
                for cond, body in branches:
                    guard_opt: Optional[tuple[str, str, bool, Span]] = None
                    if isinstance(cond, ExprIsType) and isinstance(cond.expr, ExprIdent):
                        guard_opt = (cond.expr.name, cond.type_name, cond.negated, cond.span)

                    narrowed: Optional[tuple[str, "InferredType", bool]] = None
                    error_info: Optional[tuple[str, "InferredType", Span]] = None

                    if guard_opt is not None:
                        var_name, type_name, negated, span = guard_opt
                        guard_ty = _type_from_guard_name(type_name)
                        info = self._lookup(var_name)
                        var_ty: "InferredType" = info.ty if info else TyUnresolved()
                        is_mut = info.mutable if info else False

                        if negated:
                            if isinstance(var_ty, TyUnion):
                                remaining = [t for t in var_ty.types if t != guard_ty]
                                if len(remaining) == 0:
                                    narrowed_ty: "InferredType" = TyUnresolved()
                                elif len(remaining) == 1:
                                    narrowed_ty = remaining[0]
                                else:
                                    narrowed_ty = TyUnion(tuple(remaining))
                                narrowed = (var_name, narrowed_ty, is_mut)
                            elif not isinstance(var_ty, TyUnresolved):
                                error_info = (var_name, var_ty, span)
                        else:
                            narrowed = (var_name, guard_ty, is_mut)

                    self._infer(cond)

                    if error_info is not None:
                        vn, vt, sp = error_info
                        self._report(ErrIsNotOnNonUnion(var_name=vn, var_type=vt), sp)

                    self._push_scope()
                    if narrowed is not None:
                        self._declare(narrowed[0], narrowed[1], narrowed[2])
                    self._check_stmts(body)
                    self._pop_scope()

                if else_body is not None:
                    self._push_scope()
                    self._check_stmts(else_body)
                    self._pop_scope()

            case StmtMatch(subject=subject, arms=arms):
                self._infer(subject)
                subject_name: Optional[str] = subject.name if isinstance(subject, ExprIdent) else None
                for arm in arms:
                    self._push_scope()
                    if isinstance(arm.pattern, MatchPatternCase):
                        self._infer(arm.pattern.expr)
                    elif isinstance(arm.pattern, MatchPatternIsType) and subject_name is not None:
                        info = self._lookup(subject_name)
                        self._declare(subject_name, _type_from_guard_name(arm.pattern.type_name),
                                      info.mutable if info else False)
                    self._check_stmts(arm.body)
                    self._pop_scope()

            case StmtWhile(cond=cond, body=body):
                self._infer(cond)
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()

            case StmtFor(targets=targets, iter=iter_, body=body):
                self._infer(iter_)
                self._push_scope()
                for target in targets:
                    self._declare(target, TyUnresolved(), True)
                self._check_stmts(body)
                self._pop_scope()

            case StmtBlock(stmts=stmts):
                self._push_scope()
                self._check_stmts(stmts)
                self._pop_scope()

            case StmtFnDef(name=name, params=params, return_type=rt, body=body, decorators=decs):
                for dec in decs:
                    self._check_decorator(dec, True, name)
                for p in params:
                    if p.name != "self" and p.type_ann is None:
                        self._report(ErrMissingParamTypeAnn(func_name=name, param_name=p.name))
                if rt is None:
                    self._report(ErrMissingReturnTypeAnn(func_name=name))
                self._declare(name, TyUnresolved(), False)
                self._push_scope()
                for p in params:
                    ty = (inferred_type_from_ann(p.type_ann) if p.type_ann else None) or TyUnresolved()
                    self._declare(p.name, ty, p.mutable)
                self._check_stmts(body)
                self._pop_scope()

            case StmtClassDef(name=name, body=body, decorators=decs):
                for dec in decs:
                    self._check_decorator(dec, False, name)
                self._declare(name, TyTypeValOf(TyNamedInstance(name)), False)
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()

            case StmtTraitDef(name=name, body=body):
                self._declare(name, TyTypeValOf(TyNamedInstance(name)), False)
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()

            case StmtReturn(expr=expr):
                if expr is not None:
                    self._infer(expr)

            case StmtBlockReturn(expr=expr) | StmtLoopYield(expr=expr) | StmtYield(expr=expr):
                self._infer(expr)

            case StmtField(name=name, kind=kind, type_ann=type_ann, default=default):
                ty = inferred_type_from_ann(type_ann) or TyUnresolved()
                if default is not None:
                    self._infer(default)
                self._declare(name, ty, kind == FieldKind.MUT)

            case StmtGenDef(name=name, params=params, yield_type=yt, body=body):
                for p in params:
                    if p.name != "self" and p.type_ann is None:
                        self._report(ErrMissingParamTypeAnn(func_name=name, param_name=p.name))
                if yt is None:
                    self._report(ErrMissingReturnTypeAnn(func_name=name))
                self._declare(name, TyUnresolved(), False)
                self._push_scope()
                for p in params:
                    ty = (inferred_type_from_ann(p.type_ann) if p.type_ann else None) or TyUnresolved()
                    self._declare(p.name, ty, p.mutable)
                self._check_stmts(body)
                self._pop_scope()

            case StmtNewTypeDef(name=name):
                self._declare(name, TyTypeValOf(TyNamedInstance(name)), False)

            case StmtEnumDef(name=name):
                item = f"enum_item_{name}"
                self._declare(item, TyTypeValOf(TyNamedInstance(item)), False)
                self._declare(name, TyTypeValOf(TyNamedInstance(name)), False)

            case StmtPass() | StmtBreak() | StmtContinue() | StmtFreeze():
                pass

            case StmtTry(body=body, handlers=handlers, finally_body=fb):
                self._push_scope()
                self._check_stmts(body)
                self._pop_scope()
                for handler in handlers:
                    self._push_scope()
                    if handler.name:
                        self._declare(handler.name, TyUnresolved(), True)
                    self._check_stmts(handler.body)
                    self._pop_scope()
                if fb is not None:
                    self._push_scope()
                    self._check_stmts(fb)
                    self._pop_scope()

            case StmtRaise(exc=exc):
                if exc is not None:
                    self._infer(exc)

            case StmtImport(module=module, alias=alias, body=body):
                member_types = self._collect_module_types(body)
                bind_name = alias if alias else module[-1]
                self._declare(bind_name, TyNamespace(tuple(member_types.items())), False)

            case StmtFromImport(names=names, body=body):
                member_types = self._collect_module_types(body)
                for orig_name, alias in names:
                    bind_name = alias if alias else orig_name
                    self._declare(bind_name, member_types.get(orig_name, TyUnresolved()), False)

    def _collect_module_types(self, body: list[Stmt]) -> dict[str, "InferredType"]:
        result: dict[str, "InferredType"] = {}
        for stmt in body:
            match stmt:
                case StmtClassDef(name=name):
                    result[name] = TyTypeValOf(TyNamedInstance(name))
                case StmtFnDef(name=name):
                    result[name] = TyUnresolved()
                case StmtMut(name=name) | StmtLet(name=name) | StmtConst(name=name):
                    result[name] = TyUnresolved()
                case StmtStatic(name=name):
                    result[name] = TyUnresolved()
        return result

    # ------------------------------------------------------------------
    # Expression inference
    # ------------------------------------------------------------------

    def _infer(self, expr: Expr) -> "InferredType":  # noqa: C901
        match expr:
            case ExprInt():      return TyInt()
            case ExprFloat():    return TyFloat()
            case ExprStr():      return TyStr()
            case ExprBool():     return TyBool()
            case ExprNone():     return TyNone()
            case ExprList():     return TyList()
            case ExprSet():      return TySet()

            case ExprTuple(elements=elements):
                return TyTuple(tuple(self._infer(e) for e in elements))

            case ExprAttr(object=obj):
                obj_ty = self._infer(obj)
                if isinstance(obj_ty, TyAny):
                    self._report(ErrOperationOnAny(op="attribute access"))
                elif isinstance(obj_ty, TyUnion):
                    self._report(ErrOperationOnUnion(union_type=str(obj_ty), op="attribute access"))
                return TyUnresolved()

            case ExprTraitAccess(object=obj):
                self._infer(obj)
                return TyUnresolved()

            case ExprCall(func=func, args=args):
                return self._infer_call(func, args)

            case ExprIdent(name=name):
                info = self._lookup(name)
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

    # ------------------------------------------------------------------
    # Call inference
    # ------------------------------------------------------------------

    def _infer_call(self, func: Expr, args: list[CallArg]) -> "InferredType":
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
            n = len(arg_data)
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
        sigs = (self._class_method_sigs.get(cls_name) or {}).get(method_name)
        if not sigs:
            return
        effective = len(arg_data) + 1
        count_ok = [s for s in sigs if s.required_count <= effective <= len(s.params)]
        if len(count_ok) != 1:
            return
        sig = count_ok[0]
        for arg_idx, (_, arg_ty) in enumerate(arg_data):
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
        n = len(arg_data)
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
        for key, arg_ty in arg_data:
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

    # ------------------------------------------------------------------
    # Binary operator checking
    # ------------------------------------------------------------------

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

    # ------------------------------------------------------------------
    # Type compatibility
    # ------------------------------------------------------------------

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
            return arg_ty in expected.types
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

    # ------------------------------------------------------------------
    # Decorator checking
    # ------------------------------------------------------------------

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


# ---------------------------------------------------------------------------
# Module-level helpers
# ---------------------------------------------------------------------------

def _type_from_guard_name(type_name: str) -> "InferredType":
    return {
        "int": TyInt(), "float": TyFloat(), "str": TyStr(), "bool": TyBool(),
        "None": TyNone(), "list": TyList(), "dict": TyDict(), "set": TySet(),
        "function": TyFunction(params=None, return_type=TyAny()),
    }.get(type_name, TyNamedInstance(type_name))


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
        if op in (BinOp.EQ, BinOp.NOT_EQ):                                  return TyBool()
        return TyUnresolved()

    if op in (BinOp.EQ, BinOp.NOT_EQ, BinOp.LT, BinOp.GT, BinOp.LT_EQ, BinOp.GT_EQ,
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
