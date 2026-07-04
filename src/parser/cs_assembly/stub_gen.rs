// cs_assembly/stub_gen.rs — Arrow スタブ生成関数: .ars テキスト描画、スタブ構築、シグネチャ復号、演算子名/パラメータ名処理。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::Path,
    crate::ast::{Accessibility, Param, Stmt, TemplateParam},
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// .ars text renderer (with optional docstrings)
// ---------------------------------------------------------------------------

/// Generate `.ars` stub text from parsed assembly data, embedding XML doc comments
/// as triple-quoted docstrings when the `docs` map is non-empty.
///
/// The output format mirrors `stub_gen::generate_stub` but includes
/// `"""summary"""` on the first body line of each class/method that has a doc entry.
pub(crate) fn render_cs_ars_text(pa: &ParsedAssembly, docs: &HashMap<String, String>) -> String {
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

pub(crate) fn make_param(name: &str, type_ann: &str, mutable: bool) -> Param {
    Param {
        name: name.to_string(),
        mutable,
        type_ann: Some(type_ann.to_string()),
        default: None,
        variadic: false,
    }
}


pub(crate) fn make_fn_stub(
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


pub(crate) fn generate_stubs(
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
pub(crate) fn decode_method_sig(
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
pub(crate) fn operator_name(cs_name: &str) -> Option<&'static str> {
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


pub(crate) fn sanitize_param_name(name: &str) -> String {
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
