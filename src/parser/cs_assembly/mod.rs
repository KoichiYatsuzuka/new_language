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
// Section table: RVA → file offset
// ---------------------------------------------------------------------------

pub(crate) struct PeSection {
    virt_addr: u32,
    virt_size: u32,
    raw_addr: u32,
}


// ---------------------------------------------------------------------------
// Metadata stream headers
// ---------------------------------------------------------------------------

pub(crate) struct Streams {
    tilde_off: usize,
    strings_off: usize,
    blob_off: usize,
}


// ---------------------------------------------------------------------------
// #~ stream layout
// ---------------------------------------------------------------------------

pub(crate) struct TildeLayout {
    heap_sizes: u8,
    rows: [u32; 64],
    table_offsets: [usize; 64],
    table_row_sizes: [usize; 64],
}


// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

/// All data produced by parsing a .NET assembly binary.
/// Shared between `load_cs_assembly` (runtime stubs) and `generate_cs_stub_text` (file output).
pub(crate) struct ParsedAssembly {
    data: Vec<u8>,
    streams: Streams,
    layout: TildeLayout,
    typedefs: Vec<CsTypeDef>,
    all_methods: Vec<CsMethod>,
    all_params: Vec<CsParam>,
    type_names: HashMap<u32, String>,
}


#[derive(Debug, Clone)]
pub(crate) struct CsMethod {
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
pub(crate) enum PropertyRole {
    Getter(String),
    Setter(String),
    EventAdder(String),
}


#[derive(Debug, Clone)]
pub(crate) struct CsParam {
    sequence: u16,
    name: String,
    flags: u16,
}


#[derive(Debug, Clone)]
pub(crate) struct CsTypeDef {
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

pub(crate) struct SigReader<'a> {
    data: &'a [u8],
    pos: usize,
    type_names: &'a HashMap<u32, String>,
    type_params: &'a [String],
    method_params: &'a [String],
}


const T_STANDALONESIG: usize = 0x11;

mod metadata;
mod signature;
mod parse;
mod xml_docs;
mod stub_gen;
pub(crate) use metadata::*;
pub(crate) use parse::*;
pub(crate) use xml_docs::*;
pub(crate) use stub_gen::*;
