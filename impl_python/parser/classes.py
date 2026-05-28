# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Class and trait definition parsing (mirrors src/parser/classes.rs)."""
from __future__ import annotations
from typing import Optional

from ..token import TokenKind
from ..ast import (
    Accessibility, FieldKind, Param,
    Expr, Stmt,
    StmtFnDef, StmtGenDef, StmtField, StmtClassDef, StmtTraitDef, StmtPass,
    StmtAttrAssign,
    ExprAttr, ExprIdent, ExprTraitAccess,
    TemplateParam,
)


def _init_sig_matches(required_fields: list[tuple[str, str]], params: list[Param]) -> bool:
    non_self = [p for p in params if p.name != "self"]
    if len(non_self) != len(required_fields):
        return False
    return all(p.type_ann == ftype for p, (_, ftype) in zip(non_self, required_fields))


class _ParserClasses:
    """Mixin providing class and trait definition parsing."""

    # ------------------------------------------------------------------
    # trait
    # ------------------------------------------------------------------

    def _parse_trait_def(self) -> Stmt:
        self._eat(TokenKind.TRAIT)
        name = self._expect_ident()
        template_params = self._parse_template_params()
        if self._current_kind() == TokenKind.LPAREN:
            raise self._error(f"StaticTypeError: trait `{name}` cannot inherit from another type")
        self._eat(TokenKind.COLON)
        self._class_or_trait_depth += 1
        body = self._parse_class_body()
        self._class_or_trait_depth -= 1

        for stmt in body:
            if isinstance(stmt, StmtFnDef):
                mname = stmt.name
                if not stmt.is_abstract:
                    if stmt.return_type is None:
                        raise self._error(
                            f"StaticTypeError: trait method `{mname}` is missing a return type annotation"
                        )
                    for p in stmt.params:
                        if p.name != "self" and p.type_ann is None:
                            raise self._error(
                                f"StaticTypeError: parameter `{p.name}` of trait method `{mname}` is missing a type annotation"
                            )
                else:
                    if stmt.return_type is None:
                        raise self._error(
                            f"StaticTypeError: virtual method `{mname}` is missing a return type annotation"
                        )
                    for p in stmt.params:
                        if p.name != "self" and p.type_ann is None:
                            raise self._error(
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

    # ------------------------------------------------------------------
    # class
    # ------------------------------------------------------------------

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
                raise self._error(
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
                    raise self._error(
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

    # ------------------------------------------------------------------
    # Class body
    # ------------------------------------------------------------------

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
                raise self._error(
                    f"class field `{fname}` must have a type annotation (e.g., `{kw} {fname}: int = 0`)"
                )
            self._advance()
            type_ann = self._parse_type_expr()
            default: Optional[Expr] = None
            if self._current_kind() == TokenKind.EQ:
                self._advance()
                default = self._parse_expr()
            if kind == FieldKind.CONST and default is None:
                raise self._error(
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
                    raise self._error(f"class static field `{fname}` must have a type annotation")
                self._advance()
                type_ann = self._parse_type_expr()
                default = None
                if self._current_kind() == TokenKind.EQ:
                    self._advance()
                    default = self._parse_expr()
                return StmtField(name=fname, kind=FieldKind.STATIC_MUT, type_ann=type_ann, default=default, access=Accessibility.PUBLIC)
            raise self._error(f"expected `fn` or `mut` after `static` in class body, got `{self._current().kind.name}`")
        if k == TokenKind.CLASS_METHOD:
            self._advance()
            if self._current_kind() != TokenKind.FN:
                raise self._error(f"expected `fn` after `class_method`, got `{self._current().kind.name}`")
            return self._parse_fn_def_with_flags([], False, True)
        if k == TokenKind.PASS:
            self._advance(); return StmtPass()
        raise self._error(f"unexpected statement in class body: `{self._current().kind.name}`")
