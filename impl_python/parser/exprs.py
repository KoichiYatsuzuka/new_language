# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Expression parsing (mirrors src/parser/exprs.rs)."""
from __future__ import annotations
from typing import Optional

from ..token import TokenKind
from ..ast import (
    BinOp, UnaryOp,
    ExprInt, ExprFloat, ExprImaginaryLit, ExprStr, ExprBool, ExprNone, ExprUndefined, ExprIdent,
    ExprList, ExprAttr, ExprTraitAccess, ExprBinOp, ExprUnaryOp,
    ExprCall, ExprTemplateInstantiate, ExprSubscript, ExprSlice,
    ExprDict, ExprTuple, ExprSet, ExprBlock, ExprIfExpr,
    ExprForExpr, ExprWhileExpr, ExprMatchExpr, ExprIsType, ExprMustBe, ExprCast, ExprLocalVar,
    CallArgPositional, CallArgKeyword, CallArgVariadic, Expr,
)


class _ParserExprs:
    """Mixin providing expression parsing (precedence climbing)."""

    def _parse_expr(self) -> Expr:
        return self._parse_or()

    def _parse_or(self) -> Expr:
        left = self._parse_and()
        while self._current_kind() == TokenKind.OR:
            span = self._current_span()
            self._advance()
            right = self._parse_and()
            left = ExprBinOp(op=BinOp.OR, left=left, right=right, span=span)
        return left

    def _parse_and(self) -> Expr:
        left = self._parse_not()
        while self._current_kind() == TokenKind.AND:
            span = self._current_span()
            self._advance()
            right = self._parse_not()
            left = ExprBinOp(op=BinOp.AND, left=left, right=right, span=span)
        return left

    def _parse_not(self) -> Expr:
        if self._current_kind() == TokenKind.NOT:
            self._advance()
            return ExprUnaryOp(op=UnaryOp.NOT, operand=self._parse_not())
        return self._parse_comparison()

    def _parse_comparison(self) -> Expr:
        left = self._parse_bitor()
        span = self._current_span()
        k = self._current_kind()
        if k == TokenKind.IS:
            self._advance()
            type_name = self._expect_guard_type_name()
            return ExprIsType(expr=left, negated=False, type_name=type_name, span=span)
        if k == TokenKind.IS_NOT:
            self._advance()
            type_name = self._expect_guard_type_name()
            return ExprIsType(expr=left, negated=True, type_name=type_name, span=span)
        if k == TokenKind.MUSTBE:
            self._advance()
            guard_type = self._parse_mustbe_type()
            return ExprMustBe(expr=left, guard_type=guard_type, span=span)
        if k == TokenKind.IN:
            self._advance()
            right = self._parse_bitor()
            return ExprBinOp(op=BinOp.IN, left=left, right=right, span=span)
        if k == TokenKind.NOT_IN:
            self._advance()
            right = self._parse_bitor()
            return ExprBinOp(op=BinOp.NOT_IN, left=left, right=right, span=span)
        _CMP = {
            TokenKind.EQ_EQ: BinOp.EQ,
            TokenKind.EQ_EQ_EQ: BinOp.REF_EQ,
            TokenKind.NOT_EQ: BinOp.NOT_EQ,
            TokenKind.LT: BinOp.LT,
            TokenKind.GT: BinOp.GT,
            TokenKind.LT_EQ: BinOp.LT_EQ,
            TokenKind.GT_EQ: BinOp.GT_EQ,
        }
        if k in _CMP:
            op = _CMP[k]
            self._advance()
            right = self._parse_bitor()
            return ExprBinOp(op=op, left=left, right=right, span=span)
        return left

    def _parse_bitor(self) -> Expr:
        left = self._parse_bitxor()
        while self._current_kind() == TokenKind.PIPE:
            span = self._current_span()
            self._advance()
            right = self._parse_bitxor()
            left = ExprBinOp(op=BinOp.BIT_OR, left=left, right=right, span=span)
        return left

    def _parse_bitxor(self) -> Expr:
        left = self._parse_bitand()
        while self._current_kind() == TokenKind.CARET:
            span = self._current_span()
            self._advance()
            right = self._parse_bitand()
            left = ExprBinOp(op=BinOp.BIT_XOR, left=left, right=right, span=span)
        return left

    def _parse_bitand(self) -> Expr:
        left = self._parse_shift()
        while self._current_kind() == TokenKind.AMP:
            span = self._current_span()
            self._advance()
            right = self._parse_shift()
            left = ExprBinOp(op=BinOp.BIT_AND, left=left, right=right, span=span)
        return left

    def _parse_shift(self) -> Expr:
        left = self._parse_additive()
        _SH = {TokenKind.LT_LT: BinOp.L_SHIFT, TokenKind.GT_GT: BinOp.R_SHIFT}
        while self._current_kind() in _SH:
            span = self._current_span()
            op = _SH[self._current_kind()]
            self._advance()
            right = self._parse_additive()
            left = ExprBinOp(op=op, left=left, right=right, span=span)
        return left

    def _parse_additive(self) -> Expr:
        left = self._parse_multiplicative()
        _ADD = {TokenKind.PLUS: BinOp.ADD, TokenKind.MINUS: BinOp.SUB}
        while self._current_kind() in _ADD:
            span = self._current_span()
            op = _ADD[self._current_kind()]
            self._advance()
            right = self._parse_multiplicative()
            left = ExprBinOp(op=op, left=left, right=right, span=span)
        return left

    def _parse_multiplicative(self) -> Expr:
        left = self._parse_unary()
        _MUL = {
            TokenKind.STAR: BinOp.MUL,
            TokenKind.SLASH: BinOp.DIV,
            TokenKind.SLASH_SLASH: BinOp.FLOOR_DIV,
            TokenKind.PERCENT: BinOp.MOD,
            TokenKind.AT: BinOp.MUL,
        }
        while self._current_kind() in _MUL:
            span = self._current_span()
            op = _MUL[self._current_kind()]
            self._advance()
            right = self._parse_unary()
            left = ExprBinOp(op=op, left=left, right=right, span=span)
        return left

    def _parse_unary(self) -> Expr:
        k = self._current_kind()
        if k == TokenKind.MINUS:
            self._advance()
            return ExprUnaryOp(op=UnaryOp.NEG, operand=self._parse_unary())
        if k == TokenKind.TILDE:
            self._advance()
            return ExprUnaryOp(op=UnaryOp.BIT_NOT, operand=self._parse_unary())
        if k == TokenKind.PLUS:
            self._advance()
            return self._parse_unary()
        return self._parse_power()

    def _parse_power(self) -> Expr:
        base = self._parse_call()
        if self._current_kind() == TokenKind.STAR_STAR:
            span = self._current_span()
            self._advance()
            exp = self._parse_unary()
            return ExprBinOp(op=BinOp.POW, left=base, right=exp, span=span)
        return base

    # ------------------------------------------------------------------
    # Postfix: call / attr / subscript / template
    # ------------------------------------------------------------------

    def _parse_call(self) -> Expr:
        expr = self._parse_primary()
        while True:
            k = self._current_kind()
            if k == TokenKind.LPAREN:
                self._advance()
                args: list = []
                while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
                    # 可変長引数: ... = expr, expr, ...
                    if (
                        self._current_kind() == TokenKind.ELLIPSIS
                        and self._peek1_kind() == TokenKind.EQ
                    ):
                        self._advance()  # consume ...
                        self._advance()  # consume =
                        variadic_exprs: list = []
                        while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
                            variadic_exprs.append(self._parse_expr())
                            if self._current_kind() == TokenKind.COMMA:
                                self._advance()
                            else:
                                break
                        if not variadic_exprs:
                            from . import ParseError
                            raise ParseError("ParseError: variadic argument '...' requires at least one expression")
                        args.append(CallArgVariadic(exprs=variadic_exprs))
                        break  # variadic must be last
                    elif (
                        self._current_kind() == TokenKind.IDENT
                        and self._peek1_kind() == TokenKind.EQ
                    ):
                        name = self._current().value
                        assert isinstance(name, str)
                        self._advance()
                        self._advance()
                        args.append(CallArgKeyword(name=name, value=self._parse_expr()))
                    else:
                        args.append(CallArgPositional(expr=self._parse_expr()))
                    if self._current_kind() == TokenKind.COMMA:
                        self._advance()
                    else:
                        break
                self._eat(TokenKind.RPAREN)
                expr = ExprCall(func=expr, args=args)

            elif k == TokenKind.DOT:
                self._advance()
                attr = self._expect_ident()
                expr = ExprAttr(object=expr, attr=attr)

            elif k == TokenKind.COLON_COLON:
                self._advance()
                trait_name = self._expect_ident()
                self._eat(TokenKind.DOT)
                attr = self._expect_ident()
                expr = ExprTraitAccess(object=expr, trait_name=trait_name, attr=attr)

            elif k == TokenKind.LBRACKET:
                expr = self._parse_bracket_suffix(expr)

            elif k == TokenKind.FAT_ARROW:
                span = self._current_span()
                self._advance()
                type_name = self._parse_type_expr()
                expr = ExprCast(object=expr, type_name=type_name, span=span)

            else:
                break
        return expr

    def _parse_bracket_suffix(self, expr: Expr) -> Expr:
        if self._is_template_instantiation():
            self._advance()
            type_args: list[str] = []
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                type_args.append(self._parse_type_expr())
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
                else:
                    break
            self._eat(TokenKind.RBRACKET)
            return ExprTemplateInstantiate(base=expr, type_args=type_args)

        self._advance()  # `[`

        k = self._current_kind()
        if k == TokenKind.COLON_COLON:
            self._advance()
            step = self._parse_slice_part()
            index: Expr = ExprSlice(begin=None, end=None, step=step)
        elif k == TokenKind.COLON:
            self._advance()
            end = self._parse_slice_part()
            step = self._parse_slice_step()
            index = ExprSlice(begin=None, end=end, step=step)
        else:
            first = self._parse_expr()
            if self._current_kind() == TokenKind.COLON_COLON:
                self._advance()
                step = self._parse_slice_part()
                index = ExprSlice(begin=first, end=None, step=step)
            elif self._current_kind() == TokenKind.COLON:
                self._advance()
                end = self._parse_slice_part()
                step = self._parse_slice_step()
                index = ExprSlice(begin=first, end=end, step=step)
            else:
                index = first

        self._eat(TokenKind.RBRACKET)
        return ExprSubscript(object=expr, index=index)

    def _parse_slice_part(self) -> Optional[Expr]:
        if self._current_kind() in (
            TokenKind.RBRACKET, TokenKind.COLON,
            TokenKind.COLON_COLON, TokenKind.EOF
        ):
            return None
        return self._parse_expr()

    def _parse_slice_step(self) -> Optional[Expr]:
        k = self._current_kind()
        if k == TokenKind.COLON:
            self._advance()
            return self._parse_slice_part()
        if k == TokenKind.COLON_COLON:
            self._advance()
            return self._parse_slice_part()
        return None

    def _is_template_instantiation(self) -> bool:
        i = self._pos + 1
        depth = 1
        while i < len(self._tokens):
            tk = self._tokens[i].token.kind
            if tk == TokenKind.LBRACKET:
                depth += 1
            elif tk == TokenKind.RBRACKET:
                depth -= 1
                if depth == 0:
                    nxt = self._tokens[i + 1].token.kind if i + 1 < len(self._tokens) else TokenKind.EOF
                    return nxt == TokenKind.LPAREN
            elif tk == TokenKind.EOF:
                break
            i += 1
        return False

    # ------------------------------------------------------------------
    # Primary expressions
    # ------------------------------------------------------------------

    def _parse_primary(self) -> Expr:
        k = self._current_kind()
        tok = self._current()

        if k == TokenKind.INT:
            v = tok.value; self._advance(); return ExprInt(value=v)  # type: ignore[arg-type]
        if k == TokenKind.FLOAT:
            v = tok.value; self._advance(); return ExprFloat(value=v)  # type: ignore[arg-type]
        if k == TokenKind.IMAGINARY_FLOAT:
            v = tok.value; self._advance(); return ExprImaginaryLit(value=v)  # type: ignore[arg-type]
        if k == TokenKind.STR:
            v = tok.value; self._advance(); return ExprStr(value=v)  # type: ignore[arg-type]
        if k == TokenKind.TRUE:
            self._advance(); return ExprBool(value=True)
        if k == TokenKind.FALSE:
            self._advance(); return ExprBool(value=False)
        if k == TokenKind.NONE:
            self._advance(); return ExprNone()
        if k == TokenKind.UNDEFINED:
            self._advance(); return ExprUndefined()
        if k == TokenKind.ANY:
            self._advance(); return ExprIdent(name="Any")
        if k == TokenKind.UNION:
            self._advance(); return ExprIdent(name="Union")
        if k == TokenKind.OPTION:
            self._advance(); return ExprIdent(name="Option")
        if k == TokenKind.IDENT:
            name = tok.value
            assert isinstance(name, str)
            # local::name → ExprLocalVar
            if name == "local":
                self._advance()
                if self._current_kind() != TokenKind.COLON_COLON:
                    raise self._error("ParseError: 'local' must be followed by '::name'")
                self._advance()
                var_name = self._expect_ident()
                return ExprLocalVar(name=var_name)
            self._advance(); return ExprIdent(name=name)
        if k == TokenKind.SELF_TYPE:
            if self._class_or_trait_depth == 0:
                raise self._error("ParseError: 'Self' can only be used inside class or trait definitions")
            self._advance(); return ExprIdent(name="Self")
        if k == TokenKind.LPAREN:
            return self._parse_paren_expr()
        if k == TokenKind.LBRACKET:
            return self._parse_list_literal()
        if k == TokenKind.LBRACE:
            return self._parse_dict_or_set_literal()

        if k == TokenKind.IF:
            self._advance()
            branches, else_body, return_type = self._parse_if_components()
            return ExprIfExpr(branches=branches, else_body=else_body, return_type=return_type)

        if k == TokenKind.FOR:
            self._advance()
            target = self._expect_ident()
            self._eat(TokenKind.IN)
            iter_ = self._parse_expr()
            return_type = self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return ExprForExpr(target=target, iter=iter_, body=self._parse_block(), return_type=return_type)

        if k == TokenKind.WHILE:
            self._advance()
            cond = self._parse_expr()
            return_type = self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return ExprWhileExpr(cond=cond, body=self._parse_block(), return_type=return_type)

        if k == TokenKind.MATCH:
            self._advance()
            subject = self._parse_expr()
            return_type = self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            self._eat(TokenKind.NEWLINE)
            self._eat(TokenKind.INDENT)
            arms = self._parse_match_arms()
            return ExprMatchExpr(subject=subject, arms=arms, return_type=return_type)

        if k == TokenKind.BLOCK:
            self._advance()
            return_type = self._parse_opt_return_type()
            self._eat(TokenKind.COLON)
            return ExprBlock(stmts=self._parse_block(), return_type=return_type)

        raise self._error(f"unexpected token: `{tok.kind.name}`")

    def _parse_paren_expr(self) -> Expr:
        self._advance()  # `(`
        if self._current_kind() == TokenKind.RPAREN:
            self._advance(); return ExprTuple(elements=[])
        first = self._parse_expr()
        if self._current_kind() == TokenKind.RPAREN:
            self._advance(); return first
        self._eat(TokenKind.COMMA)
        items = [first]
        while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
            items.append(self._parse_expr())
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RPAREN)
        return ExprTuple(elements=items)

    def _parse_list_literal(self) -> Expr:
        self._advance()  # `[`
        items: list[Expr] = []
        while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
            items.append(self._parse_expr())
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RBRACKET)
        return ExprList(elements=items)

    def _parse_dict_or_set_literal(self) -> Expr:
        self._advance()  # `{`
        if self._current_kind() == TokenKind.RBRACE:
            self._advance(); return ExprDict(pairs=[])
        first = self._parse_expr()
        if self._current_kind() == TokenKind.COLON:
            self._advance()
            val = self._parse_expr()
            pairs: list[tuple[Expr, Expr]] = [(first, val)]
            while self._current_kind() == TokenKind.COMMA:
                self._advance()
                if self._current_kind() == TokenKind.RBRACE:
                    break
                key = self._parse_expr()
                self._eat(TokenKind.COLON)
                val = self._parse_expr()
                pairs.append((key, val))
            self._eat(TokenKind.RBRACE)
            return ExprDict(pairs=pairs)
        else:
            items_s: list[Expr] = [first]
            while self._current_kind() == TokenKind.COMMA:
                self._advance()
                if self._current_kind() == TokenKind.RBRACE:
                    break
                items_s.append(self._parse_expr())
            self._eat(TokenKind.RBRACE)
            return ExprSet(elements=items_s)
