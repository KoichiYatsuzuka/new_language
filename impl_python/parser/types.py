# git SHA: b614502cff33c6ad5e49427ca347db8ad90c31a5
"""Type annotation and parameter parsing (mirrors src/parser/types.rs)."""
from __future__ import annotations
from typing import Optional

from ..token import Token, TokenKind
from ..ast import TemplateParam, Param, Expr


class _ParserTypes:
    """Mixin providing type annotation, parameter, and ident parsing."""

    # ------------------------------------------------------------------
    # Type annotation
    # ------------------------------------------------------------------

    def _parse_type_expr(self) -> str:
        k = self._current_kind()
        if k == TokenKind.UNION:
            self._advance()
            if self._current_kind() != TokenKind.LBRACKET:
                raise self._error("Union requires type arguments: Union[Type1, Type2, ...]")
            self._advance()
            args: list[str] = []
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                args.append(self._parse_type_expr())
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACKET)
            if len(args) < 2:
                raise self._error(f"Union requires at least 2 type arguments, got {len(args)}")
            return f"Union[{','.join(args)}]"

        if k == TokenKind.INTERSECTION:
            self._advance()
            if self._current_kind() != TokenKind.LBRACKET:
                raise self._error("Intersection requires type arguments: Intersection[Type1, Type2, ...]")
            self._advance()
            args: list[str] = []
            while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
                args.append(self._parse_type_expr())
                if self._current_kind() == TokenKind.COMMA:
                    self._advance()
            self._eat(TokenKind.RBRACKET)
            if len(args) < 2:
                raise self._error(f"Intersection requires at least 2 type arguments, got {len(args)}")
            return f"Intersection[{','.join(args)}]"

        if k == TokenKind.OPTION:
            self._advance()
            if self._current_kind() != TokenKind.LBRACKET:
                raise self._error("Option requires a type argument: Option[Type]")
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
        elif k == TokenKind.UNDEFINED:
            self._advance(); base = "Undefined"
        elif k == TokenKind.ANY:
            self._advance(); base = "Any"
        elif k == TokenKind.SELF_TYPE:
            if self._class_or_trait_depth == 0:
                raise self._error("ParseError: 'Self' can only be used inside class or trait definitions")
            self._advance(); base = "Self"
        else:
            raise self._error(f"expected type name, got `{self._current().kind.name}`")

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

        if base == "Result" and self._current_kind() == TokenKind.LBRACKET:
            self._advance()
            ok_ty = self._parse_type_expr()
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            err_ty = self._parse_type_expr()
            if self._current_kind() == TokenKind.COMMA:
                self._advance()
            self._eat(TokenKind.RBRACKET)
            return f"Result[{ok_ty}, {err_ty}]"

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
    # Template parameters and type arguments
    # ------------------------------------------------------------------

    def _parse_template_params(self) -> list[TemplateParam]:
        if self._current_kind() != TokenKind.LBRACKET:
            return []
        self._advance()
        params: list[TemplateParam] = []
        while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
            # Accept full type expressions (e.g., `int`, `dict[str, int]`, `T`)
            # for `__cast__[TypeName]` style methods.
            name = self._parse_type_expr()
            # `: constraint` is optional
            constraints: list[str] = []
            if self._current_kind() == TokenKind.COLON:
                self._advance()
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

    # ------------------------------------------------------------------
    # Parameter
    # ------------------------------------------------------------------

    def _parse_param(self) -> Param:
        mutable = False
        if self._current_kind() == TokenKind.MUT:
            self._advance(); mutable = True
        elif self._current_kind() == TokenKind.LET:
            self._advance()
        # 可変長引数: let ... : type
        if self._current_kind() == TokenKind.ELLIPSIS:
            self._advance()
            if self._current_kind() != TokenKind.COLON:
                raise self._error("variadic parameter '...' requires a type annotation")
            self._advance()
            type_ann = self._parse_type_expr()
            return Param(name="...", mutable=mutable, type_ann=type_ann, default=None, variadic=True)
        name = self._expect_ident()
        type_ann: Optional[str] = None
        if self._current_kind() == TokenKind.COLON:
            self._advance()
            type_ann = self._parse_type_expr()
        default: Optional[Expr] = None
        if self._current_kind() == TokenKind.EQ:
            self._advance()
            default = self._parse_expr()
        return Param(name=name, mutable=mutable, type_ann=type_ann, default=default, variadic=False)

    # ------------------------------------------------------------------
    # Identifier helpers
    # ------------------------------------------------------------------

    def _expect_ident(self) -> str:
        if self._current_kind() == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        raise self._error(f"expected identifier, got `{self._current().kind.name}`")

    def _expect_guard_type_name(self) -> str:
        k = self._current_kind()
        if k == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        if k == TokenKind.NONE:
            self._advance(); return "None"
        if k == TokenKind.UNDEFINED:
            self._advance(); return "Undefined"
        raise self._error(f"expected type name after `is`, got `{self._current().kind.name}`")

    def _expect_constraint_name(self) -> str:
        k = self._current_kind()
        if k == TokenKind.IDENT:
            name = self._current().value
            assert isinstance(name, str)
            self._advance()
            return name
        if k == TokenKind.ANY:
            self._advance()
            return "Any"
        raise self._error(f"expected trait/constraint name, got `{self._current().kind.name}`")

    # ------------------------------------------------------------------
    # Optional return type
    # ------------------------------------------------------------------

    def _parse_opt_return_type(self) -> Optional[str]:
        if self._current_kind() == TokenKind.ARROW:
            self._advance()
            return self._parse_type_expr()
        return None
