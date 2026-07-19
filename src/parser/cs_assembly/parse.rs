// cs_assembly/parse.rs — アセンブリ解析の中核と公開 API: parse_assembly、load_cs_assembly、generate_cs_stub_text。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::Path,
    crate::ast::{Accessibility, Param, Stmt, TemplateParam},
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// Main reader — parses all needed tables and builds CsTypeDef list
// ---------------------------------------------------------------------------

/// Parse a .NET assembly binary into intermediate tables.
/// Shared by `load_cs_assembly` and `generate_cs_stub_text`.
pub(crate) fn parse_assembly(path: &Path) -> Result<ParsedAssembly, String> {
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

        let coded = (row as u32 + 1) << 2;
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
        let seq = u16le(&data, off + 2);
        let name_idx = layout.read_idx(&data, off + 4, s_sz);
        let name = read_string(&data, streams.strings_off, name_idx).to_string();
        all_params.push(CsParam { sequence: seq, name });
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
