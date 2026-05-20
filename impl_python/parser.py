from __future__ import annotations
from dataclasses import dataclass
from typing import Optional
from pathlib import Path

from .token import Span, Token, TokenKind, Spanned
from .ast import (
    BinOp, UnaryOp, Accessibility, FieldKind,
    TemplateParam, Param, CallArgPositional, CallArgKeyword,
    MatchPatternCase, MatchPatternIsType, MatchArm, ExceptHandler,
    TupleTarget, TupleTargetLet, TupleTargetMut, TupleTargetBare, TupleTargetWildcard,
    ExprInt, ExprFloat, ExprStr, ExprBool, ExprNone, ExprIdent,
    ExprList, ExprAttr, ExprTraitAccess, ExprBinOp, ExprUnaryOp,
    ExprCall, ExprTemplateInstantiate, ExprSubscript, ExprSlice,
    ExprDict, ExprTuple, ExprSet, ExprBlock, ExprIfExpr,
    ExprForExpr, ExprWhileExpr, ExprMatchExpr, ExprIsType,
    Expr,
    StmtExpr, StmtLet, StmtConst, StmtMut, StmtStatic,
    StmtAssign, StmtAttrAssign, StmtAttrCompoundAssign, StmtCompoundAssign,
    StmtIf, StmtMatch, StmtWhile, StmtFor, StmtBlock,
    StmtReturn, StmtBreak, StmtContinue, StmtPass,
    StmtBlockReturn, StmtLoopYield, StmtYield, StmtFreeze,
    StmtFnDef, StmtGenDef, StmtClassDef, StmtTraitDef, StmtField,
    StmtNewTypeDef, StmtEnumDef, StmtTry, StmtRaise,
    StmtImport, StmtFromImport, StmtLetTuple, StmtAsyncAssign,
    Stmt,
)
from .lexer import lex


# ---------------------------------------------------------------------------
# Compound-assignment operator mapping
# ---------------------------------------------------------------------------

_COMPOUND_OPS: dict[TokenKind, BinOp] = {
    TokenKind.PLUS_EQ:       BinOp.ADD,
    TokenKind.MINUS_EQ:      BinOp.SUB,
    TokenKind.STAR_EQ:       BinOp.MUL,
    TokenKind.SLASH_EQ:      BinOp.DIV,
    TokenKind.SLASH_SLASH_EQ: BinOp.FLOOR_DIV,
    TokenKind.PERCENT_EQ:    BinOp.MOD,
    TokenKind.STAR_STAR_EQ:  BinOp.POW,
    TokenKind.AMP_EQ:        BinOp.BIT_AND,
    TokenKind.PIPE_EQ:       BinOp.BIT_OR,
    TokenKind.CARET_EQ:      BinOp.BIT_XOR,
    TokenKind.LT_LT_EQ:      BinOp.L_SHIFT,
    TokenKind.GT_GT_EQ:      BinOp.R_SHIFT,
}


def _token_to_compound_op(kind: TokenKind) -> Optional[BinOp]:
    return _COMPOUND_OPS.get(kind)


# ---------------------------------------------------------------------------
# Parser
# ---------------------------------------------------------------------------

class ParseError(Exception):
    pass


