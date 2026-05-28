# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Runtime value types for the test_lang interpreter."""
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional, TYPE_CHECKING

if TYPE_CHECKING:
    from ..ast import Param, TemplateParam, Stmt, Accessibility

# ---------------------------------------------------------------------------
# Sentinel for "variable not found" / "key absent"
# ---------------------------------------------------------------------------

class _Missing:
    _inst: Optional[_Missing] = None
    def __new__(cls) -> _Missing:
        if cls._inst is None:
            cls._inst = super().__new__(cls)
        return cls._inst
    def __repr__(self) -> str:
        return "<MISSING>"

MISSING: _Missing = _Missing()


# ---------------------------------------------------------------------------
# Captured-variable cells (closure support)
# ---------------------------------------------------------------------------

@dataclass
class CapturedImm:
    """Deep copy of an immutable variable captured at closure creation."""
    value: "Value"

@dataclass
class CapturedMut:
    """Shared mutable cell for a mut variable captured by a closure."""
    cell: list  # single-element list: cell[0] is the current value

CapturedVar = CapturedImm | CapturedMut


# ---------------------------------------------------------------------------
# Compound value types
# ---------------------------------------------------------------------------

@dataclass
class TlList:
    items: list  # list[Value]

    def __repr__(self) -> str:
        return f"[{', '.join(_repr_val(x) for x in self.items)}]"


@dataclass
class TlDict:
    key_type: str = "Any"
    item_type: str = "Any"
    keys: list = field(default_factory=list)   # list[Value]
    values: list = field(default_factory=list) # list[Value]

    def get(self, key: "Value") -> "Value | _Missing":
        for i, k in enumerate(self.keys):
            if _values_equal(k, key):
                return self.values[i]
        return MISSING

    def set(self, key: "Value", value: "Value") -> None:
        for i, k in enumerate(self.keys):
            if _values_equal(k, key):
                self.values[i] = value
                return
        self.keys.append(key)
        self.values.append(value)

    def remove(self, key: "Value") -> bool:
        for i, k in enumerate(self.keys):
            if _values_equal(k, key):
                del self.keys[i]
                del self.values[i]
                return True
        return False

    def __repr__(self) -> str:
        pairs = ", ".join(
            f"{_repr_val(k)}: {_repr_val(v)}"
            for k, v in zip(self.keys, self.values)
        )
        return "{" + pairs + "}"


@dataclass
class TlTuple:
    values: list  # list[Value]

    def __repr__(self) -> str:
        if len(self.values) == 1:
            return f"({_repr_val(self.values[0])},)"
        return "(" + ", ".join(_repr_val(v) for v in self.values) + ")"


@dataclass
class TlSet:
    items: list  # list[Value] — no duplicates, ordered by insertion

    def add(self, v: "Value") -> None:
        if not self.contains(v):
            self.items.append(v)

    def contains(self, v: "Value") -> bool:
        return any(_values_equal(x, v) for x in self.items)

    def discard(self, v: "Value") -> None:
        for i, x in enumerate(self.items):
            if _values_equal(x, v):
                del self.items[i]
                return

    def remove(self, v: "Value") -> bool:
        for i, x in enumerate(self.items):
            if _values_equal(x, v):
                del self.items[i]
                return True
        return False

    def __repr__(self) -> str:
        if not self.items:
            return "set()"
        return "{" + ", ".join(_repr_val(x) for x in self.items) + "}"


@dataclass
class TlFunction:
    name: str
    params: list         # list[Param]
    body: list           # list[Stmt]
    captured_env: dict = field(default_factory=dict)  # dict[str, CapturedVar]
    is_static: bool = False
    is_class_method: bool = False
    is_python: bool = False

    def __repr__(self) -> str:
        return f"<function {self.name}>"


@dataclass
class TlOverloadedFn:
    overloads: list  # list[TlFunction]

    def __repr__(self) -> str:
        return f"<overloaded function {self.overloads[0].name if self.overloads else '?'}>"


@dataclass
class TlGeneratorFn:
    name: str
    params: list   # list[Param]
    body: list     # list[Stmt]
    captured_env: dict = field(default_factory=dict)

    def __repr__(self) -> str:
        return f"<generator function {self.name}>"


@dataclass
class TlGenerator:
    values: list  # list[Value] — all yielded values collected upfront
    index: int = 0

    def __repr__(self) -> str:
        return f"<generator object>"


@dataclass
class TlTemplateFn:
    name: str
    template_params: list  # list[TemplateParam]
    params: list           # list[Param]
    body: list             # list[Stmt]

    def __repr__(self) -> str:
        return f"<template function {self.name}>"


