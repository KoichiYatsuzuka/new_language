"""cs_assembly.py — Python port of src/parser/cs_assembly.rs

Reads a .NET managed DLL (ECMA-335, Partition II) and generates Arrow
type stubs (list[Stmt]) for use with import[cs-dll].

Coverage:
  - TypeDef   → ClassDef / TraitDef stubs
  - MethodDef → FnDef stubs (instance + static)
  - Param     → parameter names
  - TypeRef   → external type name resolution
  - GenericParam → template parameter names
  - MethodSemantics + PropertyDef → getter/setter
"""

from __future__ import annotations
import struct
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

# ---------------------------------------------------------------------------
# PE / ECMA-335 constants
# ---------------------------------------------------------------------------

PE_SIG   = 0x00004550  # "PE\0\0"
BSJB     = 0x424A5342  # "BSJB"
CLI_DIR  = 14          # data directory index for CLI header

T_MODULE        = 0x00
T_TYPEREF       = 0x01
T_TYPEDEF       = 0x02
T_FIELD         = 0x04
T_METHODDEF     = 0x06
T_PARAM         = 0x08
T_INTERFACEIMPL = 0x09
T_MEMBERREF     = 0x0A
T_PROPERTY      = 0x17
T_METHODSEM     = 0x18
T_MODULEREF     = 0x1A
T_TYPESPEC      = 0x1B
T_ASSEMBLY      = 0x20
T_ASSEMBLYREF   = 0x23
T_GENERICPARAM  = 0x2A
T_STANDALONESIG = 0x11

TD_PUBLIC        = 0x01
TD_NESTED_PUBLIC = 0x02
TD_INTERFACE     = 0x20

MD_STATIC       = 0x10
MD_SPECIAL_NAME = 0x0800

SEM_GETTER   = 0x02
SEM_ADDON    = 0x08
SEM_REMOVEON = 0x10

PARAM_OUT = 0x0002

ET_VOID      = 0x01
ET_BOOLEAN   = 0x02
ET_CHAR      = 0x03
ET_I1        = 0x04
ET_U1        = 0x05
ET_I2        = 0x06
ET_U2        = 0x07
ET_I4        = 0x08
ET_U4        = 0x09
ET_I8        = 0x0A
ET_U8        = 0x0B
ET_R4        = 0x0C
ET_R8        = 0x0D
ET_STRING    = 0x0E
ET_PTR       = 0x0F
ET_BYREF     = 0x10
ET_VALUETYPE = 0x11
ET_CLASS     = 0x12
ET_VAR       = 0x13
ET_ARRAY     = 0x14
ET_GENERICINST = 0x15
ET_I         = 0x18
ET_U         = 0x19
ET_FNPTR     = 0x1B
ET_OBJECT    = 0x1C
ET_SZARRAY   = 0x1D
ET_MVAR      = 0x1E
ET_CMOD_REQD = 0x1F
ET_CMOD_OPT  = 0x20
ET_SENTINEL  = 0x41
ET_PINNED    = 0x45

# ---------------------------------------------------------------------------
# Binary helpers
# ---------------------------------------------------------------------------

def u16le(data: bytes, off: int) -> int:
    return struct.unpack_from("<H", data, off)[0]

def u32le(data: bytes, off: int) -> int:
    return struct.unpack_from("<I", data, off)[0]

def decompress_uint(data: bytes, pos: int) -> tuple[int, int]:
    b0 = data[pos]
    if b0 & 0x80 == 0:
        return b0, 1
    elif b0 & 0xC0 == 0x80:
        b1 = data[pos + 1]
        return ((b0 & 0x3F) << 8) | b1, 2
    else:
        b1, b2, b3 = data[pos+1], data[pos+2], data[pos+3]
        return ((b0 & 0x1F) << 24) | (b1 << 16) | (b2 << 8) | b3, 4

# ---------------------------------------------------------------------------
# PE section / RVA resolution
# ---------------------------------------------------------------------------

@dataclass
class PeSection:
    virt_addr: int
    virt_size: int
    raw_addr: int

def rva_to_offset(rva: int, sections: list[PeSection]) -> Optional[int]:
    for s in sections:
        if s.virt_addr <= rva < s.virt_addr + max(s.virt_size, 1):
            return rva - s.virt_addr + s.raw_addr
    return None

