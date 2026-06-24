# git SHA: d4bdc21ea237938cb9213f731fd60a3fe6046b78
"""Statement parsing (mirrors src/parser/stmts.rs)."""
from __future__ import annotations
from typing import Optional

from ..token import TokenKind
from ..ast import (
    BinOp, Accessibility, FieldKind, Param,
    MatchPatternCase, MatchPatternIsType, MatchArm, ExceptHandler,
    TupleTarget, TupleTargetLet, TupleTargetMut, TupleTargetBare, TupleTargetWildcard,
    Expr, Stmt,
    StmtExpr, StmtLet, StmtConst, StmtMut, StmtStatic,
    StmtAssign, StmtAttrAssign, StmtAttrCompoundAssign, StmtCompoundAssign,
    StmtIf, StmtMatch, StmtWhile, StmtFor, StmtBlock,
    StmtReturn, StmtBreak, StmtContinue, StmtPass,
    StmtBlockReturn, StmtLoopYield, StmtYield, StmtFreeze,
    StmtFnDef, StmtGenDef, StmtEnumDef, StmtNewTypeDef,
    StmtTry, StmtRaise, StmtAsyncAssign,
    StmtEventSubscribe, StmtEventUnsubscribe,
    ExprAttr, ExprIdent, ExprTraitAccess,
)


_COMPOUND_OPS: dict[TokenKind, BinOp] = {
    TokenKind.PLUS_EQ:        BinOp.ADD,
    TokenKind.MINUS_EQ:       BinOp.SUB,
    TokenKind.STAR_EQ:        BinOp.MUL,
    TokenKind.SLASH_EQ:       BinOp.DIV,
    TokenKind.SLASH_SLASH_EQ: BinOp.FLOOR_DIV,
    TokenKind.PERCENT_EQ:     BinOp.MOD,
    TokenKind.STAR_STAR_EQ:   BinOp.POW,
    TokenKind.AMP_EQ:         BinOp.BIT_AND,
    TokenKind.PIPE_EQ:        BinOp.BIT_OR,
    TokenKind.CARET_EQ:       BinOp.BIT_XOR,
    TokenKind.LT_LT_EQ:       BinOp.L_SHIFT,
    TokenKind.GT_GT_EQ:       BinOp.R_SHIFT,
}


def _token_to_compound_op(kind: TokenKind) -> Optional[BinOp]:
    return _COMPOUND_OPS.get(kind)


def _validate_param_defaults(params: list[Param]) -> None:
    seen_default = False
    for p in params:
        if p.name == "self" or p.variadic:
            continue
        if p.default is not None:
            seen_default = True
        elif seen_default:
            from . import ParseError
            raise ParseError(
                f"ParseError: non-default parameter '{p.name}' follows a parameter with a default value"
            )


def _validate_variadic_params(fn_name: str, params: list[Param]) -> None:
    from . import ParseError
    variadic_indices = [i for i, p in enumerate(params) if p.variadic]
    if len(variadic_indices) > 1:
        raise ParseError(
            f"ParseError: function `{fn_name}` has more than one variadic parameter `...`"
        )
    if variadic_indices and variadic_indices[0] != len(params) - 1:
        raise ParseError(
            f"ParseError: variadic parameter `...` must be the last parameter in function `{fn_name}`"
        )


def _body_has_return(stmts: list[Stmt]) -> bool:
    for stmt in stmts:
        if isinstance(stmt, StmtReturn):
            return True
        if isinstance(stmt, StmtIf):
            if any(_body_has_return(b) for _, b in stmt.branches):
                return True
            if stmt.else_body and _body_has_return(stmt.else_body):
                return True
        if isinstance(stmt, (StmtWhile, StmtFor, StmtBlock)):
            body = stmt.body if isinstance(stmt, (StmtWhile, StmtFor)) else stmt.stmts
            if _body_has_return(body):
                return True
    return False