@dataclass
class TlTemplateGenFn:
    name: str
    template_params: list
    params: list
    body: list

    def __repr__(self) -> str:
        return f"<template generator {self.name}>"


@dataclass
class TlTemplateClass:
    name: str
    template_params: list
    bases: list  # list[str]
    body: list   # list[Stmt]

    def __repr__(self) -> str:
        return f"<template class {self.name}>"


@dataclass
class TlClass:
    name: str
    bases: list    # list[str]
    methods: dict  # dict[str, list[TlFunction]]
    gen_methods: dict  # dict[str, TlGeneratorFn]
    field_defaults: list  # list[(str, Value, bool)] — (name, default_value, is_mutable)
    class_vars: dict       # dict[str, Value]
    field_mutability: dict # dict[str, bool]
    field_access: dict     # dict[str, Accessibility]
    method_access: dict    # dict[str, Accessibility]
    static_method_names: set  # set[str]
    class_method_names: set   # set[str]
    static_vars: dict      # dict[str, list]  — mutable cells [value]
    new_type_base: Optional[str] = None

    def __repr__(self) -> str:
        return f"<class {self.name}>"


@dataclass
class TlInstance:
    cls: TlClass
    fields: dict   # dict[str, [Value, bool]]  — {name: [value, is_mutable]}
    immutable: bool = False

    def __repr__(self) -> str:
        cls_name = self.cls.name
        if self.cls.new_type_base is not None:
            # new_type: show as ClassName(inner_value)
            inner = self.fields.get("__value__")
            if inner:
                return f"{cls_name}({_repr_val(inner[0])})"
        return f"<{cls_name} object>"


@dataclass
class TlType:
    name: str  # "int", "str", "float", "bool", "None"

    def __repr__(self) -> str:
        return f"<type '{self.name}'>"


@dataclass
class TlTrait:
    name: str

    def __repr__(self) -> str:
        return f"<trait {self.name}>"


@dataclass
class TlNamespace:
    name: str
    members: dict  # dict[str, Value]

    def __repr__(self) -> str:
        return f"<module '{self.name}'>"


@dataclass
class TlSlice:
    begin: Optional["Value"]
    end: Optional["Value"]
    step: Optional["Value"]

    def __repr__(self) -> str:
        b = _repr_val(self.begin) if self.begin is not None else ""
        e = _repr_val(self.end) if self.end is not None else ""
        s = _repr_val(self.step) if self.step is not None else ""
        if s:
            return f"slice({b}, {e}, {s})"
        return f"slice({b}, {e})"


@dataclass
class TlFileObject:
    path: str
    mode: str       # "r", "w", "rw", "rw_new", "rw_trunc"
    content: bytearray = field(default_factory=bytearray)
    pointer: int = 0
    is_closed: bool = False
    text_mode: bool = True

    def close(self) -> None:
        if self.is_closed:
            return
        self.is_closed = True
        if self.mode in ("w", "rw", "rw_new", "rw_trunc"):
            with open(self.path, "wb") as f:
                f.write(self.content)

    def __repr__(self) -> str:
        return f"<file '{self.path}'>"


# ---------------------------------------------------------------------------
# Value type alias
# ---------------------------------------------------------------------------

Value = (
    int | float | str | bool | type(None) |
    TlList | TlDict | TlTuple | TlSet |
    TlFunction | TlOverloadedFn | TlGeneratorFn | TlGenerator |
    TlClass | TlInstance |
    TlType | TlTrait |
    TlTemplateFn | TlTemplateGenFn | TlTemplateClass |
    TlNamespace | TlSlice | TlFileObject
)


# ---------------------------------------------------------------------------
# Utility: value equality (for dict keys / set membership)
# ---------------------------------------------------------------------------

def _values_equal(a: "Value", b: "Value") -> bool:
    if a is None and b is None:
        return True
    if a is None or b is None:
        return False
    if type(a) is bool and type(b) is bool:
        return a == b
    if type(a) is bool or type(b) is bool:
        return False
    if type(a) is int and type(b) is int:
        return a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return float(a) == float(b)
    if isinstance(a, str) and isinstance(b, str):
        return a == b
    if isinstance(a, TlTuple) and isinstance(b, TlTuple):
        return len(a.values) == len(b.values) and all(
            _values_equal(x, y) for x, y in zip(a.values, b.values)
        )
    return False


def _repr_val(v: "Value") -> str:
    if v is None:
        return "None"
    if type(v) is bool:
        return "True" if v else "False"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        r = repr(v)
        if "." not in r and "e" not in r:
            r += ".0"
        return r
    if isinstance(v, str):
        return repr(v)
    return repr(v)


