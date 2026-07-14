# git SHA: 33ef765a635dee99b50fccb937129e07ae6bdefb
"""Runtime value types for the Arrow interpreter."""
from __future__ import annotations
import struct as _struct
from dataclasses import dataclass, field as dc_field
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
# Undefined sentinel (external library interop)
# ---------------------------------------------------------------------------

class _TlUndefined:
    """Singleton sentinel representing Arrow's Undefined value.

    Used when interfacing with external libraries (Python, JS, etc.) where
    a member may be absent or undefined. Cannot be assigned to variables.
    """
    _inst: Optional[_TlUndefined] = None
    def __new__(cls) -> _TlUndefined:
        if cls._inst is None:
            cls._inst = super().__new__(cls)
        return cls._inst
    def __repr__(self) -> str:
        return "Undefined"
    def __bool__(self) -> bool:
        return False

UNDEFINED: _TlUndefined = _TlUndefined()


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
class TlFixedList:
    """Read-only fixed-size list; created by casting list[T] => fixed_list[T]."""
    items: list  # list[Value] — do not mutate after creation

    def __repr__(self) -> str:
        return f"fixed_list[{', '.join(_repr_val(x) for x in self.items)}]"


@dataclass
class TlDict:
    key_type: str = "Any"
    item_type: str = "Any"
    keys: list = dc_field(default_factory=list)   # list[Value]
    values: list = dc_field(default_factory=list) # list[Value]

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
    captured_env: dict = dc_field(default_factory=dict)  # dict[str, CapturedVar]
    is_static: bool = False
    is_class_method: bool = False
    is_python: bool = False
    return_type: Optional[str] = None  # for cs-dll return_type dispatch

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
    captured_env: dict = dc_field(default_factory=dict)

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


# ---------------------------------------------------------------------------
# C ABI raw field layout (mirrors src/interpreter/value/instance.rs)
# ---------------------------------------------------------------------------

# annotation → (signed struct fmt, unsigned struct fmt, byte width, is_float)
# Arrow `int`/`float` are 8 bytes; C ABI types (int32 etc.) use their declared width.
_RAW_WIDTH_TABLE: dict[str, tuple[str, str, int, bool]] = {
    "int":     ("<q", "<Q", 8, False),
    "float":   ("<d", "<d", 8, True),
    "int8":    ("<b", "<B", 1, False),
    "int16":   ("<h", "<H", 2, False),
    "int32":   ("<i", "<I", 4, False),
    "int64":   ("<q", "<Q", 8, False),
    "uint8":   ("<B", "<B", 1, False),
    "uint16":  ("<H", "<H", 2, False),
    "uint32":  ("<I", "<I", 4, False),
    "uint64":  ("<Q", "<Q", 8, False),
    "float32": ("<f", "<f", 4, True),
    "float64": ("<d", "<d", 8, True),
}


@dataclass(frozen=True)
class RawFieldDesc:
    """Position and storage format of one field inside the raw block."""
    byte_offset: int
    fmt: str        # struct format for decoding (sign-aware)
    fmt_store: str  # struct format for encoding (unsigned for ints — wraps like Rust `as` casts)
    width: int      # byte width
    is_float: bool


@dataclass
class RawLayout:
    """Raw-block layout descriptor of a class.

    Attached only to classes whose own fields are all primitive
    (int/float/C ABI types), with no trait inheritance and at most
    24 fields (init-bitmap constraint).
    """
    fields: list        # list[RawFieldDesc], slot index order (= declaration order)
    total_bytes: int    # total field area bytes (incl. tail padding, multiple of 8)

    @staticmethod
    def from_fields(fields: list) -> Optional["RawLayout"]:
        """Build a layout from a declaration-order list of (name, type_ann).

        Returns None if any field is non-primitive or there are more than 24.
        """
        if not fields or len(fields) > 24:
            return None
        descs: list[RawFieldDesc] = []
        offset = 0
        for _, ann in fields:
            entry = _RAW_WIDTH_TABLE.get(ann)
            if entry is None:
                return None
            fmt, fmt_store, w, is_float = entry
            # C alignment rule: round offset up to the type width
            offset = (offset + w - 1) // w * w
            descs.append(RawFieldDesc(
                byte_offset=offset, fmt=fmt, fmt_store=fmt_store,
                width=w, is_float=is_float,
            ))
            offset += w
        total_bytes = (offset + 7) // 8 * 8
        return RawLayout(fields=descs, total_bytes=total_bytes)


