# git SHA: c4e3615a36e8f7183b626dacab73bfc500682e96
"""Tree-walk interpreter for Havakyrie."""
from __future__ import annotations
import copy
import threading
from typing import Optional, TYPE_CHECKING

from ..ast import (
    # Expressions
    ExprInt, ExprFloat, ExprStr, ExprBool, ExprNone, ExprIdent,
    ExprList, ExprAttr, ExprTraitAccess, ExprBinOp, ExprUnaryOp,
    ExprCall, ExprTemplateInstantiate, ExprSubscript, ExprSlice,
    ExprDict, ExprTuple, ExprSet, ExprBlock, ExprIfExpr,
    ExprForExpr, ExprWhileExpr, ExprMatchExpr, ExprIsType, ExprCast,
    # Statements
    StmtExpr, StmtLet, StmtConst, StmtMut, StmtStatic,
    StmtAssign, StmtAttrAssign, StmtAttrCompoundAssign, StmtCompoundAssign,
    StmtIf, StmtMatch, StmtWhile, StmtFor, StmtBlock,
    StmtReturn, StmtBreak, StmtContinue, StmtPass,
    StmtBlockReturn, StmtLoopYield, StmtYield, StmtFreeze,
    StmtFnDef, StmtGenDef, StmtClassDef, StmtTraitDef, StmtField,
    StmtNewTypeDef, StmtEnumDef, StmtTry, StmtRaise,
    StmtImport, StmtFromImport, StmtLetTuple, StmtAsyncAssign,
    # Helpers
    BinOp, UnaryOp, Accessibility, FieldKind,
    Param, TemplateParam, CallArg, CallArgPositional, CallArgKeyword,
    MatchArm, MatchPatternCase, MatchPatternIsType, ExceptHandler,
    TupleTargetLet, TupleTargetMut, TupleTargetBare, TupleTargetWildcard,
)
from .value import (
    Value, MISSING,
    TlList, TlDict, TlTuple, TlSet,
    TlFunction, TlOverloadedFn, TlGeneratorFn, TlGenerator,
    TlTemplateFn, TlTemplateGenFn, TlTemplateClass,
    TlClass, TlInstance, TlType, TlTrait,
    TlNamespace, TlSlice, TlFileObject,
    CapturedImm, CapturedMut,
    type_name, display, is_truthy, _values_equal, deep_clone, _repr_val,
)
from .env import Environment
from .exceptions import (
    ReturnSignal, BreakSignal, ContinueSignal,
    BlockReturnSignal, LoopYieldSignal, YieldSignal,
    StopIterationSignal, RaiseSignal, InterpreterError,
)
from .builtins import (
    make_builtins, get_attr_builtin, subscript_get, subscript_set,
    apply_slice, iterate, gen_next, _NativeCallable, _make_native,
    _make_error_instance, _raise_builtin,
)


# ---------------------------------------------------------------------------
# Thread-locals for control-flow expression state
# ---------------------------------------------------------------------------

_tl = threading.local()


def _get_block_yields() -> Optional[list]:
    return getattr(_tl, "block_yields", None)


def _set_block_yields(val: Optional[list]) -> None:
    _tl.block_yields = val


def _get_loop_depth() -> int:
    return getattr(_tl, "loop_depth", 0)


def _inc_loop_depth() -> None:
    _tl.loop_depth = _get_loop_depth() + 1


def _dec_loop_depth() -> None:
    _tl.loop_depth = max(0, _get_loop_depth() - 1)


def _get_gen_yields() -> Optional[list]:
    return getattr(_tl, "gen_yields", None)


def _set_gen_yields(val: Optional[list]) -> None:
    _tl.gen_yields = val


# ---------------------------------------------------------------------------
# Static-cell registry for `static mut` variables
# ---------------------------------------------------------------------------

_STATIC_CELLS: dict[str, list] = {}  # span_key → [Value]


# ---------------------------------------------------------------------------
# Interpreter
# ---------------------------------------------------------------------------

