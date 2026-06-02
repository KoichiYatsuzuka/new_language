# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""Statement type checking and signature collection mixin (mirrors src/type_check.rs)."""
from __future__ import annotations
from typing import Optional, TYPE_CHECKING

from ..ast import (
    FieldKind,
    Expr, ExprIdent, ExprIsType,
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
from .types import (
    TyInt, TyNamedInstance, TyTypeValOf, TyUnresolved, InferredType, inferred_type_from_ann,
)
from .errors import (ErrAssignToImmutable, ErrMissingParamTypeAnn, ErrMissingReturnTypeAnn,
                     ErrIsNotOnNonUnion, ErrFieldDefaultNotAllowed)
from .scope import _FnSig
from .type_utils import _type_from_guard_name

if TYPE_CHECKING:
    from .types import TyUnion


class _TypeCheckerStmts:
    """Mixin providing statement checking for TypeChecker."""

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
                    guard_opt: Optional[tuple[str, str, bool]] = None
                    guard_span = None
                    if isinstance(cond, ExprIsType) and isinstance(cond.expr, ExprIdent):
                        guard_opt = (cond.expr.name, cond.type_name, cond.negated)
                        guard_span = cond.span

                    narrowed: Optional[tuple[str, "InferredType", bool]] = None
                    error_info = None

                    if guard_opt is not None:
                        var_name, type_name, negated = guard_opt
                        guard_ty = _type_from_guard_name(type_name)
                        info = self._lookup(var_name)
                        var_ty: "InferredType" = info.ty if info else TyUnresolved()
                        is_mut = info.mutable if info else False

                        if negated:
                            from .types import TyUnion
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
                                error_info = (var_name, var_ty, guard_span)
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
                    if kind in (FieldKind.MUT, FieldKind.LET):
                        kind_str = "mut" if kind == FieldKind.MUT else "let"
                        self._report(ErrFieldDefaultNotAllowed(field_name=name, kind=kind_str))
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
                from .types import TyNamespace
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
                            storage_name = (f"__cast__[{s.template_params[0].name}]"
                                            if s.name == "__cast__" and s.template_params
                                            else s.name)
                            cls_methods.setdefault(storage_name, []).append(sig)
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