def find_metadata_root(data: bytes) -> tuple[int, list[PeSection]]:
    e_lfanew = u32le(data, 0x3C)
    if u32le(data, e_lfanew) != PE_SIG:
        raise ValueError("CsImport: not a PE file")
    coff = e_lfanew + 4
    num_sections  = u16le(data, coff + 2)
    opt_hdr_size  = u16le(data, coff + 16)
    opt_hdr = coff + 20

    magic = u16le(data, opt_hdr)
    data_dirs_off = opt_hdr + (112 if magic == 0x020B else 96)

    cli_rva = u32le(data, data_dirs_off + CLI_DIR * 8)
    if cli_rva == 0:
        raise ValueError("CsImport: no CLI header")

    sections_off = opt_hdr + opt_hdr_size
    sections: list[PeSection] = []
    for i in range(num_sections):
        sh = sections_off + i * 40
        sections.append(PeSection(
            virt_addr = u32le(data, sh + 12),
            virt_size = u32le(data, sh + 8),
            raw_addr  = u32le(data, sh + 20),
        ))

    cli_off = rva_to_offset(cli_rva, sections)
    if cli_off is None:
        raise ValueError("CsImport: cannot resolve CLI header RVA")
    meta_rva = u32le(data, cli_off + 8)
    meta_off = rva_to_offset(meta_rva, sections)
    if meta_off is None:
        raise ValueError("CsImport: cannot resolve metadata root RVA")

    if u32le(data, meta_off) != BSJB:
        raise ValueError("CsImport: invalid metadata root signature (not BSJB)")
    return meta_off, sections

# ---------------------------------------------------------------------------
# Stream discovery
# ---------------------------------------------------------------------------

@dataclass
class Streams:
    tilde_off: int
    strings_off: int
    blob_off: int

def find_streams(data: bytes, meta: int) -> Streams:
    ver_len = u32le(data, meta + 12)
    ver_aligned = (ver_len + 3) & ~3
    pos = meta + 16 + ver_aligned
    num_streams = u16le(data, pos + 2)
    pos += 4

    tilde = strings = blob = None
    for _ in range(num_streams):
        offset = u32le(data, pos)
        _size  = u32le(data, pos + 4)
        pos += 8
        name_start = pos
        while pos < len(data) and data[pos] != 0:
            pos += 1
        name = data[name_start:pos].decode("ascii", errors="replace")
        pos += 1
        pos = (pos + 3) & ~3

        if name in ("#~", "#-"):
            tilde = meta + offset
        elif name == "#Strings":
            strings = meta + offset
        elif name == "#Blob":
            blob = meta + offset

    if tilde is None:
        raise ValueError("CsImport: no #~ stream")
    if strings is None:
        raise ValueError("CsImport: no #Strings stream")
    if blob is None:
        raise ValueError("CsImport: no #Blob stream")
    return Streams(tilde_off=tilde, strings_off=strings, blob_off=blob)

# ---------------------------------------------------------------------------
# #~ stream layout (TildeLayout)
# ---------------------------------------------------------------------------