class _ParserStmts:
    """Mixin providing statement parsing."""

    # ------------------------------------------------------------------
    # Block
    # ------------------------------------------------------------------

    def _parse_block(self) -> list[Stmt]:
        self._eat(TokenKind.NEWLINE)
        self._eat(TokenKind.INDENT)
        stmts: list[Stmt] = []
        while True:
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            if self._current_kind() in (TokenKind.DEDENT, TokenKind.EOF):
                break
            stmts.append(self._parse_stmt())
        if self._current_kind() == TokenKind.DEDENT:
            self._advance()
        return stmts

    # ------------------------------------------------------------------
    # Tuple unpack
    # ------------------------------------------------------------------

    def _parse_tuple_unpack(self, first: TupleTarget) -> Stmt:
        from ..ast import StmtLetTuple
        span = self._current_span()
        targets: list[TupleTarget] = [first]
        while self._current_kind() == TokenKind.COMMA:
            self._advance()
            k = self._current_kind()
            if k == TokenKind.LET:
                self._advance()
                targets.append(TupleTargetLet(self._expect_ident()))
            elif k == TokenKind.MUT:
                self._advance()
                targets.append(TupleTargetMut(self._expect_ident()))
            elif k == TokenKind.IDENT and self._current().value == "_":
                self._advance()
                targets.append(TupleTargetWildcard())
                break
            elif k == TokenKind.IDENT:
                name = self._current().value
                self._advance()
                targets.append(TupleTargetBare(name))
            else:
                raise self._error(
                    f"expected `let`, `mut`, or `_` in tuple unpack, got `{self._current().kind.name}`"
                )
        self._eat(TokenKind.EQ)
        return StmtLetTuple(targets=targets, value=self._parse_expr(), span=span)

    # ------------------------------------------------------------------
    # Statement dispatch
    # ------------------------------------------------------------------

    def _parse_stmt(self) -> Stmt:
        k = self._current_kind()

        if k == TokenKind.LET:
            self._advance()
            name = self._expect_ident()
            if self._current_kind() == TokenKind.COMMA:
                return self._parse_tuple_unpack(TupleTargetLet(name))
            type_ann: Optional[str] = None
            if self._current_kind() == TokenKind.COLON:
                self._advance(); type_ann = self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtLet(name=name, expr=self._parse_expr(), type_ann=type_ann)

        if k == TokenKind.CONST:
            self._advance()
            name = self._expect_ident()
            if self._current_kind() == TokenKind.COLON:
                self._advance(); self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtConst(name=name, expr=self._parse_expr())

        if k == TokenKind.MUT:
            self._advance()
            name = self._expect_ident()
            if self._current_kind() == TokenKind.COMMA:
                return self._parse_tuple_unpack(TupleTargetMut(name))
            mut_type_ann: Optional[str] = None
            if self._current_kind() == TokenKind.COLON:
                self._advance(); mut_type_ann = self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtMut(name=name, expr=self._parse_expr(), type_ann=mut_type_ann)

        if k == TokenKind.STATIC:
            span = self._current_span()
            self._advance()
            self._eat(TokenKind.MUT)
            name = self._expect_ident()
            if self._current_kind() == TokenKind.COLON:
                self._advance(); self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtStatic(name=name, expr=self._parse_expr(), span=span)

        if k == TokenKind.FREEZE:
            span = self._current_span()
            self._advance()
            name = self._expect_ident()
            return StmtFreeze(name=name, span=span)

        if k == TokenKind.PASS:
            self._advance(); return StmtPass()
        if k == TokenKind.BREAK:
            self._advance(); return StmtBreak()
        if k == TokenKind.CONTINUE:
            self._advance(); return StmtContinue()

        if k == TokenKind.RETURN:
            self._advance()
            if self._current_kind() in (
                TokenKind.NEWLINE, TokenKind.EOF,
                TokenKind.SEMICOLON, TokenKind.DEDENT
            ):
                return StmtReturn(expr=None)
            return StmtReturn(expr=self._parse_expr())

        if k == TokenKind.BLOCK_RETURN:
            self._advance(); return StmtBlockReturn(expr=self._parse_expr())
        if k == TokenKind.LOOP_YIELD:
            self._advance(); return StmtLoopYield(expr=self._parse_expr())

        if k == TokenKind.IF:
            return self._parse_if_stmt()
        if k == TokenKind.MATCH:
            return self._parse_match_stmt()

        if k == TokenKind.WHILE:
            self._advance()
            cond = self._parse_expr()
            self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return StmtWhile(cond=cond, body=self._parse_block())

        if k == TokenKind.FOR:
            self._advance()
            first = self._expect_ident()
            targets = [first]
            while self._current_kind() == TokenKind.COMMA:
                self._advance()
                targets.append(self._expect_ident())
            self._eat(TokenKind.IN)
            iter_ = self._parse_expr()
            self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return StmtFor(targets=targets, iter=iter_, body=self._parse_block())

        if k == TokenKind.BLOCK:
            self._advance()
            self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return StmtBlock(stmts=self._parse_block())

        if k == TokenKind.YIELD:
            self._advance(); return StmtYield(expr=self._parse_expr())
        if k == TokenKind.TRY:
            return self._parse_try_stmt()

        if k == TokenKind.RAISE:
            span = self._current_span()
            self._advance()
            if self._current_kind() in (
                TokenKind.NEWLINE, TokenKind.EOF,
                TokenKind.SEMICOLON, TokenKind.DEDENT
            ):
                return StmtRaise(exc=None, span=span)
            return StmtRaise(exc=self._parse_expr(), span=span)

        if k == TokenKind.AT:
            decorators = self._parse_decorators()
            k2 = self._current_kind()
            if k2 == TokenKind.FN:
                return self._parse_fn_def_with_flags(decorators, False, False)
            if k2 == TokenKind.CLASS:
                return self._parse_class_def_decorated(decorators)
            raise self._error(
                f"ParseError: '@' decorator must be followed by 'fn' or 'class', got `{self._current().kind.name}`"
            )

        if k == TokenKind.FN:
            return self._parse_fn_def()
        if k == TokenKind.GEN:
            return self._parse_gen_def()
        if k == TokenKind.CLASS:
            return self._parse_class_def()
        if k == TokenKind.ENUM:
            return self._parse_enum_def()
        if k == TokenKind.TRAIT:
            return self._parse_trait_def()
        if k == TokenKind.PROTOCOL:
            return self._parse_protocol_def()
        if k == TokenKind.NEW_TYPE:
            return self._parse_new_type_def()
        if k == TokenKind.IMPORT:
            return self._parse_import_stmt()
        if k == TokenKind.FROM:
            return self._parse_from_import_stmt()
        if k == TokenKind.IDENT:
            return self._parse_ident_stmt()

        expr = self._parse_expr()
        if self._current_kind() in (TokenKind.ON, TokenKind.ONCE, TokenKind.OFF):
            return self._try_parse_event_stmt(expr)
        return StmtExpr(expr=expr)

    # ------------------------------------------------------------------
    # if / elif / else
    # ------------------------------------------------------------------

    def _parse_if_components(self) -> tuple[list[tuple[Expr, list[Stmt]]], Optional[list[Stmt]], Optional[str]]:
        cond = self._parse_expr()
        return_type = self._parse_opt_return_type()
        self._eat(TokenKind.COLON)
        body = self._parse_block()
        branches: list[tuple[Expr, list[Stmt]]] = [(cond, body)]
        else_body: Optional[list[Stmt]] = None
        while True:
            k = self._current_kind()
            if k == TokenKind.ELIF:
                self._advance()
                c = self._parse_expr()
                self._parse_opt_return_type()
                self._eat(TokenKind.COLON)
                branches.append((c, self._parse_block()))
            elif k == TokenKind.ELSE:
                self._advance()
                self._eat(TokenKind.COLON)
                else_body = self._parse_block()
                break
            else:
                break
        return branches, else_body, return_type

    def _parse_if_stmt(self) -> Stmt:
        self._advance()
        branches, else_body, _ = self._parse_if_components()
        return StmtIf(branches=branches, else_body=else_body)

    # ------------------------------------------------------------------
    # match
    # ------------------------------------------------------------------

    def _parse_match_arms(self) -> list[MatchArm]:
        arms: list[MatchArm] = []
        is_case_kind: Optional[bool] = None
        while True:
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            if self._current_kind() in (TokenKind.DEDENT, TokenKind.EOF):
                break
            k = self._current_kind()
            if k == TokenKind.CASE:
                if is_case_kind is False:
                    raise self._error("match statement cannot mix 'case' and 'is' arms")
                is_case_kind = True
                self._advance()
                pattern_expr = self._parse_expr()
                self._eat(TokenKind.COLON)
                arms.append(MatchArm(pattern=MatchPatternCase(expr=pattern_expr), body=self._parse_block()))
            elif k == TokenKind.IS:
                if is_case_kind is True:
                    raise self._error("match statement cannot mix 'case' and 'is' arms")
                is_case_kind = False
                self._advance()
                type_name = self._expect_ident()
                self._eat(TokenKind.COLON)
                arms.append(MatchArm(pattern=MatchPatternIsType(type_name=type_name), body=self._parse_block()))
            else:
                raise self._error(f"expected 'case' or 'is' in match body, got `{self._current().kind.name}`")
        if self._current_kind() == TokenKind.DEDENT:
            self._advance()
        return arms

    def _parse_match_stmt(self) -> Stmt:
        span = self._current_span()
        self._advance()
        subject = self._parse_expr()
        self._parse_opt_return_type()
        self._eat(TokenKind.COLON)
        self._eat(TokenKind.NEWLINE)
        self._eat(TokenKind.INDENT)
        arms = self._parse_match_arms()
        return StmtMatch(subject=subject, arms=arms, span=span)

    # ------------------------------------------------------------------
    # Identifier-started statements
    # ------------------------------------------------------------------

    def _parse_ident_stmt(self) -> Stmt:
        # async assign: `target <- async [->Type]: body`
        if self._peek1_kind() == TokenKind.LT:
            if self._pos + 2 < len(self._tokens) and self._tokens[self._pos + 2].token.kind == TokenKind.MINUS:
                target = self._expect_ident()
                self._advance()  # consume `<`
                self._advance()  # consume `-`
                self._eat(TokenKind.ASYNC)
                return_type = self._parse_opt_return_type()
                self._eat(TokenKind.COLON)
                stmts = self._parse_block()
                return StmtAsyncAssign(target=target, return_type=return_type, stmts=stmts)

        if self._peek1_kind() == TokenKind.EQ:
            span = self._current_span()
            name = self._expect_ident()
            if name in self._known_new_types:
                raise self._error(
                    f"ParseError: cannot reassign new_type `{name}` — new_type bindings are const"
                )
            self._advance()  # consume `=`
            return StmtAssign(name=name, value=self._parse_expr(), span=span)

        op = _token_to_compound_op(self._peek1_kind())
        if op is not None:
            return self._parse_compound(op)

        expr = self._parse_expr()
        cur = self._current_kind()
        if cur == TokenKind.EQ:
            self._advance()
            return StmtAttrAssign(target=expr, value=self._parse_expr())
        op2 = _token_to_compound_op(cur)
        if op2 is not None:
            self._advance()
            return StmtAttrCompoundAssign(target=expr, op=op2, value=self._parse_expr())
        if cur in (TokenKind.ON, TokenKind.ONCE, TokenKind.OFF):
            return self._try_parse_event_stmt(expr)
        return StmtExpr(expr=expr)

    def _try_parse_event_stmt(self, source: Expr) -> Stmt:
        """Parse `source on/once/off handler` into an event stmt."""
        cur = self._current_kind()
        if cur in (TokenKind.ON, TokenKind.ONCE):
            is_once = cur == TokenKind.ONCE
            self._advance()
            is_async = False
            if self._current_kind() == TokenKind.ASYNC:
                self._advance()
                is_async = True
            handler = self._parse_expr()
            return StmtEventSubscribe(source=source, handler=handler,
                                      is_once=is_once, is_async=is_async)
        if cur == TokenKind.OFF:
            self._advance()
            handler = self._parse_expr()
            return StmtEventUnsubscribe(source=source, handler=handler)
        raise self._error(f"expected 'on', 'once', or 'off' in event statement")

    def _parse_compound(self, op: BinOp) -> Stmt:
        span = self._current_span()
        name = self._expect_ident()
        if name in self._known_new_types:
            raise self._error(
                f"ParseError: cannot reassign new_type `{name}` — new_type bindings are const"
            )
        self._advance()  # consume compound-assign operator
        return StmtCompoundAssign(name=name, op=op, value=self._parse_expr(), span=span)

    # ------------------------------------------------------------------
    # Function definitions
    # ------------------------------------------------------------------

    def _parse_decorators(self) -> list[Expr]:
        decorators: list[Expr] = []
        while self._current_kind() == TokenKind.AT:
            self._advance()
            decorators.append(self._parse_expr())
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
        return decorators

    def _parse_fn_def(self) -> Stmt:
        return self._parse_fn_def_with_flags([], False, False)

    def _parse_fn_def_with_flags(
        self,
        decorators: list[Expr],
        is_static: bool,
        is_class_method: bool,
    ) -> Stmt:
        self._eat(TokenKind.FN)
        name = self._expect_ident()
        template_params = self._parse_template_params()
        self._eat(TokenKind.LPAREN)
        params: list[Param] = []
        while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
            params.append(self._parse_param())
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RPAREN)
        _validate_param_defaults(params)
        _validate_variadic_params(name, params)
        return_type: Optional[str] = None
        if self._current_kind() == TokenKind.ARROW:
            self._advance()
            return_type = self._parse_type_expr()
        self._eat(TokenKind.COLON)
        if self._is_abstract_body():
            self._advance()  # NEWLINE
            self._advance()  # INDENT
            self._advance()  # ELLIPSIS
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            if self._current_kind() == TokenKind.DEDENT:
                self._advance()
            body: list[Stmt] = []
            is_abstract = True
        else:
            body = self._parse_block()
            is_abstract = False
        return StmtFnDef(
            name=name,
            template_params=template_params,
            params=params,
            return_type=return_type,
            body=body,
            is_abstract=is_abstract,
            is_static=is_static,
            is_class_method=is_class_method,
            decorators=decorators,
            access=Accessibility.PUBLIC,
        )

    def _parse_gen_def(self) -> Stmt:
        self._eat(TokenKind.GEN)
        name = self._expect_ident()
        template_params = self._parse_template_params()
        self._eat(TokenKind.LPAREN)
        params: list[Param] = []
        while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
            param = self._parse_param()
            if param.mutable and param.name != "self":
                raise self._error(
                    f"ParseError: generator function `{name}`: parameter `{param.name}` cannot be `mut`; "
                    "generator parameters must be `let` or `const`"
                )
            params.append(param)
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RPAREN)
        _validate_param_defaults(params)
        yield_type: Optional[str] = None
        if self._current_kind() == TokenKind.ARROW:
            self._advance()
            yield_type = self._parse_type_expr()
        self._eat(TokenKind.COLON)
        body = self._parse_block()
        if _body_has_return(body):
            raise self._error(
                f"ParseError: generator function `{name}` must not contain a `return` statement"
            )
        return StmtGenDef(
            name=name,
            template_params=template_params,
            params=params,
            yield_type=yield_type,
            body=body,
            access=Accessibility.PUBLIC,
        )

    def _is_abstract_body(self) -> bool:
        t0 = self._at(0)
        t1 = self._at(1)
        t2 = self._at(2)
        return (
            t0.kind == TokenKind.NEWLINE
            and t1.kind == TokenKind.INDENT
            and t2.kind == TokenKind.ELLIPSIS
        )

    # ------------------------------------------------------------------
    # try / except / finally
    # ------------------------------------------------------------------

    def _parse_try_stmt(self) -> Stmt:
        self._eat(TokenKind.TRY)
        self._eat(TokenKind.COLON)
        body = self._parse_block()
        handlers: list[ExceptHandler] = []
        finally_body: Optional[list[Stmt]] = None

        while self._current_kind() == TokenKind.EXCEPT:
            self._advance()
            if self._current_kind() == TokenKind.COLON:
                exc_type = None
                alias = None
            else:
                exc_type = self._expect_ident()
                alias = None
                if self._current_kind() == TokenKind.AS:
                    self._advance()
                    alias = self._expect_ident()
            self._eat(TokenKind.COLON)
            handler_body = self._parse_block()
            handlers.append(ExceptHandler(exc_type=exc_type, name=alias, body=handler_body))

        if self._current_kind() == TokenKind.FINALLY:
            self._advance()
            self._eat(TokenKind.COLON)
            finally_body = self._parse_block()

        if not handlers and finally_body is None:
            raise self._error("try statement requires at least one `except` or `finally` clause")
        return StmtTry(body=body, handlers=handlers, finally_body=finally_body)

    # ------------------------------------------------------------------
    # enum / new_type
    # ------------------------------------------------------------------

    def _parse_enum_def(self) -> Stmt:
        self._eat(TokenKind.ENUM)
        name = self._expect_ident()
        self._eat(TokenKind.COLON)
        self._eat(TokenKind.NEWLINE)
        self._eat(TokenKind.INDENT)
        variants: list[tuple[str, Optional[Expr]]] = []
        while True:
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            if self._current_kind() in (TokenKind.DEDENT, TokenKind.EOF):
                break
            vname = self._expect_ident()
            value: Optional[Expr] = None
            if self._current_kind() == TokenKind.EQ:
                self._advance()
                value = self._parse_expr()
            if self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            variants.append((vname, value))
        if self._current_kind() == TokenKind.DEDENT:
            self._advance()
        return StmtEnumDef(name=name, variants=variants)

    def _parse_new_type_def(self) -> Stmt:
        self._eat(TokenKind.NEW_TYPE)
        name = self._expect_ident()
        self._eat(TokenKind.COLON)
        original = self._parse_type_expr()
        self._known_new_types.add(name)
        return StmtNewTypeDef(name=name, original=original)