def type_name(v: "Value") -> str:
    """Return the language-level type name of a value."""
    if v is None:
        return "None"
    if type(v) is bool:
        return "bool"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    if isinstance(v, TlList):
        return "list"
    if isinstance(v, TlDict):
        return "dict"
    if isinstance(v, TlTuple):
        return "tuple"
    if isinstance(v, TlSet):
        return "set"
    if isinstance(v, (TlFunction, TlOverloadedFn, TlGeneratorFn)):
        return "function"
    if isinstance(v, (TlTemplateFn, TlTemplateGenFn)):
        return "function"
    if isinstance(v, TlClass):
        return v.name
    if isinstance(v, TlInstance):
        return v.cls.name
    if isinstance(v, TlType):
        return f"type({v.name})"
    if isinstance(v, TlTrait):
        return f"trait({v.name})"
    if isinstance(v, TlTemplateClass):
        return v.name
    if isinstance(v, TlGenerator):
        return "generator"
    if isinstance(v, TlNamespace):
        return f"module"
    if isinstance(v, TlSlice):
        return "slice"
    if isinstance(v, TlFileObject):
        return "file"
    return "unknown"


def display(v: "Value") -> str:
    """Return the user-visible string representation (like print())."""
    if v is None:
        return "None"
    if type(v) is bool:
        return "True" if v else "False"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        s = repr(v)
        if "." not in s and "e" not in s:
            s += ".0"
        return s
    if isinstance(v, str):
        return v
    if isinstance(v, TlList):
        return "[" + ", ".join(display(x) for x in v.items) + "]"
    if isinstance(v, TlDict):
        pairs = ", ".join(
            f"{display(k)}: {display(val)}"
            for k, val in zip(v.keys, v.values)
        )
        return "{" + pairs + "}"
    if isinstance(v, TlTuple):
        if len(v.values) == 1:
            return f"({display(v.values[0])},)"
        return "(" + ", ".join(display(x) for x in v.values) + ")"
    if isinstance(v, TlSet):
        if not v.items:
            return "set()"
        return "{" + ", ".join(display(x) for x in v.items) + "}"
    if isinstance(v, TlInstance):
        cls = v.cls
        # Check for __str__ method
        if "__str__" in cls.methods:
            return "<instance>"  # caller should invoke __str__
        if cls.new_type_base is not None:
            inner = v.fields.get("__value__")
            if inner:
                return f"{cls.name}({display(inner[0])})"
        return f"<{cls.name} object>"
    if isinstance(v, TlClass):
        return f"<class {v.name}>"
    if isinstance(v, TlType):
        return f"<type '{v.name}'>"
    if isinstance(v, TlNamespace):
        return f"<module '{v.name}'>"
    if isinstance(v, TlSlice):
        return repr(v)
    return repr(v)


def is_truthy(v: "Value") -> bool:
    """Return whether a value is truthy in the language."""
    if v is None:
        return False
    if type(v) is bool:
        return v
    if isinstance(v, int):
        return v != 0
    if isinstance(v, float):
        return v != 0.0
    if isinstance(v, str):
        return len(v) > 0
    if isinstance(v, TlList):
        return len(v.items) > 0
    if isinstance(v, TlDict):
        return len(v.keys) > 0
    if isinstance(v, TlTuple):
        return len(v.values) > 0
    if isinstance(v, TlSet):
        return len(v.items) > 0
    return True  # functions, classes, instances are truthy


def deep_clone(v: "Value") -> "Value":
    """Create a fully independent deep copy of a value."""
    if v is None or isinstance(v, (bool, int, float, str)):
        return v
    if isinstance(v, TlList):
        return TlList(items=[deep_clone(x) for x in v.items])
    if isinstance(v, TlDict):
        return TlDict(
            key_type=v.key_type,
            item_type=v.item_type,
            keys=[deep_clone(k) for k in v.keys],
            values=[deep_clone(val) for val in v.values],
        )
    if isinstance(v, TlTuple):
        return TlTuple(values=[deep_clone(x) for x in v.values])
    if isinstance(v, TlSet):
        return TlSet(items=[deep_clone(x) for x in v.items])
    if isinstance(v, TlSlice):
        return TlSlice(
            begin=deep_clone(v.begin) if v.begin is not None else None,
            end=deep_clone(v.end) if v.end is not None else None,
            step=deep_clone(v.step) if v.step is not None else None,
        )
    if isinstance(v, TlInstance):
        new_fields = {
            name: [deep_clone(fv[0]), fv[1]]
            for name, fv in v.fields.items()
        }
        return TlInstance(cls=v.cls, fields=new_fields, immutable=v.immutable)
    # Functions, classes, namespaces, etc. are shared by identity
    return v