def raw_layouts_compatible(a: "RawLayout", b: "RawLayout") -> bool:
    """True if two raw layouts match structurally (field count, offsets, widths).

    A match means an instance's raw block can be passed zero-copy as a
    C struct pointer.
    """
    return (
        a.total_bytes == b.total_bytes
        and len(a.fields) == len(b.fields)
        and all(
            x.byte_offset == y.byte_offset and x.fmt == y.fmt
            for x, y in zip(a.fields, b.fields)
        )
    )


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
    # Offset-based field access (mirrors Rust ClassValue)
    field_index: dict = dc_field(default_factory=dict)    # dict[str, int] — name → Vec slot index
    field_count: int = 0                                   # total Vec slot count per instance
    field_mutability_vec: list = dc_field(default_factory=list)  # list[bool] — per-slot original mutability
    # C ABI raw block layout (only for all-primitive classes with no bases)
    raw_layout: Optional[RawLayout] = None

    def __repr__(self) -> str:
        return f"<class {self.name}>"


@dataclass
class TlInstance:
    cls: TlClass
    fields: list   # list[Optional[list]] — each slot: None or [value, is_mutable]
    immutable: bool = False
    # Raw block (classes with raw_layout): C ABI-laid-out field bytes.
    # `fields` stays empty for such instances.
    raw: Optional[bytearray] = None
    raw_init_mask: int = 0   # init bitmap for raw slots (max 24)

    @staticmethod
    def new_empty(cls: "TlClass", immutable: bool = False) -> "TlInstance":
        """Create an instance with all slots uninitialized (mirrors InstanceData::new_empty)."""
        if cls.raw_layout is not None:
            return TlInstance(cls=cls, fields=[], immutable=immutable,
                              raw=bytearray(cls.raw_layout.total_bytes))
        return TlInstance(cls=cls, fields=[None] * cls.field_count, immutable=immutable)

    def has_raw_layout(self) -> bool:
        return self.raw is not None

    def slot_initialized(self, idx: int) -> bool:
        """Whether slot idx has been written (raw: init bitmap, boxed: non-None slot)."""
        if self.raw is not None:
            return (self.raw_init_mask >> idx) & 1 != 0
        return idx < len(self.fields) and self.fields[idx] is not None

    def field_value(self, idx: int):
        """Read slot idx. Returns MISSING when the slot is uninitialized.

        Raw classes decode with width conversion (sign extension / f32→f64).
        """
        if self.raw is not None:
            if not self.slot_initialized(idx):
                return MISSING
            layout = self.cls.raw_layout
            if layout is None or idx >= len(layout.fields):
                return MISSING
            desc = layout.fields[idx]
            v = _struct.unpack_from(desc.fmt, self.raw, desc.byte_offset)[0]
            return float(v) if desc.is_float else int(v)
        entry = self.fields[idx] if idx < len(self.fields) else None
        return entry[0] if entry is not None else MISSING

    def field_mutable(self, idx: int) -> Optional[bool]:
        """Mutability flag of slot idx (None when uninitialized)."""
        if self.raw is not None:
            if not self.slot_initialized(idx):
                return None
            if self.immutable:
                return False
            fm = self.cls.field_mutability_vec
            return fm[idx] if idx < len(fm) else True
        entry = self.fields[idx] if idx < len(self.fields) else None
        return entry[1] if entry is not None else None

    def store_field(self, idx: int, val, mutable: bool) -> bool:
        """Raw store to slot idx (no mutability check).

        Returns False when a raw class slot's format doesn't match the value type.
        """
        if self.raw is not None:
            layout = self.cls.raw_layout
            if layout is None or idx >= len(layout.fields):
                return False
            desc = layout.fields[idx]
            if desc.is_float:
                # int → float field auto-promotion (like Rust)
                if isinstance(val, bool) or not isinstance(val, (int, float)):
                    return False
                _struct.pack_into(desc.fmt, self.raw, desc.byte_offset, float(val))
            else:
                if isinstance(val, bool) or not isinstance(val, int):
                    return False
                # width truncation with wraparound (mirror Rust `as` casts)
                masked = val & ((1 << (desc.width * 8)) - 1)
                _struct.pack_into(desc.fmt_store, self.raw, desc.byte_offset, masked)
            self.raw_init_mask |= 1 << idx
            return True
        if idx >= len(self.fields):
            return False
        self.fields[idx] = [val, mutable]
        return True

    def __repr__(self) -> str:
        cls_name = self.cls.name
        if self.cls.new_type_base is not None:
            # new_type: show as ClassName(inner_value)
            idx = self.cls.field_index.get("__value__")
            if idx is not None and self.slot_initialized(idx):
                return f"{cls_name}({_repr_val(self.field_value(idx))})"
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
class TlProtocol:
    name: str

    def __repr__(self) -> str:
        return f"<protocol '{self.name}'>"


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
    content: bytearray = dc_field(default_factory=bytearray)
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

