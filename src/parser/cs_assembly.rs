// cs_assembly.rs — .NET CLI assembly metadata reader (ECMA-335, Partition II)
//
// Reads a .NET DLL directly (no external tools) and generates Arrow type stubs
// (Vec<Stmt>) for use with import[cs-dll] and import[cs-proc].
//
// Coverage:
//   • TypeDef   → ClassDef / TraitDef (interface) stubs
//   • MethodDef → FnDef stubs (instance + static)
//   • Param     → parameter names and out-flag
//   • TypeRef   → external type name resolution
//   • GenericParam → template parameter names
//   • MethodSemantics + PropertyDef → get_X() / set_X() instead of raw accessor names
//   • Type signatures → Arrow type strings
//
// Type mapping (C# → Arrow):
//   void/bool/int*/uint*/long → None/bool/int
//   float/double              → float
//   string/char               → str
//   object                    → Any
//   T[]  / List<T> / IEnumerable<T> etc. → list[T]
//   Dictionary<K,V>           → dict[K,V]
//   T? / Nullable<T>          → Option[T]
//   Task<T>                   → T  (bridge blocks for async)
//   (other generics)          → ClassName[T1,T2]
//   interfaces                → trait stubs

use std::collections::HashMap;
use std::path::Path;

use crate::ast::{Accessibility, Param, Stmt, TemplateParam};

// ---------------------------------------------------------------------------
// PE constants
// ---------------------------------------------------------------------------

const PE_SIG: u32 = 0x0000_4550;
const BSJB: u32 = 0x424A_5342; // "BSJB" as LE u32: bytes 42 53 4A 42
const CLI_DIR: usize = 14;

// Table indices (ECMA-335 §II.22)
const T_MODULE: usize = 0x00;
const T_TYPEREF: usize = 0x01;
const T_TYPEDEF: usize = 0x02;
const T_FIELD: usize = 0x04;
const T_METHODDEF: usize = 0x06;
const T_PARAM: usize = 0x08;
const T_INTERFACEIMPL: usize = 0x09;
const T_MEMBERREF: usize = 0x0A;
const T_PROPERTY: usize = 0x17;
const T_METHODSEMANTICS: usize = 0x18;
const T_MODULEREF: usize = 0x1A;
const T_TYPESPEC: usize = 0x1B;
const T_ASSEMBLY: usize = 0x20;
const T_ASSEMBLYREF: usize = 0x23;
const T_GENERICPARAM: usize = 0x2A;

// TypeDef flags
const TD_PUBLIC: u32 = 0x01;
const TD_NESTED_PUBLIC: u32 = 0x02;
const TD_INTERFACE: u32 = 0x20;

// MethodDef flags
const MD_STATIC: u32 = 0x10;
const MD_SPECIAL_NAME: u32 = 0x0800;

// MethodSemantics
const SEM_GETTER: u16 = 0x02;
const SEM_ADDON: u16 = 0x08;
const SEM_REMOVEON: u16 = 0x10;

// Param flags
const PARAM_OUT: u16 = 0x0002;

// Element types (ECMA-335 §II.23.1.16)
const ET_VOID: u8 = 0x01;
const ET_BOOLEAN: u8 = 0x02;
const ET_CHAR: u8 = 0x03;
const ET_I1: u8 = 0x04;
const ET_U1: u8 = 0x05;
const ET_I2: u8 = 0x06;
const ET_U2: u8 = 0x07;
const ET_I4: u8 = 0x08;
const ET_U4: u8 = 0x09;
const ET_I8: u8 = 0x0A;
const ET_U8: u8 = 0x0B;
const ET_R4: u8 = 0x0C;
const ET_R8: u8 = 0x0D;
const ET_STRING: u8 = 0x0E;
const ET_PTR: u8 = 0x0F;
const ET_BYREF: u8 = 0x10;
const ET_VALUETYPE: u8 = 0x11;
const ET_CLASS: u8 = 0x12;
const ET_VAR: u8 = 0x13;
const ET_ARRAY: u8 = 0x14;
const ET_GENERICINST: u8 = 0x15;
const ET_I: u8 = 0x18;
const ET_U: u8 = 0x19;
const ET_FNPTR: u8 = 0x1B;
const ET_OBJECT: u8 = 0x1C;
const ET_SZARRAY: u8 = 0x1D;
const ET_MVAR: u8 = 0x1E;
const ET_CMOD_REQD: u8 = 0x1F;
const ET_CMOD_OPT: u8 = 0x20;
const ET_SENTINEL: u8 = 0x41;
const ET_PINNED: u8 = 0x45;

// ---------------------------------------------------------------------------
// Byte helpers
// ---------------------------------------------------------------------------

fn u16le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}

fn u32le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

// Decode ECMA-335 compressed unsigned integer; returns (value, bytes_consumed).
fn decompress_uint(data: &[u8], pos: usize) -> (u32, usize) {
    let b0 = data[pos] as u32;
    if b0 & 0x80 == 0 {
        (b0, 1)
    } else if b0 & 0xC0 == 0x80 {
        let b1 = data[pos + 1] as u32;
        (((b0 & 0x3F) << 8) | b1, 2)
    } else {
        let b1 = data[pos + 1] as u32;
        let b2 = data[pos + 2] as u32;
        let b3 = data[pos + 3] as u32;
        (((b0 & 0x1F) << 24) | (b1 << 16) | (b2 << 8) | b3, 4)
    }
}

// ---------------------------------------------------------------------------
// Section table: RVA → file offset
// ---------------------------------------------------------------------------

struct PeSection {
    virt_addr: u32,
    virt_size: u32,
    raw_addr: u32,
}

