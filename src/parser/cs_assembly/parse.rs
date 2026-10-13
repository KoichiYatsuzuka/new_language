// cs_assembly/parse.rs — アセンブリ解析の中核と公開 API: parse_assembly、load_cs_assembly、generate_cs_stub_text。

use {
    std::collections::HashMap, std::path::Path,
    crate::ast::Stmt,
};
use super::*;


// ---------------------------------------------------------------------------
// Main reader — parses all needed tables and builds CsTypeDef list
// ---------------------------------------------------------------------------

/// Parse a .NET assembly binary into intermediate tables.
/// Shared by `load_cs_assembly` and `generate_cs_stub_text`.
pub(crate) fn parse_assembly(path: &Path) -> Result<ParsedAssembly, String> {
    let data = crate::import_fs::read(path)
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
                if sem & SEM_ADDON != 0 || sem & SEM_REMOVEON != 0 {
                    method_role.insert(meth_1, PropertyRole::EventAdder);
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

    let nullable = read_nullable(&data, &streams, &layout, &typedefs, &type_names);

    Ok(ParsedAssembly { data, streams, layout, typedefs, all_methods, all_params, type_names, nullable })
}


/// nullable 注釈の属性（`NullableAttribute` / `NullableContextAttribute`）を CustomAttribute の表から読む（2-1）。
///
/// 属性の型は、同じアセンブリに埋め込まれた型（コンストラクタが MethodDef）か、参照した型（MemberRef）の
/// どちらか。名前空間は `System.Runtime.CompilerServices` だが、型の単純名で見分ける。
fn read_nullable(
    data: &[u8],
    streams: &Streams,
    layout: &TildeLayout,
    typedefs: &[CsTypeDef],
    type_names: &HashMap<u32, String>,
) -> NullableInfo {
    let mut info = NullableInfo::default();

    // 入れ子の型 → 外側の型
    let td_sz = layout.tbl_idx(T_TYPEDEF);
    for row in 0..layout.rows[T_NESTEDCLASS] as usize {
        let off = layout.table_offsets[T_NESTEDCLASS] + row * layout.table_row_sizes[T_NESTEDCLASS];
        let nested = layout.read_idx(data, off, td_sz);
        let outer = layout.read_idx(data, off + td_sz, td_sz);
        info.enclosing.insert(nested, outer);
    }

    let ca_rows = layout.rows[T_CUSTOMATTRIBUTE] as usize;
    if ca_rows == 0 {
        return info;
    }
    let b_sz = layout.blob_idx_size();
    let mrp_sz = layout.member_ref_parent_idx();
    let parent_sz = layout.has_custom_attribute_idx();
    let type_sz = layout.custom_attribute_type_idx();

    // 型の単純名（TypeDef の行 / TypeRef の行から）
    let typedef_name = |row1: u32| -> Option<&str> {
        typedefs.get((row1 as usize).checked_sub(1)?).map(|t| t.name.as_str())
    };
    let typeref_name = |row1: u32| -> Option<&str> {
        type_names.get(&((row1 << 2) | 1)).map(|n| n.rsplit('.').next().unwrap_or(n))
    };
    // メソッドの行 → それを持つ型の行
    let owner_of_method = |m1: u32| -> Option<u32> {
        typedefs
            .iter()
            .position(|t| t.method_list_start <= m1 && m1 < t.method_list_end)
            .map(|i| i as u32 + 1)
    };
    let member_ref_class = |row1: u32| -> Option<&str> {
        let off = layout.table_offsets[T_MEMBERREF]
            + (row1 as usize).checked_sub(1)? * layout.table_row_sizes[T_MEMBERREF];
        let class = layout.read_idx(data, off, mrp_sz);
        match class & 0x7 {
            0 => typedef_name(class >> 3),
            1 => typeref_name(class >> 3),
            _ => None,
        }
    };

    for row in 0..ca_rows {
        let off = layout.table_offsets[T_CUSTOMATTRIBUTE] + row * layout.table_row_sizes[T_CUSTOMATTRIBUTE];
        let parent = layout.read_idx(data, off, parent_sz);
        let ty = layout.read_idx(data, off + parent_sz, type_sz);
        let value = layout.read_idx(data, off + parent_sz + type_sz, b_sz);
        let attr = match ty & 0x7 {
            2 => owner_of_method(ty >> 3).and_then(typedef_name),
            3 => member_ref_class(ty >> 3),
            _ => None,
        };
        let is_context = match attr {
            Some("NullableContextAttribute") => true,
            Some("NullableAttribute") => false,
            _ => continue,
        };
        let Some(flag) = first_nullable_flag(read_blob(data, streams.blob_off, value)) else { continue };
        let (tag, prow) = (parent & 0x1F, parent >> 5);
        match (tag, is_context) {
            (0, true) => { info.method_ctx.insert(prow, flag); }
            (3, true) => { info.type_ctx.insert(prow, flag); }
            (4, false) => { info.param.insert(prow, flag); }
            _ => {}
        }
    }
    info
}


/// 属性の値の blob（`01 00` の後に固定引数・最後に名前付き引数の個数 2 バイト）から、先頭の値を取る。
/// `NullableAttribute(byte)` / `NullableContextAttribute(byte)` は 1 バイト、`NullableAttribute(byte[])` は
/// 要素数（4 バイト）の後に並ぶ。配列の先頭が、いちばん外側の型の値。
fn first_nullable_flag(blob: &[u8]) -> Option<u8> {
    if blob.len() < 5 || blob[0] != 1 || blob[1] != 0 {
        return None;
    }
    if blob.len() == 5 {
        return Some(blob[2]);
    }
    let count = u32::from_le_bytes(blob.get(2..6)?.try_into().ok()?);
    if count == 0 || count == u32::MAX {
        return None;
    }
    blob.get(6).copied()
}


impl NullableInfo {
    /// メソッドの戻り値（`seq` = 0）か `seq` 番目の引数の nullable の値（2-1）。
    /// 引数自身の注釈 → メソッドの既定値 → 型の既定値 → 外側の型の既定値 → 0（注釈なし）の順に探す。
    /// ⚠ 値型（`int` など・`Nullable<T>` も）には意味が無い。参照型にだけ使うこと。
    pub(crate) fn flag(&self, m: &CsMethod, method_row: u32, td_row: u32, params: &[CsParam], seq: u16) -> u8 {
        for r in m.param_list_start..m.param_list_end {
            let Some(p) = (r as usize).checked_sub(1).and_then(|i| params.get(i)) else { continue };
            if p.sequence == seq {
                if let Some(f) = self.param.get(&r) {
                    return *f;
                }
                break;
            }
        }
        if let Some(f) = self.method_ctx.get(&method_row) {
            return *f;
        }
        let mut td = td_row;
        for _ in 0..32 {
            if let Some(f) = self.type_ctx.get(&td) {
                return *f;
            }
            match self.enclosing.get(&td) {
                Some(outer) => td = *outer,
                None => break,
            }
        }
        0
    }
}


/// Read a .NET assembly and return a map of (type_coded_index → Arrow type name)
/// plus all type definitions for stub generation.
pub fn load_cs_assembly(path: &Path) -> Result<Vec<Stmt>, String> {
    let pa = parse_assembly(path)?;
    generate_stubs(
        &pa.data, &pa.streams, &pa.layout,
        &pa.typedefs, &pa.all_methods, &pa.all_params, &pa.type_names, &pa.nullable,
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
        &pa.typedefs, &pa.all_methods, &pa.all_params, &pa.type_names, &pa.nullable,
    )?;
    let xml_path = path.with_extension("xml");
    let docs = parse_xml_docs(&xml_path);
    let text = render_cs_ars_text(&pa, &docs);
    Ok((stmts, text))
}


#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::ast::Expr;

    /// `examples/interop/cs_probe/cs_probe.dll`（build_cs_probe.ps1・nullable 有効）の型スタブ。
    /// キーは `クラス.メソッド`、値は `(引数の型, 戻り値の型, 未対応の理由)`。
    fn probe_stubs() -> HashMap<String, (Vec<String>, String, Option<String>)> {
        // ⚠ 拡張のクレート（`crates/arrow-frontend`）も同じファイルを取り込んでテストするので、リポジトリの直下を
        //   クレートの場所から上へ探す（`compare_wasm_frontend.ps1` の前段で走る）。
        let rel = Path::new("examples/interop/cs_probe/cs_probe.dll");
        let dll = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|d| d.join(rel))
            .find(|p| p.exists())
            .expect("examples/interop/cs_probe/cs_probe.dll (build_cs_probe.ps1)");
        let mut out = HashMap::new();
        for st in load_cs_assembly(&dll).expect("cs_probe.dll") {
            let Stmt::ClassDef { name: cls, body, .. } = st else { continue };
            for m in body {
                let Stmt::FnDef { name, params, return_type, body, .. } = m else { continue };
                let ps = params.iter().filter(|p| p.name != "self").map(|p| p.type_ann.clone().unwrap_or_default()).collect();
                let reason = match body.first() {
                    Some(Stmt::Expr(Expr::Str(s))) => s.strip_prefix(crate::cpp_header::FFI_UNSUPPORTED_MARK).map(str::to_string),
                    _ => None,
                };
                out.insert(format!("{cls}.{name}"), (ps, return_type.unwrap_or_default(), reason));
            }
        }
        out
    }

    /// `string?` / `Person?` / `long?` は `Option[T]`、注釈の無い参照型は `T` のまま（2-1）。
    #[test]
    fn nullable_annotations_become_option() {
        let st = probe_stubs();
        let sig = |k: &str| {
            let (ps, r, _) = st.get(k).unwrap_or_else(|| panic!("{k} missing"));
            (ps.iter().map(String::as_str).collect::<Vec<_>>().join(", "), r.clone())
        };
        assert_eq!(sig("Probe.greet"), (String::new(), "str".to_string()));
        assert_eq!(sig("Probe.maybe"), ("bool".to_string(), "Option[str]".to_string()));
        assert_eq!(sig("Probe.nothing").1, "Option[str]");
        assert_eq!(sig("Probe.maybe_num").1, "Option[int]");
        assert_eq!(sig("Probe.find"), ("str".to_string(), "Option[Person]".to_string()));
        assert_eq!(sig("Probe.describe_opt"), ("Option[Person]".to_string(), "str".to_string()));
        assert_eq!(sig("Probe.add_opt"), ("int, Option[int]".to_string(), "int".to_string()));
        assert_eq!(sig("Probe.make_person").1, "Person");
        assert_eq!(sig("Person.best_friend").1, "Option[Person]");
        // 値型の `char` は nullable の文脈に関わらず `str` のまま
        assert_eq!(sig("Probe.initial").1, "str");
        assert_eq!(sig("Probe.any_obj").1, "Any");
    }

    /// コレクション・デリゲート・タプル・型変数・他のアセンブリの型を使うメソッドだけに印が付く（2-1）。
    #[test]
    fn unsupported_methods_are_marked() {
        let st = probe_stubs();
        let why = |k: &str| st.get(k).unwrap_or_else(|| panic!("{k} missing")).2.clone();
        for k in ["Probe.words", "Probe.words_list", "Probe.counts", "Probe.ints"] {
            assert!(why(k).is_some_and(|w| w.starts_with("it returns `") && w.contains("a C# collection")), "{k}");
        }
        for k in ["Probe.sum", "Probe.join"] {
            assert!(why(k).is_some_and(|w| w.starts_with("parameter `xs` is `list[")), "{k}");
        }
        assert!(why("Probe.apply").is_some_and(|w| w.contains("delegate")));
        assert!(why("Probe.pair").is_some_and(|w| w.contains("tuple")));
        assert!(why("Probe.first").is_some_and(|w| w == "it returns `T`, a generic type parameter, which cannot cross the bridge"));
        assert!(why("Probe.builder").is_some_and(|w| w.contains("System.Text.StringBuilder")));
        for k in ["Probe.greet", "Probe.make_person", "Probe.find", "Probe.describe_opt", "Probe.any_obj",
                  "Probe.initial", "Probe.add_opt", "Person.older", "Person.best_friend"] {
            assert_eq!(why(k), None, "{k}");
        }
    }
}
