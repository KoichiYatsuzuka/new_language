# git SHA: 4a937ed4f6e246e10a462c337360a817357c060c
"""InferredType variants and type annotation parsing (mirrors src/type_check.rs)."""
from __future__ import annotations
from dataclasses import dataclass
from typing import Optional


@dataclass(frozen=True)
class TyInt:
    def __str__(self) -> str: return "int"

@dataclass(frozen=True)
class TyFloat:
    def __str__(self) -> str: return "float"

@dataclass(frozen=True)
class TyStr:
    def __str__(self) -> str: return "str"

@dataclass(frozen=True)
class TyBool:
    def __str__(self) -> str: return "bool"

@dataclass(frozen=True)
class TyNone:
    def __str__(self) -> str: return "None"

@dataclass(frozen=True)
class TyList:
    def __str__(self) -> str: return "list"

@dataclass(frozen=True)
class TyDict:
    def __str__(self) -> str: return "dict"

@dataclass(frozen=True)
class TySet:
    def __str__(self) -> str: return "set"

@dataclass(frozen=True)
class TyTypeVal:
    def __str__(self) -> str: return "type"

@dataclass(frozen=True)
class TyTypeValOf:
    inner: "InferredType"
    def __str__(self) -> str: return f"type[{self.inner}]"

@dataclass(frozen=True)
class TySelfType:
    def __str__(self) -> str: return "Self"

@dataclass(frozen=True)
class TyNamedInstance:
    name: str
    def __str__(self) -> str: return self.name

@dataclass(frozen=True)
class TyAny:
    def __str__(self) -> str: return "Any"

@dataclass(frozen=True)
class TyUnion:
    types: tuple["InferredType", ...]

    def __str__(self) -> str:
        if len(self.types) == 2 and self.types[1] == TyNone():
            return f"Option[{self.types[0]}]"
        return "Union[" + ", ".join(str(t) for t in self.types) + "]"

@dataclass(frozen=True)
class TyTuple:
    types: tuple["InferredType", ...]
    def __str__(self) -> str:
        return "tuple[" + ", ".join(str(t) for t in self.types) + "]"

@dataclass(frozen=True)
class TyNamespace:
    members: tuple[tuple[str, "InferredType"], ...]

    def as_dict(self) -> dict[str, "InferredType"]:
        return dict(self.members)

    def __str__(self) -> str:
        return f"<module({len(self.members)} members)>"

@dataclass(frozen=True)
class TyUnresolved:
    def __str__(self) -> str: return "unknown"

@dataclass(frozen=True)
class FnTypeParam:
    name: str
    mutable: bool
    ty: "InferredType"

@dataclass(frozen=True)
class TyFunction:
    params: Optional[tuple["FnTypeParam", ...]]
    return_type: "InferredType"

    def __str__(self) -> str:
        if self.params is None:
            result = "function"
        else:
            parts = [
                f"{'mut' if p.mutable else 'let'} {p.name}:{p.ty}"
                for p in self.params
            ]
            result = "function{" + ",".join(parts) + "}"
        if self.return_type != TyAny():
            result += f"->{self.return_type}"
        return result


InferredType = (
    TyInt | TyFloat | TyStr | TyBool | TyNone | TyList | TyDict | TySet |
    TyTypeVal | TyTypeValOf | TySelfType | TyNamedInstance | TyAny |
    TyUnion | TyTuple | TyNamespace | TyUnresolved | TyFunction
)


# ---------------------------------------------------------------------------
# Utility functions for parsing type annotation strings
# ---------------------------------------------------------------------------

def _split_top_level_commas(s: str) -> list[str]:
    result: list[str] = []
    depth = 0
    start = 0
    for i, c in enumerate(s):
        if c == "[":
            depth += 1
        elif c == "]":
            if depth > 0:
                depth -= 1
        elif c == "," and depth == 0:
            result.append(s[start:i])
            start = i + 1
    result.append(s[start:])
    return result


def _split_top_level_commas_fn(s: str) -> list[str]:
    result: list[str] = []
    depth = 0
    start = 0
    for i, c in enumerate(s):
        if c in ("[", "{"):
            depth += 1
        elif c in ("]", "}"):
            if depth > 0:
                depth -= 1
        elif c == "," and depth == 0:
            result.append(s[start:i])
            start = i + 1
    result.append(s[start:])
    return result


def _find_closing_bracket(s: str, open_ch: str, close_ch: str) -> Optional[int]:
    depth = 0
    for i, c in enumerate(s):
        if c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return i
    return None


