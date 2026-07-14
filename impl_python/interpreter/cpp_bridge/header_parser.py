# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""C/C++ header parser (mirrors cpp_bridge/header_parser.rs).

Extracts function signatures and struct definitions from C/C++ headers using
regex-based parsing. Handles namespaces, extern "C" blocks, comments, and
common Windows/platform type aliases.
"""
from __future__ import annotations
import re
from pathlib import Path
from typing import Optional

from .types import (
    CType, CInt, CLong, CFloat, CDouble, CBool, CVoid, CVoidPtr, CCharPtr,
    CPtr, COpaqueStructPtr, CByValueStruct, CFnPtr, CFnSig, CStructDef,
)

DEFAULTPARAM_MACRO = "DEFAULTPARAM"

# ── Primitive type name → tl primitive string ─────────────────────────────────

_PRIMITIVE_MAP: dict[str, str] = {
    # Void
    "void": "void",
    # Integer (int)
    "int": "int", "short": "int", "char": "int", "signed": "int",
    "unsigned int": "int", "unsigned short": "int", "unsigned char": "int",
    "signed int": "int", "signed short": "int", "signed char": "int",
    "int32_t": "int", "uint32_t": "int", "int16_t": "int", "uint16_t": "int",
    "int8_t": "int", "uint8_t": "int",
    "DWORD": "int", "WORD": "int", "BYTE": "int", "UINT": "int", "INT": "int",
    "DWORD32": "int", "HRESULT": "int", "UINT32": "int", "INT32": "int",
    "LONG": "long",
    # Integer (long)
    "long long": "long", "unsigned long long": "long",
    "int64_t": "long", "uint64_t": "long",
    "size_t": "long", "ptrdiff_t": "long", "ssize_t": "long",
    "LONGLONG": "long", "ULONGLONG": "long", "INT64": "long", "UINT64": "long",
    "DWORD64": "long", "DWORD_PTR": "long", "ULONG_PTR": "long",
    "ULONG": "long",
    # Float / Double
    "float": "float", "double": "double",
    # Bool
    "bool": "bool", "BOOL": "bool",
}

_TL_TO_CTYPE: dict[str, CType] = {
    "int": CInt(), "long": CLong(), "float": CFloat(),
    "double": CDouble(), "bool": CBool(), "void": CVoid(),
}

# Opaque pointer type aliases that map to int (handle)
_OPAQUE_HANDLE_ALIASES = {
    "HANDLE", "HWND", "HDC", "HMODULE", "HINSTANCE", "HBITMAP", "HFONT",
    "HPEN", "HBRUSH", "HRGN", "HMENU", "HCURSOR", "HICON", "HKEY",
    "HTEXTURE", "HSHADER", "HSOUND", "HMUSIC",
    "LPVOID", "PVOID",
}


# ── Comment stripping ─────────────────────────────────────────────────────────

def _strip_comments(text: str) -> str:
    """Remove C/C++ line comments and block comments."""
    result = []
    i = 0
    n = len(text)
    while i < n:
        if text[i:i+2] == "//":
            # Line comment: skip to end of line
            while i < n and text[i] != "\n":
                i += 1
        elif text[i:i+2] == "/*":
            # Block comment: skip to */
            i += 2
            while i < n and text[i:i+2] != "*/":
                i += 1
            i += 2
        elif text[i] == '"':
            # String literal: pass through verbatim
            result.append(text[i])
            i += 1
            while i < n and text[i] != '"':
                if text[i] == "\\" and i + 1 < n:
                    result.append(text[i]); result.append(text[i+1])
                    i += 2
                else:
                    result.append(text[i]); i += 1
            if i < n:
                result.append(text[i]); i += 1
        else:
            result.append(text[i])
            i += 1
    return "".join(result)


# ── Type string → CType ───────────────────────────────────────────────────────

def _parse_type_str(
    type_str: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
    struct_names: set[str],
) -> Optional[CType]:
    """Convert a C type string to a CType instance."""
    s = type_str.strip()
    is_const = s.startswith("const ")
    if is_const:
        s = s[6:].strip()

    # Function pointer: contains (*) or (*)
    if "(*)" in s or re.search(r"\(\s*\*\s*\w*\s*\)", s):
        return CFnPtr()

    # Pointer: ends with *
    ptr_count = 0
    while s.endswith("*"):
        ptr_count += 1
        s = s[:-1].strip()
        if s.endswith("const"):
            s = s[:-5].strip()

    # Resolve typedef aliases
    s = typedefs.get(s, s)

    # Check custom type map
    if s in custom:
        prim = custom[s]
        ct = _TL_TO_CTYPE.get(prim)
        if ct is not None:
            if ptr_count == 0:
                return ct
            if ptr_count == 1:
                if isinstance(ct, CVoid):
                    return CVoidPtr()
                return CPtr(inner=ct, mutable=not is_const)

    # Check opaque handle aliases (these are already pointer-sized ints)
    if s in _OPAQUE_HANDLE_ALIASES:
        if ptr_count == 0:
            return CVoidPtr()
        # pointer to handle → opaque
        return CVoidPtr()

    # char pointer → CCharPtr
    if s in ("char", "TCHAR", "wchar_t") and ptr_count == 1:
        return CCharPtr()
    if s in ("char", "TCHAR", "wchar_t") and ptr_count == 0:
        return CInt()

    # Primitive lookup (handles multi-word like "long long", "unsigned int")
    prim = _PRIMITIVE_MAP.get(s)
    if prim is not None:
        ct = _TL_TO_CTYPE.get(prim)
        if ct is not None:
            if ptr_count == 0:
                return ct
            if ptr_count == 1:
                if isinstance(ct, CVoid):
                    return CVoidPtr()
                if isinstance(ct, CInt) and s in ("char", "TCHAR"):
                    return CCharPtr()
                return CPtr(inner=ct, mutable=not is_const)
            if ptr_count == 2 and isinstance(ct, CVoid):
                return CVoidPtr()
            return None  # double+ pointer to primitive: unsupported

    # Unknown type: if it looks like a struct name, make it OpaqueStructPtr
    if ptr_count >= 1 and re.fullmatch(r"[A-Za-z_]\w*", s):
        return COpaqueStructPtr(type_name=s, mutable=(ptr_count == 1 and not is_const))

    # Unknown non-pointer type
    return None


# ── Struct definition parsing ─────────────────────────────────────────────────
# Mirrors src/interpreter/cpp_bridge/header_parser/structs.rs (brace-walking
# parser with namespace descent, C++ classes, member classification and the
# `complete` layout flag).

def _find_matching_brace(s: str) -> Optional[int]:
    """`s` starts at a '{'; return the index of its matching '}' (or None)."""
    depth = 0
    for i, ch in enumerate(s):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i
    return None


def _parse_alias_list(aliases_str: str) -> list[tuple[str, str]]:
    """Parse `A, *PA` after a typedef body into (alias, pointer_suffix) pairs."""
    out: list[tuple[str, str]] = []
    for part in aliases_str.split(","):
        part = part.strip()
        stars = part.count("*")
        name = part.replace("*", "").strip()
        if name and re.fullmatch(r"[A-Za-z_]\w*", name):
            out.append((name, "*" * stars))
    return out


def _classify_member_segment(seg: str) -> str:
    """Classify one struct-body member segment (leading access label stripped).

    Returns:
    - "reject"   — virtual (vtable changes the layout) / friend: not a simple class
    - "ignore"   — members that don't contribute to the layout (methods, static,
                   nested type definitions, using, ...)
    - "bitfield" — bitfield: layout cannot be computed (complete=False)
    - "field"    — data field candidate
    """
    words = seg.split()
    start = 1 if (words and words[0].rstrip(":") in ("public", "private", "protected")) else 0
    words = words[start:]
    if not words:
        return "ignore"
    first = words[0]
    # virtual member function → vtable is inserted, not a simple class
    if first == "virtual" or "virtual" in words:
        return "reject"
    if first == "friend":
        return "reject"
    if first in ("static", "typedef", "using", "enum", "struct", "class", "union"):
        return "ignore"
    # Method declaration (contains parentheses)
    if "(" in seg:
        return "ignore"
    # Bitfield: `int flags : 3`
    if any(":" in w for w in words):
        return "bitfield"
    return "field"


def _parse_field_segment(
    seg: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
    out: list,
) -> None:
    """Parse one `;`-separated field segment (`float x, y, z` etc.) and append
    resolved (name, CType) pairs to `out`."""
    if "(" in seg:
        return
    all_words = seg.split()
    start = 1 if (all_words and all_words[0].rstrip(":") in
                  ("public", "private", "protected", "virtual")) else 0
    words = all_words[start:]
    if len(words) < 2:
        return

    joined = " ".join(words)
    parts = joined.split(",")
    first_words = parts[0].split()
    if len(first_words) < 2:
        return

    raw_last = first_words[-1]
    # Skip array fields like `m[4][4]`
    if "[" in raw_last or "(" in raw_last:
        return
    first_stars = 0
    name = raw_last
    while name.startswith("*"):
        first_stars += 1
        name = name[1:]
    if not name or not re.fullmatch(r"[A-Za-z_]\w*", name):
        return

    type_words = [w for w in first_words[:-1] if w != "*"]
    standalone_stars = sum(1 for w in first_words[:-1] if w == "*")
    base_str = " ".join(type_words)
    total_stars = first_stars + standalone_stars
    type_str = base_str + "*" * total_stars

    ctype = _parse_type_str(type_str, custom, typedefs, set())
    if ctype is None:
        return
    out.append((name, ctype))
    for part in parts[1:]:
        pw = part.split()
        extra_stars = sum(1 for w in pw if w == "*")
        raw_name = next((w for w in pw if w != "*"), None)
        if raw_name is None or "[" in raw_name or "(" in raw_name:
            continue
        alias_stars = 0
        alias = raw_name
        while alias.startswith("*"):
            alias_stars += 1
            alias = alias[1:]
        if not alias or not re.fullmatch(r"[A-Za-z_]\w*", alias):
            continue
        stars = alias_stars + extra_stars
        if stars == 0:
            out.append((alias, ctype))
        else:
            ptr_ct = _parse_type_str(base_str + "*" * stars, custom, typedefs, set())
            if ptr_ct is not None:
                out.append((alias, ptr_ct))


def _parse_struct_field_decls(
    body: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
) -> Optional[tuple[list, bool]]:
    """Parse field declarations inside a struct body. `float x, y, z;` → 3 fields.

    Returns:
    - None — the body contains `virtual` / `friend` (not a simple class:
      the whole struct is excluded)
    - (fields, complete) — `complete` is True when every layout member was
      captured as a field (skipped array/bitfield/nested-struct/unresolved
      fields make it False — no raw layout can be attached)
    """
    fields: list = []
    complete = True
    i = 0
    seg_start = 0
    n = len(body)
    while i < n:
        if body[i] == "{":
            end = _find_matching_brace(body[i:])
            if end is not None:
                # Nested type definition (enum/struct/union/method body):
                # not a layout member — skip (complete is kept)
                i += end + 1
                seg_start = i
                continue
        if body[i] == ";":
            seg = body[seg_start:i].strip()
            if seg:
                kind = _classify_member_segment(seg)
                if kind == "reject":
                    return None  # virtual / friend
                if kind == "bitfield":
                    complete = False
                elif kind == "field":
                    before = len(fields)
                    _parse_field_segment(seg, custom, typedefs, fields)
                    if len(fields) == before:
                        # Should have been a field but couldn't be parsed
                        # (array / unresolved type etc.)
                        complete = False
            i += 1
            seg_start = i
            continue
        i += 1
    return fields, complete


def _parse_structs(
    text: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
) -> list[CStructDef]:
    """Scan stripped source for struct/class definitions.

    Handles `typedef struct/union Tag { … } Alias;`, `struct Name { … };`
    and `class Name { … };`, descending into `namespace X { … }` and
    `extern "C" { … }` scope blocks.
    """
    result: list[CStructDef] = []
    i = 0
    seg_start = 0
    n = len(text)

    while i < n:
        if text[i] == "{":
            # `namespace X { … }` / `extern "C" { … }` are scope blocks:
            # descend into them instead of skipping (DxLib.h wraps everything
            # in namespace DxLib).
            seg_before = text[seg_start:i].strip()
            w = seg_before.split()
            is_scope_block = (bool(w) and w[0] == "namespace") or \
                ("extern" in w and '"C"' in seg_before)
            if is_scope_block:
                i += 1
                seg_start = i
                continue

            brace_end = _find_matching_brace(text[i:])
            if brace_end is not None:
                seg_before = text[seg_start:i].lstrip()
                is_union = seg_before.startswith("typedef") and " union " in seg_before
                is_struct_typedef = seg_before.startswith("typedef") and \
                    (" struct " in seg_before or is_union)

                # `class Name { … }` or `struct Name { … }` (not typedef).
                # Inheritance (`class D : public B`) makes the layout include
                # the base part → complete=False.
                class_name: Optional[str] = None
                has_inheritance = False
                if not is_struct_typedef:
                    w = seg_before.split()
                    if len(w) >= 2 and w[0] in ("class", "struct"):
                        cand = w[1].rstrip(":")
                        if cand and re.fullmatch(r"[A-Za-z_]\w*", cand):
                            class_name = cand
                            has_inheritance = ":" in seg_before

                if is_struct_typedef:
                    body = text[i + 1:i + brace_end]
                    rest = text[i + brace_end + 1:]
                    semi_pos = rest.find(";")
                    if semi_pos != -1:
                        aliases_str = rest[:semi_pos].strip()
                        if aliases_str:
                            parsed = _parse_struct_field_decls(body, custom, typedefs)
                            if parsed is not None:
                                fields, fields_complete = parsed
                                if fields:
                                    # union fields overlap → layout incomplete
                                    complete = fields_complete and not is_union
                                    for alias, ptr_suffix in _parse_alias_list(aliases_str):
                                        if "*" not in ptr_suffix:
                                            result.append(CStructDef(
                                                name=alias,
                                                fields=list(fields),
                                                complete=complete,
                                            ))
                elif class_name is not None:
                    body = text[i + 1:i + brace_end]
                    parsed = _parse_struct_field_decls(body, custom, typedefs)
                    if parsed is not None:
                        fields, fields_complete = parsed
                        if fields:
                            result.append(CStructDef(
                                name=class_name,
                                fields=fields,
                                complete=fields_complete and not has_inheritance,
                            ))

                i += brace_end + 1
                seg_start = i
                continue

        if text[i] == ";":
            seg_start = i + 1
        i += 1
    return result


# ── Function declaration parsing ──────────────────────────────────────────────

def _parse_param(
    param_str: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
    struct_names: set[str],
) -> Optional[tuple[str, CType]]:
    """Parse a single parameter declaration into (name, CType)."""
    s = param_str.strip()
    if not s or s == "void" or s == "...":
        return None

    # Handle DEFAULTPARAM macro: DEFAULTPARAM(type, name, default)
    dp = re.fullmatch(
        r"DEFAULTPARAM\s*\(\s*(.+?)\s*,\s*(\w+)\s*,\s*.+\s*\)", s, re.DOTALL
    )
    if dp:
        ct = _parse_type_str(dp.group(1), custom, typedefs, struct_names)
        return (dp.group(2), ct) if ct else None

    # Split type from name: last word (or *name) is the parameter name
    # Handle: "const int* foo", "int foo", "void* bar", "int (*callback)(int)"
    if "(*)" in s or re.search(r"\(\s*\*", s):
        return ("fn_ptr_param", CFnPtr())

    # Match trailing name (possibly with leading *)
    m = re.match(r"^(.*?)(\*?\w+)\s*$", s, re.DOTALL)
    if not m:
        return None
    type_part = m.group(1).strip()
    name_part = m.group(2)

    # If name_part starts with *, move * to type_part
    while name_part.startswith("*"):
        type_part += "*"
        name_part = name_part[1:]

    # type_part might be empty for e.g. "int" — then name is the type, no name
    if not type_part:
        ct = _parse_type_str(name_part, custom, typedefs, struct_names)
        return ("", ct) if ct else None

    ct = _parse_type_str(type_part, custom, typedefs, struct_names)
    return (name_part, ct) if ct else None


def _parse_fn_decl(
    decl: str,
    namespace: Optional[str],
    custom: dict[str, str],
    typedefs: dict[str, str],
    struct_names: set[str],
) -> Optional[CFnSig]:
    """Parse a function declaration string into a CFnSig."""
    s = decl.strip().rstrip(";").strip()
    if not s:
        return None

    # Remove storage class / calling convention keywords
    for kw in ("__declspec(dllexport)", "__declspec(dllimport)", "__cdecl",
               "__stdcall", "__fastcall", "WINAPI", "CALLBACK", "APIENTRY",
               "extern", "static", "inline", "__inline", "__forceinline",
               "virtual", "explicit", "__attribute__((visibility(\"default\")))"):
        s = s.replace(kw, " ")
    s = re.sub(r"\s+", " ", s).strip()

    # Match: RetType Name(Params)
    m = re.match(r"^(.*?)\b(\w+)\s*\(([^)]*)\)\s*$", s, re.DOTALL)
    if not m:
        return None

    ret_str = m.group(1).strip()
    fn_name = m.group(2)
    params_str = m.group(3).strip()

    # Skip constructor/destructor patterns, operators, templates
    if fn_name in ("if", "while", "for", "switch", "return", "struct",
                   "class", "typedef", "namespace") or "operator" in fn_name:
        return None
    if "<" in ret_str or ">" in ret_str:
        return None

    # Remove 'const' suffix (member function const qualifier)
    ret_str = re.sub(r"\bconst\b\s*$", "", ret_str).strip()

    ret_ct = _parse_type_str(ret_str, custom, typedefs, struct_names)
    if ret_ct is None:
        return None

    # Parse parameters
    params: list[tuple[str, CType]] = []
    n_required = 0
    has_default = False

    if params_str and params_str != "void":
        # Split on commas not inside <>() brackets
        depth = 0
        parts: list[str] = []
        cur = []
        for ch in params_str:
            if ch in "(<":
                depth += 1
            elif ch in ")>":
                depth -= 1
            elif ch == "," and depth == 0:
                parts.append("".join(cur).strip())
                cur = []
                continue
            cur.append(ch)
        if cur:
            parts.append("".join(cur).strip())

        for i, part in enumerate(parts):
            is_default = DEFAULTPARAM_MACRO in part
            p = _parse_param(part, custom, typedefs, struct_names)
            if p is None:
                return None  # unsupported param type → skip whole function
            params.append(p)
            if is_default and not has_default:
                has_default = True
                n_required = i

    if not has_default:
        n_required = len(params)

    return CFnSig(
        name=fn_name,
        params=params,
        ret=ret_ct,
        namespace=namespace,
        n_required=n_required,
    )


# ── Top-level scanner ─────────────────────────────────────────────────────────

def _scan_scope(
    text: str,
    namespace: Optional[str],
    out: list[tuple[str, Optional[str]]],
) -> None:
    """Recursively scan a C/C++ text block, collecting (declaration, namespace) pairs."""
    i = 0
    n = len(text)
    while i < n:
        # Skip whitespace
        while i < n and text[i] in " \t\n\r":
            i += 1
        if i >= n:
            break

        # namespace Foo { ... }
        ns_m = re.match(r"namespace\s+(\w+)\s*\{", text[i:])
        if ns_m:
            ns_name = ns_m.group(1)
            start = i + ns_m.end()
            # Find matching }
            depth = 1
            j = start
            while j < n and depth > 0:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                j += 1
            inner = text[start:j-1]
            _scan_scope(inner, ns_name, out)
            i = j
            continue

        # extern "C" { ... }
        ec_m = re.match(r'extern\s+"C"\s*\{', text[i:])
        if ec_m:
            start = i + ec_m.end()
            depth = 1
            j = start
            while j < n and depth > 0:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                j += 1
            inner = text[start:j-1]
            _scan_scope(inner, namespace, out)
            i = j
            continue

        # extern "C" RetType Name(...);  (single declaration)
        ec_single = re.match(r'extern\s+"C"\s+', text[i:])
        if ec_single:
            i += ec_single.end()
            continue

        # Skip struct/class/union/enum body: struct Foo { ... };
        struct_m = re.match(
            r"(typedef\s+)?(struct|class|union|enum)\s*\w*\s*\{", text[i:]
        )
        if struct_m:
            start = i + struct_m.end()
            depth = 1
            j = start
            while j < n and depth > 0:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                j += 1
            # Collect the rest of the typedef line (e.g. "} Name;")
            end_m = re.match(r"\s*\w*\s*;", text[j:])
            if end_m:
                j += end_m.end()
            i = j
            continue

        # Preprocessor directives
        if text[i] == "#":
            while i < n and text[i] != "\n":
                i += 1
            continue

        # Try to collect a statement (up to ; or {)
        j = i
        while j < n and text[j] not in (";", "{", "}"):
            j += 1

        if j >= n:
            break

        if text[j] == ";":
            stmt = text[i:j+1].strip()
            if stmt:
                out.append((stmt, namespace))
            i = j + 1
        elif text[j] == "{":
            # Skip function body or initialiser
            depth = 1
            j += 1
            while j < n and depth > 0:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                j += 1
            # Skip optional ; after }
            while j < n and text[j] in " \t\n\r":
                j += 1
            if j < n and text[j] == ";":
                j += 1
            i = j
        else:
            # } — end of outer scope
            i = j + 1


# ── Public API ────────────────────────────────────────────────────────────────

def _strip_preprocessor(text: str) -> str:
    """Remove preprocessor directive lines (`#include`, `#pragma`, ...) including
    backslash continuations (mirrors the Rust preprocess step)."""
    out_lines: list[str] = []
    in_directive = False
    for line in text.splitlines():
        if in_directive:
            in_directive = line.rstrip().endswith("\\")
            out_lines.append("")
            continue
        if line.lstrip().startswith("#"):
            in_directive = line.rstrip().endswith("\\")
            out_lines.append("")
            continue
        out_lines.append(line)
    return "\n".join(out_lines)


def parse_header_full(
    content: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
) -> tuple[list[CFnSig], list[CStructDef]]:
    """Parse a C/C++ header string. Returns (functions, structs)."""
    stripped = _strip_comments(content)
    stripped = _strip_preprocessor(stripped)

    struct_defs = _parse_structs(stripped, custom, typedefs)
    struct_names: set[str] = {s.name for s in struct_defs}

    decls: list[tuple[str, Optional[str]]] = []
    _scan_scope(stripped, None, decls)

    sigs: list[CFnSig] = []
    seen: set[str] = set()
    for decl, ns in decls:
        sig = _parse_fn_decl(decl, ns, custom, typedefs, struct_names)
        if sig and sig.name not in seen:
            seen.add(sig.name)
            sigs.append(sig)

    return sigs, struct_defs


def collect_included_headers(raw_content: str, header_dir: Path) -> list[Path]:
    """Find local #include "filename.h" paths that exist next to header_dir."""
    result: list[Path] = []
    for line in raw_content.splitlines():
        t = line.strip()
        if not t.startswith("#include"):
            continue
        m = re.search(r'#include\s+"([^"]+)"', t)
        if m:
            candidate = header_dir / m.group(1)
            if candidate.exists():
                result.append(candidate)
    return result
