"use strict";
// cs_assembly.ts — ECMA-335 (.NET assembly) metadata reader for the VS Code extension.
// Ported from src/parser/cs_assembly.rs.
// Reads a .NET DLL directly (no external tools) and returns NativeModuleInfo.
Object.defineProperty(exports, "__esModule", { value: true });
exports.parseNetAssembly = void 0;
// ---------------------------------------------------------------------------
// PE / CLI constants
// ---------------------------------------------------------------------------
const PE_SIG = 0x4550; // "PE\0\0" little-endian
const BSJB = 0x424A5342; // "BSJB" — metadata root signature
// Metadata table indices (ECMA-335 §II.22)
const T_MODULE = 0x00;
const T_TYPEREF = 0x01;
const T_TYPEDEF = 0x02;
const T_FIELD = 0x04;
const T_METHODDEF = 0x06;
const T_PARAM = 0x08;
const T_INTERFACEIMPL = 0x09;
const T_MEMBERREF = 0x0A;
const T_STANDALONESIG = 0x11;
const T_PROPERTY = 0x17;
const T_METHODSEMANTICS = 0x18;
const T_MODULEREF = 0x1A;
const T_TYPESPEC = 0x1B;
const T_ASSEMBLY = 0x20;
const T_ASSEMBLYREF = 0x23;
const T_GENERICPARAM = 0x2A;
// TypeDef visibility flags
const TD_PUBLIC = 0x01;
const TD_NESTED_PUBLIC = 0x02;
const TD_INTERFACE = 0x20;
// MethodDef flags
const MD_STATIC = 0x10;
const MD_SPECIAL_NAME = 0x0800;
// MethodSemantics
const SEM_GETTER = 0x02;
const SEM_ADDON = 0x08;
const SEM_REMOVEON = 0x10;
// Element type codes (ECMA-335 §II.23.1.16)
const ET_VOID = 0x01;
const ET_BOOLEAN = 0x02;
const ET_CHAR = 0x03;
const ET_I1 = 0x04;
const ET_U1 = 0x05;
const ET_I2 = 0x06;
const ET_U2 = 0x07;
const ET_I4 = 0x08;
const ET_U4 = 0x09;
const ET_I8 = 0x0A;
const ET_U8 = 0x0B;
const ET_R4 = 0x0C;
const ET_R8 = 0x0D;
const ET_STRING = 0x0E;
const ET_PTR = 0x0F;
const ET_BYREF = 0x10;
const ET_VALUETYPE = 0x11;
const ET_CLASS = 0x12;
const ET_VAR = 0x13;
const ET_ARRAY = 0x14;
const ET_GENERICINST = 0x15;
const ET_I = 0x18;
const ET_U = 0x19;
const ET_FNPTR = 0x1B;
const ET_OBJECT = 0x1C;
const ET_SZARRAY = 0x1D;
const ET_MVAR = 0x1E;
const ET_CMOD_REQD = 0x1F;
const ET_CMOD_OPT = 0x20;
const ET_SENTINEL = 0x41;
const ET_PINNED = 0x45;
// ---------------------------------------------------------------------------
// Byte helpers
// ---------------------------------------------------------------------------
function u16le(buf, off) { return buf.readUInt16LE(off); }
function u32le(buf, off) { return buf.readUInt32LE(off); }
function decompressUInt(buf, pos) {
    const b0 = buf[pos];
    if ((b0 & 0x80) === 0)
        return [b0, 1];
    if ((b0 & 0xC0) === 0x80)
        return [((b0 & 0x3F) << 8) | buf[pos + 1], 2];
    return [((b0 & 0x1F) << 24) | (buf[pos + 1] << 16) | (buf[pos + 2] << 8) | buf[pos + 3], 4];
}
function rvaToOffset(rva, sections) {
    for (const s of sections) {
        if (rva >= s.virtAddr && rva < s.virtAddr + Math.max(s.virtSize, 1)) {
            return rva - s.virtAddr + s.rawAddr;
        }
    }
}
// ---------------------------------------------------------------------------
// Find CLI metadata root
// ---------------------------------------------------------------------------
function findMetadataRoot(buf) {
    try {
        const eLfanew = u32le(buf, 0x3C);
        if (u32le(buf, eLfanew) !== PE_SIG)
            return null;
        const coff = eLfanew + 4;
        const numSections = u16le(buf, coff + 2);
        const optHdrSize = u16le(buf, coff + 16);
        const optHdr = coff + 20;
        const magic = u16le(buf, optHdr);
        const dataDirsOff = optHdr + (magic === 0x020B ? 112 : 96);
        const cliRva = u32le(buf, dataDirsOff + 14 * 8);
        if (cliRva === 0)
            return null;
        const sections = [];
        const sectOff = optHdr + optHdrSize;
        for (let i = 0; i < numSections; i++) {
            const sh = sectOff + i * 40;
            sections.push({ virtAddr: u32le(buf, sh + 12), virtSize: u32le(buf, sh + 8), rawAddr: u32le(buf, sh + 20) });
        }
        const cliOff = rvaToOffset(cliRva, sections);
        if (cliOff === undefined)
            return null;
        const metaRva = u32le(buf, cliOff + 8);
        const metaOff = rvaToOffset(metaRva, sections);
        if (metaOff === undefined)
            return null;
        if (u32le(buf, metaOff) !== BSJB)
            return null;
        return { metaOff, sections };
    }
    catch {
        return null;
    }
}
function findStreams(buf, meta) {
    try {
        const verLen = u32le(buf, meta + 12);
        const verAligned = (verLen + 3) & ~3;
        let pos = meta + 16 + verAligned;
        const numStreams = u16le(buf, pos + 2);
        pos += 4;
        let tilde, strings, blob;
        for (let i = 0; i < numStreams; i++) {
            const offset = u32le(buf, pos);
            pos += 8;
            const nameStart = pos;
            while (pos < buf.length && buf[pos] !== 0)
                pos++;
            const name = buf.toString('ascii', nameStart, pos);
            pos = (pos + 4) & ~3;
            if (name === '#~' || name === '#-')
                tilde = meta + offset;
            else if (name === '#Strings')
                strings = meta + offset;
            else if (name === '#Blob')
                blob = meta + offset;
        }
        if (tilde === undefined || strings === undefined || blob === undefined)
            return null;
        return { tildeOff: tilde, stringsOff: strings, blobOff: blob };
    }
    catch {
        return null;
    }
}
function strSz(t) { return (t.heapSizes & 0x01) ? 4 : 2; }
function blobSz(t) { return (t.heapSizes & 0x04) ? 4 : 2; }
function guidSz(t) { return (t.heapSizes & 0x02) ? 4 : 2; }
function tblSz(t, tbl) { return t.rows[tbl] > 0xFFFF ? 4 : 2; }
function codedSz(t, tables, tagBits) {
    const max = tables.reduce((m, x) => Math.max(m, t.rows[x]), 0);
    return max > ((1 << (16 - tagBits)) - 1) ? 4 : 2;
}
function readIdx(buf, off, sz) {
    return sz === 2 ? u16le(buf, off) : u32le(buf, off);
}
function computeRowSizes(t) {
    const s = strSz(t), g = guidSz(t), b = blobSz(t);
    const typeDefOrRef = codedSz(t, [T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2);
    const hasSem = codedSz(t, [0x14, T_PROPERTY], 1);
    const resScp = codedSz(t, [T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2);
    const tomDef = codedSz(t, [T_TYPEDEF, T_METHODDEF], 1);
    const td = tblSz(t, T_TYPEDEF), fi = tblSz(t, T_FIELD), md = tblSz(t, T_METHODDEF), pa = tblSz(t, T_PARAM), pr = tblSz(t, T_PROPERTY), gp = tblSz(t, T_GENERICPARAM), ar = tblSz(t, T_ASSEMBLYREF);
    const rs = t.tableRowSizes;
    rs[T_MODULE] = 2 + s + g + g + g;
    rs[T_TYPEREF] = resScp + s + s;
    rs[T_TYPEDEF] = 4 + s + s + typeDefOrRef + fi + md;
    rs[T_FIELD] = 2 + s + b;
    rs[T_METHODDEF] = 4 + 2 + 2 + s + b + pa;
    rs[T_PARAM] = 2 + 2 + s;
    rs[T_INTERFACEIMPL] = td + typeDefOrRef;
    rs[T_MEMBERREF] = codedSz(t, [T_TYPEREF, T_MODULEREF, T_METHODDEF, T_TYPEDEF, T_TYPESPEC], 3) + s + b;
    rs[0x0B] = 2 + codedSz(t, [T_FIELD, T_PARAM, 0x17], 2) + b;
    rs[0x0C] = codedSz(t, [T_METHODDEF, T_FIELD, T_TYPEREF, T_TYPEDEF, T_PARAM, T_INTERFACEIMPL, T_MEMBERREF, T_MODULE, 0x0E, T_PROPERTY, 0x14, T_STANDALONESIG, T_MODULEREF, T_TYPESPEC, T_ASSEMBLY, T_ASSEMBLYREF, T_FIELD, T_PARAM, 0x2A], 5)
        + codedSz(t, [T_METHODDEF, T_MEMBERREF], 3) + b;
    rs[0x0D] = codedSz(t, [T_FIELD, T_PARAM], 1) + b;
    rs[0x0E] = 2 + codedSz(t, [T_TYPEDEF, T_METHODDEF, T_ASSEMBLY], 2) + b;
    rs[0x0F] = 2 + 4 + td;
    rs[0x10] = 4 + fi;
    rs[T_STANDALONESIG] = b;
    rs[0x12] = td + tblSz(t, 0x14);
    rs[0x14] = 2 + s + typeDefOrRef;
    rs[0x15] = td + pr;
    rs[T_PROPERTY] = 2 + s + b;
    rs[T_METHODSEMANTICS] = 2 + md + hasSem;
    rs[0x19] = td + codedSz(t, [T_METHODDEF, T_MEMBERREF], 1) * 2;
    rs[T_MODULEREF] = s;
    rs[T_TYPESPEC] = b;
    rs[0x1C] = 2 + codedSz(t, [T_FIELD, T_METHODDEF], 1) + s + tblSz(t, T_MODULEREF);
    rs[0x1D] = 4 + fi;
    rs[T_ASSEMBLY] = 4 + 2 + 2 + 2 + 2 + 4 + b + s + s;
    rs[0x21] = 4;
    rs[0x22] = 12;
    rs[T_ASSEMBLYREF] = 2 + 2 + 2 + 2 + 4 + b + s + s + b;
    rs[0x24] = 4 + ar;
    rs[0x25] = 12 + ar;
    rs[0x26] = 4 + s + b;
    rs[0x27] = 4 + 4 + s + s + codedSz(t, [T_ASSEMBLY, 0x26, 0x1B, 0x27, T_TYPEDEF], 2);
    rs[0x28] = 4 + 4 + s + codedSz(t, [T_ASSEMBLY, 0x26, 0x23, 0x1B], 2);
    rs[0x29] = td + td;
    rs[T_GENERICPARAM] = 2 + 2 + tomDef + s;
    rs[0x2B] = codedSz(t, [T_METHODDEF, T_MEMBERREF], 1) + b;
    rs[0x2C] = gp + typeDefOrRef;
}
function parseTilde(buf, tildeStart) {
    const heapSizes = buf[tildeStart + 6];
    const validLo = u32le(buf, tildeStart + 8);
    const validHi = u32le(buf, tildeStart + 12);
    const rows = new Array(64).fill(0);
    let pos = tildeStart + 24;
    for (let i = 0; i < 64; i++) {
        const present = i < 32 ? (validLo >>> i) & 1 : (validHi >>> (i - 32)) & 1;
        if (present) {
            rows[i] = u32le(buf, pos);
            pos += 4;
        }
    }
    const dataStart = pos;
    const layout = { heapSizes, rows, tableOffsets: new Array(64).fill(0), tableRowSizes: new Array(64).fill(0) };
    computeRowSizes(layout);
    let off = dataStart;
    for (let i = 0; i < 64; i++) {
        const present = i < 32 ? (validLo >>> i) & 1 : (validHi >>> (i - 32)) & 1;
        if (present && rows[i] > 0) {
            layout.tableOffsets[i] = off;
            off += layout.tableRowSizes[i] * rows[i];
        }
    }
    return layout;
}
// ---------------------------------------------------------------------------
// Heap accessors
// ---------------------------------------------------------------------------
function readString(buf, stringsOff, idx) {
    const start = stringsOff + idx;
    let end = start;
    while (end < buf.length && buf[end] !== 0)
        end++;
    return buf.toString('utf8', start, end);
}
function readBlob(buf, blobOff, idx) {
    const start = blobOff + idx;
    const [len, hdr] = decompressUInt(buf, start);
    return buf.subarray(start + hdr, start + hdr + len);
}
// ---------------------------------------------------------------------------
// Generic type name mapping (C# → Arrow)
// ---------------------------------------------------------------------------
function mapGeneric(base, args) {
    var _a;
    const stripped = ((_a = base.split('.').pop()) !== null && _a !== void 0 ? _a : base).split('`')[0];
    switch (stripped) {
        case 'List':
        case 'IList':
        case 'ICollection':
        case 'IEnumerable':
        case 'IReadOnlyList':
        case 'IReadOnlyCollection':
        case 'ObservableCollection':
        case 'Collection':
        case 'Queue':
        case 'Stack':
        case 'LinkedList':
        case 'ImmutableList':
            return args.length === 1 ? `list[${args[0]}]` : 'list';
        case 'Dictionary':
        case 'IDictionary':
        case 'IReadOnlyDictionary':
        case 'SortedDictionary':
        case 'ConcurrentDictionary':
            return args.length === 2 ? `dict[${args[0]},${args[1]}]` : 'dict';
        case 'HashSet':
        case 'SortedSet':
        case 'ISet':
        case 'ImmutableHashSet':
            return args.length === 1 ? `set[${args[0]}]` : 'set';
        case 'Tuple':
        case 'ValueTuple':
            return `tuple[${args.join(',')}]`;
        case 'Nullable':
            return args.length === 1 ? `Option[${args[0]}]` : 'Any';
        case 'Task':
        case 'ValueTask':
            return args.length === 1 ? args[0] : 'None';
        case 'Action':
        case 'Func':
        case 'Predicate':
        case 'EventHandler':
        case 'Delegate':
            return 'function';
        case 'KeyValuePair':
            return args.length === 2 ? `tuple[${args[0]},${args[1]}]` : 'tuple';
        default:
            return args.length === 0 ? stripped : `${stripped}[${args.join(',')}]`;
    }
}
// ---------------------------------------------------------------------------
// Method signature blob decoder
// ---------------------------------------------------------------------------
class SigReader {
    constructor(data, typeNames, typeParams, methodParams) {
        this.data = data;
        this.typeNames = typeNames;
        this.typeParams = typeParams;
        this.methodParams = methodParams;
        this.pos = 0;
    }
    peek() { return this.pos < this.data.length ? this.data[this.pos] : 0; }
    eat() { var _a; return (_a = this.data[this.pos++]) !== null && _a !== void 0 ? _a : 0; }
    eatUInt() { const [v, n] = decompressUInt(this.data, this.pos); this.pos += n; return v; }
    skipCmods() {
        while (this.peek() === ET_CMOD_REQD || this.peek() === ET_CMOD_OPT) {
            this.eat();
            this.eatUInt();
        }
    }
    parseType() {
        var _a, _b, _c, _d;
        this.skipCmods();
        const et = this.eat();
        switch (et) {
            case ET_VOID: return 'None';
            case ET_BOOLEAN: return 'bool';
            case ET_CHAR: return 'str';
            case ET_I1:
            case ET_U1:
            case ET_I2:
            case ET_U2:
            case ET_I4:
            case ET_U4:
            case ET_I8:
            case ET_U8:
            case ET_I:
            case ET_U: return 'int';
            case ET_R4:
            case ET_R8: return 'float';
            case ET_STRING: return 'str';
            case ET_OBJECT: return 'Any';
            case ET_BYREF: return this.parseType();
            case ET_VALUETYPE:
            case ET_CLASS:
                return (_a = this.typeNames.get(this.eatUInt())) !== null && _a !== void 0 ? _a : 'Any';
            case ET_GENERICINST: {
                this.eat(); // CLASS or VALUETYPE
                const base = (_b = this.typeNames.get(this.eatUInt())) !== null && _b !== void 0 ? _b : '';
                const cnt = this.eatUInt();
                const args = [];
                for (let i = 0; i < cnt; i++)
                    args.push(this.parseType());
                return mapGeneric(base, args);
            }
            case ET_SZARRAY:
                this.skipCmods();
                return `list[${this.parseType()}]`;
            case ET_ARRAY: {
                const elem = this.parseType();
                const rank = this.eatUInt();
                const nsz = this.eatUInt();
                for (let i = 0; i < nsz; i++)
                    this.eatUInt();
                const nlb = this.eatUInt();
                for (let i = 0; i < nlb; i++)
                    this.eatUInt();
                void rank;
                return `list[${elem}]`;
            }
            case ET_VAR: {
                const i = this.eatUInt();
                return (_c = this.typeParams[i]) !== null && _c !== void 0 ? _c : `T${i}`;
            }
            case ET_MVAR: {
                const i = this.eatUInt();
                return (_d = this.methodParams[i]) !== null && _d !== void 0 ? _d : `M${i}`;
            }
            case ET_FNPTR:
                this.skipMethodSig();
                return 'function';
            case ET_PTR:
                this.skipCmods();
                this.parseType();
                return 'int';
            case ET_SENTINEL:
            case ET_PINNED: return this.parseType();
            default: return 'Any';
        }
    }
    skipMethodSig() {
        const c = this.eat();
        if (c & 0x10)
            this.eatUInt();
        const n = this.eatUInt();
        this.parseType();
        for (let i = 0; i < n; i++) {
            if (this.peek() === ET_SENTINEL)
                this.eat();
            this.parseType();
        }
    }
    parseMethodSig() {
        const calling = this.eat();
        if (calling & 0x10)
            this.eatUInt(); // generic param count
        const cnt = this.eatUInt();
        const ret = this.parseType();
        const params = [];
        for (let i = 0; i < cnt; i++) {
            if (this.peek() === ET_SENTINEL) {
                this.eat();
                continue;
            }
            const isByRef = this.peek() === ET_BYREF;
            params.push({ type: this.parseType(), isByRef });
        }
        return { ret, params };
    }
}
// ---------------------------------------------------------------------------
// Operator method name mapping
// ---------------------------------------------------------------------------
const OP_NAMES = {
    op_Addition: '__add__', op_Subtraction: '__sub__', op_Multiply: '__mul__',
    op_Division: '__truediv__', op_Modulus: '__mod__',
    op_Equality: '__eq__', op_Inequality: '__ne__',
    op_LessThan: '__lt__', op_GreaterThan: '__gt__',
    op_LessThanOrEqual: '__le__', op_GreaterThanOrEqual: '__ge__',
    op_UnaryNegation: '__neg__', op_UnaryPlus: '__pos__',
    op_BitwiseAnd: '__and__', op_BitwiseOr: '__or__', op_ExclusiveOr: '__xor__',
    op_LeftShift: '__lshift__', op_RightShift: '__rshift__', op_OnesComplement: '__invert__',
};
function sanitizeParam(name) {
    var _a;
    return (_a = { type: 'type_', class: 'class_', fn: 'fn_', let: 'let_', mut: 'mut_' }[name]) !== null && _a !== void 0 ? _a : name;
}
// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------
function parseNetAssembly(buf) {
    var _a, _b, _c, _d, _e, _f, _g, _h, _j, _k, _l, _m, _o;
    const empty = { funcs: new Map(), sigs: new Map(), docs: new Map(), classes: new Map() };
    const root = findMetadataRoot(buf);
    if (!root)
        return empty;
    const streams = findStreams(buf, root.metaOff);
    if (!streams)
        return empty;
    const { tildeOff, stringsOff, blobOff } = streams;
    let layout;
    try {
        layout = parseTilde(buf, tildeOff);
    }
    catch {
        return empty;
    }
    const sSz = strSz(layout), bSz = blobSz(layout);
    const fiSz = tblSz(layout, T_FIELD), mdSz = tblSz(layout, T_METHODDEF);
    const paSz = tblSz(layout, T_PARAM), tdIdxSz = tblSz(layout, T_TYPEDEF);
    const tdrSz = codedSz(layout, [T_TYPEDEF, T_TYPEREF, T_TYPESPEC], 2);
    const hasSemSz = codedSz(layout, [0x14, T_PROPERTY], 1);
    const tomSz = codedSz(layout, [T_TYPEDEF, T_METHODDEF], 1);
    const resSz = codedSz(layout, [T_MODULE, T_MODULEREF, T_ASSEMBLYREF, T_TYPEREF], 2);
    try {
        // ── TypeRef → coded-index name table ──────────────────────────────────
        const typeNames = new Map();
        const trRows = layout.rows[T_TYPEREF];
        for (let row = 0; row < trRows; row++) {
            const off = layout.tableOffsets[T_TYPEREF] + row * layout.tableRowSizes[T_TYPEREF];
            const nameIdx = readIdx(buf, off + resSz, sSz);
            const nsIdx = readIdx(buf, off + resSz + sSz, sSz);
            const name = readString(buf, stringsOff, nameIdx);
            const ns = readString(buf, stringsOff, nsIdx);
            const coded = ((row + 1) << 2) | 1;
            const simple = name.split('`')[0];
            typeNames.set(coded, ns ? `${ns}.${simple}` : simple);
        }
        // ── GenericParam → (TypeDef|MethodDef) row → sorted param name list ──
        const typeGP = new Map();
        const methodGP = new Map();
        const gpRows = layout.rows[T_GENERICPARAM];
        for (let row = 0; row < gpRows; row++) {
            const off = layout.tableOffsets[T_GENERICPARAM] + row * layout.tableRowSizes[T_GENERICPARAM];
            const num = u16le(buf, off);
            const owner = readIdx(buf, off + 4, tomSz);
            const nIdx = readIdx(buf, off + 4 + tomSz, sSz);
            const name = readString(buf, stringsOff, nIdx);
            const tag = owner & 1;
            const row1 = owner >> 1;
            const map = tag === 0 ? typeGP : methodGP;
            if (!map.has(row1))
                map.set(row1, []);
            map.get(row1).push([num, name]);
        }
        const sortedGP = (map, key) => { var _a; return ((_a = map.get(key)) !== null && _a !== void 0 ? _a : []).sort((a, b) => a[0] - b[0]).map(([, n]) => n); };
        const tdRows = layout.rows[T_TYPEDEF];
        const typedefs = [];
        for (let row = 0; row < tdRows; row++) {
            const off = layout.tableOffsets[T_TYPEDEF] + row * layout.tableRowSizes[T_TYPEDEF];
            const flags = u32le(buf, off);
            const nameIdx = readIdx(buf, off + 4, sSz);
            const methOff = 4 + sSz + sSz + tdrSz + fiSz;
            const methStart = readIdx(buf, off + methOff, mdSz);
            const name = readString(buf, stringsOff, nameIdx);
            const coded = ((row + 1) << 2) | 0;
            const simple = name.split('`')[0];
            typeNames.set(coded, simple);
            typedefs.push({ name: simple, flags, methodListStart: methStart, methodListEnd: 0, genericParams: sortedGP(typeGP, row + 1), interfaceNames: [] });
        }
        const mdTotal = layout.rows[T_METHODDEF] + 1;
        for (let i = 0; i < typedefs.length; i++) {
            typedefs[i].methodListEnd = i + 1 < typedefs.length ? typedefs[i + 1].methodListStart : mdTotal;
        }
        // ── InterfaceImpl ─────────────────────────────────────────────────────
        const iiRows = layout.rows[T_INTERFACEIMPL];
        for (let row = 0; row < iiRows; row++) {
            const off = layout.tableOffsets[T_INTERFACEIMPL] + row * layout.tableRowSizes[T_INTERFACEIMPL];
            const td1 = readIdx(buf, off, tdIdxSz);
            const ifCoded = readIdx(buf, off + tdIdxSz, tdrSz);
            const ifName = (_b = ((_a = typeNames.get(ifCoded)) !== null && _a !== void 0 ? _a : '').split('.').pop()) !== null && _b !== void 0 ? _b : '';
            if (ifName && ifName !== 'IDisposable')
                (_c = typedefs[td1 - 1]) === null || _c === void 0 ? void 0 : _c.interfaceNames.push(ifName);
        }
        const paramRows = layout.rows[T_PARAM];
        const allParams = [];
        for (let row = 0; row < paramRows; row++) {
            const off = layout.tableOffsets[T_PARAM] + row * layout.tableRowSizes[T_PARAM];
            const flags = u16le(buf, off);
            const seq = u16le(buf, off + 2);
            const nIdx = readIdx(buf, off + 4, sSz);
            allParams.push({ sequence: seq, name: readString(buf, stringsOff, nIdx), flags });
        }
        const paramTotal = paramRows + 1;
        const methodRole = new Map();
        if (layout.rows[T_METHODSEMANTICS] > 0 && layout.rows[T_PROPERTY] > 0) {
            const propNames = new Map();
            for (let row = 0; row < layout.rows[T_PROPERTY]; row++) {
                const off = layout.tableOffsets[T_PROPERTY] + row * layout.tableRowSizes[T_PROPERTY];
                const nIdx = readIdx(buf, off + 2, sSz);
                propNames.set(row + 1, readString(buf, stringsOff, nIdx));
            }
            for (let row = 0; row < layout.rows[T_METHODSEMANTICS]; row++) {
                const off = layout.tableOffsets[T_METHODSEMANTICS] + row * layout.tableRowSizes[T_METHODSEMANTICS];
                const sem = u16le(buf, off);
                const meth1 = readIdx(buf, off + 2, mdSz);
                const assoc = readIdx(buf, off + 2 + mdSz, hasSemSz);
                const aTag = assoc & 1;
                const aRow = assoc >> 1;
                if (aTag === 1) {
                    const pn = (_d = propNames.get(aRow)) !== null && _d !== void 0 ? _d : '';
                    if (pn)
                        methodRole.set(meth1, { kind: (sem & SEM_GETTER) ? 'getter' : 'setter', name: pn });
                }
                else if (sem & SEM_ADDON || sem & SEM_REMOVEON) {
                    methodRole.set(meth1, { kind: 'event', name: '' });
                }
            }
        }
        const mdRows = layout.rows[T_METHODDEF];
        const allMethods = [];
        for (let row = 0; row < mdRows; row++) {
            const off = layout.tableOffsets[T_METHODDEF] + row * layout.tableRowSizes[T_METHODDEF];
            const flags = u16le(buf, off + 6);
            const nIdx = readIdx(buf, off + 8, sSz);
            const sigIdx = readIdx(buf, off + 8 + sSz, bSz);
            const pStart = readIdx(buf, off + 8 + sSz + bSz, paSz);
            const meth1 = row + 1;
            allMethods.push({
                name: readString(buf, stringsOff, nIdx),
                isStatic: (flags & MD_STATIC) !== 0,
                isPublic: (flags & 0x07) === 6,
                flags, sigBlobIdx: sigIdx,
                paramListStart: pStart, paramListEnd: 0,
                genericParams: sortedGP(methodGP, meth1),
                role: methodRole.get(meth1),
            });
        }
        for (let i = 0; i < allMethods.length; i++) {
            allMethods[i].paramListEnd = i + 1 < allMethods.length ? allMethods[i + 1].paramListStart : paramTotal;
        }
        // ── Build NativeModuleInfo ────────────────────────────────────────────
        const classes = new Map();
        for (const td of typedefs) {
            const vis = td.flags & 0x07;
            if (vis !== TD_PUBLIC && vis !== TD_NESTED_PUBLIC)
                continue;
            if (!td.name || td.name === '<Module>')
                continue;
            if (td.name.startsWith('<') || td.name.startsWith('_'))
                continue;
            const isInterface = (td.flags & TD_INTERFACE) !== 0;
            const methods = new Map();
            const methodSigs = [];
            for (let md1 = td.methodListStart; md1 < td.methodListEnd; md1++) {
                const m = allMethods[md1 - 1];
                if (!m || !m.isPublic)
                    continue;
                if (((_e = m.role) === null || _e === void 0 ? void 0 : _e.kind) === 'event')
                    continue;
                const isAccessor = ((_f = m.role) === null || _f === void 0 ? void 0 : _f.kind) === 'getter' || ((_g = m.role) === null || _g === void 0 ? void 0 : _g.kind) === 'setter';
                if ((m.flags & MD_SPECIAL_NAME) !== 0 && !isAccessor) {
                    if (m.name === '.ctor' || m.name === '.cctor')
                        continue;
                    const opName = OP_NAMES[m.name];
                    if (!opName)
                        continue;
                    const blob = readBlob(buf, blobOff, m.sigBlobIdx);
                    const { ret, params } = new SigReader(blob, typeNames, td.genericParams, m.genericParams).parseMethodSig();
                    const sig = `fn ${opName}(self: Self${params.map((p, i) => `, p${i}: ${p.type}`).join('')}) -> ${ret}`;
                    methods.set(opName, { ret, sig });
                    methodSigs.push(sig);
                    continue;
                }
                const arrowName = ((_h = m.role) === null || _h === void 0 ? void 0 : _h.kind) === 'getter' ? `get${m.role.name}`
                    : ((_j = m.role) === null || _j === void 0 ? void 0 : _j.kind) === 'setter' ? `set${m.role.name}`
                        : m.name;
                const blob = readBlob(buf, blobOff, m.sigBlobIdx);
                const { ret, params } = new SigReader(blob, typeNames, td.genericParams, m.genericParams).parseMethodSig();
                const pstart = m.paramListStart;
                const pend = m.paramListEnd;
                const mParams = allParams.slice(pstart > 0 ? pstart - 1 : 0, pend - 1).filter(p => p.sequence > 0);
                const parts = [];
                if (!m.isStatic)
                    parts.push('self: Self');
                if (((_k = m.role) === null || _k === void 0 ? void 0 : _k.kind) === 'setter') {
                    parts.push(`value: ${(_m = (_l = params[0]) === null || _l === void 0 ? void 0 : _l.type) !== null && _m !== void 0 ? _m : 'Any'}`);
                }
                else {
                    for (let i = 0; i < params.length; i++) {
                        const pname = mParams[i] ? sanitizeParam(mParams[i].name) : `p${i}`;
                        parts.push(`${pname}: ${params[i].type}`);
                    }
                }
                const effRet = ((_o = m.role) === null || _o === void 0 ? void 0 : _o.kind) === 'setter' ? 'None' : ret;
                const kwStatic = m.isStatic ? 'static ' : '';
                const sig = `${kwStatic}fn ${arrowName}(${parts.join(', ')}) -> ${effRet}`;
                methods.set(arrowName, { ret: effRet, sig });
                methodSigs.push(sig);
            }
            classes.set(td.name, { fields: new Map(), fieldSigs: [], methods, methodSigs });
        }
        return { funcs: new Map(), sigs: new Map(), docs: new Map(), classes };
    }
    catch {
        return empty;
    }
}
exports.parseNetAssembly = parseNetAssembly;
//# sourceMappingURL=cs_assembly.js.map