def _parse_fn_type_ann(rest: str) -> Optional["InferredType"]:
    params: Optional[list[FnTypeParam]]
    after: str

    if rest.startswith("["):
        close = _find_closing_bracket(rest, "[", "]")
        if close is None:
            return None
        inner = rest[1:close]
        after = rest[close + 1:]
        if not inner.strip():
            params = []
        else:
            params = []
            for idx, part in enumerate(_split_top_level_commas_fn(inner)):
                p = part.strip()
                if p.startswith("mut "):
                    mutable, type_str = True, p[4:].strip()
                elif p.startswith("let "):
                    mutable, type_str = False, p[4:].strip()
                else:
                    mutable, type_str = False, p
                colon = type_str.find(":")
                if colon != -1:
                    name = type_str[:colon].strip()
                    ty_s = type_str[colon + 1:].strip()
                else:
                    name = f"param{idx + 1}"
                    ty_s = type_str
                ty = inferred_type_from_ann(ty_s) or TyAny()
                params.append(FnTypeParam(name=name, mutable=mutable, ty=ty))
    elif rest.startswith("{"):
        close = _find_closing_bracket(rest, "{", "}")
        if close is None:
            return None
        inner = rest[1:close]
        after = rest[close + 1:]
        if not inner.strip():
            params = []
        else:
            params = []
            for part in _split_top_level_commas_fn(inner):
                p = part.strip()
                if p.startswith("mut "):
                    mutable, rest_p = True, p[4:].strip()
                elif p.startswith("let "):
                    mutable, rest_p = False, p[4:].strip()
                else:
                    mutable, rest_p = False, p
                colon = rest_p.find(":")
                if colon == -1:
                    return None
                name = rest_p[:colon].strip()
                ty_s = rest_p[colon + 1:].strip()
                ty = inferred_type_from_ann(ty_s) or TyAny()
                params.append(FnTypeParam(name=name, mutable=mutable, ty=ty))
    else:
        params = None
        after = rest

    if after.startswith("->"):
        return_type = inferred_type_from_ann(after[2:].strip()) or TyAny()
    else:
        return_type = TyAny()

    return TyFunction(
        params=tuple(params) if params is not None else None,
        return_type=return_type,
    )


def inferred_type_from_ann(ann: str) -> Optional["InferredType"]:
    if ann.startswith("Union[") and ann.endswith("]"):
        inner = ann[6:-1]
        parts = _split_top_level_commas(inner)
        resolved = [t for t in (inferred_type_from_ann(p.strip()) for p in parts) if t is not None]
        return TyUnion(tuple(resolved)) if len(resolved) >= 2 else None

    if ann.startswith("Option[") and ann.endswith("]"):
        t = inferred_type_from_ann(ann[7:-1].strip())
        return TyUnion((t, TyNone())) if t is not None else None

    if ann.startswith("list[") and ann.endswith("]"):
        return TyList()
    if ann.startswith("set[") and ann.endswith("]"):
        return TySet()
    if ann.startswith("dict[") and ann.endswith("]"):
        return TyDict()

    if ann.startswith("tuple[") and ann.endswith("]"):
        inner = ann[6:-1]
        parts = _split_top_level_commas(inner)
        resolved = [t for t in (inferred_type_from_ann(p.strip()) for p in parts) if t is not None]
        return TyTuple(tuple(resolved))

    if ann.startswith("type[") and ann.endswith("]"):
        inner = ann[5:-1].strip()
        inner_ty = inferred_type_from_ann(inner)
        if inner_ty is None and inner and all(c.isalnum() or c == "_" for c in inner) and inner[0].isalpha():
            inner_ty = TyNamedInstance(inner)
        return TyTypeValOf(inner_ty) if inner_ty is not None else None

    if ann.startswith("function"):
        return _parse_fn_type_ann(ann[8:])

    result = {
        "int": TyInt(), "float": TyFloat(), "str": TyStr(), "bool": TyBool(),
        "None": TyNone(), "list": TyList(), "dict": TyDict(), "set": TySet(),
        "type": TyTypeVal(), "Self": TySelfType(), "Any": TyAny(),
    }.get(ann)
    if result is not None:
        return result
    # Unknown identifier that looks like a class name → treat as instance type.
    if ann and ann[0].isupper() and all(c.isalnum() or c == "_" for c in ann):
        return TyNamedInstance(ann)
    return None