class TildeLayout:
    def __init__(self):
        self.heap_sizes: int = 0
        self.rows: list[int] = [0] * 64
        self.table_offsets: list[int] = [0] * 64
        self.table_row_sizes: list[int] = [0] * 64

    def str_idx_size(self) -> int: return 4 if self.heap_sizes & 0x01 else 2
    def guid_idx_size(self) -> int: return 4 if self.heap_sizes & 0x02 else 2
    def blob_idx_size(self) -> int: return 4 if self.heap_sizes & 0x04 else 2

    def tbl_idx(self, tbl: int) -> int:
        return 4 if self.rows[tbl] > 0xFFFF else 2

    def coded_idx(self, tables: list[int], tag_bits: int) -> int:
        max_rows = max((self.rows[t] for t in tables), default=0)
        threshold = (1 << (16 - tag_bits)) - 1
        return 4 if max_rows > threshold else 2

    def read_idx(self, data: bytes, off: int, size: int) -> int:
        return u32le(data, off) if size == 4 else u16le(data, off)

    def compute_row_sizes(self) -> None:
        s = self.str_idx_size()
        g = self.guid_idx_size()
        b = self.blob_idx_size()

        tdr   = self.coded_idx([T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2)
        has_s = self.coded_idx([0x14, T_PROPERTY], 1)
        res   = self.coded_idx([T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2)
        tom   = self.coded_idx([T_TYPEDEF, T_METHODDEF], 1)

        td = self.tbl_idx(T_TYPEDEF)
        fi = self.tbl_idx(T_FIELD)
        md = self.tbl_idx(T_METHODDEF)
        pa = self.tbl_idx(T_PARAM)
        pr = self.tbl_idx(T_PROPERTY)
        gp = self.tbl_idx(T_GENERICPARAM)

        rs = self.table_row_sizes
        rs[T_MODULE]        = 2 + s + g + g + g
        rs[T_TYPEREF]       = res + s + s
        rs[T_TYPEDEF]       = 4 + s + s + tdr + fi + md
        rs[T_FIELD]         = 2 + s + b
        rs[T_METHODDEF]     = 4 + 2 + 2 + s + b + pa
        rs[T_PARAM]         = 2 + 2 + s
        rs[T_INTERFACEIMPL] = td + tdr
        mr_cls = self.coded_idx([T_TYPEREF, T_MODULEREF, T_METHODDEF, T_TYPEDEF, T_TYPESPEC], 3)
        rs[T_MEMBERREF]     = mr_cls + s + b
        rs[0x0B]            = 2 + self.coded_idx([T_FIELD, T_PARAM, 0x17], 2) + b
        ca_parent = self.coded_idx([T_METHODDEF,T_FIELD,T_TYPEREF,T_TYPEDEF,T_PARAM,T_INTERFACEIMPL,
                                     T_MEMBERREF,T_MODULE,0x0E,T_PROPERTY,0x14,T_STANDALONESIG,
                                     T_MODULEREF,T_TYPESPEC,T_ASSEMBLY,T_ASSEMBLYREF,T_FIELD,T_PARAM,0x2A], 5)
        rs[0x0C]            = ca_parent + self.coded_idx([T_METHODDEF, T_MEMBERREF], 3) + b
        rs[0x0D]            = self.coded_idx([T_FIELD, T_PARAM], 1) + b
        rs[0x0E]            = 2 + self.coded_idx([T_TYPEDEF, T_METHODDEF, T_ASSEMBLY], 2) + b
        rs[0x0F]            = 2 + 4 + td
        rs[0x10]            = 4 + fi
        rs[T_STANDALONESIG] = b
        rs[0x12]            = td + self.tbl_idx(0x14)
        rs[0x14]            = 2 + s + tdr
        rs[0x15]            = td + pr
        rs[T_PROPERTY]      = 2 + s + b
        rs[T_METHODSEM]     = 2 + md + has_s
        rs[0x19]            = td + self.coded_idx([T_METHODDEF,T_MEMBERREF],1)*2
        rs[T_MODULEREF]     = s
        rs[T_TYPESPEC]      = b
        rs[0x1C]            = 2 + self.coded_idx([T_FIELD,T_METHODDEF],1) + s + self.tbl_idx(T_MODULEREF)
        rs[0x1D]            = 4 + fi
        rs[T_ASSEMBLY]      = 4+2+2+2+2+4+b+s+s
        rs[0x21]            = 4
        rs[0x22]            = 4+4+4
        rs[T_ASSEMBLYREF]   = 2+2+2+2+4+b+s+s+b
        rs[0x24]            = 4 + self.tbl_idx(T_ASSEMBLYREF)
        rs[0x25]            = 4+4+4 + self.tbl_idx(T_ASSEMBLYREF)
        rs[0x26]            = 4+s+b
        rs[0x27]            = 4+4+s+s + self.coded_idx([T_ASSEMBLY,0x26,0x1B,0x27,T_TYPEDEF], 2)
        rs[0x28]            = 4+4+s + self.coded_idx([T_ASSEMBLY,0x26,0x23,0x1B], 2)
        rs[0x29]            = td + td
        rs[T_GENERICPARAM]  = 2+2+tom+s
        rs[0x2B]            = self.coded_idx([T_METHODDEF,T_MEMBERREF],1) + b
        rs[0x2C]            = gp + tdr


def parse_tilde(data: bytes, tilde_start: int) -> TildeLayout:
    layout = TildeLayout()
    layout.heap_sizes = data[tilde_start + 6]
    valid_lo = u32le(data, tilde_start + 8)
    valid_hi = u32le(data, tilde_start + 12)
    valid = valid_lo | (valid_hi << 32)

    pos = tilde_start + 24
    for i in range(64):
        if valid & (1 << i):
            layout.rows[i] = u32le(data, pos)
            pos += 4

    layout.compute_row_sizes()

    # Compute table offsets
    off = pos  # tables data starts right after row counts
    for i in range(64):
        if valid & (1 << i):
            layout.table_offsets[i] = off
            off += layout.table_row_sizes[i] * layout.rows[i]

    return layout

# ---------------------------------------------------------------------------
# Heap accessors
# ---------------------------------------------------------------------------

def read_string(data: bytes, strings_off: int, idx: int) -> str:
    start = strings_off + idx
    end = data.index(0, start)
    return data[start:end].decode("utf-8", errors="replace")

def read_blob(data: bytes, blob_off: int, idx: int) -> bytes:
    start = blob_off + idx
    length, hdr = decompress_uint(data, start)
    return data[start + hdr : start + hdr + length]

# ---------------------------------------------------------------------------
# Intermediate data structures
# ---------------------------------------------------------------------------

@dataclass
class CsParam:
    sequence: int
    name: str
    flags: int

@dataclass
class PropertyRole:
    kind: str     # "getter" | "setter" | "event"
    name: str

@dataclass
class CsMethod:
    name: str
    is_static: bool
    is_public: bool
    flags: int
    sig_blob_idx: int
    param_list_start: int
    param_list_end: int
    generic_param_names: list[str]
    property_role: Optional[PropertyRole] = None

@dataclass
class CsTypeDef:
    name: str
    namespace: str
    flags: int
    method_list_start: int
    method_list_end: int
    generic_param_names: list[str]
    interface_names: list[str] = field(default_factory=list)

# ---------------------------------------------------------------------------
# Type signature reader
# ---------------------------------------------------------------------------

class SigReader:
    def __init__(self, blob: bytes, type_names: dict, type_params: list[str], method_params: list[str]):
        self.data = blob
        self.pos = 0
        self.type_names = type_names
        self.type_params = type_params
        self.method_params = method_params

    def peek(self) -> int:
        return self.data[self.pos] if self.pos < len(self.data) else 0

    def eat(self) -> int:
        b = self.peek()
        self.pos += 1
        return b

    def eat_uint(self) -> int:
        v, n = decompress_uint(self.data, self.pos)
        self.pos += n
        return v

    def skip_cmods(self) -> None:
        while self.peek() in (ET_CMOD_REQD, ET_CMOD_OPT):
            self.eat()
            self.eat_uint()

    def parse_type(self) -> str:
        self.skip_cmods()
        b = self.eat()
        if b == ET_VOID:     return "None"
        if b == ET_BOOLEAN:  return "bool"
        if b in (ET_CHAR, ET_STRING): return "str"
        if b in (ET_I1, ET_U1, ET_I2, ET_U2, ET_I4, ET_U4,
                 ET_I8, ET_U8, ET_I, ET_U): return "int"
        if b in (ET_R4, ET_R8): return "float"
        if b == ET_OBJECT: return "Any"
        if b == ET_BYREF:  return self.parse_type()
        if b in (ET_VALUETYPE, ET_CLASS):
            token = self.eat_uint()
            return self.type_names.get(token, "Any")
        if b == ET_GENERICINST:
            self.eat()  # CLASS or VALUETYPE
            token = self.eat_uint()
            base = self.type_names.get(token, "")
            argc = self.eat_uint()
            args = [self.parse_type() for _ in range(argc)]
            return _map_generic(base, args)
        if b == ET_SZARRAY:
            self.skip_cmods()
            elem = self.parse_type()
            return f"list[{elem}]"
        if b == ET_ARRAY:
            elem = self.parse_type()
            rank = self.eat_uint()
            nsizes = self.eat_uint()
            for _ in range(nsizes): self.eat_uint()
            nlb = self.eat_uint()
            for _ in range(nlb): self.eat_uint()
            _ = rank
            return f"list[{elem}]"
        if b == ET_VAR:
            idx = self.eat_uint()
            return self.type_params[idx] if idx < len(self.type_params) else f"T{idx}"
        if b == ET_MVAR:
            idx = self.eat_uint()
            return self.method_params[idx] if idx < len(self.method_params) else f"M{idx}"
        if b == ET_FNPTR:
            self._skip_method_sig()
            return "function"
        if b == ET_PTR:
            self.skip_cmods()
            self.parse_type()
            return "int"  # raw pointer → int handle
        if b in (ET_SENTINEL, ET_PINNED):
            return self.parse_type()
        return "Any"

    def _skip_method_sig(self) -> None:
        calling = self.eat()
        if calling & 0x10:
            self.eat_uint()
        n = self.eat_uint()
        self.parse_type()
        for _ in range(n):
            if self.peek() == ET_SENTINEL:
                self.eat()
            self.parse_type()

    def parse_method_sig(self, _generic_names: list[str]) -> tuple[str, list[tuple[str, bool]]]:
        calling = self.eat()
        if calling & 0x10:
            self.eat_uint()  # generic param count
        n = self.eat_uint()
        ret = self.parse_type()
        params: list[tuple[str, bool]] = []
        for _ in range(n):
            if self.peek() == ET_SENTINEL:
                self.eat()
                continue
            is_byref = self.peek() == ET_BYREF
            ty = self.parse_type()
            params.append((ty, is_byref))
        return ret, params


def _map_generic(base: str, args: list[str]) -> str:
    simple = base.rsplit(".", 1)[-1]
    simple = simple.split("`")[0]
    if simple in ("List","IList","ICollection","IEnumerable","IReadOnlyList",
                  "IReadOnlyCollection","ObservableCollection","Collection",
                  "Queue","Stack","LinkedList","ImmutableList"):
        return f"list[{args[0]}]" if len(args) == 1 else "list"
    if simple in ("Dictionary","IDictionary","IReadOnlyDictionary",
                  "SortedDictionary","ConcurrentDictionary"):
        return f"dict[{args[0]},{args[1]}]" if len(args) == 2 else "dict"
    if simple in ("HashSet","SortedSet","ISet","ImmutableHashSet"):
        return f"set[{args[0]}]" if len(args) == 1 else "set"
    if simple in ("Tuple","ValueTuple"):
        return f"tuple[{','.join(args)}]"
    if simple == "Nullable":
        return f"Option[{args[0]}]" if len(args) == 1 else "Any"
    if simple in ("Task","ValueTask"):
        return args[0] if len(args) == 1 else "None"
    if simple in ("Action","Func","Predicate","EventHandler","Delegate"):
        return "function"
    if simple == "KeyValuePair":
        return f"tuple[{args[0]},{args[1]}]" if len(args) == 2 else "tuple"
    return f"{simple}[{','.join(args)}]" if args else simple

# ---------------------------------------------------------------------------
# Operator name mapping
# ---------------------------------------------------------------------------

_OP_MAP = {
    "op_Addition": "__add__",
    "op_Subtraction": "__sub__",
    "op_Multiply": "__mul__",
    "op_Division": "__truediv__",
    "op_Modulus": "__mod__",
    "op_Equality": "__eq__",
    "op_Inequality": "__ne__",
    "op_LessThan": "__lt__",
    "op_GreaterThan": "__gt__",
    "op_LessThanOrEqual": "__le__",
    "op_GreaterThanOrEqual": "__ge__",
    "op_UnaryNegation": "__neg__",
    "op_UnaryPlus": "__pos__",
    "op_BitwiseAnd": "__and__",
    "op_BitwiseOr": "__or__",
    "op_ExclusiveOr": "__xor__",
    "op_LeftShift": "__lshift__",
    "op_RightShift": "__rshift__",
    "op_OnesComplement": "__invert__",
}

_RESERVED = {"type": "type_", "class": "class_", "fn": "fn_",
             "let": "let_", "mut": "mut_"}

def _sanitize(name: str) -> str:
    return _RESERVED.get(name, name)

# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

def load_cs_assembly(path: Path) -> list:
    """Read a .NET managed DLL and return Arrow stub AST nodes (list[Stmt])."""
    from ..ast import (
        Param as AstParam, TemplateParam, Accessibility,
        StmtFnDef, StmtClassDef, StmtTraitDef, StmtPass,
    )

    data = path.read_bytes()
    meta_off, _sections = find_metadata_root(data)
    streams = find_streams(data, meta_off)
    layout  = parse_tilde(data, streams.tilde_off)

    s_sz  = layout.str_idx_size()
    b_sz  = layout.blob_idx_size()
    fi_sz = layout.tbl_idx(T_FIELD)
    md_sz = layout.tbl_idx(T_METHODDEF)
    pa_sz = layout.tbl_idx(T_PARAM)
    tdr_sz = layout.coded_idx([T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2)
    has_s_sz = layout.coded_idx([0x14, T_PROPERTY], 1)
    tom_sz = layout.coded_idx([T_TYPEDEF, T_METHODDEF], 1)
    res_sz = layout.coded_idx([T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2)
    td_idx_sz = layout.tbl_idx(T_TYPEDEF)

    # --- TypeRef name table ---
    type_names: dict[int, str] = {}
    typeref_rows = layout.rows[T_TYPEREF]
    for row in range(typeref_rows):
        off = layout.table_offsets[T_TYPEREF] + row * layout.table_row_sizes[T_TYPEREF]
        name_idx = layout.read_idx(data, off + res_sz, s_sz)
        ns_idx   = layout.read_idx(data, off + res_sz + s_sz, s_sz)
        name = read_string(data, streams.strings_off, name_idx)
        ns   = read_string(data, streams.strings_off, ns_idx)
        coded = ((row + 1) << 2) | 1
        simple = name.split("`")[0]
        full = f"{ns}.{simple}" if ns else simple
        type_names[coded] = full

    # --- GenericParam table ---
    type_gp:   dict[int, list[tuple[int,str]]] = {}
    method_gp: dict[int, list[tuple[int,str]]] = {}
    gp_rows = layout.rows[T_GENERICPARAM]
    for row in range(gp_rows):
        off = layout.table_offsets[T_GENERICPARAM] + row * layout.table_row_sizes[T_GENERICPARAM]
        number     = u16le(data, off)
        owner_coded = layout.read_idx(data, off + 4, tom_sz)
        name_idx   = layout.read_idx(data, off + 4 + tom_sz, s_sz)
        name = read_string(data, streams.strings_off, name_idx)
        tag   = owner_coded & 0x1
        row_1 = owner_coded >> 1
        if tag == 0:
            type_gp.setdefault(row_1, []).append((number, name))
        else:
            method_gp.setdefault(row_1, []).append((number, name))

    def sorted_gp(m: dict[int,list], key: int) -> list[str]:
        return [n for _,n in sorted(m.get(key, []))]

    # --- TypeDef table ---
    td_rows = layout.rows[T_TYPEDEF]
    typedefs: list[CsTypeDef] = []
    for row in range(td_rows):
        off = layout.table_offsets[T_TYPEDEF] + row * layout.table_row_sizes[T_TYPEDEF]
        flags    = u32le(data, off)
        name_idx = layout.read_idx(data, off + 4, s_sz)
        ns_idx   = layout.read_idx(data, off + 4 + s_sz, s_sz)
        mlist_off = 4 + s_sz + s_sz + tdr_sz + fi_sz
        mlist_start = layout.read_idx(data, off + mlist_off, md_sz)

        name = read_string(data, streams.strings_off, name_idx)
        ns   = read_string(data, streams.strings_off, ns_idx)

        coded = ((row + 1) << 2) | 0
        simple = name.split("`")[0]
        type_names[coded] = simple

        gnames = sorted_gp(type_gp, row + 1)
        typedefs.append(CsTypeDef(
            name=simple,
            namespace=ns,
            flags=flags,
            method_list_start=mlist_start,
            method_list_end=0,
            generic_param_names=gnames,
        ))

    # Fill method_list_end
    md_total = layout.rows[T_METHODDEF] + 1
    for i in range(len(typedefs)):
        typedefs[i].method_list_end = typedefs[i+1].method_list_start if i+1 < len(typedefs) else md_total

    # --- InterfaceImpl (0x09) ---
    ii_rows = layout.rows[T_INTERFACEIMPL]
    for row in range(ii_rows):
        off = layout.table_offsets[T_INTERFACEIMPL] + row * layout.table_row_sizes[T_INTERFACEIMPL]
        td_1       = layout.read_idx(data, off, td_idx_sz)
        iface_coded = layout.read_idx(data, off + td_idx_sz, tdr_sz)
        iface_name  = type_names.get(iface_coded, "")
        simple = iface_name.rsplit(".", 1)[-1]
        if simple and simple != "IDisposable":
            idx = td_1 - 1
            if 0 <= idx < len(typedefs):
                typedefs[idx].interface_names.append(simple)

    # --- Param table ---
    param_rows = layout.rows[T_PARAM]
    all_params: list[CsParam] = []
    for row in range(param_rows):
        off = layout.table_offsets[T_PARAM] + row * layout.table_row_sizes[T_PARAM]
        flags    = u16le(data, off)
        seq      = u16le(data, off + 2)
        name_idx = layout.read_idx(data, off + 4, s_sz)
        name = read_string(data, streams.strings_off, name_idx)
        all_params.append(CsParam(sequence=seq, name=name, flags=flags))
    param_total = param_rows + 1

    # --- PropertyDef + MethodSemantics ---
    method_role: dict[int, PropertyRole] = {}

    if layout.rows[T_METHODSEM] > 0 and layout.rows[T_PROPERTY] > 0:
        prop_names: dict[int, str] = {}
        pr_rows = layout.rows[T_PROPERTY]
        for row in range(pr_rows):
            off = layout.table_offsets[T_PROPERTY] + row * layout.table_row_sizes[T_PROPERTY]
            name_idx = layout.read_idx(data, off + 2, s_sz)
            name = read_string(data, streams.strings_off, name_idx)
            prop_names[row + 1] = name

        ms_rows = layout.rows[T_METHODSEM]
        for row in range(ms_rows):
            off = layout.table_offsets[T_METHODSEM] + row * layout.table_row_sizes[T_METHODSEM]
            sem    = u16le(data, off)
            meth_1 = layout.read_idx(data, off + 2, md_sz)
            assoc  = layout.read_idx(data, off + 2 + md_sz, has_s_sz)
            assoc_tag = assoc & 1
            assoc_row = assoc >> 1

            if assoc_tag == 1:  # Property
                prop_name = prop_names.get(assoc_row, "")
                if prop_name:
                    kind = "getter" if sem & SEM_GETTER else "setter"
                    method_role[meth_1] = PropertyRole(kind=kind, name=prop_name)
            else:  # Event
                if sem & SEM_ADDON or sem & SEM_REMOVEON:
                    method_role[meth_1] = PropertyRole(kind="event", name="")

    # --- MethodDef table ---
    md_rows = layout.rows[T_METHODDEF]
    all_methods: list[CsMethod] = []
    for row in range(md_rows):
        off = layout.table_offsets[T_METHODDEF] + row * layout.table_row_sizes[T_METHODDEF]
        meth_flags = u16le(data, off + 6)
        name_idx   = layout.read_idx(data, off + 8, s_sz)
        sig_idx    = layout.read_idx(data, off + 8 + s_sz, b_sz)
        plist_start = layout.read_idx(data, off + 8 + s_sz + b_sz, pa_sz)

        name     = read_string(data, streams.strings_off, name_idx)
        access   = meth_flags & 0x07
        is_public  = access == 6
        is_static  = bool(meth_flags & MD_STATIC)

        method_1 = row + 1
        gnames   = sorted_gp(method_gp, method_1)
        role     = method_role.pop(method_1, None)

        all_methods.append(CsMethod(
            name=name, is_static=is_static, is_public=is_public,
            flags=meth_flags, sig_blob_idx=sig_idx,
            param_list_start=plist_start, param_list_end=0,
            generic_param_names=gnames, property_role=role,
        ))

    for i in range(len(all_methods)):
        all_methods[i].param_list_end = (
            all_methods[i+1].param_list_start if i+1 < len(all_methods) else param_total
        )

    # --- Stub generation ---
    # C# `ref`/`out` (ELEMENT_TYPE_BYREF) parameters are passed with mutable=True.
    # They are type-checked as Arrow `mut` parameters, so passing a `let`
    # variable is caught statically (same rule as the cpp/rs bridges).
    # Limitation: C# `in` (read-only reference) is also BYREF in the signature,
    # so it is treated as `mut` (errs on the strict side).
    def make_param(name: str, ty: str, mutable: bool = False) -> AstParam:
        return AstParam(name=name, mutable=mutable, type_ann=ty)

    def make_fn_stub(name: str, params: list, ret: str,
                     is_static: bool, is_abstract: bool,
                     tmpl: list) -> StmtFnDef:
        return StmtFnDef(
            name=name,
            template_params=tmpl,
            params=params,
            return_type=ret,
            body=[StmtPass()],
            is_abstract=is_abstract,
            is_static=is_static,
            is_class_method=False,
            decorators=[],
            access=Accessibility.PUBLIC,
        )

    def decode_sig(m: CsMethod, td_gnames: list[str]) -> tuple[str, list[tuple[str,bool]]]:
        blob = read_blob(data, streams.blob_off, m.sig_blob_idx)
        reader = SigReader(blob, type_names, td_gnames, m.generic_param_names)
        return reader.parse_method_sig(m.generic_param_names)

    stmts = []

    for td in typedefs:
        vis = td.flags & 0x07
        if vis not in (TD_PUBLIC, TD_NESTED_PUBLIC):
            continue
        if not td.name or td.name in ("<Module>",):
            continue
        if td.name.startswith("<") or td.name.startswith("_"):
            continue

        is_iface = bool(td.flags & TD_INTERFACE)

        tmpl_params = [TemplateParam(name=n) for n in td.generic_param_names]
        bases = list(td.interface_names)

        body: list = []

        if not is_iface:
            body.append(make_fn_stub("__init__",
                [make_param("self", "Self")], "None",
                False, False, []))

        for md_1 in range(td.method_list_start, td.method_list_end):
            md_idx = md_1 - 1
            if md_idx < 0 or md_idx >= len(all_methods):
                continue
            m = all_methods[md_idx]
            if not m.is_public:
                continue

            role = m.property_role
            if role and role.kind == "event":
                continue

            is_accessor = role and role.kind in ("getter", "setter")
            if m.flags & MD_SPECIAL_NAME and not is_accessor:
                if m.name in (".ctor", ".cctor"):
                    continue
                op = _OP_MAP.get(m.name)
                if op:
                    ret, sig_ps = decode_sig(m, td.generic_param_names)
                    ps = [make_param("self", "Self")] + [
                        make_param(f"p{i}", ty, is_byref)
                        for i, (ty, is_byref) in enumerate(sig_ps)
                    ]
                    body.append(make_fn_stub(op, ps, ret, False, is_iface, []))
                continue

            if role and role.kind == "getter":
                arrow_name = f"get{role.name}"
            elif role and role.kind == "setter":
                arrow_name = f"set{role.name}"
            else:
                arrow_name = m.name

            ret, sig_ps = decode_sig(m, td.generic_param_names)

            arrow_params: list = [] if m.is_static else [make_param("self", "Self")]

            if role and role.kind == "setter":
                if sig_ps:
                    arrow_params.append(make_param("value", sig_ps[0][0], sig_ps[0][1]))
            else:
                pstart = m.param_list_start - 1
                pend   = m.param_list_end - 1
                mparams = [p for p in all_params[pstart:pend] if p.sequence > 0]
                for i, (ty, is_byref) in enumerate(sig_ps):
                    pname = _sanitize(mparams[i].name) if i < len(mparams) else f"p{i}"
                    arrow_params.append(make_param(pname, ty, is_byref))

            tmpl_m = [TemplateParam(name=n) for n in m.generic_param_names]
            eff_ret = "None" if (role and role.kind == "setter") else ret

            body.append(make_fn_stub(arrow_name, arrow_params, eff_ret,
                                     m.is_static, is_iface, tmpl_m))

        if is_iface:
            stmts.append(StmtTraitDef(name=td.name, template_params=tmpl_params, body=body))
        else:
            stmts.append(StmtClassDef(name=td.name, template_params=tmpl_params,
                                       bases=bases, decorators=[], body=body))

    return stmts