class Parser:
    def __init__(self, tokens: list[Spanned], source_dir: Optional[Path] = None) -> None:
        self._tokens = tokens
        self._pos = 0
        self._known_traits: dict[str, tuple[list[TemplateParam], list, list[str]]] = {}
        # Pre-register built-in Error trait
        self._known_traits["Error"] = (
            [],
            [
                ("message",      FieldKind.LET, "str",  False),
                ("code_context", FieldKind.MUT, "str",  True),
                ("file",         FieldKind.MUT, "str",  True),
                ("line",         FieldKind.MUT, "int",  True),
                ("col",          FieldKind.MUT, "int",  True),
            ],
            [],
        )
        self._class_or_trait_depth = 0
        self._known_new_types: set[str] = set()
        resolved = source_dir if source_dir is not None else Path(".")
        self._source_dir = resolved
        self._root_dir = resolved
        self._module_cache: dict[tuple[str, Path], list[Stmt]] = {}
        self._loading: set[Path] = set()

    # ------------------------------------------------------------------
    # Token access helpers
    # ------------------------------------------------------------------

    def _current(self) -> Token:
        if self._pos < len(self._tokens):
            return self._tokens[self._pos].token
        return Token(TokenKind.EOF)

    def _current_kind(self) -> TokenKind:
        return self._current().kind

    def _peek1(self) -> Token:
        p = self._pos + 1
        if p < len(self._tokens):
            return self._tokens[p].token
        return Token(TokenKind.EOF)

    def _peek1_kind(self) -> TokenKind:
        return self._peek1().kind

    def _at(self, offset: int) -> Token:
        p = self._pos + offset
        if p < len(self._tokens):
            return self._tokens[p].token
        return Token(TokenKind.EOF)

    def _current_span(self) -> Span:
        if self._pos < len(self._tokens):
            return self._tokens[self._pos].span
        return Span.unknown()

    def _advance(self) -> None:
        if self._pos < len(self._tokens):
            self._pos += 1

    def _eat(self, expected: TokenKind) -> None:
        if self._current_kind() == expected:
            self._advance()
        else:
            cur = self._current()
            raise ParseError(
                f"expected `{expected.name}`, got `{cur.kind.name}`"
            )

    def _skip_newlines(self) -> None:
        while self._current_kind() in (
            TokenKind.NEWLINE, TokenKind.INDENT, TokenKind.DEDENT, TokenKind.SEMICOLON
        ):
            self._advance()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def parse_program(self) -> list[Stmt]:
        stmts: list[Stmt] = []
        self._skip_newlines()
        while self._current_kind() != TokenKind.EOF:
            stmts.append(self._parse_stmt())
            self._skip_newlines()
        return stmts

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
                raise ParseError(
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
            if self._current_kind() == TokenKind.COLON:
                self._advance(); self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtLet(name=name, expr=self._parse_expr())

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
            if self._current_kind() == TokenKind.COLON:
                self._advance(); self._parse_type_expr()
            self._eat(TokenKind.EQ)
            return StmtMut(name=name, expr=self._parse_expr())

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
            raise ParseError(
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
        if k == TokenKind.NEW_TYPE:
            return self._parse_new_type_def()
        if k == TokenKind.IMPORT:
            return self._parse_import_stmt()
        if k == TokenKind.FROM:
            return self._parse_from_import_stmt()
        if k == TokenKind.IDENT:
            return self._parse_ident_stmt()

        return StmtExpr(expr=self._parse_expr())

    # ------------------------------------------------------------------
    # Optional return type
    # ------------------------------------------------------------------

    def _parse_opt_return_type(self) -> Optional[str]:
        if self._current_kind() == TokenKind.ARROW:
            self._advance()
            return self._parse_type_expr()
        return None

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
                    raise ParseError("match statement cannot mix 'case' and 'is' arms")
                is_case_kind = True
                self._advance()
                pattern_expr = self._parse_expr()
                self._eat(TokenKind.COLON)
                arms.append(MatchArm(pattern=MatchPatternCase(expr=pattern_expr), body=self._parse_block()))
            elif k == TokenKind.IS:
                if is_case_kind is True:
                    raise ParseError("match statement cannot mix 'case' and 'is' arms")
                is_case_kind = False
                self._advance()
                type_name = self._expect_ident()
                self._eat(TokenKind.COLON)
                arms.append(MatchArm(pattern=MatchPatternIsType(type_name=type_name), body=self._parse_block()))
            else:
                raise ParseError(f"expected 'case' or 'is' in match body, got `{self._current().kind.name}`")
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
    # Identifier-started statement
    # ------------------------------------------------------------------

    def _parse_ident_stmt(self) -> Stmt:
        # async assign: `target <- async [->Type]: body`
        if self._peek1_kind() == TokenKind.LT:
            # Check if it's actually `<-` by looking at position+2
            # The lexer emits LT for `<` and MINUS for `-` separately (no LeftArrow token)
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
                raise ParseError(
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
        return StmtExpr(expr=expr)

    def _parse_compound(self, op: BinOp) -> Stmt:
        span = self._current_span()
        name = self._expect_ident()
        if name in self._known_new_types:
            raise ParseError(
                f"ParseError: cannot reassign new_type `{name}` — new_type bindings are const"
            )
        self._advance()  # consume the compound-assign operator
        return StmtCompoundAssign(name=name, op=op, value=self._parse_expr(), span=span)

    # ------------------------------------------------------------------
    # Function definition
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
                raise ParseError(
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
            raise ParseError(
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
            raise ParseError("try statement requires at least one `except` or `finally` clause")
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

    # ------------------------------------------------------------------
    # import
    # ------------------------------------------------------------------

    def _parse_import_stmt(self) -> Stmt:
        self._eat(TokenKind.IMPORT)
        lang = self._parse_lang_bracket() if self._current_kind() == TokenKind.LBRACKET else "tl-auto"
        module = self._parse_module_path()
        alias: Optional[str] = None
        if self._current_kind() == TokenKind.AS:
            self._advance()
            alias = self._expect_ident()
        body = self._load_module(lang, module)
        return StmtImport(lang=lang, module=module, alias=alias, body=body)

    def _parse_from_import_stmt(self) -> Stmt:
        self._eat(TokenKind.FROM)
        module = self._parse_module_path()
        self._eat(TokenKind.IMPORT)
        lang = self._parse_lang_bracket() if self._current_kind() == TokenKind.LBRACKET else "tl-auto"
        names: list[tuple[str, Optional[str]]] = []
        while True:
            iname = self._expect_ident()
            ialias: Optional[str] = None
            if self._current_kind() == TokenKind.AS:
                self._advance()
                ialias = self._expect_ident()
            names.append((iname, ialias))
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
                if self._current_kind() in (
                    TokenKind.NEWLINE, TokenKind.EOF,
                    TokenKind.SEMICOLON, TokenKind.DEDENT
                ):
                    break
            else:
                break
        body = self._load_module(lang, module)
        return StmtFromImport(lang=lang, module=module, names=names, body=body)

    def _parse_lang_bracket(self) -> str:
        self._eat(TokenKind.LBRACKET)
        if self._current_kind() != TokenKind.IDENT:
            raise ParseError(f"expected language identifier, got `{self._current().kind.name}`")
        lang = self._current().value
        assert isinstance(lang, str)
        self._advance()
        while self._current_kind() == TokenKind.MINUS:
            self._advance()
            if self._current_kind() != TokenKind.IDENT:
                raise ParseError(f"expected identifier after '-' in lang tag, got `{self._current().kind.name}`")
            lang = lang + "-" + self._current().value
            self._advance()
        self._eat(TokenKind.RBRACKET)
        return lang

    def _parse_module_path(self) -> list[str]:
        segments = [self._expect_ident()]
        while self._current_kind() == TokenKind.DOT:
            self._advance()
            segments.append(self._expect_ident())
        return segments

    def _load_module(self, lang: str, module: list[str]) -> list[Stmt]:
        if lang in ("tl-auto", "tl"):
            return self._load_tl_module(module, force_source=(lang == "tl"))
        if lang == "tlc":
            return self._load_tlc_module(module)
        if lang in ("py", "py-int"):
            return []  # Python modules not converted in Python impl
        raise ParseError(f"unknown import language '{lang}'")

    def _load_tl_module(self, module: list[str], force_source: bool = False) -> list[Stmt]:
        module_base = Path(*module) if len(module) > 1 else Path(module[0])
        candidates: list[tuple[Path, bool]] = []  # (path, is_tlc)
        search_dirs = [self._source_dir]
        if self._root_dir != self._source_dir:
            search_dirs.append(self._root_dir)
        for d in search_dirs:
            if not force_source:
                candidates.append((d / module_base.with_suffix(".tlc"), True))
            candidates.append((d / module_base.with_suffix(".tl"), False))
            candidates.append((d / module_base / "__init__.tl", False))

        found: Optional[tuple[Path, bool]] = None
        for path, is_tlc in candidates:
            if path.exists():
                found = (path, is_tlc)
                break

        if found is None:
            checked = ", ".join(f"'{p}'" for p, _ in candidates)
            raise ParseError(f"cannot find module '{'.'.join(module)}' (looked at {checked})")

        abs_path, is_tlc = found
        cache_key = ("tl-auto", abs_path)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if abs_path in self._loading:
            raise ParseError(f"circular import detected: '{abs_path}'")

        if is_tlc:
            source = _extract_tlc_source(abs_path)
            filename = f"<compiled:{module_base.stem}>"
        else:
            source = abs_path.read_text(encoding="utf-8")
            filename = str(abs_path)

        self._loading.add(abs_path)
        tokens = lex(source, filename)
        module_dir = abs_path.parent
        sub = Parser(tokens, module_dir)
        sub._module_cache = self._module_cache.copy()
        sub._loading = self._loading.copy()
        sub._root_dir = self._root_dir
        body = sub.parse_program()
        self._module_cache.update(sub._module_cache)
        self._loading.discard(abs_path)
        self._module_cache[cache_key] = body
        return body

    def _load_tlc_module(self, module: list[str]) -> list[Stmt]:
        module_base = Path(*module) if len(module) > 1 else Path(module[0])
        search_dirs = [self._source_dir]
        if self._root_dir != self._source_dir:
            search_dirs.append(self._root_dir)
        candidates = [d / module_base.with_suffix(".tlc") for d in search_dirs]
        found: Optional[Path] = next((p for p in candidates if p.exists()), None)
        if found is None:
            checked = ", ".join(f"'{p}'" for p in candidates)
            raise ParseError(
                f"cannot find compiled module '{'.'.join(module)}' (looked at {checked}; "
                "compile with: cargo run --release -- --compile <source.tl>)"
            )
        cache_key = ("tlc", found)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if found in self._loading:
            raise ParseError(f"circular import detected: '{found}'")
        source = _extract_tlc_source(found)
        filename = f"<compiled:{module_base.stem}>"
        self._loading.add(found)
        tokens = lex(source, filename)
        sub = Parser(tokens, found.parent)
        sub._module_cache = self._module_cache.copy()
        sub._loading = self._loading.copy()
        sub._root_dir = self._root_dir
        body = sub.parse_program()
        self._module_cache.update(sub._module_cache)
        self._loading.discard(found)
        self._module_cache[cache_key] = body
        return body

    # ------------------------------------------------------------------
    # trait / class
    # ------------------------------------------------------------------

    def _parse_trait_def(self) -> Stmt:
        self._eat(TokenKind.TRAIT)
        name = self._expect_ident()
        template_params = self._parse_template_params()
        if self._current_kind() == TokenKind.LPAREN:
            raise ParseError(f"StaticTypeError: trait `{name}` cannot inherit from another type")
        self._eat(TokenKind.COLON)
        self._class_or_trait_depth += 1
        body = self._parse_class_body()
        self._class_or_trait_depth -= 1

        for stmt in body:
            if isinstance(stmt, StmtFnDef):
                mname = stmt.name
                if not stmt.is_abstract:
                    if stmt.return_type is None:
                        raise ParseError(
                            f"StaticTypeError: trait method `{mname}` is missing a return type annotation"
                        )
                    for p in stmt.params:
                        if p.name != "self" and p.type_ann is None:
                            raise ParseError(
                                f"StaticTypeError: parameter `{p.name}` of trait method `{mname}` is missing a type annotation"
                            )
                else:
                    if stmt.return_type is None:
                        raise ParseError(
                            f"StaticTypeError: virtual method `{mname}` is missing a return type annotation"
                        )
                    for p in stmt.params:
                        if p.name != "self" and p.type_ann is None:
                            raise ParseError(
                                f"StaticTypeError: parameter `{p.name}` of virtual method `{mname}` is missing a type annotation"
                            )

        fields = [
            (s.name, s.kind, s.type_ann, s.default is not None)
            for s in body if isinstance(s, StmtField)
        ]
        virtual_methods = [
            s.name for s in body if isinstance(s, StmtFnDef) and s.is_abstract
        ]
        self._known_traits[name] = (template_params, fields, virtual_methods)
        return StmtTraitDef(name=name, template_params=template_params, body=body)

    def _parse_class_def(self) -> Stmt:
        return self._parse_class_def_decorated([])

    def _parse_class_def_decorated(self, decorators: list[Expr]) -> Stmt:
        self._eat(TokenKind.CLASS)
        name = self._expect_ident()
        template_params = self._parse_template_params()
        bases_with_args: list[tuple[str, list[str]]] = []
        bases: list[str] = []
        if self._current_kind() == TokenKind.LPAREN:
            self._advance()
            while self._current_kind() not in (TokenKind.RPAREN, TokenKind.EOF):
                base_name = self._expect_ident()
                type_args = self._parse_type_args()
                bases_with_args.append((base_name, type_args))
                bases.append(base_name)
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
                else:
                    break
            self._eat(TokenKind.RPAREN)
        for base in bases:
            if base not in self._known_traits:
                raise ParseError(
                    f"ParseError: class `{name}` cannot inherit from `{base}` (only traits are allowed as bases)"
                )
        self._eat(TokenKind.COLON)
        self._class_or_trait_depth += 1
        body = self._parse_class_body()
        self._class_or_trait_depth -= 1

        trait_required = self._collect_trait_fields_and_check_virtuals(name, bases_with_args, body)
        class_required: list[tuple[str, str]] = [
            (s.name, s.type_ann)
            for s in body
            if isinstance(s, StmtField)
            and s.kind in (FieldKind.MUT, FieldKind.LET)
            and s.default is None
        ]
        self._generate_auto_init_if_needed(trait_required, class_required, body)
        return StmtClassDef(
            name=name,
            template_params=template_params,
            bases=bases,
            decorators=decorators,
            body=body,
        )

    def _collect_trait_fields_and_check_virtuals(
        self,
        class_name: str,
        bases_with_args: list[tuple[str, list[str]]],
        body: list[Stmt],
    ) -> list[tuple[str, str, str]]:
        trait_required: list[tuple[str, str, str]] = []
        for base, concrete_args in bases_with_args:
            if base not in self._known_traits:
                continue
            trait_tparams, trait_fields, virtual_methods = self._known_traits[base]
            type_map: dict[str, str] = {
                tp.name: arg
                for tp, arg in zip(trait_tparams, concrete_args)
            }
            for virt in virtual_methods:
                overridden = any(
                    isinstance(s, StmtFnDef) and s.name == virt and not s.is_abstract
                    for s in body
                )
                if not overridden:
                    raise ParseError(
                        f"StaticTypeError: class `{class_name}` must override virtual method "
                        f"`{virt}` from trait `{base}`"
                    )
            for fname, _kind, ftype, has_default in trait_fields:
                if not has_default:
                    resolved = type_map.get(ftype, ftype)
                    trait_required.append((base, fname, resolved))
        return trait_required

    def _generate_auto_init_if_needed(
        self,
        trait_required: list[tuple[str, str, str]],
        class_required: list[tuple[str, str]],
        body: list[Stmt],
    ) -> None:
        if not trait_required and not class_required:
            return
        all_required = (
            [(fname, ftype) for _, fname, ftype in trait_required]
            + list(class_required)
        )
        has_init = any(
            isinstance(s, StmtFnDef) and s.name == "__init__"
            for s in body
        )
        if has_init:
            return
        has_exact = any(
            isinstance(s, StmtFnDef)
            and s.name == "__init__"
            and _init_sig_matches(all_required, s.params)
            for s in body
        )
        if has_exact:
            return
        params: list[Param] = [Param(name="self", mutable=True)]
        for _, fname, ftype in trait_required:
            params.append(Param(name=fname, mutable=False, type_ann=ftype))
        for fname, ftype in class_required:
            params.append(Param(name=fname, mutable=False, type_ann=ftype))
        init_body: list[Stmt] = []
        for tname, fname, _ in trait_required:
            init_body.append(StmtAttrAssign(
                target=ExprTraitAccess(
                    object=ExprIdent(name="self"),
                    trait_name=tname,
                    attr=fname,
                ),
                value=ExprIdent(name=fname),
            ))
        for fname, _ in class_required:
            init_body.append(StmtAttrAssign(
                target=ExprAttr(object=ExprIdent(name="self"), attr=fname),
                value=ExprIdent(name=fname),
            ))
        body.append(StmtFnDef(
            name="__init__",
            template_params=[],
            params=params,
            return_type="None",
            body=init_body,
            is_abstract=False,
            is_static=False,
            is_class_method=False,
            decorators=[],
            access=Accessibility.PUBLIC,
        ))

    def _parse_class_body(self) -> list[Stmt]:
        self._eat(TokenKind.NEWLINE)
        self._eat(TokenKind.INDENT)
        stmts: list[Stmt] = []
        current_access = Accessibility.PUBLIC
        while True:
            while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                self._advance()
            if self._current_kind() in (TokenKind.DEDENT, TokenKind.EOF):
                break
            k = self._current_kind()
            if k == TokenKind.PUBLIC:
                current_access = Accessibility.PUBLIC
                self._advance(); self._eat(TokenKind.COLON)
                while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                    self._advance()
                continue
            if k == TokenKind.PRIVATE:
                current_access = Accessibility.PRIVATE
                self._advance(); self._eat(TokenKind.COLON)
                while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                    self._advance()
                continue
            if k == TokenKind.PROTECTED:
                current_access = Accessibility.PROTECTED
                self._advance(); self._eat(TokenKind.COLON)
                while self._current_kind() in (TokenKind.NEWLINE, TokenKind.SEMICOLON):
                    self._advance()
                continue
            stmt = self._parse_class_stmt()
            if isinstance(stmt, (StmtField, StmtFnDef, StmtGenDef)):
                stmt.access = current_access  # type: ignore[attr-defined]
            stmts.append(stmt)
        if self._current_kind() == TokenKind.DEDENT:
            self._advance()
        return stmts

    def _parse_class_stmt(self) -> Stmt:
        k = self._current_kind()
        if k in (TokenKind.MUT, TokenKind.LET, TokenKind.CONST):
            kind = {
                TokenKind.MUT: FieldKind.MUT,
                TokenKind.LET: FieldKind.LET,
                TokenKind.CONST: FieldKind.CONST,
            }[k]
            kw = {FieldKind.MUT: "mut", FieldKind.LET: "let", FieldKind.CONST: "const"}[kind]
            self._advance()
            fname = self._expect_ident()
            if self._current_kind() != TokenKind.COLON:
                raise ParseError(
                    f"class field `{fname}` must have a type annotation (e.g., `{kw} {fname}: int = 0`)"
                )
            self._advance()
            type_ann = self._parse_type_expr()
            default: Optional[Expr] = None
            if self._current_kind() == TokenKind.EQ:
                self._advance()
                default = self._parse_expr()
            if kind == FieldKind.CONST and default is None:
                raise ParseError(
                    f"class variable `{fname}` declared with `const` must have an initial value"
                )
            return StmtField(name=fname, kind=kind, type_ann=type_ann, default=default, access=Accessibility.PUBLIC)

        if k == TokenKind.FN:
            return self._parse_fn_def()
        if k == TokenKind.GEN:
            return self._parse_gen_def()
        if k == TokenKind.STATIC:
            self._advance()
            k2 = self._current_kind()
            if k2 == TokenKind.FN:
                return self._parse_fn_def_with_flags([], True, False)
            if k2 == TokenKind.MUT:
                self._advance()
                fname = self._expect_ident()
                if self._current_kind() != TokenKind.COLON:
                    raise ParseError(
                        f"class static field `{fname}` must have a type annotation"
                    )
                self._advance()
                type_ann = self._parse_type_expr()
                default = None
                if self._current_kind() == TokenKind.EQ:
                    self._advance()
                    default = self._parse_expr()
                return StmtField(name=fname, kind=FieldKind.STATIC_MUT, type_ann=type_ann, default=default, access=Accessibility.PUBLIC)
            raise ParseError(f"expected `fn` or `mut` after `static` in class body, got `{self._current().kind.name}`")
        if k == TokenKind.CLASS_METHOD:
            self._advance()
            if self._current_kind() != TokenKind.FN:
                raise ParseError(f"expected `fn` after `class_method`, got `{self._current().kind.name}`")
            return self._parse_fn_def_with_flags([], False, True)
        if k == TokenKind.PASS:
            self._advance(); return StmtPass()
        raise ParseError(f"unexpected statement in class body: `{self._current().kind.name}`")

    # ------------------------------------------------------------------
    # Template / type helpers
    # ------------------------------------------------------------------

    def _parse_type_args(self) -> list[str]:
        if self._current_kind() != TokenKind.LBRACKET:
            return []
        self._advance()
        args: list[str] = []
        while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
            args.append(self._parse_type_expr())
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RBRACKET)
        return args

    def _parse_template_params(self) -> list[TemplateParam]:
        if self._current_kind() != TokenKind.LBRACKET:
            return []
        self._advance()
        params: list[TemplateParam] = []
        while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
            name = self._expect_ident()
            self._eat(TokenKind.COLON)
            constraints = [self._expect_constraint_name()]
            while self._current_kind() == TokenKind.AND:
                self._advance()
                constraints.append(self._expect_constraint_name())
            params.append(TemplateParam(name=name, constraints=constraints))
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            else:
                break
        self._eat(TokenKind.RBRACKET)
        return params

    def _expect_constraint_name(self) -> str:
        """Like _expect_ident but also accepts 'Any' which may be tokenized as TokenKind.ANY."""
        k = self._current_kind()
        if k == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        if k == TokenKind.ANY:
            self._advance()
            return "Any"
        raise ParseError(f"expected trait/constraint name, got `{self._current().kind.name}`")

    def _parse_type_expr(self) -> str:
        k = self._current_kind()
        if k == TokenKind.UNION:
            self._advance()
            if self._current_kind() != TokenKind.LBRACKET:
                raise ParseError("Union requires type arguments: Union[Type1, Type2, ...]")
            self._advance()
            args: list[str] = []
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                args.append(self._parse_type_expr())
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACKET)
            if len(args) < 2:
                raise ParseError(f"Union requires at least 2 type arguments, got {len(args)}")
            return f"Union[{','.join(args)}]"

        if k == TokenKind.OPTION:
            self._advance()
            if self._current_kind() != TokenKind.LBRACKET:
                raise ParseError("Option requires a type argument: Option[Type]")
            self._advance()
            inner = self._parse_type_expr()
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            self._eat(TokenKind.RBRACKET)
            return f"Option[{inner}]"

        if k == TokenKind.IDENT:
            base = self._current().value
            assert isinstance(base, str)
            self._advance()
        elif k == TokenKind.NONE:
            self._advance(); base = "None"
        elif k == TokenKind.ANY:
            self._advance(); base = "Any"
        elif k == TokenKind.SELF_TYPE:
            if self._class_or_trait_depth == 0:
                raise ParseError("ParseError: 'Self' can only be used inside class or trait definitions")
            self._advance(); base = "Self"
        else:
            raise ParseError(f"expected type name, got `{self._current().kind.name}`")

        if base == "type" and self._current_kind() == TokenKind.LBRACKET:
            self._advance()
            inner = self._parse_type_expr()
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            self._eat(TokenKind.RBRACKET)
            return f"type[{inner}]"

        if base == "tuple" and self._current_kind() == TokenKind.LBRACKET:
            self._advance()
            parts: list[str] = []
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                parts.append(self._parse_type_expr())
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACKET)
            return f"tuple[{','.join(parts)}]"

        if base == "function":
            return self._parse_function_type_ann()

        if self._current_kind() == TokenKind.LBRACKET:
            self._advance()
            depth = 1
            while depth > 0 and self._current_kind() != TokenKind.EOF:
                if self._current_kind() == TokenKind.LBRACKET:
                    depth += 1
                elif self._current_kind() == TokenKind.RBRACKET:
                    depth -= 1
                self._advance()

        return base

    def _parse_function_type_ann(self) -> str:
        k = self._current_kind()
        params_str: Optional[str] = None

        if k == TokenKind.LBRACKET:
            self._advance()
            parts: list[str] = []
            auto_idx = 1
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                mut = False
                if self._current_kind() == TokenKind.MUT:
                    self._advance(); mut = True
                elif self._current_kind() == TokenKind.LET:
                    self._advance()
                ty = self._parse_type_expr()
                prefix = "mut" if mut else "let"
                parts.append(f"{prefix} param{auto_idx}:{ty}")
                auto_idx += 1
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACKET)
            params_str = f"[{','.join(parts)}]"

        elif k == TokenKind.LBRACE:
            self._advance()
            parts = []
            while self._current_kind() not in (TokenKind.RBRACE, TokenKind.EOF):
                mut = False
                if self._current_kind() == TokenKind.MUT:
                    self._advance(); mut = True
                elif self._current_kind() == TokenKind.LET:
                    self._advance()
                pname = self._expect_ident()
                self._eat(TokenKind.COLON)
                ty = self._parse_type_expr()
                prefix = "mut" if mut else "let"
                parts.append(f"{prefix} {pname}:{ty}")
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACE)
            params_str = "{" + ",".join(parts) + "}"

        ret_str = ""
        if self._current_kind() == TokenKind.ARROW:
            self._advance()
            ret = self._parse_type_expr()
            ret_str = f"->{ret}"

        if params_str is not None:
            return f"function{params_str}{ret_str}"
        if ret_str:
            return f"function{ret_str}"
        return "function"

    # ------------------------------------------------------------------
    # Parameter
    # ------------------------------------------------------------------

    def _parse_param(self) -> Param:
        mutable = False
        if self._current_kind() == TokenKind.MUT:
            self._advance(); mutable = True
        elif self._current_kind() == TokenKind.LET:
            self._advance()
        name = self._expect_ident()
        type_ann: Optional[str] = None
        if self._current_kind() == TokenKind.COLON:
            self._advance()
            type_ann = self._parse_type_expr()
        default: Optional[Expr] = None
        if self._current_kind() == TokenKind.EQ:
            self._advance()
            default = self._parse_expr()
        return Param(name=name, mutable=mutable, type_ann=type_ann, default=default)

    # ------------------------------------------------------------------
    # expect_ident helpers
    # ------------------------------------------------------------------

    def _expect_ident(self) -> str:
        if self._current_kind() == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        raise ParseError(f"expected identifier, got `{self._current().kind.name}`")

    def _expect_guard_type_name(self) -> str:
        k = self._current_kind()
        if k == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        if k == TokenKind.NONE:
            self._advance(); return "None"
        raise ParseError(f"expected type name after `is`, got `{self._current().kind.name}`")

    # ------------------------------------------------------------------
    # Expression parsing (precedence climbing)
    # ------------------------------------------------------------------

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
            TokenKind.AT: BinOp.MUL,  # matrix mul — treat as MUL for now
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
                    if (
                        self._current_kind() == TokenKind.IDENT
                        and self._peek1_kind() == TokenKind.EQ
                    ):
                        name = self._current().value
                        assert isinstance(name, str)
                        self._advance()  # Ident
                        self._advance()  # `=`
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

            else:
                break
        return expr

    def _parse_bracket_suffix(self, expr: Expr) -> Expr:
        if self._is_template_instantiation():
            self._advance()  # `[`
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

        # Slice detection
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
        i = self._pos + 1  # skip opening `[`
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
        if k == TokenKind.STR:
            v = tok.value; self._advance(); return ExprStr(value=v)  # type: ignore[arg-type]
        if k == TokenKind.TRUE:
            self._advance(); return ExprBool(value=True)
        if k == TokenKind.FALSE:
            self._advance(); return ExprBool(value=False)
        if k == TokenKind.NONE:
            self._advance(); return ExprNone()
        if k == TokenKind.ANY:
            self._advance(); return ExprIdent(name="Any")
        if k == TokenKind.UNION:
            self._advance(); return ExprIdent(name="Union")
        if k == TokenKind.OPTION:
            self._advance(); return ExprIdent(name="Option")
        if k == TokenKind.IDENT:
            name = tok.value; self._advance(); return ExprIdent(name=name)  # type: ignore[arg-type]
        if k == TokenKind.SELF_TYPE:
            if self._class_or_trait_depth == 0:
                raise ParseError("ParseError: 'Self' can only be used inside class or trait definitions")
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

        raise ParseError(f"unexpected token: `{tok.kind.name}`")

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
            items: list[Expr] = [first]
            while self._current_kind() == TokenKind.COMMA:
                self._advance()
                if self._current_kind() == TokenKind.RBRACE:
                    break
                items.append(self._parse_expr())
            self._eat(TokenKind.RBRACE)
            return ExprSet(elements=items)