class Interpreter:
    def __init__(self) -> None:
        self._env = Environment()
        # known_classes is shared and updated as classes are defined
        self._known_classes: dict[str, Value] = {}
        self._known_traits: dict[str, TlTrait] = {}
        # current_class: name of class being executed for access-control checks
        self._current_class: Optional[str] = None
        self._current_method: Optional[str] = None
        # Install built-ins into global scope
        for name, val in make_builtins(self._known_classes).items():
            self._env.declare(name, val, mutable=False)
        # Install built-in enums
        self._install_builtin_enums()

    # ------------------------------------------------------------------
    # Built-in enum installation
    # ------------------------------------------------------------------

    def _install_builtin_enums(self) -> None:
        def _make_enum_ns(name: str, variants: list[tuple[str, int]]) -> TlNamespace:
            cls_name = f"enum_item_{name}"
            enum_cls = TlClass(
                name=cls_name, bases=[], methods={}, gen_methods={},
                field_defaults=[("value", None, False), ("name", None, False)],
                class_vars={}, field_mutability={"value": False, "name": False},
                field_access={}, method_access={},
                static_method_names=set(), class_method_names=set(),
                static_vars={}, new_type_base=None,
            )
            self._known_classes[cls_name] = enum_cls
            members: dict = {cls_name: enum_cls}
            for vname, vval in variants:
                inst = TlInstance(cls=enum_cls, fields={
                    "value": [vval, False],
                    "name":  [vname, False],
                }, immutable=True)
                members[vname] = inst
            return TlNamespace(name=name, members=members)

        file_open_mode = _make_enum_ns("FileOpenMode", [
            ("read", 2), ("write", 0), ("rewrite", 1), ("make_and_write", 3),
            ("read_write", 4),
        ])
        start_point = _make_enum_ns("StartPoint", [
            ("top", 0), ("current", 1), ("end", 2),
        ])
        byte_mode = _make_enum_ns("ByteRecognizingMode", [
            ("text", 1), ("byte", 0),
        ])
        encoding = _make_enum_ns("Encoding", [
            ("UTF_8", 1), ("UTF_16", 2), ("ASCII", 0),
        ])

        for ns_name, ns_val in [
            ("FileOpenMode", file_open_mode),
            ("StartPoint", start_point),
            ("ByteRecognizingMode", byte_mode),
            ("Encoding", encoding),
        ]:
            self._env.declare(ns_name, ns_val, mutable=False)

    # ------------------------------------------------------------------
    # Public entry points
    # ------------------------------------------------------------------

    def exec_stmts(self, stmts: list) -> None:
        for stmt in stmts:
            self.exec(stmt)

    # ------------------------------------------------------------------
    # Statement execution
    # ------------------------------------------------------------------

    def exec(self, stmt) -> None:  # noqa: C901 (complex but matches interpreter)
        match stmt:
            case StmtExpr(expr=expr):
                self.eval(expr)

            case StmtLet(name=name, expr=expr):
                val = self.eval(expr)
                self._env.declare(name, val, mutable=False)

            case StmtConst(name=name, expr=expr):
                val = self.eval(expr)
                self._env.declare(name, val, mutable=False)

            case StmtMut(name=name, expr=expr):
                val = self.eval(expr)
                self._env.declare(name, val, mutable=True)

            case StmtStatic(name=name, expr=expr, span=span):
                key = f"{span.file}:{span.line}:{span.col}"
                if key not in _STATIC_CELLS:
                    val = self.eval(expr)
                    _STATIC_CELLS[key] = [val]
                self._env.declare_cell(name, _STATIC_CELLS[key], mutable=True)

            case StmtAssign(name=name, value=value):
                val = self.eval(value)
                self._env.assign(name, val)

            case StmtCompoundAssign(name=name, op=op, value=value):
                left = self._env.get(name)
                right = self.eval(value)
                result = self._apply_binop(op, left, right)
                self._env.assign(name, result)

            case StmtAttrAssign(target=target, value=value):
                val = self.eval(value)
                self._exec_attr_assign(target, val)

            case StmtAttrCompoundAssign(target=target, op=op, value=value):
                current = self.eval(target)
                right = self.eval(value)
                result = self._apply_binop(op, current, right)
                self._exec_attr_assign(target, result)

            case StmtLetTuple(targets=targets, value=value):
                val = self.eval(value)
                self._unpack_tuple(targets, val)

            case StmtIf(branches=branches, else_body=else_body):
                for cond_expr, body in branches:
                    if is_truthy(self.eval(cond_expr)):
                        self._env.push_scope()
                        try:
                            self.exec_stmts(body)
                        finally:
                            self._env.pop_scope()
                        return
                if else_body is not None:
                    self._env.push_scope()
                    try:
                        self.exec_stmts(else_body)
                    finally:
                        self._env.pop_scope()

            case StmtWhile(cond=cond, body=body):
                _inc_loop_depth()
                try:
                    while is_truthy(self.eval(cond)):
                        self._env.push_scope()
                        try:
                            self.exec_stmts(body)
                        except ContinueSignal:
                            pass
                        except BreakSignal:
                            break
                        finally:
                            self._env.pop_scope()
                finally:
                    _dec_loop_depth()

            case StmtFor(targets=targets, iter=iter_expr, body=body):
                items = self._iterate_val(self.eval(iter_expr))
                _inc_loop_depth()
                try:
                    for item in items:
                        self._env.push_scope()
                        try:
                            self._bind_for_targets(targets, item)
                            self.exec_stmts(body)
                        except ContinueSignal:
                            pass
                        except BreakSignal:
                            break
                        finally:
                            self._env.pop_scope()
                finally:
                    _dec_loop_depth()

            case StmtMatch(subject=subject, arms=arms):
                val = self.eval(subject)
                self._exec_match(val, arms)

            case StmtBlock(stmts=stmts):
                self._env.push_scope()
                try:
                    self.exec_stmts(stmts)
                except BlockReturnSignal:
                    pass  # block_return exits the block; discard the value
                finally:
                    self._env.pop_scope()

            case StmtReturn(expr=expr):
                val = self.eval(expr) if expr is not None else None
                raise ReturnSignal(val)

            case StmtBreak():
                if _get_loop_depth() == 0:
                    raise InterpreterError("break outside loop")
                raise BreakSignal()

            case StmtContinue():
                raise ContinueSignal()

            case StmtPass():
                pass

            case StmtBlockReturn(expr=expr):
                val = self.eval(expr)
                raise BlockReturnSignal(val)

            case StmtLoopYield(expr=expr):
                val = self.eval(expr)
                yields = _get_block_yields()
                if yields is None:
                    raise InterpreterError("loop_yield used outside for/while expression")
                yields.append(val)

            case StmtYield(expr=expr):
                val = self.eval(expr)
                gen_yields = _get_gen_yields()
                if gen_yields is not None:
                    gen_yields.append(val)
                else:
                    raise YieldSignal(val)

            case StmtFreeze(name=name):
                self._env.freeze(name)

            case StmtFnDef(name=name, template_params=tparams, params=params,
                           return_type=return_type, body=body,
                           is_abstract=is_abstract, is_static=is_static,
                           is_class_method=is_class_method,
                           decorators=decorators, access=access):
                if tparams:
                    fn_val: Value = TlTemplateFn(name=name, template_params=tparams,
                                                  params=params, body=body)
                elif is_abstract:
                    fn_val = TlFunction(name=name, params=params, body=[],
                                        is_static=is_static, is_class_method=is_class_method)
                else:
                    captured = self._env.capture_all()
                    fn_val = TlFunction(name=name, params=params, body=body,
                                        captured_env=captured,
                                        is_static=is_static, is_class_method=is_class_method)
                # Apply decorators (outermost first = bottom-up)
                for dec_expr in reversed(decorators):
                    dec = self.eval(dec_expr)
                    fn_val = self._call(dec, [fn_val], {})
                self._env.declare(name, fn_val, mutable=False)

            case StmtGenDef(name=name, template_params=tparams, params=params,
                            yield_type=yield_type, body=body, access=access):
                if tparams:
                    gen_val: Value = TlTemplateGenFn(name=name, template_params=tparams,
                                                      params=params, body=body)
                else:
                    captured = self._env.capture_all()
                    gen_val = TlGeneratorFn(name=name, params=params, body=body,
                                            captured_env=captured)
                self._env.declare(name, gen_val, mutable=False)

            case StmtClassDef(name=name, template_params=tparams, bases=bases,
                               decorators=decorators, body=body):
                if tparams:
                    cls_val: Value = TlTemplateClass(name=name, template_params=tparams,
                                                      bases=bases, body=body)
                else:
                    cls_val = self._build_class(name, bases, body)
                for dec_expr in reversed(decorators):
                    dec = self.eval(dec_expr)
                    cls_val = self._call(dec, [cls_val], {})
                self._env.declare(name, cls_val, mutable=False)
                if isinstance(cls_val, TlClass):
                    self._known_classes[name] = cls_val

            case StmtTraitDef(name=name, template_params=tparams, body=body):
                trait = TlTrait(name=name)
                self._env.declare(name, trait, mutable=False)
                self._known_traits[name] = trait

            case StmtNewTypeDef(name=name, original=original):
                cls = self._build_new_type(name, original)
                self._env.declare(name, cls, mutable=False)
                self._known_classes[name] = cls

            case StmtEnumDef(name=name, variants=variants):
                ns = self._build_enum(name, variants)
                self._env.declare(name, ns, mutable=False)
                # Also expose the enum_item class for `is` checks
                enum_cls_name = f"enum_item_{name}"
                if enum_cls_name in self._known_classes:
                    self._env.declare(enum_cls_name, self._known_classes[enum_cls_name], mutable=False)

            case StmtField():
                pass  # handled by _build_class

            case StmtTry(body=try_body, handlers=handlers, finally_body=finally_body):
                self._exec_try(try_body, handlers, finally_body)

            case StmtRaise(exc=exc):
                if exc is None:
                    raise RaiseSignal(None, "re-raise")
                val = self.eval(exc)
                msg = display(val)
                raise RaiseSignal(val, msg)

            case StmtImport(lang=lang, module=module, alias=alias, body=body):
                self._exec_import(lang, module, alias, body)

            case StmtFromImport(lang=lang, module=module, names=names, body=body):
                self._exec_from_import(lang, module, names, body)

            case StmtAsyncAssign(target=target, return_type=return_type, stmts=stmts):
                self._exec_async_assign(target, stmts)

            case _:
                raise InterpreterError(f"Unknown statement type: {type(stmt).__name__}")

    # ------------------------------------------------------------------
    # Expression evaluation
    # ------------------------------------------------------------------

    def eval(self, expr) -> Value:  # noqa: C901
        match expr:
            case ExprInt(value=v): return v
            case ExprFloat(value=v): return v
            case ExprStr(value=v): return v
            case ExprBool(value=v): return v
            case ExprNone(): return None

            case ExprIdent(name=name):
                return self._env.get(name)

            case ExprList(elements=elements):
                return TlList(items=[self.eval(e) for e in elements])

            case ExprTuple(elements=elements):
                return TlTuple(values=[self.eval(e) for e in elements])

            case ExprSet(elements=elements):
                s = TlSet(items=[])
                for e in elements:
                    s.add(self.eval(e))
                return s

            case ExprDict(pairs=pairs):
                d = TlDict()
                for k_expr, v_expr in pairs:
                    d.set(self.eval(k_expr), self.eval(v_expr))
                return d

            case ExprSlice(begin=begin, end=end, step=step):
                b = self.eval(begin) if begin is not None else None
                e = self.eval(end) if end is not None else None
                s = self.eval(step) if step is not None else None
                return TlSlice(begin=b, end=e, step=s)

            case ExprAttr(object=obj_expr, attr=attr):
                return self._eval_attr(self.eval(obj_expr), attr)

            case ExprTraitAccess(object=obj_expr, trait_name=trait_name, attr=attr):
                obj = self.eval(obj_expr)
                return self._eval_trait_attr(obj, trait_name, attr)

            case ExprBinOp(op=op, left=left, right=right):
                return self._eval_binop(op, left, right)

            case ExprUnaryOp(op=op, operand=operand):
                val = self.eval(operand)
                return self._apply_unary(op, val)

            case ExprCall(func=func_expr, args=args):
                return self._eval_call(func_expr, args)

            case ExprTemplateInstantiate(base=base, type_args=type_args):
                base_val = self.eval(base)
                return self._instantiate_template(base_val, type_args)

            case ExprSubscript(object=obj_expr, index=index_expr):
                obj = self.eval(obj_expr)
                idx = self.eval(index_expr)
                return self._subscript_get(obj, idx)

            case ExprBlock(stmts=stmts, return_type=_):
                return self._eval_block_expr(stmts)

            case ExprIfExpr(branches=branches, else_body=else_body, return_type=_):
                return self._eval_if_expr(branches, else_body)

            case ExprForExpr(target=target, iter=iter_expr, body=body, return_type=_):
                return self._eval_for_expr(target, iter_expr, body)

            case ExprWhileExpr(cond=cond, body=body, return_type=_):
                return self._eval_while_expr(cond, body)

            case ExprMatchExpr(subject=subject, arms=arms, return_type=_):
                return self._eval_match_expr(subject, arms)

            case ExprIsType(expr=inner, negated=negated, type_name=tname):
                val = self.eval(inner)
                result = self._is_type(val, tname)
                return result if not negated else not result

            case ExprCast(object=obj_expr, type_name=tname):
                return self._eval_cast(obj_expr, tname)

            case _:
                raise InterpreterError(f"Unknown expression type: {type(expr).__name__}")

    # ------------------------------------------------------------------
    # Binary / unary operators
    # ------------------------------------------------------------------

    def _eval_binop(self, op: BinOp, left_expr, right_expr) -> Value:
        # Short-circuit logical ops
        if op == BinOp.AND:
            lv = self.eval(left_expr)
            return lv if not is_truthy(lv) else self.eval(right_expr)
        if op == BinOp.OR:
            lv = self.eval(left_expr)
            return lv if is_truthy(lv) else self.eval(right_expr)
        left = self.eval(left_expr)
        right = self.eval(right_expr)
        return self._apply_binop(op, left, right)

    def _apply_binop(self, op: BinOp, left: Value, right: Value) -> Value:  # noqa: C901
        match op:
            case BinOp.ADD:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left + right
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    return float(left) + float(right)
                if isinstance(left, str) and isinstance(right, str):
                    return left + right
                if isinstance(left, TlList) and isinstance(right, TlList):
                    return TlList(items=left.items + right.items)
                raise RuntimeError(f"TypeError: unsupported operand types for +: '{type_name(left)}' and '{type_name(right)}'")
            case BinOp.SUB:
                if isinstance(left, TlSet) and isinstance(right, TlSet):
                    return TlSet(items=[x for x in left.items if not right.contains(x)])
                return self._numeric_op(left, right, "-")
            case BinOp.MUL:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left * right
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    return float(left) * float(right)
                if isinstance(left, str) and isinstance(right, int): return left * right
                if isinstance(left, int) and isinstance(right, str): return right * left
                raise RuntimeError(f"TypeError: unsupported operand types for *: '{type_name(left)}' and '{type_name(right)}'")
            case BinOp.DIV:
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    if right == 0: raise RuntimeError("ZeroDivisionError: division by zero")
                    return float(left) / float(right)
                raise RuntimeError(f"TypeError: unsupported operand types for /: '{type_name(left)}' and '{type_name(right)}'")
            case BinOp.FLOOR_DIV:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    if right == 0: raise RuntimeError("ZeroDivisionError: division by zero")
                    return left // right
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    if right == 0: raise RuntimeError("ZeroDivisionError: division by zero")
                    return float(left // right)
                raise RuntimeError(f"TypeError: unsupported operand types for //")
            case BinOp.MOD:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    if right == 0: raise RuntimeError("ZeroDivisionError: modulo by zero")
                    return left % right
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    if right == 0: raise RuntimeError("ZeroDivisionError: modulo by zero")
                    return float(left) % float(right)
                raise RuntimeError(f"TypeError: unsupported operand types for %")
            case BinOp.POW:
                if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
                    result = float(left) ** float(right)
                    if isinstance(left, int) and isinstance(right, int) and right >= 0:
                        return left ** right
                    return result
                raise RuntimeError(f"TypeError: unsupported operand types for **")
            case BinOp.EQ:
                return self._values_eq(left, right)
            case BinOp.NOT_EQ:
                return not self._values_eq(left, right)
            case BinOp.LT:
                return self._cmp(left, right, "<")
            case BinOp.GT:
                return self._cmp(left, right, ">")
            case BinOp.LT_EQ:
                return self._cmp(left, right, "<=")
            case BinOp.GT_EQ:
                return self._cmp(left, right, ">=")
            case BinOp.BIT_AND:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left & right
                if isinstance(left, TlSet) and isinstance(right, TlSet):
                    return TlSet(items=[x for x in left.items if right.contains(x)])
                raise RuntimeError(f"TypeError: unsupported operand types for &")
            case BinOp.BIT_OR:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left | right
                if isinstance(left, TlSet) and isinstance(right, TlSet):
                    result_set = TlSet(items=list(left.items))
                    for x in right.items: result_set.add(x)
                    return result_set
                raise RuntimeError(f"TypeError: unsupported operand types for |")
            case BinOp.BIT_XOR:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left ^ right
                if isinstance(left, TlSet) and isinstance(right, TlSet):
                    a = [x for x in left.items if not right.contains(x)]
                    b = [y for y in right.items if not left.contains(y)]
                    return TlSet(items=a + b)
                raise RuntimeError(f"TypeError: unsupported operand types for ^")
            case BinOp.L_SHIFT:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left << right
                raise RuntimeError(f"TypeError: unsupported operand types for <<")
            case BinOp.R_SHIFT:
                if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
                    return left >> right
                raise RuntimeError(f"TypeError: unsupported operand types for >>")
            case BinOp.IN:
                return self._contains(right, left)
            case BinOp.NOT_IN:
                return not self._contains(right, left)
            case _:
                raise InterpreterError(f"Unknown binary op: {op}")

    def _apply_unary(self, op: UnaryOp, val: Value) -> Value:
        match op:
            case UnaryOp.NEG:
                if isinstance(val, int) and not isinstance(val, bool): return -val
                if isinstance(val, float): return -val
                raise RuntimeError(f"TypeError: bad operand type for unary -: '{type_name(val)}'")
            case UnaryOp.NOT:
                return not is_truthy(val)
            case UnaryOp.BIT_NOT:
                if isinstance(val, int) and not isinstance(val, bool): return ~val
                raise RuntimeError(f"TypeError: bad operand type for unary ~: '{type_name(val)}'")

    def _numeric_op(self, left: Value, right: Value, op: str) -> Value:
        if isinstance(left, int) and not isinstance(left, bool) and isinstance(right, int) and not isinstance(right, bool):
            if op == "-": return left - right
        if isinstance(left, (int, float)) and isinstance(right, (int, float)) and not isinstance(left, bool) and not isinstance(right, bool):
            if op == "-": return float(left) - float(right)
        raise RuntimeError(f"TypeError: unsupported operand types for {op}: '{type_name(left)}' and '{type_name(right)}'")

    def _values_eq(self, a: Value, b: Value) -> bool:
        if a is None and b is None: return True
        if a is None or b is None: return False
        if type(a) is bool and type(b) is bool: return a == b
        if type(a) is bool or type(b) is bool: return False
        if isinstance(a, (int, float)) and isinstance(b, (int, float)): return float(a) == float(b)
        if isinstance(a, str) and isinstance(b, str): return a == b
        if isinstance(a, TlList) and isinstance(b, TlList):
            return len(a.items) == len(b.items) and all(self._values_eq(x, y) for x, y in zip(a.items, b.items))
        if isinstance(a, TlTuple) and isinstance(b, TlTuple):
            return len(a.values) == len(b.values) and all(self._values_eq(x, y) for x, y in zip(a.values, b.values))
        if isinstance(a, TlSet) and isinstance(b, TlSet):
            if len(a.items) != len(b.items): return False
            return all(b.contains(x) for x in a.items)
        if isinstance(a, TlDict) and isinstance(b, TlDict):
            if len(a.keys) != len(b.keys): return False
            for k, v in zip(a.keys, a.values):
                bv = b.get(k)
                if bv is MISSING: return False
                if not self._values_eq(v, bv): return False  # type: ignore[arg-type]
            return True
        if isinstance(a, TlInstance) and isinstance(b, TlInstance):
            # Check for __eq__ method
            if "__eq__" in a.cls.methods:
                result = self._call_method(a, "__eq__", [b])
                return is_truthy(result)
            return a is b
        return a is b

    def _cmp(self, left: Value, right: Value, op: str) -> bool:
        if isinstance(left, bool) or isinstance(right, bool):
            raise RuntimeError(f"TypeError: '<' not supported between bool types")
        if isinstance(left, (int, float)) and isinstance(right, (int, float)):
            lf, rf = float(left), float(right)
            if op == "<": return lf < rf
            if op == ">": return lf > rf
            if op == "<=": return lf <= rf
            if op == ">=": return lf >= rf
        if isinstance(left, str) and isinstance(right, str):
            if op == "<": return left < right
            if op == ">": return left > right
            if op == "<=": return left <= right
            if op == ">=": return left >= right
        raise RuntimeError(f"TypeError: '{op}' not supported between '{type_name(left)}' and '{type_name(right)}'")

    def _contains(self, container: Value, item: Value) -> bool:
        if isinstance(container, TlList):
            return any(self._values_eq(x, item) for x in container.items)
        if isinstance(container, TlSet):
            return any(self._values_eq(x, item) for x in container.items)
        if isinstance(container, TlDict):
            return any(self._values_eq(k, item) for k in container.keys)
        if isinstance(container, str):
            if isinstance(item, str): return item in container
        if isinstance(container, TlTuple):
            return any(self._values_eq(x, item) for x in container.values)
        raise RuntimeError(f"TypeError: 'in' not supported for '{type_name(container)}'")

    # ------------------------------------------------------------------
    # Attribute access
    # ------------------------------------------------------------------

    def _eval_attr(self, obj: Value, attr: str) -> Value:
        if isinstance(obj, TlInstance):
            return self._get_instance_attr(obj, attr)
        if isinstance(obj, TlClass):
            return self._get_class_attr(obj, attr)
        if isinstance(obj, TlNamespace):
            if attr in obj.members:
                return obj.members[attr]
            raise RuntimeError(f"AttributeError: module '{obj.name}' has no attribute '{attr}'")
        if isinstance(obj, TlType):
            return obj.name
        # Built-in type attribute dispatch
        return get_attr_builtin(obj, attr, self._known_classes)

    def _eval_trait_attr(self, obj: Value, trait_name: str, attr: str) -> Value:
        if isinstance(obj, TlInstance):
            key = f"{trait_name}::{attr}"
            if key in obj.fields:
                return obj.fields[key][0]
            # Try plain attr
            if attr in obj.fields:
                return obj.fields[attr][0]
        raise RuntimeError(f"AttributeError: trait '{trait_name}' has no attribute '{attr}' on '{type_name(obj)}'")

    def _get_instance_attr(self, inst: TlInstance, attr: str) -> Value:
        cls = inst.cls
        # new_type `.value` is an alias for `__value__`
        if attr == "value" and cls.new_type_base is not None and "__value__" in inst.fields:
            return inst.fields["__value__"][0]
        # Access control check
        self._check_access(cls.name, attr, cls.method_access.get(attr), cls.field_access.get(attr))

        # Check fields
        if attr in inst.fields:
            return inst.fields[attr][0]

        # Check class vars
        if attr in cls.class_vars:
            return cls.class_vars[attr]

        # Check static vars
        if attr in cls.static_vars:
            return cls.static_vars[attr][0]

        # Check methods
        if attr in cls.methods:
            overloads = cls.methods[attr]
            if len(overloads) == 1:
                fn = overloads[0]
                if isinstance(fn, _NativeCallable):
                    def _bound_native(args, kwargs, _fn=fn, _inst=inst):
                        return _fn.call([_inst] + list(args), kwargs)
                    return _make_native(attr, _bound_native)
                return self._bind_method(inst, fn)
            return self._bind_overloaded(inst, overloads)

        if attr in cls.gen_methods:
            gfn = cls.gen_methods[attr]
            return self._bind_gen_method(inst, gfn)

        raise RuntimeError(f"AttributeError: '{cls.name}' object has no attribute '{attr}'")

    def _get_class_attr(self, cls: TlClass, attr: str) -> Value:
        if attr == "name":
            return cls.name
        if attr in cls.class_vars:
            return cls.class_vars[attr]
        if attr in cls.static_vars:
            return cls.static_vars[attr][0]
        if attr in cls.methods:
            overloads = cls.methods[attr]
            fn = overloads[0] if len(overloads) == 1 else None
            if fn and fn.is_static:
                return fn
            if fn and fn.is_class_method:
                return self._bind_class_method(cls, fn)
            # Unbound method — return as-is for now
            if len(overloads) == 1: return overloads[0]
            return TlOverloadedFn(overloads=overloads)
        raise RuntimeError(f"AttributeError: class '{cls.name}' has no attribute '{attr}'")

    def _check_access(self, cls_name: str, attr: str,
                      method_acc: Optional[Accessibility],
                      field_acc: Optional[Accessibility]) -> None:
        acc = method_acc or field_acc
        if acc is None or acc == Accessibility.PUBLIC:
            return
        if acc == Accessibility.PRIVATE:
            if self._current_class != cls_name:
                raise RaiseSignal(
                    _make_error_instance("AccessError", f"private attribute '{attr}' of '{cls_name}' is not accessible", self._known_classes),
                    f"AccessError: private attribute '{attr}' of '{cls_name}'"
                )
        # PROTECTED: allow same class or same trait implementors (simplified: allow same class)
        if acc == Accessibility.PROTECTED:
            if self._current_class is None:
                raise RaiseSignal(
                    _make_error_instance("AccessError", f"protected attribute '{attr}' of '{cls_name}' is not accessible", self._known_classes),
                    f"AccessError: protected attribute '{attr}' of '{cls_name}'"
                )

    def _bind_method(self, inst: TlInstance, fn: TlFunction) -> _NativeCallable:
        def bound(args, kwargs):
            return self._exec_function(fn, args, kwargs, self_val=inst)
        return _make_native(fn.name, bound)

    def _bind_gen_method(self, inst: TlInstance, gfn: TlGeneratorFn) -> _NativeCallable:
        def bound(args, kwargs):
            return self._exec_generator(gfn, args, kwargs, self_val=inst)
        return _make_native(gfn.name, bound)

    def _bind_overloaded(self, inst: TlInstance, overloads: list) -> _NativeCallable:
        def bound(args, kwargs):
            fn = self._resolve_overload(overloads, args, kwargs)
            return self._exec_function(fn, args, kwargs, self_val=inst)
        return _make_native(overloads[0].name, bound)

    def _bind_class_method(self, cls: TlClass, fn: TlFunction) -> _NativeCallable:
        def bound(args, kwargs):
            return self._exec_function(fn, args, kwargs, self_val=cls)
        return _make_native(fn.name, bound)

    # ------------------------------------------------------------------
    # Subscript
    # ------------------------------------------------------------------

    def _subscript_get(self, obj: Value, idx: Value) -> Value:
        if isinstance(obj, TlInstance):
            cls = obj.cls
            if "__getitem__" in cls.methods:
                return self._call_method(obj, "__getitem__", [idx])
        return subscript_get(obj, idx, self._known_classes)

    def _subscript_set(self, obj: Value, idx: Value, val: Value) -> None:
        if isinstance(obj, TlInstance):
            cls = obj.cls
            if "__setitem__" in cls.methods:
                self._call_method(obj, "__setitem__", [idx, val])
                return
        subscript_set(obj, idx, val, self._known_classes)

    # ------------------------------------------------------------------
    # Attribute assignment
    # ------------------------------------------------------------------

    def _exec_attr_assign(self, target_expr, val: Value) -> None:
        match target_expr:
            case ExprAttr(object=obj_expr, attr=attr):
                obj = self.eval(obj_expr)
                self._set_attr(obj, attr, val)
            case ExprTraitAccess(object=obj_expr, trait_name=_, attr=attr):
                obj = self.eval(obj_expr)
                self._set_attr(obj, attr, val)
            case ExprSubscript(object=obj_expr, index=idx_expr):
                obj = self.eval(obj_expr)
                idx = self.eval(idx_expr)
                self._subscript_set(obj, idx, val)
            case _:
                raise InterpreterError(f"Cannot assign to {type(target_expr).__name__}")

    def _set_attr(self, obj: Value, attr: str, val: Value) -> None:
        if isinstance(obj, TlInstance):
            cls = obj.cls
            if obj.immutable:
                raise RuntimeError(f"TypeError: cannot modify immutable '{cls.name}' instance")
            # Access control
            self._check_access(cls.name, attr, cls.method_access.get(attr), cls.field_access.get(attr))
            if attr in obj.fields:
                entry = obj.fields[attr]
                if not entry[1] and self._current_method != "__init__":
                    raise RuntimeError(f"TypeError: cannot assign to immutable field '{attr}'")
                entry[0] = val
            else:
                obj.fields[attr] = [val, True]
            return
        if isinstance(obj, TlClass):
            if attr in obj.class_vars:
                obj.class_vars[attr] = val
                return
            if attr in obj.static_vars:
                obj.static_vars[attr][0] = val
                return
        raise RuntimeError(f"AttributeError: cannot set '{attr}' on '{type_name(obj)}'")

    # ------------------------------------------------------------------
    # Call dispatch
    # ------------------------------------------------------------------

    def _eval_call(self, func_expr, args_ast: list[CallArg]) -> Value:
        # Method calls: obj.method(args) — pass self
        if isinstance(func_expr, ExprAttr):
            obj = self.eval(func_expr.object)
            attr = func_expr.attr
            args, kwargs = self._eval_args(args_ast)
            return self._call_attr(obj, attr, args, kwargs)

        func = self.eval(func_expr)
        args, kwargs = self._eval_args(args_ast)
        return self._call(func, args, kwargs)

    def _eval_args(self, args_ast: list[CallArg]) -> tuple[list, dict]:
        args: list = []
        kwargs: dict = {}
        for arg in args_ast:
            match arg:
                case CallArgPositional(expr=e):
                    args.append(self.eval(e))
                case CallArgKeyword(name=name, value=e):
                    kwargs[name] = self.eval(e)
        return args, kwargs

    def _call(self, func: Value, args: list, kwargs: dict) -> Value:
        if isinstance(func, TlFunction):
            return self._exec_function(func, args, kwargs)
        if isinstance(func, TlOverloadedFn):
            fn = self._resolve_overload(func.overloads, args, kwargs)
            return self._exec_function(fn, args, kwargs)
        if isinstance(func, TlGeneratorFn):
            return self._exec_generator(func, args, kwargs)
        if isinstance(func, TlTemplateFn):
            raise RuntimeError("TypeError: cannot call template function without type arguments")
        if isinstance(func, TlClass):
            return self._instantiate_class(func, args, kwargs)
        if isinstance(func, _NativeCallable):
            return func.call(args, kwargs)
        if isinstance(func, TlInstance):
            # Try __call__
            if "__call__" in func.cls.methods:
                return self._call_method(func, "__call__", args)
        raise RuntimeError(f"TypeError: '{type_name(func)}' object is not callable")

    def _call_attr(self, obj: Value, attr: str, args: list, kwargs: dict) -> Value:
        if isinstance(obj, TlInstance):
            return self._call_method(obj, attr, args, kwargs)
        if isinstance(obj, TlClass):
            cls_attr = self._get_class_attr(obj, attr)
            return self._call(cls_attr, args, kwargs)
        if isinstance(obj, TlNamespace):
            if attr in obj.members:
                return self._call(obj.members[attr], args, kwargs)
            raise RuntimeError(f"AttributeError: module '{obj.name}' has no attribute '{attr}'")
        # Built-in type methods
        method = get_attr_builtin(obj, attr, self._known_classes)
        return self._call(method, args, kwargs)

    def _call_method(self, inst: TlInstance, name: str, args: list, kwargs: dict = {}) -> Value:
        cls = inst.cls
        if name in cls.methods:
            overloads = cls.methods[name]
            if len(overloads) == 1:
                fn = overloads[0]
                if isinstance(fn, _NativeCallable):
                    return fn.call([inst] + list(args), kwargs)
                return self._exec_function(fn, args, kwargs, self_val=inst)
            fn = self._resolve_overload(overloads, args, kwargs)
            return self._exec_function(fn, args, kwargs, self_val=inst)
        if name in cls.gen_methods:
            return self._exec_generator(cls.gen_methods[name], args, kwargs, self_val=inst)
        raise RuntimeError(f"AttributeError: '{cls.name}' object has no method '{name}'")

    # ------------------------------------------------------------------
    # Function execution
    # ------------------------------------------------------------------

    def _exec_function(self, fn: TlFunction, args: list, kwargs: dict,
                       self_val: Value = None) -> Value:
        saved_class = self._current_class
        saved_method = self._current_method
        if isinstance(self_val, TlInstance):
            self._current_class = self_val.cls.name
        elif isinstance(self_val, TlClass):
            self._current_class = self_val.name
        self._current_method = fn.name

        self._env.push_scope()
        try:
            # Install captured closure env
            if fn.captured_env:
                self._env.apply_captured(fn.captured_env)

            # Bind self
            params_to_bind = fn.params
            if self_val is not None and not fn.is_static:
                self._env.declare("self", self_val, mutable=False)
                if isinstance(self_val, TlInstance):
                    # Self refers to the class for constructor calls
                    self._env.declare("Self", self_val.cls, mutable=False)
                elif isinstance(self_val, TlClass):
                    self._env.declare("cls", self_val, mutable=False)
                    self._env.declare("Self", self_val, mutable=False)
                # Skip the first param if it is named "self" or "cls"
                if params_to_bind and params_to_bind[0].name in ("self", "cls"):
                    params_to_bind = params_to_bind[1:]

            # Bind parameters
            self._bind_params(params_to_bind, args, kwargs)

            # Execute body
            self.exec_stmts(fn.body)
            return None
        except ReturnSignal as sig:
            return sig.value
        finally:
            self._env.pop_scope()
            self._current_class = saved_class
            self._current_method = saved_method

    def _exec_generator(self, gfn: TlGeneratorFn, args: list, kwargs: dict,
                        self_val: Value = None) -> TlGenerator:
        yields: list = []

        saved_class = self._current_class
        if isinstance(self_val, TlInstance):
            self._current_class = self_val.cls.name

        self._env.push_scope()
        try:
            if gfn.captured_env:
                self._env.apply_captured(gfn.captured_env)
            gen_params = gfn.params
            if self_val is not None:
                self._env.declare("self", self_val, mutable=False)
                if gen_params and gen_params[0].name in ("self", "cls"):
                    gen_params = gen_params[1:]
            self._bind_params(gen_params, args, kwargs)
            self._exec_gen_body(gfn.body, yields)
        except ReturnSignal:
            pass
        finally:
            self._env.pop_scope()
            self._current_class = saved_class

        return TlGenerator(values=yields)

    def _exec_gen_body(self, stmts: list, yields: list) -> None:
        saved = _get_gen_yields()
        _set_gen_yields(yields)
        try:
            self.exec_stmts(stmts)
        except ReturnSignal:
            pass
        finally:
            _set_gen_yields(saved)

    def _bind_params(self, params: list[Param], args: list, kwargs: dict) -> None:
        positional = list(args)
        for i, param in enumerate(params):
            if param.name in kwargs:
                val = kwargs[param.name]
            elif i < len(positional):
                val = positional[i]
            elif param.default is not None:
                val = self.eval(param.default)
            else:
                raise RuntimeError(f"TypeError: missing argument '{param.name}'")
            # Auto-cast: let parameters with type annotation — call __cast__[T] if needed
            if not param.mutable and param.type_ann is not None:
                if isinstance(val, TlInstance) and val.cls.name != param.type_ann:
                    cast_key = f"__cast__[{param.type_ann}]"
                    if cast_key in val.cls.methods:
                        val = self._call_method(val, cast_key, [], {})
            self._env.declare(param.name, val, mutable=param.mutable)

    def _resolve_overload(self, overloads: list, args: list, kwargs: dict) -> TlFunction:
        provided = len(args) + len(kwargs)
        for fn in overloads:
            # Exclude self/cls from param count since it's bound separately
            effective_params = [p for p in fn.params if p.name not in ("self", "cls")]
            n_required = sum(1 for p in effective_params if p.default is None and p.name not in kwargs)
            n_total = len(effective_params)
            if n_required <= provided <= n_total:
                return fn
        return overloads[-1]

    # ------------------------------------------------------------------
    # Class building
    # ------------------------------------------------------------------

    def _build_class(self, name: str, bases: list[str], body: list) -> TlClass:
        methods: dict[str, list] = {}
        gen_methods: dict[str, TlGeneratorFn] = {}
        field_defaults: list = []
        class_vars: dict = {}
        field_mutability: dict[str, bool] = {}
        field_access: dict[str, Accessibility] = {}
        method_access: dict[str, Accessibility] = {}
        static_method_names: set = set()
        class_method_names: set = set()
        static_vars: dict[str, list] = {}

        # Inherit from bases
        for base_name in bases:
            if base_name in self._known_classes:
                base = self._known_classes[base_name]
                if isinstance(base, TlClass):
                    self._inherit_class(base, methods, gen_methods, field_defaults,
                                        class_vars, field_mutability, field_access,
                                        method_access, static_method_names, class_method_names,
                                        static_vars)

        current_access = Accessibility.PUBLIC

        for stmt in body:
            match stmt:
                case StmtFnDef(name=fname, template_params=tparams, params=params,
                               return_type=_, body=fbody, is_abstract=is_abstract,
                               is_static=is_static, is_class_method=is_cm, access=access):
                    if access != Accessibility.PUBLIC:
                        current_access = access
                    acc = access if access != Accessibility.PUBLIC else current_access
                    # __cast__[TypeName] methods stored with namespaced key
                    if fname == "__cast__" and tparams:
                        storage_name = f"__cast__[{tparams[0].name}]"
                        captured = self._env.capture_all()
                        fn = TlFunction(name=storage_name, params=params, body=fbody,
                                        captured_env=captured,
                                        is_static=is_static, is_class_method=is_cm)
                    elif tparams:
                        storage_name = fname
                        fn = TlTemplateFn(name=fname, template_params=tparams, params=params, body=fbody)
                    else:
                        storage_name = fname
                        captured = self._env.capture_all()
                        fn = TlFunction(name=fname, params=params, body=fbody,
                                        captured_env=captured,
                                        is_static=is_static, is_class_method=is_cm)
                    if storage_name not in methods:
                        methods[storage_name] = []
                    methods[storage_name].append(fn)
                    method_access[storage_name] = acc
                    if is_static: static_method_names.add(storage_name)
                    if is_cm: class_method_names.add(storage_name)

                case StmtGenDef(name=fname, template_params=tparams, params=params,
                                yield_type=_, body=fbody, access=access):
                    captured = self._env.capture_all()
                    gfn = TlGeneratorFn(name=fname, params=params, body=fbody, captured_env=captured)
                    gen_methods[fname] = gfn
                    method_access[fname] = access

                case StmtField(name=fname, kind=kind, type_ann=_, default=default, access=access):
                    is_mut = kind in (FieldKind.MUT, FieldKind.STATIC_MUT)
                    default_val = self.eval(default) if default is not None else None
                    if kind == FieldKind.STATIC_MUT:
                        static_vars[fname] = [default_val]
                    else:
                        field_defaults.append((fname, default_val, is_mut))
                    field_mutability[fname] = is_mut
                    field_access[fname] = access

                case StmtConst(name=cname, expr=expr):
                    class_vars[cname] = self.eval(expr)

                case StmtLet(name=cname, expr=expr):
                    class_vars[cname] = self.eval(expr)

                case _:
                    pass  # access markers handled by parser into StmtFnDef.access

        return TlClass(
            name=name, bases=bases, methods=methods, gen_methods=gen_methods,
            field_defaults=field_defaults, class_vars=class_vars,
            field_mutability=field_mutability, field_access=field_access,
            method_access=method_access, static_method_names=static_method_names,
            class_method_names=class_method_names, static_vars=static_vars,
        )

    def _inherit_class(self, base: TlClass, methods, gen_methods, field_defaults,
                        class_vars, field_mutability, field_access, method_access,
                        static_method_names, class_method_names, static_vars) -> None:
        for fname, overloads in base.methods.items():
            if fname not in methods:
                methods[fname] = list(overloads)
                method_access[fname] = base.method_access.get(fname, Accessibility.PUBLIC)
        for fname, gfn in base.gen_methods.items():
            if fname not in gen_methods:
                gen_methods[fname] = gfn
        for fd in base.field_defaults:
            field_defaults.append(fd)
        for k, v in base.class_vars.items():
            if k not in class_vars: class_vars[k] = v
        for k, v in base.field_mutability.items():
            if k not in field_mutability: field_mutability[k] = v
        for k, v in base.field_access.items():
            if k not in field_access: field_access[k] = v
        static_method_names.update(base.static_method_names)
        class_method_names.update(base.class_method_names)
        for k, cell in base.static_vars.items():
            if k not in static_vars: static_vars[k] = cell

    def _instantiate_class(self, cls: TlClass, args: list, kwargs: dict) -> TlInstance:
        PRIMITIVES = {"int", "float", "str", "bool", "None"}

        # Primitive new_type: single positional arg sets __value__
        if cls.new_type_base in PRIMITIVES and "__value__" in {f[0] for f in cls.field_defaults}:
            val = args[0] if args else None
            return TlInstance(cls=cls, fields={"__value__": [val, True]}, immutable=False)

        # Initialize fields
        fields: dict[str, list] = {}
        for fname, default_val, is_mut in cls.field_defaults:
            fields[fname] = [deep_clone(default_val) if default_val is not None else None, is_mut]

        inst = TlInstance(cls=cls, fields=fields, immutable=False)

        # Call __init__ if present
        if "__init__" in cls.methods:
            self._call_method(inst, "__init__", args, kwargs)

        return inst

    def _build_new_type(self, name: str, original: str) -> TlClass:
        PRIMITIVES = {"int", "float", "str", "bool", "None"}
        if original in self._known_classes and original not in PRIMITIVES:
            base = self._known_classes[original]
            if isinstance(base, TlClass):
                cls = TlClass(
                    name=name,
                    bases=[original],
                    methods=dict(base.methods),
                    gen_methods=dict(base.gen_methods),
                    field_defaults=list(base.field_defaults),
                    class_vars=dict(base.class_vars),
                    field_mutability=dict(base.field_mutability),
                    field_access=dict(base.field_access),
                    method_access=dict(base.method_access),
                    static_method_names=set(base.static_method_names),
                    class_method_names=set(base.class_method_names),
                    static_vars=dict(base.static_vars),
                    new_type_base=original,
                )
                return cls
        # Primitive new_type: wrap a single __value__
        cls = TlClass(
            name=name, bases=[original] if original else [],
            methods={}, gen_methods={},
            field_defaults=[("__value__", None, True)],
            class_vars={}, field_mutability={"__value__": True},
            field_access={}, method_access={},
            static_method_names=set(), class_method_names=set(),
            static_vars={}, new_type_base=original,
        )
        return cls

    def _build_enum(self, name: str, variants: list) -> TlNamespace:
        # Build an enum class so instances can be type-checked with `is`
        enum_cls = TlClass(
            name=f"enum_item_{name}", bases=[], methods={}, gen_methods={},
            field_defaults=[("value", None, False), ("name", None, False)],
            class_vars={}, field_mutability={"value": False, "name": False},
            field_access={}, method_access={},
            static_method_names=set(), class_method_names=set(),
            static_vars={}, new_type_base=None,
        )
        self._known_classes[f"enum_item_{name}"] = enum_cls

        members: dict = {f"enum_item_{name}": enum_cls}
        auto_val = 0
        for variant_name, value_expr in variants:
            if value_expr is not None:
                val = self.eval(value_expr)
                if isinstance(val, int): auto_val = val
            else:
                val = auto_val
            inst = TlInstance(cls=enum_cls, fields={
                "value": [val, False],
                "name":  [variant_name, False],
            }, immutable=True)
            members[variant_name] = inst
            auto_val += 1

        return TlNamespace(name=name, members=members)

    # ------------------------------------------------------------------
    # Template instantiation
    # ------------------------------------------------------------------

    def _instantiate_template(self, base: Value, type_args: list[str]) -> Value:
        if isinstance(base, TlTemplateFn):
            # Substitute type params in body (simplified: just return a plain function)
            subs = {p.name: t for p, t in zip(base.template_params, type_args)}
            new_body = self._substitute_types(base.body, subs)
            captured = self._env.capture_all()
            return TlFunction(name=base.name, params=base.params, body=new_body,
                              captured_env=captured)
        if isinstance(base, TlTemplateGenFn):
            subs = {p.name: t for p, t in zip(base.template_params, type_args)}
            new_body = self._substitute_types(base.body, subs)
            captured = self._env.capture_all()
            return TlGeneratorFn(name=base.name, params=base.params, body=new_body,
                                  captured_env=captured)
        if isinstance(base, TlTemplateClass):
            subs = {p.name: t for p, t in zip(base.template_params, type_args)}
            new_body = self._substitute_types(base.body, subs)
            cls = self._build_class(base.name, base.bases, new_body)
            self._known_classes[base.name] = cls
            return cls
        return base

    def _substitute_types(self, stmts: list, subs: dict) -> list:
        return stmts  # simplified: no substitution needed at runtime

    # ------------------------------------------------------------------
    # is / is not type check
    # ------------------------------------------------------------------

    def _is_type(self, val: Value, tname: str) -> bool:
        if tname == "None": return val is None
        if tname == "int": return isinstance(val, int) and not isinstance(val, bool)
        if tname == "float": return isinstance(val, float)
        if tname == "str": return isinstance(val, str)
        if tname == "bool": return isinstance(val, bool)
        if tname == "list": return isinstance(val, TlList)
        if tname == "dict": return isinstance(val, TlDict)
        if tname == "tuple": return isinstance(val, TlTuple)
        if tname == "set": return isinstance(val, TlSet)
        if tname == "function":
            return isinstance(val, (TlFunction, TlOverloadedFn, TlGeneratorFn, _NativeCallable))
        if isinstance(val, TlInstance):
            cls = val.cls
            # Check class name
            if cls.name == tname: return True
            # Check bases (traits)
            if tname in cls.bases: return True
            return False
        if isinstance(val, TlClass):
            return val.name == tname
        return False

    # ------------------------------------------------------------------
    # Cast operator (=>)
    # ------------------------------------------------------------------

    def _eval_cast(self, obj_expr, tname: str) -> Value:
        obj = self.eval(obj_expr)

        def _get_new_type_inner(inst: TlInstance):
            """Extract the inner value from a new_type instance (handles both 'value' and '__value__')."""
            raw = inst.fields.get("value") or inst.fields.get("__value__")
            return raw[0] if raw is not None else None

        # If obj is a new_type instance, extract inner value to avoid nesting
        inner_val = None
        if isinstance(obj, TlInstance) and obj.cls.new_type_base is not None:
            inner_val = _get_new_type_inner(obj)

        # --- new_type downcast: TypeName(obj) equivalent ---
        target_cls_val = self._env.get(tname)
        if target_cls_val is not None and isinstance(target_cls_val, TlClass):
            if target_cls_val.new_type_base is not None:
                arg = inner_val if inner_val is not None else obj
                return self._instantiate_class(target_cls_val, [arg], {})

        # --- instance cast ---
        if isinstance(obj, TlInstance):
            cls = obj.cls
            # new_type instance to its base type: return inner value
            if cls.new_type_base is not None and cls.new_type_base == tname:
                val = _get_new_type_inner(obj)
                if val is not None:
                    return val
                raise InterpreterError(f"TypeError: '{cls.name}' has no 'value' field")

            # __cast__[TypeName] method
            method_key = f"__cast__[{tname}]"
            if method_key in cls.methods:
                return self._call_method(obj, method_key, [], {})
            raise InterpreterError(
                f"TypeError: '{cls.name}' is not castable to '{tname}' "
                f"(no __cast__[{tname}] method defined)"
            )

        raise InterpreterError(
            f"TypeError: cast operator '=>' requires an instance or new_type target, "
            f"got '{type_name(obj)}' cast to '{tname}'"
        )

    # ------------------------------------------------------------------
    # for-loop target binding
    # ------------------------------------------------------------------

    def _bind_for_targets(self, targets: list[str], item: Value) -> None:
        if len(targets) == 1:
            self._env.declare(targets[0], item, mutable=True)
        else:
            # Unpack tuple
            if isinstance(item, TlTuple):
                vals = item.values
            elif isinstance(item, TlList):
                vals = item.items
            else:
                vals = [item]
            for i, tgt in enumerate(targets):
                v = vals[i] if i < len(vals) else None
                self._env.declare(tgt, v, mutable=True)

    def _iterate_val(self, val: Value) -> list:
        if isinstance(val, TlInstance):
            cls = val.cls
            # Generator __iter__ method
            if "__iter__" in cls.gen_methods:
                gen = self._exec_generator(cls.gen_methods["__iter__"], [], {}, self_val=val)
                return list(gen.values)
            # Prefer __next__ for objects that are already iterators
            if "__next__" in cls.methods:
                items = []
                while True:
                    try:
                        item = self._call_method(val, "__next__", [])
                        items.append(item)
                    except StopIterationSignal:
                        break
                    except RaiseSignal as sig:
                        exc = sig.exception_value
                        if isinstance(exc, TlInstance) and exc.cls.name == "StopIteration":
                            break
                        raise
                return items
            if "__iter__" in cls.methods:
                it = self._call_method(val, "__iter__", [])
                if it is val:
                    return []
                return self._iterate_val(it)
        return iterate(val)

    # ------------------------------------------------------------------
    # Tuple unpack (let-tuple)
    # ------------------------------------------------------------------

    def _unpack_tuple(self, targets: list, val: Value) -> None:
        if isinstance(val, TlTuple):
            vals = val.values
        elif isinstance(val, TlList):
            vals = val.items
        else:
            vals = [val]

        for i, tgt in enumerate(targets):
            v = vals[i] if i < len(vals) else None
            match tgt:
                case TupleTargetLet(name=n):
                    self._env.declare(n, v, mutable=False)
                case TupleTargetMut(name=n):
                    self._env.declare(n, v, mutable=True)
                case TupleTargetBare(name=n):
                    self._env.declare(n, v, mutable=False)
                case TupleTargetWildcard():
                    pass

    # ------------------------------------------------------------------
    # Match statement
    # ------------------------------------------------------------------

    def _exec_match(self, val: Value, arms: list[MatchArm]) -> None:
        for arm in arms:
            match arm.pattern:
                case MatchPatternCase(expr=pat_expr):
                    # wildcard check first, before evaluating
                    if isinstance(pat_expr, ExprIdent) and pat_expr.name == "_":
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                        finally:
                            self._env.pop_scope()
                        return
                    pat_val = self.eval(pat_expr)
                    if self._values_eq(val, pat_val):
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                        finally:
                            self._env.pop_scope()
                        return
                case MatchPatternIsType(type_name=tname):
                    if tname == "_" or self._is_type(val, tname):
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                        finally:
                            self._env.pop_scope()
                        return

    # ------------------------------------------------------------------
    # Control-flow expressions
    # ------------------------------------------------------------------

    def _eval_block_expr(self, stmts: list) -> Value:
        self._env.push_scope()
        try:
            self.exec_stmts(stmts)
            return None
        except BlockReturnSignal as sig:
            return sig.value
        finally:
            self._env.pop_scope()

    def _eval_if_expr(self, branches: list, else_body: Optional[list]) -> Value:
        for cond_expr, body in branches:
            if is_truthy(self.eval(cond_expr)):
                self._env.push_scope()
                try:
                    self.exec_stmts(body)
                    return None
                except BlockReturnSignal as sig:
                    return sig.value
                finally:
                    self._env.pop_scope()
        if else_body is not None:
            self._env.push_scope()
            try:
                self.exec_stmts(else_body)
                return None
            except BlockReturnSignal as sig:
                return sig.value
            finally:
                self._env.pop_scope()
        return None

    def _eval_for_expr(self, target: str, iter_expr, body: list) -> Value:
        items = self._iterate_val(self.eval(iter_expr))
        saved_yields = _get_block_yields()
        _set_block_yields([])
        _inc_loop_depth()
        try:
            for item in items:
                self._env.push_scope()
                try:
                    self._env.declare(target, item, mutable=True)
                    self.exec_stmts(body)
                except ContinueSignal:
                    pass
                except BreakSignal:
                    _dec_loop_depth()
                    result = _get_block_yields()
                    _set_block_yields(saved_yields)
                    return TlList(items=result) if result else None
                except BlockReturnSignal as sig:
                    _dec_loop_depth()
                    _set_block_yields(saved_yields)
                    return sig.value
                finally:
                    self._env.pop_scope()
        finally:
            _dec_loop_depth()
        result = _get_block_yields()
        _set_block_yields(saved_yields)
        if result:
            return TlList(items=result)
        return None

    def _eval_while_expr(self, cond_expr, body: list) -> Value:
        saved_yields = _get_block_yields()
        _set_block_yields([])
        _inc_loop_depth()
        try:
            while is_truthy(self.eval(cond_expr)):
                self._env.push_scope()
                try:
                    self.exec_stmts(body)
                except ContinueSignal:
                    pass
                except BreakSignal:
                    break
                except BlockReturnSignal as sig:
                    _dec_loop_depth()
                    _set_block_yields(saved_yields)
                    return sig.value
                finally:
                    self._env.pop_scope()
        finally:
            _dec_loop_depth()
        result = _get_block_yields()
        _set_block_yields(saved_yields)
        if result:
            return TlList(items=result)
        return None

    def _eval_match_expr(self, subject_expr, arms: list) -> Value:
        val = self.eval(subject_expr)
        for arm in arms:
            match arm.pattern:
                case MatchPatternCase(expr=pat_expr):
                    if isinstance(pat_expr, ExprIdent) and pat_expr.name == "_":
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                            return None
                        except BlockReturnSignal as sig:
                            return sig.value
                        finally:
                            self._env.pop_scope()
                    pat_val = self.eval(pat_expr)
                    if self._values_eq(val, pat_val):  # type: ignore[arg-type]
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                            return None
                        except BlockReturnSignal as sig:
                            return sig.value
                        finally:
                            self._env.pop_scope()
                case MatchPatternIsType(type_name=tname):
                    if tname == "_" or self._is_type(val, tname):
                        self._env.push_scope()
                        try:
                            self.exec_stmts(arm.body)
                            return None
                        except BlockReturnSignal as sig:
                            return sig.value
                        finally:
                            self._env.pop_scope()
        return None

    # ------------------------------------------------------------------
    # try / except / finally
    # ------------------------------------------------------------------

    def _exec_try(self, try_body: list, handlers: list[ExceptHandler],
                  finally_body: Optional[list]) -> None:
        try:
            self._env.push_scope()
            try:
                self.exec_stmts(try_body)
            finally:
                self._env.pop_scope()
        except RaiseSignal as exc:
            handled = False
            for handler in handlers:
                if self._handler_matches(handler, exc.exception_value):
                    self._env.push_scope()
                    try:
                        if handler.name:
                            self._env.declare(handler.name, exc.exception_value, mutable=False)
                        self.exec_stmts(handler.body)
                        handled = True
                    finally:
                        self._env.pop_scope()
                    break
            if not handled:
                raise
        finally:
            if finally_body:
                self._env.push_scope()
                try:
                    self.exec_stmts(finally_body)
                finally:
                    self._env.pop_scope()

    def _handler_matches(self, handler: ExceptHandler, exc_val: Value) -> bool:
        if handler.exc_type is None:
            return True  # bare except
        return self._is_type(exc_val, handler.exc_type)

    # ------------------------------------------------------------------
    # Import
    # ------------------------------------------------------------------

    def _exec_cpp_import(self, lang: str, header_path_str: str) -> "TlNamespace":
        """Load a C DLL (cpp-dll) or compile+load a static lib (cpp-lib).

        Mirrors load_cpp_module from exec.rs but uses ctypes directly instead
        of a Rust wrapper DLL.  Results are cached by header path.
        """
        from pathlib import Path
        from .cpp_bridge import (
            parse_header_full, collect_included_headers, load_cpp_config,
            compile_tl_dll, load_cpp_dll, native_lib_ext,
        )

        header_path = Path(header_path_str)
        if not header_path.exists():
            raise RuntimeError(
                f"CppImport: header file not found: '{header_path_str}'"
            )

        header_dir = header_path.parent
        stem = header_path.stem
        mod_name = stem

        config = load_cpp_config(header_dir)
        content = header_path.read_bytes().decode("utf-8", errors="replace")
        sigs, struct_defs = parse_header_full(content, config.custom_type_map, {})

        if lang == "cpp-lib":
            # Also parse headers included via precompile_macros
            if config.precompile_macros:
                included = collect_included_headers(content, header_dir)
                known_names: set[str] = {s.name for s in sigs}
                for inc_path in included:
                    try:
                        inc_text = inc_path.read_bytes().decode("utf-8", errors="replace")
                        inc_sigs, inc_structs = parse_header_full(
                            inc_text, config.custom_type_map, {}
                        )
                        for s in inc_sigs:
                            if s.name not in known_names:
                                known_names.add(s.name)
                                sigs.append(s)
                        for d in inc_structs:
                            if d.name not in {sd.name for sd in struct_defs}:
                                struct_defs.append(d)
                    except Exception:
                        pass

            import sys
            print(
                f"CppImport[{lang}]: {len(sigs)} function(s) total",
                file=sys.stderr,
            )
            dll_path, effective_sigs = compile_tl_dll(
                header_path, sigs, struct_defs, config
            )
            return load_cpp_dll(dll_path, effective_sigs, struct_defs, mod_name)

        else:  # cpp-dll
            ext = native_lib_ext()
            dll_path = header_dir / f"{stem}.{ext}"
            if not dll_path.exists():
                raise RuntimeError(
                    f"CppImport: DLL not found: '{dll_path}' "
                    f"(expected next to '{header_path_str}')"
                )
            return load_cpp_dll(dll_path, sigs, struct_defs, mod_name)

    # Module-level cache for cpp imports: header_path → TlNamespace
    _cpp_module_cache: dict = {}

    def _exec_import(self, lang: str, module: list[str], alias: Optional[str], body: list) -> None:
        # cpp-dll / cpp-lib: bypass tree-walk; load via ctypes
        if lang in ("cpp-dll", "cpp-lib"):
            header_path_str = module[0] if module else ""
            cache_key = (lang, header_path_str)
            if cache_key not in Interpreter._cpp_module_cache:
                Interpreter._cpp_module_cache[cache_key] = self._exec_cpp_import(
                    lang, header_path_str
                )
            ns = Interpreter._cpp_module_cache[cache_key]
            from pathlib import Path
            bound_name = alias if alias else Path(header_path_str).stem
            self._env.declare(bound_name, ns, mutable=False)
            return

        # import[rs]: load pre-compiled DLL via ctypes, wire through handle arena
        if lang == "rs":
            crate_name = ".".join(module)
            cache_key = ("rs", crate_name)
            if cache_key not in Interpreter._cpp_module_cache:
                Interpreter._cpp_module_cache[cache_key] = self._exec_rs_import(
                    crate_name, body
                )
            ns = Interpreter._cpp_module_cache[cache_key]
            bound_name = alias if alias else module[-1]
            self._env.declare(bound_name, ns, mutable=False)
            return

        mod_name = ".".join(module)
        # Execute pre-parsed body in a sub-interpreter, collect as namespace
        sub = Interpreter()
        sub._known_classes = self._known_classes
        sub._known_traits = self._known_traits
        try:
            sub.exec_stmts(body)
        except ReturnSignal:
            pass

        # Build namespace from sub's global scope
        members: dict = {}
        for scope in sub._env._scopes:
            for name, entry in scope.items():
                if name.startswith("_"): continue
                val = entry[2][0] if entry[2] is not None else entry[0]
                members[name] = val

        ns = TlNamespace(name=mod_name, members=members)
        bound_name = alias if alias else module[-1]
        self._env.declare(bound_name, ns, mutable=False)

        # If using py import
        if lang == "py" or lang == "py-int":
            try:
                import importlib
                py_mod = importlib.import_module(mod_name)
                members2: dict = {}
                for attr in dir(py_mod):
                    if attr.startswith("_"): continue
                    try:
                        val = getattr(py_mod, attr)
                        if callable(val):
                            def make_py_fn(f):
                                def py_fn(args, kwargs):
                                    try:
                                        return f(*[_py_to_tl(a) for a in args],
                                                 **{k: _py_to_tl(v) for k, v in kwargs.items()})
                                    except Exception as e:
                                        raise RuntimeError(str(e))
                                return _make_native(attr, py_fn)
                            members2[attr] = make_py_fn(val)
                        else:
                            members2[attr] = _py_to_tl(val)
                    except Exception:
                        pass
                ns2 = TlNamespace(name=mod_name, members=members2)
                self._env.assign(bound_name, ns2)
            except ImportError:
                pass

    def _exec_from_import(self, lang: str, module: list[str], names: list, body: list) -> None:
        if lang in ("cpp-dll", "cpp-lib"):
            header_path_str = module[0] if module else ""
            cache_key = (lang, header_path_str)
            if cache_key not in Interpreter._cpp_module_cache:
                Interpreter._cpp_module_cache[cache_key] = self._exec_cpp_import(
                    lang, header_path_str
                )
            ns = Interpreter._cpp_module_cache[cache_key]
            for orig_name, alias_name in names:
                val = ns.members.get(orig_name)
                if val is not None:
                    bound = alias_name if alias_name else orig_name
                    self._env.declare(bound, val, mutable=False)
            return

        if lang == "rs":
            crate_name = ".".join(module)
            cache_key = ("rs", crate_name)
            if cache_key not in Interpreter._cpp_module_cache:
                Interpreter._cpp_module_cache[cache_key] = self._exec_rs_import(
                    crate_name, body
                )
            ns = Interpreter._cpp_module_cache[cache_key]
            for orig_name, alias_name in names:
                val = ns.members.get(orig_name)
                if val is not None:
                    bound = alias_name if alias_name else orig_name
                    self._env.declare(bound, val, mutable=False)
            return

        mod_name = ".".join(module)
        sub = Interpreter()
        sub._known_classes = self._known_classes
        sub._known_traits = self._known_traits
        try:
            sub.exec_stmts(body)
        except ReturnSignal:
            pass

        members: dict = {}
        for scope in sub._env._scopes:
            for name, entry in scope.items():
                val = entry[2][0] if entry[2] is not None else entry[0]
                members[name] = val

        for orig_name, alias in names:
            if orig_name in members:
                bound = alias if alias else orig_name
                self._env.declare(bound, members[orig_name], mutable=False)

    def _exec_rs_import(self, crate_name: str, body: list) -> "TlNamespace":
        """Load the compiled Rust crate DLL and return a TlNamespace.

        The DLL was compiled at parse time and its bytes are cached in
        rs_loader._RS_DLL_CACHE[crate_name].  We write them to a temp file,
        load via ctypes, call hv_init() to register the Python arena callbacks,
        then wrap each _tl symbol as a native callable.
        """
        import ctypes
        import tempfile
        from pathlib import Path
        from ..ast import StmtFnDef, StmtClassDef, StmtField
        from .native_api import (
            make_hv_callbacks, HvCallbacks,
            alloc_handle, get_handle,
        )
        from .value import TlNamespace, TlClass, TlInstance
        from .builtins import _make_native
        from ..partial_compiler.rs_loader import _RS_DLL_CACHE, native_lib_ext

        dll_bytes = _RS_DLL_CACHE.get(crate_name)
        if dll_bytes is None:
            raise RuntimeError(
                f"import[rs]: DLL for '{crate_name}' not in cache "
                "(re-run to trigger recompilation)"
            )

        # Write DLL bytes to a temp file that persists for the process lifetime
        ext = native_lib_ext()
        stem = crate_name.replace(".", "_").replace("-", "_")
        tmp_path = Path(tempfile.gettempdir()) / f"hv_rs_{stem}_rt.{ext}"
        tmp_path.write_bytes(dll_bytes)

        try:
            lib = ctypes.CDLL(str(tmp_path))
        except OSError as e:
            raise RuntimeError(
                f"import[rs]: cannot load DLL for '{crate_name}': {e}"
            ) from e

        # Register Python arena callbacks with the DLL
        cb = make_hv_callbacks()
        try:
            hv_init_fn = lib.hv_init
            hv_init_fn.argtypes = [ctypes.POINTER(HvCallbacks)]
            hv_init_fn.restype = None
            hv_init_fn(ctypes.byref(cb))
        except AttributeError:
            pass

        def _make_tl_wrapper(tl_sym: str, n_params: int):
            """Return a callable(args, kwargs)->value that calls {tl_sym} in the DLL."""
            try:
                fn = getattr(lib, tl_sym)
            except AttributeError:
                return None
            fn.argtypes = [ctypes.POINTER(ctypes.c_int64), ctypes.c_int32]
            fn.restype = ctypes.c_int64

            def wrapper(args: list, kwargs: dict) -> object:
                padded = (list(args) + [None] * n_params)[:n_params]
                handles = (ctypes.c_int64 * n_params)(
                    *[alloc_handle(a) for a in padded]
                )
                ret_h = fn(handles, n_params)
                return get_handle(ret_h)

            return wrapper

        members: dict[str, object] = {}
        built_classes: dict[str, object] = {}

        for stmt in body:
            if isinstance(stmt, StmtFnDef):
                # Free function: args[0..n] are the positional params
                n = len(stmt.params)
                w = _make_tl_wrapper(f"{stmt.name}_tl", n)
                if w is not None:
                    members[stmt.name] = _make_native(stmt.name, w)

            elif isinstance(stmt, StmtClassDef):
                cls_name = stmt.name

                # Collect field names from StmtField stubs (skip __rs_handle__)
                field_names = [
                    s.name for s in stmt.body
                    if isinstance(s, StmtField) and s.name != "__rs_handle__"
                ]

                # Build method dict: method_name → _NativeCallable
                # _call_method / _get_instance_attr handle _NativeCallable in methods
                methods: dict[str, list] = {}
                for m in stmt.body:
                    if not isinstance(m, StmtFnDef):
                        continue
                    mname = m.name
                    if mname == "__init__":
                        tl_sym = f"{cls_name}____init___tl"
                    else:
                        tl_sym = f"{cls_name}__{mname}_tl"
                    n = len(m.params)
                    w = _make_tl_wrapper(tl_sym, n)
                    if w is not None:
                        methods[mname] = [_make_native(mname, w)]

                # field_defaults: include __rs_handle__ so _instantiate_class fills it
                field_defaults = [("__rs_handle__", 0, True)] + [
                    (fname, None, True) for fname in field_names
                ]

                cls = TlClass(
                    name=cls_name,
                    bases=[],
                    methods=methods,
                    gen_methods={},
                    field_defaults=field_defaults,
                    class_vars={},
                    field_mutability={"__rs_handle__": True, **{f: True for f in field_names}},
                    field_access={},
                    method_access={},
                    static_method_names=set(),
                    class_method_names=set(),
                    static_vars={},
                )
                members[cls_name] = cls
                built_classes[cls_name] = cls

        # Register classes in the global var registry so cb_get_global can find them
        from .native_api import _GLOBAL_VARS
        _GLOBAL_VARS.update(built_classes)

        return TlNamespace(name=crate_name, members=members)

    # ------------------------------------------------------------------
    # Async assign
    # ------------------------------------------------------------------

    def _exec_async_assign(self, target: str, stmts: list) -> None:
        mgr_val = self._env.get(target)
        if not isinstance(mgr_val, TlInstance):
            raise RuntimeError(f"TypeError: '{target}' is not an AsyncManager")

        # Capture current environment
        env_snapshot: dict = {}
        for scope in self._env._scopes:
            for name, entry in scope.items():
                val = entry[2][0] if entry[2] is not None else entry[0]
                env_snapshot[name] = deep_clone(val)

        # Submit task as a thread
        import threading as _threading
        results = mgr_val.fields.get("results")
        progress = mgr_val.fields.get("progress_status")
        error_list = mgr_val.fields.get("error_list")
        task_idx = len(results[0].items) if results else 0

        if results:
            results[0].items.append(None)
        if progress:
            progress[0].items.append("Waiting")
        if error_list:
            error_list[0].items.append(None)

        def run_task():
            sub = Interpreter()
            # Populate sub env
            for n, v in env_snapshot.items():
                if not sub._env.contains(n):
                    sub._env.declare(n, v, mutable=True)
            try:
                if progress and task_idx < len(progress[0].items):
                    progress[0].items[task_idx] = "Running"
                sub.exec_stmts(stmts)
                if results and task_idx < len(results[0].items):
                    results[0].items[task_idx] = None
                if progress and task_idx < len(progress[0].items):
                    progress[0].items[task_idx] = "Done"
            except RaiseSignal as e:
                if error_list and task_idx < len(error_list[0].items):
                    error_list[0].items[task_idx] = display(e.exception_value)
                if progress and task_idx < len(progress[0].items):
                    progress[0].items[task_idx] = "Done"
                raise_imm = mgr_val.fields.get("raise_immediately")
                if raise_imm and is_truthy(raise_imm[0]):
                    pass  # error stored, wait_for_finish re-raises
            except Exception as ex:
                if error_list and task_idx < len(error_list[0].items):
                    error_list[0].items[task_idx] = str(ex)
                if progress and task_idx < len(progress[0].items):
                    progress[0].items[task_idx] = "Done"

        t = _threading.Thread(target=run_task, daemon=True)
        t.start()


# ---------------------------------------------------------------------------
# Python ↔ tl value bridging (for py imports)
# ---------------------------------------------------------------------------

def _py_to_tl(v) -> Value:
    if v is None: return None
    if isinstance(v, bool): return v
    if isinstance(v, int): return v
    if isinstance(v, float): return v
    if isinstance(v, str): return v
    if isinstance(v, list): return TlList(items=[_py_to_tl(x) for x in v])
    if isinstance(v, tuple): return TlTuple(values=[_py_to_tl(x) for x in v])
    if isinstance(v, dict):
        d = TlDict()
        for k, val in v.items():
            d.set(_py_to_tl(k), _py_to_tl(val))
        return d
    return v  # pass through opaque