@dataclass
class TlComplex:
    """Complex number: real + imag*j."""
    real: float
    imag: float

    def __repr__(self) -> str:
        def _fmt(f: float) -> str:
            f = 0.0 if f == 0.0 else f  # normalize -0.0
            if f == int(f) and abs(f) < 1e15:
                return f"{f:.1f}"
            return str(f)
        re = 0.0 if self.real == 0.0 else self.real
        im = 0.0 if self.imag == 0.0 else self.imag
        if im >= 0:
            return f"({_fmt(re)}+{_fmt(im)}j)"
        return f"({_fmt(re)}-{_fmt(abs(im))}j)"


@dataclass
class TlSignal:
    """Runtime value for Signal[T]: holds a list of registered handlers."""
    # Each entry: (func: Value, is_once: bool, is_async: bool)
    handlers: list = dc_field(default_factory=list)
    # Values queued by emit_async(); drained by EventLoop.run()
    async_queue: list = dc_field(default_factory=list)

    def __repr__(self) -> str:
        return f"<Signal handlers={len(self.handlers)}>"


@dataclass
class TlEventLoop:
    """Runtime value for the EventLoop singleton."""
    # (TlSignal, value) pairs queued by emit_async()
    signal_queue: list = dc_field(default_factory=list)
    # Zero-arg callables posted via EventLoop.post()
    post_queue: list = dc_field(default_factory=list)

    def __repr__(self) -> str:
        return "<EventLoop>"


@dataclass
class TlCsObject:
    """A C# object accessed via a bridge DLL (cs-dll) or proc IPC (cs-proc)."""
    handle: int        # i64 object handle
    class_name: str    # C# class name
    bridge_path: str   # path to the native DLL (cs-dll) or proc exe (cs-proc)
    cls: "TlClass"     # Arrow class stub (for return_type lookup)
    is_proc: bool = False  # True = cs-proc IPC, False = cs-dll direct

    def __repr__(self) -> str:
        kind = "cs-proc" if self.is_proc else "cs-dll"
        return f"<CsObject({kind}) {self.class_name} #{self.handle}>"


@dataclass
class TlResultVal:
    """Result[T, E] 値。Ok(value) または Err(error) で生成される。"""
    ok: bool
    inner: "Value"

    def __repr__(self) -> str:
        label = "Ok" if self.ok else "Err"
        return f"{label}({self.inner!r})"


