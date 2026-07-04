// cs_assembly/metadata.rs — PE/CLI メタデータの低レベル読み取り関数: バイトヘルパー、PE セクション/メタデータルート/ストリーム解決、#~ テーブルレイアウト計算、文字列/blob 読み取り。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::Path,
    crate::ast::{Accessibility, Param, Stmt, TemplateParam},
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// Byte helpers
// ---------------------------------------------------------------------------

pub(crate) fn u16le(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}


pub(crate) fn u32le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}


// Decode ECMA-335 compressed unsigned integer; returns (value, bytes_consumed).
pub(crate) fn decompress_uint(data: &[u8], pos: usize) -> (u32, usize) {
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


pub(crate) fn rva_to_offset(rva: u32, sections: &[PeSection]) -> Option<usize> {
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

pub(crate) fn find_metadata_root(data: &[u8]) -> Result<(usize, Vec<PeSection>), String> {
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


pub(crate) fn find_streams(data: &[u8], meta: usize) -> Result<Streams, String> {
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


impl TildeLayout {
    // Size of a string heap index (2 or 4 bytes)
    pub(crate) fn str_idx_size(&self) -> usize {
        if self.heap_sizes & 0x01 != 0 { 4 } else { 2 }
    }
    // Size of a GUID heap index (2 or 4 bytes)
    pub(crate) fn guid_idx_size(&self) -> usize {
        if self.heap_sizes & 0x02 != 0 { 4 } else { 2 }
    }
    // Size of a blob heap index (2 or 4 bytes)
    pub(crate) fn blob_idx_size(&self) -> usize {
        if self.heap_sizes & 0x04 != 0 { 4 } else { 2 }
    }
    // Size of an index into a single table
    pub(crate) fn tbl_idx(&self, tbl: usize) -> usize {
        if self.rows[tbl] > 0xFFFF { 4 } else { 2 }
    }
    // Coded index: tables list + tag bits
    pub(crate) fn coded_idx(&self, tables: &[usize], tag_bits: u32) -> usize {
        let max_rows = tables.iter().map(|&t| self.rows[t]).max().unwrap_or(0);
        let threshold = (1u32 << (16 - tag_bits)).saturating_sub(1);
        if max_rows > threshold { 4 } else { 2 }
    }
}


pub(crate) fn parse_tilde(data: &[u8], tilde_start: usize) -> TildeLayout {
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
    pub(crate) fn compute_row_sizes(&mut self) {
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
    pub(crate) fn read_idx(&self, data: &[u8], off: usize, size: usize) -> u32 {
        if size == 2 { u16le(data, off) as u32 } else { u32le(data, off) }
    }

    // Read a column within a table row
    #[allow(dead_code)]
    pub(crate) fn col(&self, data: &[u8], tbl: usize, row: usize, col_offset: usize, col_size: usize) -> u32 {
        let off = self.table_offsets[tbl] + row * self.table_row_sizes[tbl] + col_offset;
        self.read_idx(data, off, col_size)
    }
}


// ---------------------------------------------------------------------------
// Heap accessors
// ---------------------------------------------------------------------------

pub(crate) fn read_string(data: &[u8], strings_off: usize, idx: u32) -> &str {
    let start = strings_off + idx as usize;
    let end = data[start..].iter().position(|&b| b == 0).map(|n| start + n).unwrap_or(start);
    std::str::from_utf8(&data[start..end]).unwrap_or("")
}


pub(crate) fn read_blob<'a>(data: &'a [u8], blob_off: usize, idx: u32) -> &'a [u8] {
    let start = blob_off + idx as usize;
    let (len, hdr) = decompress_uint(data, start);
    &data[start + hdr..start + hdr + len as usize]
}
