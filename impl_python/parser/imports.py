# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""Import statement parsing (mirrors src/parser/imports.rs)."""
from __future__ import annotations
import struct
from pathlib import Path
from typing import Optional

from ..token import TokenKind
from ..ast import Stmt, StmtImport, StmtFromImport
from ..lexer import lex


def _extract_hvc_source(path: Path) -> str:
    """Extract embedded source text from a .hvc binary (v0/v1 format)."""
    data = path.read_bytes()
    if len(data) < 8 or data[:3] != b"TLC":
        raise _make_parse_error(f"invalid .hvc file: '{path}'")
    version = data[3]
    src_offset = struct.unpack_from("<I", data, 4)[0]
    if version == 0:
        return data[src_offset:].decode("utf-8")
    if version == 1:
        src_len = struct.unpack_from("<I", data, src_offset)[0]
        return data[src_offset + 4: src_offset + 4 + src_len].decode("utf-8")
    raise _make_parse_error(f"unknown .hvc version {version} in '{path}'")


def _make_parse_error(msg: str) -> Exception:
    from . import ParseError
    return ParseError(msg)


class _ParserImports:
    """Mixin providing import statement parsing."""

    # ------------------------------------------------------------------
    # import / from ... import
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
            raise self._error(f"expected language identifier, got `{self._current().kind.name}`")
        lang = self._current().value
        assert isinstance(lang, str)
        self._advance()
        while self._current_kind() == TokenKind.MINUS:
            self._advance()
            if self._current_kind() != TokenKind.IDENT:
                raise self._error(f"expected identifier after '-' in lang tag, got `{self._current().kind.name}`")
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

    # ------------------------------------------------------------------
    # Module loading
    # ------------------------------------------------------------------

    def _load_module(self, lang: str, module: list[str]) -> list[Stmt]:
        if lang in ("tl-auto", "hv-auto", "tl", "hv"):
            return self._load_tl_module(module, force_source=(lang in ("tl", "hv")))
        if lang in ("tlc", "hvc"):
            return self._load_tlc_module(module)
        if lang in ("py", "py-int"):
            return []  # Python modules have no AST body in Python impl
        raise self._error(f"unknown import language '{lang}'")

    def _load_tl_module(self, module: list[str], force_source: bool = False) -> list[Stmt]:
        module_base = Path(*module) if len(module) > 1 else Path(module[0])
        candidates: list[tuple[Path, bool]] = []
        search_dirs = [self._source_dir]
        if self._root_dir != self._source_dir:
            search_dirs.append(self._root_dir)
        for d in search_dirs:
            if not force_source:
                candidates.append((d / module_base.with_suffix(".hvc"), True))
            candidates.append((d / module_base.with_suffix(".hv"), False))
            candidates.append((d / module_base / "__init__.hv", False))

        found: Optional[tuple[Path, bool]] = None
        for path, is_tlc in candidates:
            if path.exists():
                found = (path, is_tlc)
                break

        if found is None:
            checked = ", ".join(f"'{p}'" for p, _ in candidates)
            raise self._error(f"cannot find module '{'.'.join(module)}' (looked at {checked})")

        abs_path, is_tlc = found
        cache_key = ("tl-auto", abs_path)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if abs_path in self._loading:
            raise self._error(f"circular import detected: '{abs_path}'")

        if is_tlc:
            source = _extract_hvc_source(abs_path)
            filename = f"<compiled:{module_base.stem}>"
        else:
            source = abs_path.read_text(encoding="utf-8")
            filename = str(abs_path)

        self._loading.add(abs_path)
        tokens = lex(source, filename)
        from . import Parser
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
        candidates = [d / module_base.with_suffix(".hvc") for d in search_dirs]
        found: Optional[Path] = next((p for p in candidates if p.exists()), None)
        if found is None:
            checked = ", ".join(f"'{p}'" for p in candidates)
            raise self._error(
                f"cannot find compiled module '{'.'.join(module)}' (looked at {checked}; "
                "compile with: cargo run --release -- --compile <source.hv>)"
            )
        cache_key = ("tlc", found)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if found in self._loading:
            raise self._error(f"circular import detected: '{found}'")
        source = _extract_hvc_source(found)
        filename = f"<compiled:{module_base.stem}>"
        self._loading.add(found)
        tokens = lex(source, filename)
        from . import Parser
        sub = Parser(tokens, found.parent)
        sub._module_cache = self._module_cache.copy()
        sub._loading = self._loading.copy()
        sub._root_dir = self._root_dir
        body = sub.parse_program()
        self._module_cache.update(sub._module_cache)
        self._loading.discard(found)
        self._module_cache[cache_key] = body
        return body
