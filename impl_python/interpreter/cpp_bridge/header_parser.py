# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
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

def _parse_structs(
    text: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
) -> list[CStructDef]:
    """Extract typedef struct { ... } Name; definitions from a header."""
    structs: list[CStructDef] = []
    # Match: typedef struct { ... } Name;  or  typedef struct Tag { ... } Name;
    pat = re.compile(
        r"typedef\s+struct\s*\w*\s*\{([^}]*)\}\s*(\w+)\s*;",
        re.DOTALL,
    )
    for m in pat.finditer(text):
        body_text, alias = m.group(1), m.group(2)
        fields: list[tuple[str, CType]] = []
        # Split on semicolons — each declaration ends with ;
        for decl in body_text.split(";"):
            decl = decl.strip()
            if not decl:
                continue
            # Skip arrays and function pointers
            if "[" in decl or "(*)" in decl:
                continue
            # rsplit on whitespace to separate type from field name
            parts = decl.rsplit(None, 1)
            if len(parts) != 2:
                continue
            type_s, field_name = parts
            # Move leading * from field_name to type_s
            while field_name.startswith("*"):
                type_s += "*"
                field_name = field_name[1:]
            field_name = field_name.strip()
            if not re.fullmatch(r"[A-Za-z_]\w*", field_name):
                continue
            ct = _parse_type_str(type_s, custom, typedefs, set())
            if ct is not None and not isinstance(ct, (CVoid, CFnPtr)):
                fields.append((field_name, ct))
        if fields:
            structs.append(CStructDef(name=alias, fields=fields))
    return structs


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

def parse_header_full(
    content: str,
    custom: dict[str, str],
    typedefs: dict[str, str],
) -> tuple[list[CFnSig], list[CStructDef]]:
    """Parse a C/C++ header string. Returns (functions, structs)."""
    stripped = _strip_comments(content)

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
