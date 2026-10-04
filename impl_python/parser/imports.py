# git SHA: 2dc0dd65bef79508c5acee495394233b12618f36
"""Import statement parsing (mirrors src/parser/imports.rs)."""
from __future__ import annotations
import os
import struct
from pathlib import Path
from typing import Optional

from ..token import TokenKind
from ..ast import Stmt, StmtImport, StmtFromImport
from ..lexer import lex


def _extract_arc_source(path: Path) -> str:
    """Extract embedded source text from a .arc binary (v0/v1 format)."""
    data = path.read_bytes()
    if len(data) < 8 or data[:3] != b"TLC":
        raise _make_parse_error(f"invalid .arc file: '{path}'")
    version = data[3]
    src_offset = struct.unpack_from("<I", data, 4)[0]
    if version == 0:
        return data[src_offset:].decode("utf-8")
    if version == 1:
        src_len = struct.unpack_from("<I", data, src_offset)[0]
        return data[src_offset + 4: src_offset + 4 + src_len].decode("utf-8")
    raise _make_parse_error(f"unknown .arc version {version} in '{path}'")


def _make_parse_error(msg: str) -> Exception:
    from . import ParseError
    return ParseError(msg)


_ARROW_LANGS = ("ar", "tl", "ar-auto", "tl-auto", "arc", "tlc")


def _search_base(source_dir: Path, level: int) -> Path:
    """相対 import の起点（mirrors `module_path::search_base`）。

    level 0・1（`.`）は source_dir そのもの、k（k >= 2）は k-1 個上。
    ドット無しの import の起点はエントリのディレクトリ（`_ParserImports._import_base`）。
    """
    base = source_dir
    for _ in range(1, level):
        base = base / ".."
    return Path(os.path.normpath(base)) if level >= 2 else base


def _written(level: int, module: list[str]) -> str:
    """ソースに書かれた綴り（`..lib.helper`・mirrors `module_path::written_spelling`）。"""
    return "." * level + ".".join(module)


def _identity_of_file(path: Path) -> Path:
    """モジュールファイルの同一性の鍵（mirrors `module_path::module_identity`）。"""
    p = Path(os.path.normpath(os.path.abspath(path)))
    return p.parent if p.stem == "__init__" else p.with_suffix("")