# ---------------------------------------------------------------------------
# Module-level helpers
# ---------------------------------------------------------------------------

def _validate_param_defaults(params: list[Param]) -> None:
    seen_default = False
    for p in params:
        if p.name == "self":
            continue
        if p.default is not None:
            seen_default = True
        elif seen_default:
            raise ParseError(
                f"ParseError: non-default parameter '{p.name}' follows a parameter with a default value"
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


def _init_sig_matches(required_fields: list[tuple[str, str]], params: list[Param]) -> bool:
    non_self = [p for p in params if p.name != "self"]
    if len(non_self) != len(required_fields):
        return False
    return all(p.type_ann == ftype for p, (_, ftype) in zip(non_self, required_fields))


def _extract_tlc_source(path: Path) -> str:
    """Extract embedded source from a .tlc binary (v0/v1 format)."""
    data = path.read_bytes()
    # Magic: b"TLC\x00" or b"TLC\x01"
    if len(data) < 8 or data[:3] != b"TLC":
        raise ParseError(f"invalid .tlc file: '{path}'")
    version = data[3]
    # Source offset at bytes 4-7 (little-endian u32)
    import struct
    src_offset = struct.unpack_from("<I", data, 4)[0]
    if version == 0:
        return data[src_offset:].decode("utf-8")
    if version == 1:
        # v1: source length at src_offset (u32), then source bytes
        src_len = struct.unpack_from("<I", data, src_offset)[0]
        return data[src_offset + 4: src_offset + 4 + src_len].decode("utf-8")
    raise ParseError(f"unknown .tlc version {version} in '{path}'")


# ---------------------------------------------------------------------------
# Convenience function
# ---------------------------------------------------------------------------

def parse(source: str, filename: str = "", source_dir: Optional[Path] = None) -> list[Stmt]:
    tokens = lex(source, filename)
    sd = source_dir or (Path(filename).parent if filename else Path("."))
    return Parser(tokens, sd).parse_program()