Value = (
    int | float | TlComplex | str | bool | type(None) |
    TlList | TlFixedList | TlDict | TlTuple | TlSet |
    TlFunction | TlOverloadedFn | TlGeneratorFn | TlGenerator |
    TlClass | TlInstance |
    TlType | TlTrait | TlProtocol |
    TlTemplateFn | TlTemplateGenFn | TlTemplateClass |
    TlNamespace | TlSlice | TlFileObject |
    TlSignal | TlEventLoop | TlCsObject | TlResultVal
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
    if isinstance(a, TlComplex) and isinstance(b, TlComplex):
        return a.real == b.real and a.imag == b.imag
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
    if isinstance(v, TlComplex):
        return "complex"
    if isinstance(v, str):
        return "str"
    if isinstance(v, TlFixedList):
        return "fixed_list"
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
    if isinstance(v, TlProtocol):
        return "protocol"
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
    if isinstance(v, TlSignal):
        return "Signal"
    if isinstance(v, TlEventLoop):
        return "EventLoop"
    if isinstance(v, TlResultVal):
        return "Ok" if v.ok else "Err"
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
    if isinstance(v, TlComplex):
        return repr(v)
    if isinstance(v, str):
        return v
    if isinstance(v, TlFixedList):
        return "[" + ", ".join(display(x) for x in v.items) + "]"
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
        if cls.new_type_base is not None:
            idx = cls.field_index.get("__value__")
            if idx is not None and v.slot_initialized(idx):
                return f"{cls.name}({display(v.field_value(idx))})"
        return f"<{cls.name} object>"
    if isinstance(v, TlClass):
        return f"<class {v.name}>"
    if isinstance(v, TlType):
        return f"<type '{v.name}'>"
    if isinstance(v, TlProtocol):
        return f"<protocol '{v.name}'>"
    if isinstance(v, TlNamespace):
        return f"<module '{v.name}'>"
    if isinstance(v, TlSlice):
        return repr(v)
    if isinstance(v, TlResultVal):
        label = "Ok" if v.ok else "Err"
        return f"{label}({display(v.inner)})"
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
    if isinstance(v, TlComplex):
        return v.real != 0.0 or v.imag != 0.0
    if isinstance(v, str):
        return len(v) > 0
    if isinstance(v, TlFixedList):
        return len(v.items) > 0
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
    if isinstance(v, TlComplex):
        return TlComplex(real=v.real, imag=v.imag)
    if isinstance(v, TlFixedList):
        return TlFixedList(items=[deep_clone(x) for x in v.items])
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
        if v.raw is not None:
            # Raw block is POD: clone = memcpy (flags and raw fields all kept)
            return TlInstance(cls=v.cls, fields=[], immutable=v.immutable,
                              raw=bytearray(v.raw), raw_init_mask=v.raw_init_mask)
        new_fields = [
            [deep_clone(slot[0]), slot[1]] if slot is not None else None
            for slot in v.fields
        ]
        return TlInstance(cls=v.cls, fields=new_fields, immutable=v.immutable)
    # Functions, classes, namespaces, etc. are shared by identity
    return v


def deep_clone_unfrozen(v: "Value") -> "Value":
    """copy() メソッド用のディープコピー。フリーズ状態をリセットして新鮮な可変インスタンスを返す。

    deep_clone との違い:
    - TlInstance: immutable=False にリセットし、フィールド可変性をクラス定義から復元する
    - その他: deep_clone と同様
    """
    if v is None or isinstance(v, (bool, int, float, str)):
        return v
    if isinstance(v, TlComplex):
        return TlComplex(real=v.real, imag=v.imag)
    if isinstance(v, TlFixedList):
        return TlFixedList(items=[deep_clone_unfrozen(x) for x in v.items])
    if isinstance(v, TlList):
        return TlList(items=[deep_clone_unfrozen(x) for x in v.items])
    if isinstance(v, TlDict):
        return TlDict(
            key_type=v.key_type,
            item_type=v.item_type,
            keys=[deep_clone_unfrozen(k) for k in v.keys],
            values=[deep_clone_unfrozen(val) for val in v.values],
        )
    if isinstance(v, TlTuple):
        return TlTuple(values=[deep_clone_unfrozen(x) for x in v.values])
    if isinstance(v, TlSet):
        return TlSet(items=[deep_clone_unfrozen(x) for x in v.items])
    if isinstance(v, TlSlice):
        return TlSlice(
            begin=deep_clone_unfrozen(v.begin) if v.begin is not None else None,
            end=deep_clone_unfrozen(v.end) if v.end is not None else None,
            step=deep_clone_unfrozen(v.step) if v.step is not None else None,
        )
    if isinstance(v, TlInstance):
        cls = v.cls
        if v.raw is not None:
            # Raw fields carry no per-slot mutability; resetting immutable suffices
            return TlInstance(cls=cls, fields=[], immutable=False,
                              raw=bytearray(v.raw), raw_init_mask=v.raw_init_mask)
        new_fields = [
            [deep_clone_unfrozen(slot[0]),
             cls.field_mutability_vec[i] if i < len(cls.field_mutability_vec) else True]
            if slot is not None else None
            for i, slot in enumerate(v.fields)
        ]
        return TlInstance(cls=cls, fields=new_fields, immutable=False)
    return v