class _ParserImports:
    """Mixin providing import statement parsing."""

    def _import_base(self, level: int) -> Path:
        """探索の起点（mirrors `module_path::import_base`・CPython 準拠）。

        ドット無しはエントリのディレクトリ（CPython の sys.path[0]）、ドット付きは書いたファイルから数える。
        """
        return self._root_dir if level == 0 else _search_base(self._source_dir, level)

    def _canonical(self, identity: Path, written: list[str]) -> list[str]:
        """ファイルごとの名前（エントリのディレクトリからの相対・mirrors `root_relative_name`）。"""
        try:
            rel = os.path.relpath(identity, os.path.abspath(self._root_dir))
        except ValueError:
            return list(written)
        parts = ["__parent__" if p == ".." else p for p in Path(rel).parts]
        if not parts or parts == ["."] or not all(p.isidentifier() for p in parts):
            return list(written)
        return parts

    def _bind_for(self, lang: str, module: list[str], alias: Optional[str],
                  canonical: list[str], chain: list) -> tuple[str, list[str]]:
        """この文の束縛（mirrors `chain_and_bind`）。`import a.b` は `a`、`as m` は `m`。"""
        if alias:
            return alias, canonical
        if lang in _ARROW_LANGS and len(module) > 1 and chain:
            return module[0], chain[0].module
        if lang in ("py", "py-int") and len(module) > 1:
            return module[0], module[:1]
        return module[-1], canonical

    # ------------------------------------------------------------------
    # import / from ... import
    # ------------------------------------------------------------------

    def _parse_import_stmt(self) -> Stmt:
        self._eat(TokenKind.IMPORT)
        lang = self._parse_lang_bracket() if self._current_kind() == TokenKind.LBRACKET else "ar-auto"

        if lang in ("cpp-dll", "cpp-lib"):
            return self._parse_cpp_import(lang)

        level, module = self._parse_module_ref()
        # import[rs] crate_name[0.2] — optional version bracket
        version: Optional[str] = None
        if lang == "rs":
            version = self._parse_version_bracket()
        alias: Optional[str] = None
        if self._current_kind() == TokenKind.AS:
            self._advance()
            alias = self._expect_ident()
        body = self._load_module(lang, module, version=version, level=level)
        canonical, root = self._last_loaded if lang in _ARROW_LANGS else (module, None)
        chain = self._package_chain(lang, level, module, root)
        self._pending.extend(chain)
        bind_name, bind_module = self._bind_for(lang, module, alias, canonical, chain)
        return StmtImport(lang=lang, module=canonical, alias=alias, body=body,
                          level=level, base_dir=str(self._import_base(level)),
                          bind_name=bind_name, bind_module=bind_module)

    def _parse_from_import_stmt(self) -> Stmt:
        self._eat(TokenKind.FROM)
        level, module = self._parse_module_ref(allow_bare_dots=True)
        self._eat(TokenKind.IMPORT)
        lang = self._parse_lang_bracket() if self._current_kind() == TokenKind.LBRACKET else "ar-auto"
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
        if not module:
            return self._from_dots_import(lang, level, names)
        body = self._load_module(lang, module, version=None, level=level)
        canonical, root = self._last_loaded if lang in _ARROW_LANGS else (module, None)
        self._pending.extend(self._package_chain(lang, level, module, root))
        if lang in _ARROW_LANGS:
            self._pending.extend(self._from_submodules(lang, level, module, canonical, body, names))
        return StmtFromImport(lang=lang, module=canonical, names=names, body=body,
                              level=level, base_dir=str(self._import_base(level)))

    # ------------------------------------------------------------------
    # CPython と同じパッケージの扱い（mirrors src/parser/imports/packages.rs）
    # ------------------------------------------------------------------

    def _package_chain(self, lang: str, level: int, module: list[str], root) -> list:
        """`import a.b.c` の連鎖（`a`・`a.b`）を先に読み込む、束縛しない StmtImport。"""
        out: list = []
        if lang not in _ARROW_LANGS or root is None:
            return out
        for i in range(1, len(module)):
            prefix = module[:i]
            d = Path(root).joinpath(*prefix)
            init = d / "__init__.ar"
            if Path(os.path.normpath(os.path.abspath(init))) in self._loading:
                continue  # 読み込み中のパッケージ（その __init__ の中からの import）
            if init.exists():
                body = self._load_tl_file(init, False, prefix)
                canonical = self._canonical(_identity_of_file(init), prefix)
            else:
                body = []
                canonical = self._canonical(Path(os.path.normpath(os.path.abspath(d))), prefix)
            out.append(StmtImport(lang=lang, module=canonical, alias=None, body=body,
                                  level=level, base_dir=str(self._import_base(level)), no_bind=True))
        return out

    def _ar_module_exists(self, lang: str, level: int, module: list[str]) -> bool:
        base = self._import_base(level)
        mb = Path(*module)
        return any(p.exists() for p in (
            base / mb.with_suffix(".ar"), base / mb.with_suffix(".arc"), base / mb / "__init__.ar"
        )) or (base / mb).is_dir()

    def _from_submodules(self, lang: str, level: int, module: list[str], canonical: list[str],
                         body: list, names: list) -> list:
        """`from a.b import x` の x がサブモジュールなら先に読み込む（mirrors `from_import_submodules`）。"""
        defined = {getattr(st, "name", None) for st in body}
        for st in body:
            if isinstance(st, StmtImport):
                defined.add(st.bind_name or st.alias or st.module[-1])
            elif isinstance(st, StmtFromImport):
                for orig, al in st.names:
                    defined.add(al or orig)
        out: list = []
        for orig, _ in names:
            if orig in defined:
                continue
            sub = list(module) + [orig]
            if not self._ar_module_exists(lang, level, sub):
                continue
            if not out:
                out.append(StmtImport(lang=lang, module=canonical, alias=None, body=body,
                                      level=level, base_dir=str(self._import_base(level)), no_bind=True))
            sbody = self._load_module(lang, sub, version=None, level=level)
            scan, _ = self._last_loaded
            out.append(StmtImport(lang=lang, module=scan, alias=None, body=sbody,
                                  level=level, base_dir=str(self._import_base(level)), no_bind=True))
        return out

    def _from_dots_import(self, lang: str, level: int, names: list) -> Stmt:
        """`from . import x`（mirrors `from_dots_import`）。x はサブモジュールとして読み込んで束縛する。"""
        stmts: list = []
        rest: list = []
        for orig, alias in names:
            if lang in _ARROW_LANGS and self._ar_module_exists(lang, level, [orig]):
                body = self._load_module(lang, [orig], version=None, level=level)
                canonical, _ = self._last_loaded
                stmts.append(StmtImport(lang=lang, module=canonical, alias=alias, body=body,
                                        level=level, base_dir=str(self._import_base(level)),
                                        bind_name=alias or orig, bind_module=canonical))
            else:
                rest.append((orig, alias))
        if rest:
            base = self._import_base(level)
            init = base / "__init__.ar"
            if not init.exists():
                raise self._error(
                    f"cannot import name '{rest[0][0]}' from '{'.' * level}' (no module '{rest[0][0]}' in '{base}', "
                    "and it is not a package with an __init__)"
                )
            body = self._load_tl_file(init, False, [])
            canonical = self._canonical(_identity_of_file(init), [])
            stmts.append(StmtFromImport(lang=lang, module=canonical, names=rest, body=body,
                                        level=level, base_dir=str(base)))
        last = stmts.pop()
        self._pending.extend(stmts)
        return last

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

    def _parse_module_ref(self, allow_bare_dots: bool = False) -> tuple[int, list[str]]:
        """`[.]* IDENT ('.' IDENT)*` → (先頭のドットの数, 各部分)。

        mirrors `Parser::parse_module_ref`（src/parser/import_syntax.rs）。
        ⚠ 字句解析器は `...` を ELLIPSIS の 1 トークンにするので 3 と数える。
        """
        level = 0
        while self._current_kind() in (TokenKind.DOT, TokenKind.ELLIPSIS):
            level += 3 if self._current_kind() == TokenKind.ELLIPSIS else 1
            self._advance()
        if level > 0 and allow_bare_dots and self._current_kind() == TokenKind.IMPORT:
            return level, []
        if level > 0 and self._current_kind() != TokenKind.IDENT:
            raise self._error(
                f"expected a module name after `{'.' * level}`, got `{self._current().kind.name}` "
                "(to import a module from this directory, write `import .name`)"
            )
        segments = [self._expect_ident()]
        while self._current_kind() == TokenKind.DOT:
            self._advance()
            segments.append(self._expect_ident())
        return level, segments

    # ------------------------------------------------------------------
    # Module loading
    # ------------------------------------------------------------------

    def _parse_cpp_import(self, lang: str) -> Stmt:
        """Parse import[cpp-dll] Dir.Name or import[cpp-lib] Dir.Name.

        The dotted path is resolved to a header file:
          Dir.Name → {source_dir}/Dir/Name.h
        The full resolved header path is stored as module[0].
        The body contains StmtFnDef stubs generated from the header (for type checking).
        """
        from ..ast import StmtFnDef, StmtField, StmtClassDef
        from ..ast import Param as AstParam, FieldKind, Accessibility

        # Parse dotted identifier: [.]* Dir.Name（相対 import・mirrors parse_cpp_import）
        level, parts = self._parse_module_ref()

        # Resolve to header path: last part gets .h extension
        resolved = self._import_base(level)
        for i, part in enumerate(parts):
            if i == len(parts) - 1:
                resolved = resolved / f"{part}.h"
            else:
                resolved = resolved / part

        file_path = str(resolved)

        # Optional alias
        alias: Optional[str] = None
        if self._current_kind() == TokenKind.AS:
            self._advance()
            alias = self._expect_ident()

        # Generate type stubs from the header if it exists
        body: list[Stmt] = []
        if resolved.exists():
            try:
                from ..interpreter.cpp_bridge import (
                    parse_header_full, load_cpp_config, ctype_to_tl_str,
                )
                header_dir = resolved.parent
                config = load_cpp_config(header_dir)
                content = resolved.read_text(encoding="utf-8", errors="replace")
                sigs, struct_defs = parse_header_full(content, config.custom_type_map, {})

                for sdef in struct_defs:
                    field_stmts = [
                        StmtField(
                            name=fname,
                            kind=FieldKind.MUT,
                            type_ann=ctype_to_tl_str(fct),
                            default=None,
                            access=Accessibility.PUBLIC,
                        )
                        for fname, fct in sdef.fields
                    ]
                    body.append(StmtClassDef(
                        name=sdef.name,
                        template_params=[],
                        bases=[],
                        decorators=[],
                        body=field_stmts,
                    ))

                from ..interpreter.cpp_bridge.types import CPtr, COpaqueStructPtr
                for sig in sigs:
                    # Non-const pointers (T* / VECTOR*) become Arrow `mut`
                    # parameters so the type checker's
                    # CallMutParamWithImmutableArg check statically rejects
                    # passing a `let` variable to a write pointer.
                    params = [
                        AstParam(
                            name=pname or f"p{i}",
                            mutable=isinstance(ct, (CPtr, COpaqueStructPtr)) and ct.mutable,
                            type_ann=ctype_to_tl_str(ct),
                            default=None,
                        )
                        for i, (pname, ct) in enumerate(sig.params)
                    ]
                    body.append(StmtFnDef(
                        name=sig.name,
                        template_params=[],
                        params=params,
                        return_type=ctype_to_tl_str(sig.ret),
                        body=[],
                        is_abstract=False,
                        is_static=False,
                        is_class_method=False,
                        decorators=[],
                        access=Accessibility.PUBLIC,
                    ))
            except Exception:
                pass  # header parse errors are non-fatal; runtime handles errors

        return StmtImport(lang=lang, module=[file_path], alias=alias, body=body,
                          level=level, base_dir=str(self._import_base(level)))

    def _parse_version_bracket(self) -> Optional[str]:
        """Parse optional [X.Y.Z] version tag after a crate name."""
        if self._current_kind() != TokenKind.LBRACKET:
            return None
        # Peek: if this looks like a version string, consume it
        self._advance()  # eat '['
        parts = []
        while self._current_kind() not in (TokenKind.RBRACKET, TokenKind.EOF):
            parts.append(str(self._current().value))
            self._advance()
        if self._current_kind() == TokenKind.RBRACKET:
            self._advance()  # eat ']'
        return "".join(parts) if parts else None

    def _load_module(
        self, lang: str, module: list[str], version: Optional[str] = None, level: int = 0
    ) -> list[Stmt]:
        # "tl-auto" / "tl" は旧名の別名。既存ソース互換のため受理し続ける
        if lang in ("ar-auto", "tl-auto", "ar", "tl"):
            return self._load_tl_module(module, force_source=(lang in ("tl", "ar")), level=level)
        if lang in ("tlc", "arc"):
            return self._load_tlc_module(module, level=level)
        if lang in ("py", "py-int"):
            return []  # Python modules have no AST body in Python impl
        if lang in ("cpp-dll", "cpp-lib"):
            return []  # handled by _parse_cpp_import; body already filled there
        if lang == "rs":
            if level > 0:
                raise self._error(
                    f"import[rs] takes a crate name, not a file path: `{_written(level, module)}` cannot be relative"
                )
            return self._load_rs_module(module, version)
        if lang in ("cs-dll", "cs-proc"):
            return self._load_cs_module(module, level=level)
        raise self._error(f"unknown import language '{lang}'")

    def _load_tl_module(self, module: list[str], force_source: bool = False, level: int = 0) -> list[Stmt]:
        module_base = Path(*module) if len(module) > 1 else Path(module[0])
        candidates: list[tuple[Path, bool]] = []
        # ⚠ 探索先は起点 1 か所だけ（ドット無しはエントリのディレクトリ・mirrors module_search_dirs）
        search_dirs = [self._import_base(level)]
        for d in search_dirs:
            if not force_source:
                candidates.append((d / module_base.with_suffix(".arc"), True))
            candidates.append((d / module_base.with_suffix(".ar"), False))
            candidates.append((d / module_base / "__init__.ar", False))

        found: Optional[tuple[Path, bool]] = None
        for path, is_tlc in candidates:
            if path.exists():
                found = (path, is_tlc)
                break

        if found is None:
            # `__init__` の無いディレクトリは名前空間パッケージ（CPython と同じ）。
            d = search_dirs[0] / module_base
            if d.is_dir():
                self._last_loaded = (self._canonical(Path(os.path.normpath(os.path.abspath(d))), module),
                                     search_dirs[0])
                return []
            checked = ", ".join(f"'{p}'" for p, _ in candidates)
            raise self._error(f"cannot find module '{_written(level, module)}' (looked at {checked})")

        self._last_loaded = (self._canonical(_identity_of_file(found[0]), module), search_dirs[0])
        return self._load_tl_file(found[0], found[1], module)

    def _load_tl_file(self, path: Path, is_tlc: bool, module: list[str]) -> list[Stmt]:
        """見つけた .ar / .arc を読み込む（キャッシュ・循環検出・子パーサ）。"""
        module_base = Path(*module) if module else Path("__init__")
        abs_path, is_tlc = path, is_tlc
        # ⚠ 鍵（キャッシュ・循環検出）は絶対パスに正規化する（`..` を含む相互 import が
        #   循環検出をすり抜けないように・mirrors found_paths）。
        abs_path = Path(os.path.normpath(abs_path.resolve()))
        cache_key = ("ar-auto", abs_path)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if abs_path in self._loading:
            raise self._error(f"circular import detected: '{abs_path}'")

        if is_tlc:
            source = _extract_arc_source(abs_path)
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

    def _load_tlc_module(self, module: list[str], level: int = 0) -> list[Stmt]:
        module_base = Path(*module) if len(module) > 1 else Path(module[0])
        search_dirs = [self._import_base(level)]
        candidates = [d / module_base.with_suffix(".arc") for d in search_dirs]
        found: Optional[Path] = next((p for p in candidates if p.exists()), None)
        if found is None:
            checked = ", ".join(f"'{p}'" for p in candidates)
            raise self._error(
                f"cannot find compiled module '{_written(level, module)}' (looked at {checked}; "
                "compile with: cargo run --release -- --compile <source.ar>)"
            )
        self._last_loaded = (self._canonical(_identity_of_file(found), module), search_dirs[0])
        cache_key = ("tlc", found)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]
        if found in self._loading:
            raise self._error(f"circular import detected: '{found}'")
        source = _extract_arc_source(found)
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

    def _load_rs_module(
        self, module: list[str], version: Optional[str] = None
    ) -> list[Stmt]:
        crate_name = ".".join(module)
        cache_key = ("rs", crate_name)
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]

        from ..partial_compiler.rs_loader import load as rs_load, _RS_DLL_CACHE
        # ar_config.json はエントリのディレクトリから祖先へ探す（mirrors load_rs_module）
        start = self._root_dir.resolve()
        search_dirs = [start, *start.parents]

        try:
            stmts, _dll_bytes = rs_load(crate_name, search_dirs, version)
        except Exception as e:
            raise self._error(str(e))

        self._module_cache[cache_key] = stmts
        return stmts

    def _load_cs_module(self, module: list[str], level: int = 0) -> list[Stmt]:
        """Load a .NET managed DLL and generate Arrow type stubs via ECMA-335 parsing."""
        from ..parser.cs_assembly import load_cs_assembly

        mod_path = Path(*module) if len(module) > 1 else Path(module[0])
        managed_dll_name = f"{module[-1]}.dll"

        # ⚠ 起点 1 か所だけ（ドット無しはエントリのディレクトリ・mirrors load_cs_module）
        search_dirs = [self._import_base(level)]

        found: Optional[Path] = None
        for d in search_dirs:
            # First try subdirectory structure: e.g. cs_form_test/FormBridge.dll
            sub = mod_path.parent / managed_dll_name if len(module) > 1 else None
            candidates = []
            if sub:
                candidates.append(d / sub)
            candidates.append(d / mod_path.with_suffix(".dll"))
            candidates.append(d / managed_dll_name)
            for c in candidates:
                if c.exists():
                    found = c
                    break
            if found:
                break

        if found is None:
            checked = [str(d / mod_path.with_suffix(".dll")) for d in search_dirs]
            raise self._error(
                f"cs-dll: cannot find managed DLL for '{'.'.join(module)}' "
                f"(looked at {checked})"
            )

        cache_key = ("cs-dll", str(found))
        if cache_key in self._module_cache:
            return self._module_cache[cache_key]

        try:
            stmts = load_cs_assembly(found)
        except Exception as e:
            raise self._error(f"cs-dll: failed to read '{found}': {e}")

        self._module_cache[cache_key] = stmts
        return stmts