fn rva_to_offset(rva: u32, sections: &[PeSection]) -> Option<usize> {
    for s in sections {
        if rva >= s.virt_addr && rva < s.virt_addr + s.virt_size.max(1) {
            return Some((rva - s.virt_addr + s.raw_addr) as usize);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// PE/CLI navigation — find metadata root offset in file
// ---------------------------------------------------------------------------

fn find_metadata_root(data: &[u8]) -> Result<(usize, Vec<PeSection>), String> {
    let e_lfanew = u32le(data, 0x3C) as usize;
    if u32le(data, e_lfanew) != PE_SIG {
        return Err("CsImport: not a PE file".to_string());
    }
    let coff = e_lfanew + 4;
    let num_sections = u16le(data, coff + 2) as usize;
    let opt_hdr_size = u16le(data, coff + 16) as usize;
    let opt_hdr = coff + 20;

    // PE32 or PE32+?
    let magic = u16le(data, opt_hdr);
    let data_dirs_off = opt_hdr + if magic == 0x020B { 112 } else { 96 };

    // CLI header data directory (index 14)
    let cli_rva = u32le(data, data_dirs_off + CLI_DIR * 8);
    if cli_rva == 0 {
        return Err("CsImport: no CLI header — not a .NET assembly".to_string());
    }

    // Parse section headers
    let sections_off = opt_hdr + opt_hdr_size;
    let mut sections = Vec::new();
    for i in 0..num_sections {
        let sh = sections_off + i * 40;
        sections.push(PeSection {
            virt_addr: u32le(data, sh + 12),
            virt_size: u32le(data, sh + 8),
            raw_addr: u32le(data, sh + 20),
        });
    }

    // CLI header → MetaData RVA
    let cli_off = rva_to_offset(cli_rva, &sections)
        .ok_or("CsImport: cannot resolve CLI header RVA")?;
    let meta_rva = u32le(data, cli_off + 8);
    let meta_off = rva_to_offset(meta_rva, &sections)
        .ok_or("CsImport: cannot resolve metadata root RVA")?;

    if u32le(data, meta_off) != BSJB {
        return Err("CsImport: invalid metadata root signature".to_string());
    }
    Ok((meta_off, sections))
}

// ---------------------------------------------------------------------------
// Metadata stream headers
// ---------------------------------------------------------------------------

struct Streams {
    tilde_off: usize,
    strings_off: usize,
    blob_off: usize,
}

fn find_streams(data: &[u8], meta: usize) -> Result<Streams, String> {
    let ver_len = u32le(data, meta + 12) as usize;
    // align to 4
    let ver_aligned = (ver_len + 3) & !3;
    let mut pos = meta + 16 + ver_aligned; // skip flags(2) then streams count(2)
    let num_streams = u16le(data, pos + 2) as usize;
    pos += 4;

    let mut tilde = None;
    let mut strings = None;
    let mut blob = None;

    for _ in 0..num_streams {
        let offset = u32le(data, pos) as usize;
        let size = u32le(data, pos + 4) as usize;
        pos += 8;
        // read null-terminated name, 4-byte aligned
        let name_start = pos;
        while pos < data.len() && data[pos] != 0 {
            pos += 1;
        }
        let name = std::str::from_utf8(&data[name_start..pos]).unwrap_or("");
        pos += 1;
        pos = (pos + 3) & !3;

        match name {
            "#~" | "#-" => tilde = Some(meta + offset),
            "#Strings" => {
                strings = Some(meta + offset);
            }
            "#Blob" => {
                blob = Some(meta + offset);
            }
            _ => {}
        }
    }

    Ok(Streams {
        tilde_off: tilde.ok_or("CsImport: no #~ stream")?,
        strings_off: strings.ok_or("CsImport: no #Strings stream")?,
        blob_off: blob.ok_or("CsImport: no #Blob stream")?,
    })
}

// ---------------------------------------------------------------------------
// #~ stream layout
// ---------------------------------------------------------------------------

struct TildeLayout {
    heap_sizes: u8,
    rows: [u32; 64],
    table_offsets: [usize; 64],
    table_row_sizes: [usize; 64],
}

impl TildeLayout {
    // Size of a string heap index (2 or 4 bytes)
    fn str_idx_size(&self) -> usize {
        if self.heap_sizes & 0x01 != 0 { 4 } else { 2 }
    }
    // Size of a GUID heap index (2 or 4 bytes)
    fn guid_idx_size(&self) -> usize {
        if self.heap_sizes & 0x02 != 0 { 4 } else { 2 }
    }
    // Size of a blob heap index (2 or 4 bytes)
    fn blob_idx_size(&self) -> usize {
        if self.heap_sizes & 0x04 != 0 { 4 } else { 2 }
    }
    // Size of an index into a single table
    fn tbl_idx(&self, tbl: usize) -> usize {
        if self.rows[tbl] > 0xFFFF { 4 } else { 2 }
    }
    // Coded index: tables list + tag bits
    fn coded_idx(&self, tables: &[usize], tag_bits: u32) -> usize {
        let max_rows = tables.iter().map(|&t| self.rows[t]).max().unwrap_or(0);
        let threshold = (1u32 << (16 - tag_bits)).saturating_sub(1);
        if max_rows > threshold { 4 } else { 2 }
    }
}

fn parse_tilde(data: &[u8], tilde_start: usize) -> TildeLayout {
    let heap_sizes = data[tilde_start + 6];
    let valid_lo = u32le(data, tilde_start + 8) as u64;
    let valid_hi = u32le(data, tilde_start + 12) as u64;
    let valid = valid_lo | (valid_hi << 32);

    let mut rows = [0u32; 64];
    let mut pos = tilde_start + 24;
    for i in 0..64usize {
        if valid & (1u64 << i) != 0 {
            rows[i] = u32le(data, pos);
            pos += 4;
        }
    }
    let tables_data_start = pos;

    // Build layout (row sizes + offsets) for each present table.
    // We only need a subset; the sizes of the others still matter for skipping.
    let mut layout = TildeLayout {
        heap_sizes,
        rows,
        table_offsets: [0; 64],
        table_row_sizes: [0; 64],
    };
    layout.compute_row_sizes();
    // Compute table offsets by summing row sizes of preceding tables
    let mut off = tables_data_start;
    for i in 0..64usize {
        if valid & (1u64 << i) != 0 {
            layout.table_offsets[i] = off;
            off += layout.table_row_sizes[i] * layout.rows[i] as usize;
        }
    }
    layout
}

impl TildeLayout {
    fn compute_row_sizes(&mut self) {
        let s = self.str_idx_size();
        let g = self.guid_idx_size();
        let b = self.blob_idx_size();

        // Coded index sizes
        let type_def_or_ref = self.coded_idx(&[T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2);
        let has_semantics = self.coded_idx(&[0x14 /*Event*/, T_PROPERTY], 1);
        let resolution_scope =
            self.coded_idx(&[T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2);
        let type_or_method_def = self.coded_idx(&[T_TYPEDEF, T_METHODDEF], 1);

        let td = self.tbl_idx(T_TYPEDEF);
        let fi = self.tbl_idx(T_FIELD);
        let md = self.tbl_idx(T_METHODDEF);
        let pa = self.tbl_idx(T_PARAM);
        let pr = self.tbl_idx(T_PROPERTY);

        // Row sizes for tables we care about (others: 0 → computed separately)
        self.table_row_sizes[T_MODULE] = 2 + s + g + g + g;
        self.table_row_sizes[T_TYPEREF] = resolution_scope + s + s;
        self.table_row_sizes[T_TYPEDEF] = 4 + s + s + type_def_or_ref + fi + md;
        self.table_row_sizes[T_FIELD] = 2 + s + b;
        self.table_row_sizes[T_METHODDEF] = 4 + 2 + 2 + s + b + pa;
        self.table_row_sizes[T_PARAM] = 2 + 2 + s;
        self.table_row_sizes[T_INTERFACEIMPL] = td + type_def_or_ref;
        self.table_row_sizes[T_MEMBERREF] = self.coded_idx(&[T_TYPEREF, T_MODULEREF, T_METHODDEF, T_TYPEDEF, T_TYPESPEC], 3) + s + b;
        self.table_row_sizes[0x0B /*Constant*/] = 2 + self.coded_idx(&[T_FIELD, T_PARAM, 0x17], 2) + b;
        self.table_row_sizes[0x0C /*CustomAttribute*/] =
            self.coded_idx(&[T_METHODDEF, T_FIELD, T_TYPEREF, T_TYPEDEF, T_PARAM, T_INTERFACEIMPL, T_MEMBERREF, T_MODULE, 0x0E, T_PROPERTY, 0x14, T_STANDALONESIG, T_MODULEREF, T_TYPESPEC, T_ASSEMBLY, T_ASSEMBLYREF, T_FIELD, T_PARAM, 0x2A], 5)
            + self.coded_idx(&[T_METHODDEF, T_MEMBERREF], 3) + b;
        self.table_row_sizes[0x0D /*FieldMarshal*/] = self.coded_idx(&[T_FIELD, T_PARAM], 1) + b;
        self.table_row_sizes[0x0E /*DeclSecurity*/] = 2 + self.coded_idx(&[T_TYPEDEF, T_METHODDEF, T_ASSEMBLY], 2) + b;
        self.table_row_sizes[0x0F /*ClassLayout*/] = 2 + 4 + td;
        self.table_row_sizes[0x10 /*FieldLayout*/] = 4 + fi;
        self.table_row_sizes[T_STANDALONESIG] = b;
        self.table_row_sizes[0x12 /*EventMap*/] = td + self.tbl_idx(0x14);
        self.table_row_sizes[0x14 /*Event*/] = 2 + s + type_def_or_ref;
        self.table_row_sizes[0x15 /*PropertyMap*/] = td + pr;
        self.table_row_sizes[T_PROPERTY] = 2 + s + b;
        self.table_row_sizes[T_METHODSEMANTICS] = 2 + md + has_semantics;
        self.table_row_sizes[0x19 /*MethodImpl*/] = td + self.coded_idx(&[T_METHODDEF, T_MEMBERREF], 1) + self.coded_idx(&[T_METHODDEF, T_MEMBERREF], 1);
        self.table_row_sizes[T_MODULEREF] = s;
        self.table_row_sizes[T_TYPESPEC] = b;
        self.table_row_sizes[0x1C /*ImplMap*/] = 2 + self.coded_idx(&[T_FIELD, T_METHODDEF], 1) + s + self.tbl_idx(T_MODULEREF);
        self.table_row_sizes[0x1D /*FieldRVA*/] = 4 + fi;
        self.table_row_sizes[T_ASSEMBLY] = 4 + 2 + 2 + 2 + 2 + 4 + b + s + s;
        self.table_row_sizes[0x21 /*AssemblyProcessor*/] = 4;
        self.table_row_sizes[0x22 /*AssemblyOS*/] = 4 + 4 + 4;
        self.table_row_sizes[T_ASSEMBLYREF] = 2 + 2 + 2 + 2 + 4 + b + s + s + b;
        self.table_row_sizes[0x24 /*AssemblyRefProcessor*/] = 4 + self.tbl_idx(T_ASSEMBLYREF);
        self.table_row_sizes[0x25 /*AssemblyRefOS*/] = 4 + 4 + 4 + self.tbl_idx(T_ASSEMBLYREF);
        self.table_row_sizes[0x26 /*File*/] = 4 + s + b;
        self.table_row_sizes[0x27 /*ExportedType*/] = 4 + 4 + s + s + self.coded_idx(&[T_ASSEMBLY, 0x26, 0x1B, 0x27, T_TYPEDEF], 2);
        self.table_row_sizes[0x28 /*ManifestResource*/] = 4 + 4 + s + self.coded_idx(&[T_ASSEMBLY, 0x26, 0x23, 0x1B], 2);
        self.table_row_sizes[0x29 /*NestedClass*/] = td + td;
        self.table_row_sizes[T_GENERICPARAM] = 2 + 2 + type_or_method_def + s;
        self.table_row_sizes[0x2B /*MethodSpec*/] = self.coded_idx(&[T_METHODDEF, T_MEMBERREF], 1) + b;
        self.table_row_sizes[0x2C /*GenericParamConstraint*/] = self.tbl_idx(T_GENERICPARAM) + type_def_or_ref;
    }

    // Read an n-byte little-endian index
    fn read_idx(&self, data: &[u8], off: usize, size: usize) -> u32 {
        if size == 2 { u16le(data, off) as u32 } else { u32le(data, off) }
    }

    // Read a column within a table row
    #[allow(dead_code)]
    fn col(&self, data: &[u8], tbl: usize, row: usize, col_offset: usize, col_size: usize) -> u32 {
        let off = self.table_offsets[tbl] + row * self.table_row_sizes[tbl] + col_offset;
        self.read_idx(data, off, col_size)
    }
}

// ---------------------------------------------------------------------------
// Heap accessors
// ---------------------------------------------------------------------------

fn read_string(data: &[u8], strings_off: usize, idx: u32) -> &str {
    let start = strings_off + idx as usize;
    let end = data[start..].iter().position(|&b| b == 0).map(|n| start + n).unwrap_or(start);
    std::str::from_utf8(&data[start..end]).unwrap_or("")
}

fn read_blob<'a>(data: &'a [u8], blob_off: usize, idx: u32) -> &'a [u8] {
    let start = blob_off + idx as usize;
    let (len, hdr) = decompress_uint(data, start);
    &data[start + hdr..start + hdr + len as usize]
}

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

/// All data produced by parsing a .NET assembly binary.
/// Shared between `load_cs_assembly` (runtime stubs) and `generate_cs_stub_text` (file output).
struct ParsedAssembly {
    data: Vec<u8>,
    streams: Streams,
    layout: TildeLayout,
    typedefs: Vec<CsTypeDef>,
    all_methods: Vec<CsMethod>,
    all_params: Vec<CsParam>,
    type_names: HashMap<u32, String>,
}

#[derive(Debug, Clone)]
struct CsMethod {
    name: String,
    is_static: bool,
    is_public: bool,
    flags: u32,
    sig_blob_idx: u32,
    param_list_start: u32,
    param_list_end: u32, // exclusive (next MethodDef's param_list)
    generic_param_names: Vec<String>,
    /// None = raw method; Some(prop_name) = getter or setter
    property_role: Option<PropertyRole>,
}

#[derive(Debug, Clone)]
enum PropertyRole {
    Getter(String),
    Setter(String),
    EventAdder(String),
}

#[derive(Debug, Clone)]
struct CsParam {
    sequence: u16,
    name: String,
    flags: u16,
}

#[derive(Debug, Clone)]
struct CsTypeDef {
    name: String,
    #[allow(dead_code)]
    namespace: String,
    flags: u32,
    method_list_start: u32,
    method_list_end: u32,
    generic_param_names: Vec<String>,
    interface_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Arrow type name from a type signature blob
// ---------------------------------------------------------------------------

struct SigReader<'a> {
    data: &'a [u8],
    pos: usize,
    type_names: &'a HashMap<u32, String>,
    type_params: &'a [String],
    method_params: &'a [String],
}

impl<'a> SigReader<'a> {
    fn peek(&self) -> u8 {
        if self.pos < self.data.len() { self.data[self.pos] } else { 0 }
    }

    fn eat(&mut self) -> u8 {
        let b = self.peek();
        self.pos += 1;
        b
    }

    fn eat_uint(&mut self) -> u32 {
        let (v, n) = decompress_uint(self.data, self.pos);
        self.pos += n;
        v
    }

    // Skip custom modifiers (CMOD_REQD / CMOD_OPT + compressed token)
    fn skip_cmods(&mut self) {
        while self.peek() == ET_CMOD_REQD || self.peek() == ET_CMOD_OPT {
            self.eat();
            self.eat_uint(); // TypeDefOrRef token
        }
    }

    // Parse one Type element; returns Arrow type string.
    fn parse_type(&mut self) -> String {
        self.skip_cmods();
        match self.eat() {
            ET_VOID    => "None".to_string(),
            ET_BOOLEAN => "bool".to_string(),
            ET_CHAR    => "str".to_string(),
            ET_I1 | ET_U1 | ET_I2 | ET_U2 | ET_I4 | ET_U4 |
            ET_I8 | ET_U8 | ET_I | ET_U => "int".to_string(),
            ET_R4 | ET_R8 => "float".to_string(),
            ET_STRING  => "str".to_string(),
            ET_OBJECT  => "Any".to_string(),
            ET_BYREF   => {
                // byref → same type (out/ref handled at param level)
                self.parse_type()
            }
            ET_VALUETYPE | ET_CLASS => {
                let token = self.eat_uint();
                self.type_names.get(&token).cloned().unwrap_or_else(|| "Any".to_string())
            }
            ET_GENERICINST => {
                self.eat(); // CLASS or VALUETYPE
                let token = self.eat_uint();
                let base = self.type_names.get(&token).cloned().unwrap_or_default();
                let argc = self.eat_uint() as usize;
                let mut args = Vec::with_capacity(argc);
                for _ in 0..argc {
                    args.push(self.parse_type());
                }
                map_generic(&base, args)
            }
            ET_SZARRAY => {
                self.skip_cmods();
                let elem = self.parse_type();
                format!("list[{elem}]")
            }
            ET_ARRAY => {
                // multi-dim array: Type ArrayShape — just return list[T]
                let elem = self.parse_type();
                // skip ArrayShape (rank + sizes + lobounds)
                let rank = self.eat_uint();
                let nsizes = self.eat_uint();
                for _ in 0..nsizes { self.eat_uint(); }
                let nlb = self.eat_uint();
                for _ in 0..nlb { self.eat_uint(); }
                let _ = rank;
                format!("list[{elem}]")
            }
            ET_VAR => {
                let idx = self.eat_uint() as usize;
                self.type_params.get(idx).cloned().unwrap_or_else(|| format!("T{idx}"))
            }
            ET_MVAR => {
                let idx = self.eat_uint() as usize;
                self.method_params.get(idx).cloned().unwrap_or_else(|| format!("M{idx}"))
            }
            ET_FNPTR => {
                // skip entire MethodDefSig
                self.skip_method_sig();
                "function".to_string()
            }
            ET_PTR => {
                self.skip_cmods();
                self.parse_type();
                "int".to_string() // raw pointer → int handle
            }
            ET_SENTINEL | ET_PINNED => self.parse_type(),
            _ => "Any".to_string(),
        }
    }

    fn skip_method_sig(&mut self) {
        let _calling = self.eat();
        // if GENERIC bit set, eat generic param count
        if _calling & 0x10 != 0 { self.eat_uint(); }
        let param_count = self.eat_uint();
        self.parse_type(); // return type
        for _ in 0..param_count {
            if self.peek() == ET_SENTINEL { self.eat(); }
            self.parse_type();
        }
    }

    // Parse MethodDefSig: calling convention, gen params, return type, params
    fn parse_method_sig(&mut self, generic_param_names: &[String]) -> (String, Vec<(String, bool)>) {
        let calling = self.eat();
        let is_generic = calling & 0x10 != 0;
        let _is_instance = calling & 0x20 != 0;

        if is_generic {
            let _gen_count = self.eat_uint();
        }
        let param_count = self.eat_uint() as usize;
        let ret = self.parse_type();
        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            if self.peek() == ET_SENTINEL { self.eat(); continue; }
            let is_byref = self.peek() == ET_BYREF;
            let ty = self.parse_type();
            params.push((ty, is_byref));
        }
        let _ = generic_param_names;
        (ret, params)
    }
}

// Map well-known generic C# types to Arrow equivalents
fn map_generic(base: &str, args: Vec<String>) -> String {
    let simple = base.rsplit('.').next().unwrap_or(base);
    // strip arity suffix like `List`1` → `List`
    let simple = simple.split('`').next().unwrap_or(simple);
    match simple {
        "List" | "IList" | "ICollection" | "IEnumerable" | "IReadOnlyList"
        | "IReadOnlyCollection" | "ObservableCollection" | "Collection"
        | "Queue" | "Stack" | "LinkedList" | "ImmutableList" => {
            if args.len() == 1 { format!("list[{}]", args[0]) } else { "list".to_string() }
        }
        "Dictionary" | "IDictionary" | "IReadOnlyDictionary" | "SortedDictionary"
        | "ConcurrentDictionary" => {
            if args.len() == 2 {
                format!("dict[{},{}]", args[0], args[1])
            } else {
                "dict".to_string()
            }
        }
        "HashSet" | "SortedSet" | "ISet" | "ImmutableHashSet" => {
            if args.len() == 1 { format!("set[{}]", args[0]) } else { "set".to_string() }
        }
        "Tuple" | "ValueTuple" => {
            format!("tuple[{}]", args.join(","))
        }
        "Nullable" => {
            if args.len() == 1 { format!("Option[{}]", args[0]) } else { "Any".to_string() }
        }
        "Task" | "ValueTask" => {
            // Treat async as blocking in bridge; strip wrapper
            if args.len() == 1 { args[0].clone() } else { "None".to_string() }
        }
        "Action" | "Func" | "Predicate" | "EventHandler" | "Delegate" => {
            "function".to_string()
        }
        "KeyValuePair" => {
            if args.len() == 2 {
                format!("tuple[{},{}]", args[0], args[1])
            } else {
                "tuple".to_string()
            }
        }
        other => {
            if args.is_empty() {
                other.to_string()
            } else {
                format!("{}[{}]", other, args.join(","))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main reader — parses all needed tables and builds CsTypeDef list
// ---------------------------------------------------------------------------

/// Parse a .NET assembly binary into intermediate tables.
/// Shared by `load_cs_assembly` and `generate_cs_stub_text`.
fn parse_assembly(path: &Path) -> Result<ParsedAssembly, String> {
    let data = std::fs::read(path)
        .map_err(|e| format!("CsImport: cannot read '{}': {e}", path.display()))?;

    let (meta_off, _sections) = find_metadata_root(&data)?;
    let streams = find_streams(&data, meta_off)?;
    let layout = parse_tilde(&data, streams.tilde_off);

    let s_sz = layout.str_idx_size();
    let b_sz = layout.blob_idx_size();
    let fi_sz = layout.tbl_idx(T_FIELD);
    let md_sz = layout.tbl_idx(T_METHODDEF);
    let pa_sz = layout.tbl_idx(T_PARAM);
    let _pr_sz = layout.tbl_idx(T_PROPERTY);
    let tdr_sz = layout.coded_idx(&[T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2);
    let has_sem_sz = layout.coded_idx(&[0x14, T_PROPERTY], 1);
    let tom_sz = layout.coded_idx(&[T_TYPEDEF, T_METHODDEF], 1);
    let res_sz = layout.coded_idx(&[T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2);

    // --- Build TypeRef name table (coded-index → name string) ---
    let mut type_names: HashMap<u32, String> = HashMap::new();
    let typeref_rows = layout.rows[T_TYPEREF] as usize;
    for row in 0..typeref_rows {
        let off = layout.table_offsets[T_TYPEREF] + row * layout.table_row_sizes[T_TYPEREF];
        let name_idx = layout.read_idx(&data, off + res_sz, s_sz);
        let ns_idx = layout.read_idx(&data, off + res_sz + s_sz, s_sz);
        let name = read_string(&data, streams.strings_off, name_idx);
        let ns = read_string(&data, streams.strings_off, ns_idx);
        let coded = ((row as u32 + 1) << 2) | 1;
        let simple = name.split('`').next().unwrap_or(name);
        let full = if ns.is_empty() { simple.to_string() } else { format!("{ns}.{simple}") };
        type_names.insert(coded, full);
    }

    // --- GenericParam table ---
    let gp_rows = layout.rows[T_GENERICPARAM] as usize;
    let mut type_generic_params: HashMap<u32, Vec<(u16, String)>> = HashMap::new();
    let mut method_generic_params: HashMap<u32, Vec<(u16, String)>> = HashMap::new();
    for row in 0..gp_rows {
        let off = layout.table_offsets[T_GENERICPARAM]
            + row * layout.table_row_sizes[T_GENERICPARAM];
        let number = u16le(&data, off);
        let _flags = u16le(&data, off + 2);
        let owner_coded = layout.read_idx(&data, off + 4, tom_sz);
        let name_idx = layout.read_idx(&data, off + 4 + tom_sz, s_sz);
        let name = read_string(&data, streams.strings_off, name_idx).to_string();
        let tag = owner_coded & 0x1;
        let row_1 = owner_coded >> 1;
        if tag == 0 {
            type_generic_params.entry(row_1).or_default().push((number, name));
        } else {
            method_generic_params.entry(row_1).or_default().push((number, name));
        }
    }

    // --- TypeDef table ---
    let td_rows = layout.rows[T_TYPEDEF] as usize;
    let mut typedefs: Vec<CsTypeDef> = Vec::with_capacity(td_rows);
    for row in 0..td_rows {
        let off = layout.table_offsets[T_TYPEDEF] + row * layout.table_row_sizes[T_TYPEDEF];
        let flags = u32le(&data, off);
        let name_idx = layout.read_idx(&data, off + 4, s_sz);
        let ns_idx = layout.read_idx(&data, off + 4 + s_sz, s_sz);
        let method_col_off = 4 + s_sz + s_sz + tdr_sz + fi_sz;
        let method_list_start = layout.read_idx(&data, off + method_col_off, md_sz);

        let name = read_string(&data, streams.strings_off, name_idx).to_string();
        let namespace = read_string(&data, streams.strings_off, ns_idx).to_string();

        let coded = ((row as u32 + 1) << 2) | 0;
        let simple = name.split('`').next().unwrap_or(&name);
        type_names.insert(coded, simple.to_string());

        let gp = type_generic_params.get(&(row as u32 + 1));
        let generic_param_names: Vec<String> = gp.map(|v| {
            let mut sorted = v.clone();
            sorted.sort_by_key(|(n, _)| *n);
            sorted.into_iter().map(|(_, s)| s).collect()
        }).unwrap_or_default();

        typedefs.push(CsTypeDef {
            name: name.split('`').next().unwrap_or(&name).to_string(),
            namespace: namespace.clone(),
            flags,
            method_list_start,
            method_list_end: 0,
            generic_param_names,
            interface_names: Vec::new(),
        });
    }
    let md_total = layout.rows[T_METHODDEF] as u32 + 1;
    for i in 0..typedefs.len() {
        typedefs[i].method_list_end = if i + 1 < typedefs.len() {
            typedefs[i + 1].method_list_start
        } else {
            md_total
        };
    }

    // --- InterfaceImpl table ---
    let ii_rows = layout.rows[T_INTERFACEIMPL] as usize;
    let td_idx_sz = layout.tbl_idx(T_TYPEDEF);
    for row in 0..ii_rows {
        let off = layout.table_offsets[T_INTERFACEIMPL]
            + row * layout.table_row_sizes[T_INTERFACEIMPL];
        let td_1 = layout.read_idx(&data, off, td_idx_sz);
        let iface_coded = layout.read_idx(&data, off + td_idx_sz, tdr_sz);
        let iface_name = type_names.get(&iface_coded).cloned().unwrap_or_default();
        let simple = iface_name.rsplit('.').next().unwrap_or(&iface_name);
        if !simple.is_empty() && simple != "IDisposable" {
            if let Some(td) = typedefs.get_mut((td_1 as usize).saturating_sub(1)) {
                td.interface_names.push(simple.to_string());
            }
        }
    }

    // --- Param table ---
    let param_rows = layout.rows[T_PARAM] as usize;
    let mut all_params: Vec<CsParam> = Vec::with_capacity(param_rows);
    for row in 0..param_rows {
        let off = layout.table_offsets[T_PARAM] + row * layout.table_row_sizes[T_PARAM];
        let flags = u16le(&data, off);
        let seq = u16le(&data, off + 2);
        let name_idx = layout.read_idx(&data, off + 4, s_sz);
        let name = read_string(&data, streams.strings_off, name_idx).to_string();
        all_params.push(CsParam { sequence: seq, name, flags });
    }
    let param_total = param_rows as u32 + 1;

    // --- PropertyDef / MethodSemantics ---
    let mut method_role: HashMap<u32, PropertyRole> = HashMap::new();

    if layout.rows[T_METHODSEMANTICS] > 0 && layout.rows[T_PROPERTY] > 0 {
        let mut prop_names: HashMap<u32, String> = HashMap::new();
        let pr_rows = layout.rows[T_PROPERTY] as usize;
        for row in 0..pr_rows {
            let off = layout.table_offsets[T_PROPERTY]
                + row * layout.table_row_sizes[T_PROPERTY];
            let name_idx = layout.read_idx(&data, off + 2, s_sz);
            let name = read_string(&data, streams.strings_off, name_idx).to_string();
            prop_names.insert(row as u32 + 1, name);
        }

        let ms_rows = layout.rows[T_METHODSEMANTICS] as usize;
        for row in 0..ms_rows {
            let off = layout.table_offsets[T_METHODSEMANTICS]
                + row * layout.table_row_sizes[T_METHODSEMANTICS];
            let sem = u16le(&data, off);
            let meth_1 = layout.read_idx(&data, off + 2, md_sz);
            let assoc = layout.read_idx(&data, off + 2 + md_sz, has_sem_sz);
            let assoc_tag = assoc & 1;
            let assoc_row = assoc >> 1;
            if assoc_tag == 1 {
                let prop_name = prop_names.get(&assoc_row).cloned().unwrap_or_default();
                if !prop_name.is_empty() {
                    let role = if sem & SEM_GETTER != 0 {
                        PropertyRole::Getter(prop_name)
                    } else {
                        PropertyRole::Setter(prop_name)
                    };
                    method_role.insert(meth_1, role);
                }
            } else {
                let event_name = String::new();
                if sem & SEM_ADDON != 0 || sem & SEM_REMOVEON != 0 {
                    method_role.insert(meth_1, PropertyRole::EventAdder(event_name));
                }
            }
        }
    }

    // --- MethodDef table ---
    let md_rows = layout.rows[T_METHODDEF] as usize;
    let mut all_methods: Vec<CsMethod> = Vec::with_capacity(md_rows);
    for row in 0..md_rows {
        let off = layout.table_offsets[T_METHODDEF]
            + row * layout.table_row_sizes[T_METHODDEF];
        let meth_flags = u16le(&data, off + 6) as u32;
        let name_idx = layout.read_idx(&data, off + 8, s_sz);
        let sig_idx = layout.read_idx(&data, off + 8 + s_sz, b_sz);
        let param_list_start = layout.read_idx(&data, off + 8 + s_sz + b_sz, pa_sz);

        let name = read_string(&data, streams.strings_off, name_idx).to_string();
        let access = meth_flags & 0x07;
        let is_public = access == 6;
        let is_static = meth_flags & MD_STATIC != 0;

        let method_1 = row as u32 + 1;

        let gp = method_generic_params.get(&method_1);
        let generic_param_names: Vec<String> = gp.map(|v| {
            let mut sorted = v.clone();
            sorted.sort_by_key(|(n, _)| *n);
            sorted.into_iter().map(|(_, s)| s).collect()
        }).unwrap_or_default();

        let property_role = method_role.remove(&method_1);

        all_methods.push(CsMethod {
            name,
            is_static,
            is_public,
            flags: meth_flags,
            sig_blob_idx: sig_idx,
            param_list_start,
            param_list_end: 0,
            generic_param_names,
            property_role,
        });
    }
    for i in 0..all_methods.len() {
        all_methods[i].param_list_end = if i + 1 < all_methods.len() {
            all_methods[i + 1].param_list_start
        } else {
            param_total
        };
    }

    Ok(ParsedAssembly { data, streams, layout, typedefs, all_methods, all_params, type_names })
}

/// Read a .NET assembly and return a map of (type_coded_index → Arrow type name)
/// plus all type definitions for stub generation.
pub fn load_cs_assembly(path: &Path) -> Result<Vec<Stmt>, String> {
    let pa = parse_assembly(path)?;
    generate_stubs(
        &pa.data, &pa.streams, &pa.layout,
        &pa.typedefs, &pa.all_methods, &pa.all_params, &pa.type_names,
    )
}

/// Generate a `.ars` stub text from a .NET DLL, including XML doc comments if a
/// companion `{stem}.xml` documentation file is present alongside the DLL.
/// Returns `(stmts, stub_text)` where `stmts` is used by the interpreter at runtime
/// and `stub_text` is written to the `.ars` file.
pub fn generate_cs_stub_text(path: &Path) -> Result<(Vec<Stmt>, String), String> {
    let pa = parse_assembly(path)?;
    let stmts = generate_stubs(
        &pa.data, &pa.streams, &pa.layout,
        &pa.typedefs, &pa.all_methods, &pa.all_params, &pa.type_names,
    )?;
    let xml_path = path.with_extension("xml");
    let docs = parse_xml_docs(&xml_path);
    let text = render_cs_ars_text(&pa, &docs);
    Ok((stmts, text))
}

// ---------------------------------------------------------------------------
// XML documentation helpers
// ---------------------------------------------------------------------------

/// Strip XML tags from a string and normalize whitespace.
fn strip_xml_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the text content of the first occurrence of `<tag>...</tag>` in `text`.
fn extract_xml_element(text: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = text.find(&open) {
        if let Some(gt) = text[start..].find('>') {
            let content_start = start + gt + 1;
            if let Some(end) = text[content_start..].find(&close) {
                return strip_xml_tags(&text[content_start..content_start + end]);
            }
        }
    }
    String::new()
}

/// Convert an XML doc member ID to a simplified lookup key.
/// - `T:Namespace.ClassName`  → `T:ClassName`
/// - `M:Namespace.Class.Method(args)` → `M:ClassName.Method`
/// - `P:Namespace.Class.Prop`  → `P:ClassName.Prop`
fn simplify_member_id(member_id: &str) -> Option<String> {
    if member_id.len() < 2 { return None; }
    let prefix = &member_id[..2];
    let rest   = &member_id[2..];
    match prefix {
        "T:" => {
            let simple = rest.rsplit('.').next().unwrap_or(rest);
            let simple = simple.split('`').next().unwrap_or(simple);
            Some(format!("T:{simple}"))
        }
        "M:" => {
            let without_args = rest.split('(').next().unwrap_or(rest);
            let parts: Vec<&str> = without_args.split('.').collect();
            if parts.len() >= 2 {
                let cls = parts[parts.len() - 2].split('`').next().unwrap_or(parts[parts.len() - 2]);
                let method = parts[parts.len() - 1];
                Some(format!("M:{cls}.{method}"))
            } else { None }
        }
        "P:" => {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 2 {
                let cls = parts[parts.len() - 2].split('`').next().unwrap_or(parts[parts.len() - 2]);
                let prop = parts[parts.len() - 1];
                Some(format!("P:{cls}.{prop}"))
            } else { None }
        }
        _ => None,
    }
}

/// Parse an XML documentation file (companion to a .NET DLL) and return a map
/// of simplified member key → summary text.
/// Returns an empty map if the file does not exist or cannot be read.
fn parse_xml_docs(xml_path: &Path) -> HashMap<String, String> {
    let content = match std::fs::read_to_string(xml_path) {
        Ok(c)  => c,
        Err(_) => return HashMap::new(),
    };
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut search = 0usize;
    while let Some(rel) = content[search..].find("<member ") {
        let m_start = search + rel;
        let after   = m_start + 8;
        let name_val_start = match content[after..].find("name=\"").map(|p| after + p + 6) {
            Some(n) => n,
            None    => { search = m_start + 1; continue; }
        };
        let name_end = match content[name_val_start..].find('"').map(|p| name_val_start + p) {
            Some(e) => e,
            None    => { search = m_start + 1; continue; }
        };
        let member_id = &content[name_val_start..name_end];
        let block_end = content[name_end..].find("</member>")
            .map(|p| name_end + p)
            .unwrap_or(content.len());
        let block   = &content[name_end..block_end];
        let summary = extract_xml_element(block, "summary");
        if !summary.is_empty() {
            if let Some(key) = simplify_member_id(member_id) {
                docs.entry(key).or_insert(summary);
            }
        }
        search = if block_end < content.len() { block_end + 9 } else { content.len() };
    }
    docs
}

// ---------------------------------------------------------------------------
// .ars text renderer (with optional docstrings)
// ---------------------------------------------------------------------------

/// Generate `.ars` stub text from parsed assembly data, embedding XML doc comments
/// as triple-quoted docstrings when the `docs` map is non-empty.
///
/// The output format mirrors `stub_gen::generate_stub` but includes
/// `"""summary"""` on the first body line of each class/method that has a doc entry.
fn render_cs_ars_text(pa: &ParsedAssembly, docs: &HashMap<String, String>) -> String {
    let data       = &pa.data;
    let streams    = &pa.streams;
    let layout     = &pa.layout;
    let typedefs   = &pa.typedefs;
    let methods    = &pa.all_methods;
    let params     = &pa.all_params;
    let type_names = &pa.type_names;

    let mut out   = String::new();
    let mut first = true;

    for td in typedefs {
        let vis = td.flags & 0x07;
        if vis != TD_PUBLIC && vis != TD_NESTED_PUBLIC { continue; }
        if td.name.is_empty() || td.name == "<Module>"   { continue; }
        if td.name.starts_with('<') || td.name.starts_with('_') { continue; }

        let is_interface = td.flags & TD_INTERFACE != 0;

        let tparams = if td.generic_param_names.is_empty() {
            String::new()
        } else {
            format!("[{}]", td.generic_param_names.join(", "))
        };
        let bases_str = if td.interface_names.is_empty() {
            String::new()
        } else {
            format!("({})", td.interface_names.join(", "))
        };

        if !first { out.push('\n'); }
        first = false;

        if is_interface {
            out.push_str(&format!("trait {}{tparams}:\n", td.name));
        } else {
            let n = &td.name;
            out.push_str(&format!("class {n}{tparams}{bases_str}->{n}:\n"));
        }

        // Class-level docstring
        if let Some(doc) = docs.get(&format!("T:{}", td.name)) {
            out.push_str(&format!("    \"\"\"{doc}\"\"\"\n"));
        }

        // __init__ stub for classes
        if !is_interface {
            out.push_str("    fn __init__(self: Self) -> None:\n        ...\n");
        }

        // Methods
        let mstart = td.method_list_start as usize;
        let mend   = td.method_list_end   as usize;

        for md_1 in mstart..mend {
            let md_idx = md_1.saturating_sub(1);
            if md_idx >= methods.len() { continue; }
            let m = &methods[md_idx];

            if !m.is_public { continue; }
            if let Some(PropertyRole::EventAdder(_)) = &m.property_role { continue; }

            let is_accessor = matches!(&m.property_role,
                Some(PropertyRole::Getter(_)) | Some(PropertyRole::Setter(_)));
            if m.flags & MD_SPECIAL_NAME != 0 && !is_accessor {
                if m.name == ".ctor" || m.name == ".cctor" { continue; }
                if let Some(op) = operator_name(&m.name) {
                    let (ret, sig_params) = decode_method_sig(
                        data, streams, layout, m, params, type_names, &td.generic_param_names);
                    let mut p_parts = vec!["self: Self".to_string()];
                    for (i, (ty, _)) in sig_params.iter().enumerate() {
                        p_parts.push(format!("p{i}: {ty}"));
                    }
                    out.push_str(&format!("    fn {op}({}) -> {ret}:\n        ...\n",
                        p_parts.join(", ")));
                    continue;
                }
                continue;
            }

            let arrow_name = match &m.property_role {
                Some(PropertyRole::Getter(prop)) => format!("get{prop}"),
                Some(PropertyRole::Setter(prop)) => format!("set{prop}"),
                _ => m.name.clone(),
            };

            let (ret_type, sig_params) = decode_method_sig(
                data, streams, layout, m, params, type_names, &td.generic_param_names);

            let mut p_parts: Vec<String> = if m.is_static {
                vec![]
            } else {
                vec!["self: Self".to_string()]
            };

            let pstart = m.param_list_start as usize;
            let pend   = m.param_list_end   as usize;
            let method_params: Vec<&CsParam> = params
                .iter()
                .skip(pstart.saturating_sub(1))
                .take((pend - pstart).min(params.len()))
                .filter(|p| p.sequence > 0)
                .collect();

            if let Some(PropertyRole::Setter(_)) = &m.property_role {
                if let Some((ty, _)) = sig_params.first() {
                    p_parts.push(format!("value: {ty}"));
                }
            } else {
                for (i, (ty, _)) in sig_params.iter().enumerate() {
                    let pname = method_params.get(i)
                        .map(|p| sanitize_param_name(&p.name))
                        .unwrap_or_else(|| format!("p{i}"));
                    p_parts.push(format!("{pname}: {ty}"));
                }
            }

            let eff_ret = match &m.property_role {
                Some(PropertyRole::Setter(_)) => "None".to_string(),
                _ => ret_type,
            };

            let tmpl = if m.generic_param_names.is_empty() {
                String::new()
            } else {
                format!("[{}]", m.generic_param_names.join(", "))
            };

            out.push_str(&format!("    fn {arrow_name}{tmpl}({}) -> {eff_ret}:\n",
                p_parts.join(", ")));

            // Method docstring — look up by original C# name, fall back to property key
            let mkey = format!("M:{}.{}", td.name, m.name);
            let pkey = match &m.property_role {
                Some(PropertyRole::Getter(prop)) | Some(PropertyRole::Setter(prop)) =>
                    Some(format!("P:{}.{}", td.name, prop)),
                _ => None,
            };
            let doc = docs.get(&mkey).or_else(|| pkey.as_ref().and_then(|k| docs.get(k)));
            if let Some(d) = doc {
                out.push_str(&format!("        \"\"\"{d}\"\"\"\n"));
            }
            out.push_str("        ...\n");
        }

        out.push('\n');
    }

    out
}

// ---------------------------------------------------------------------------
// Stub generation
// ---------------------------------------------------------------------------

fn make_param(name: &str, type_ann: &str, mutable: bool) -> Param {
    Param {
        name: name.to_string(),
        mutable,
        type_ann: Some(type_ann.to_string()),
        default: None,
        variadic: false,
    }
}

fn make_fn_stub(
    name: &str,
    params: Vec<Param>,
    ret: &str,
    is_static: bool,
    is_abstract: bool,
    template_params: Vec<TemplateParam>,
) -> Stmt {
    Stmt::FnDef {
        name: name.to_string(),
        template_params,
        params,
        return_type: Some(ret.to_string()),
        body: vec![Stmt::Pass],
        is_abstract,
        is_static,
        is_class_method: false,
        decorators: vec![],
        access: Accessibility::Public,
    }
}

fn generate_stubs(
    data: &[u8],
    streams: &Streams,
    layout: &TildeLayout,
    typedefs: &[CsTypeDef],
    methods: &[CsMethod],
    params: &[CsParam],
    type_names: &HashMap<u32, String>,
) -> Result<Vec<Stmt>, String> {
    let mut stmts: Vec<Stmt> = Vec::new();

    for (_td_idx, td) in typedefs.iter().enumerate() {
        // Skip non-public types (top-level: Public=1; nested: NestedPublic=2)
        let vis = td.flags & 0x07;
        if vis != TD_PUBLIC && vis != TD_NESTED_PUBLIC {
            continue;
        }
        // Skip the <Module> pseudo-type
        if td.name.is_empty() || td.name == "<Module>" {
            continue;
        }
        // Skip compiler-generated types
        if td.name.starts_with('<') || td.name.starts_with('_') {
            continue;
        }

        let is_interface = td.flags & TD_INTERFACE != 0;

        // Template params
        let template_params: Vec<TemplateParam> = td
            .generic_param_names
            .iter()
            .map(|n| TemplateParam { name: n.clone(), constraints: vec![] })
            .collect();

        // Bases: interface names
        let bases: Vec<String> = td.interface_names.clone();

        // Methods for this type
        let mstart = td.method_list_start as usize;
        let mend = td.method_list_end as usize;

        let mut body_stmts: Vec<Stmt> = Vec::new();

        // Always emit __init__ as stub if not interface
        if !is_interface {
            body_stmts.push(make_fn_stub(
                "__init__",
                vec![make_param("self", "Self", false)],
                "None",
                false,
                false,
                vec![],
            ));
        }

        for md_1 in mstart..mend {
            let md_idx = md_1.saturating_sub(1);
            if md_idx >= methods.len() { continue; }
            let m = &methods[md_idx];

            if !m.is_public { continue; }

            // Skip event add/remove
            if let Some(PropertyRole::EventAdder(_)) = &m.property_role {
                continue;
            }

            // Skip compiler-generated special names that are NOT property accessors
            let is_accessor = matches!(&m.property_role, Some(PropertyRole::Getter(_)) | Some(PropertyRole::Setter(_)));
            if m.flags & MD_SPECIAL_NAME != 0 && !is_accessor {
                // Could be .ctor, .cctor, op_xxx — handle .ctor → skip (we emit __init__),
                // operator overloads → emit as __add__ etc.
                if m.name == ".ctor" || m.name == ".cctor" {
                    continue;
                }
                if let Some(op) = operator_name(&m.name) {
                    // Operator overload
                    let (ret, sig_params) = decode_method_sig(
                        data, streams, layout, m, params, type_names, &td.generic_param_names,
                    );
                    let mut arrow_params = vec![make_param("self", "Self", false)];
                    for (i, (ty, _is_byref)) in sig_params.iter().enumerate() {
                        let pname = format!("p{i}");
                        arrow_params.push(make_param(&pname, ty, false));
                    }
                    body_stmts.push(make_fn_stub(op, arrow_params, &ret, false, is_interface, vec![]));
                    continue;
                }
                continue; // skip other special names
            }

            // Determine Arrow method name
            let arrow_name = match &m.property_role {
                Some(PropertyRole::Getter(prop)) => format!("get{prop}"),
                Some(PropertyRole::Setter(prop)) => format!("set{prop}"),
                _ => m.name.clone(),
            };

            // Parse signature
            let (ret_type, sig_params) = decode_method_sig(
                data, streams, layout, m, params, type_names, &td.generic_param_names,
            );

            // Build Arrow params
            let mut arrow_params: Vec<Param> = if m.is_static {
                vec![]
            } else {
                vec![make_param("self", "Self", false)]
            };

            // For setter: single value param
            let mut param_iter = sig_params.iter().enumerate();
            if let Some(PropertyRole::Setter(_prop)) = &m.property_role {
                // setter: (self, value: T) → setX(self, value: T)
                if let Some((_, (ty, _))) = param_iter.next() {
                    arrow_params.push(make_param("value", ty, false));
                }
            } else {
                // Get actual param names from Param table
                let pstart = m.param_list_start as usize;
                let pend = m.param_list_end as usize;
                let method_params: Vec<&CsParam> = params
                    .iter()
                    .skip(pstart.saturating_sub(1))
                    .take((pend - pstart).min(params.len()))
                    .filter(|p| p.sequence > 0)
                    .collect();

                for (i, (ty, _is_byref)) in sig_params.iter().enumerate() {
                    let pname = method_params.get(i)
                        .map(|p| sanitize_param_name(&p.name))
                        .unwrap_or_else(|| format!("p{i}"));
                    let _is_out = method_params.get(i)
                        .map(|p| p.flags & PARAM_OUT != 0)
                        .unwrap_or(false);
                    arrow_params.push(make_param(&pname, ty, false));
                }
            }

            // Template params for generic methods
            let tmpl: Vec<TemplateParam> = m.generic_param_names.iter()
                .map(|n| TemplateParam { name: n.clone(), constraints: vec![] })
                .collect();

            // Setter returns None
            let eff_ret = match &m.property_role {
                Some(PropertyRole::Setter(_)) => "None".to_string(),
                _ => ret_type,
            };

            body_stmts.push(make_fn_stub(
                &arrow_name,
                arrow_params,
                &eff_ret,
                m.is_static,
                is_interface,
                tmpl,
            ));
        }

        if is_interface {
            stmts.push(Stmt::TraitDef {
                name: td.name.clone(),
                template_params,
                body: body_stmts,
            });
        } else {
            stmts.push(Stmt::ClassDef {
                name: td.name.clone(),
                template_params,
                bases,
                decorators: vec![],
                body: body_stmts,
            });
        }
    }

    Ok(stmts)
}

// Decode a method's signature blob → (return_type_arrow, [(param_type, is_byref)])
fn decode_method_sig(
    data: &[u8],
    streams: &Streams,
    layout: &TildeLayout,
    m: &CsMethod,
    params: &[CsParam],
    type_names: &HashMap<u32, String>,
    type_params: &[String],
) -> (String, Vec<(String, bool)>) {
    let blob = read_blob(data, streams.blob_off, m.sig_blob_idx);
    let mut reader = SigReader {
        data: blob,
        pos: 0,
        type_names,
        type_params,
        method_params: &m.generic_param_names,
    };
    let (ret, sig_params) = reader.parse_method_sig(&m.generic_param_names);
    (ret, sig_params)
}

// Map C# operator method names to Arrow dunder names
fn operator_name(cs_name: &str) -> Option<&'static str> {
    match cs_name {
        "op_Addition" => Some("__add__"),
        "op_Subtraction" => Some("__sub__"),
        "op_Multiply" => Some("__mul__"),
        "op_Division" => Some("__truediv__"),
        "op_Modulus" => Some("__mod__"),
        "op_Equality" => Some("__eq__"),
        "op_Inequality" => Some("__ne__"),
        "op_LessThan" => Some("__lt__"),
        "op_GreaterThan" => Some("__gt__"),
        "op_LessThanOrEqual" => Some("__le__"),
        "op_GreaterThanOrEqual" => Some("__ge__"),
        "op_UnaryNegation" => Some("__neg__"),
        "op_UnaryPlus" => Some("__pos__"),
        "op_BitwiseAnd" => Some("__and__"),
        "op_BitwiseOr" => Some("__or__"),
        "op_ExclusiveOr" => Some("__xor__"),
        "op_LeftShift" => Some("__lshift__"),
        "op_RightShift" => Some("__rshift__"),
        "op_OnesComplement" => Some("__invert__"),
        _ => None,
    }
}

fn sanitize_param_name(name: &str) -> String {
    // C# reserved words that are valid C# param names but not valid Arrow names
    match name {
        "type" => "type_".to_string(),
        "class" => "class_".to_string(),
        "fn" => "fn_".to_string(),
        "let" => "let_".to_string(),
        "mut" => "mut_".to_string(),
        other => other.to_string(),
    }
}

const T_STANDALONESIG: usize = 0x11;
