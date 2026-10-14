// cs_assembly/model.rs — アセンブリの公開メンバーの一覧: assembly_model（ffi_value_exchange_plan.md 5-1）。
//
// 型スタブ（`generate_stubs`）・`.ars` の文字列（`render_cs_ars_text`）・export の生成器（`export_gen`）が、
// 同じ一覧を読む。
// ⚠ 以前はスタブと `.ars` がそれぞれアセンブリを歩いていた（同じ判定の写し）。生成器が 3 本目を持つと、export の
//   名前や型がスタブとずれる（Arrow が呼ぶ名前の export が無い・違う型で受け取る）ので、ここに 1 本にした。

use std::collections::HashSet;

use super::*;

/// 型スタブに出す型（クラス・インターフェース）。
pub(crate) struct CsClassModel {
    /// スタブの名前（TypeDef の名前）
    pub(crate) name: String,
    /// C# のソースで書く名前（名前空間・外側の型つき。`ArrowBridge.Calculator` / `Outer.Inner`）
    #[cfg_attr(feature = "editor", allow(dead_code))] // export の生成器だけが読む（エディタには無い）
    pub(crate) cs_name: String,
    pub(crate) is_interface: bool,
    /// `abstract`（`static class` を含む）。コンストラクタは呼べない
    #[cfg_attr(feature = "editor", allow(dead_code))]
    pub(crate) is_abstract: bool,
    pub(crate) generic_param_names: Vec<String>,
    pub(crate) interface_names: Vec<String>,
    /// メタデータの順。コンストラクタ（`MemberRole::Ctor`）も含む
    pub(crate) members: Vec<CsMemberModel>,
}

/// メンバーの種類。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MemberRole {
    Method,
    /// プロパティの getter（中身はプロパティ名）
    Getter(String),
    /// プロパティの setter（中身はプロパティ名）
    Setter(String),
    /// 演算子（中身は Arrow の名前 `__add__` …）
    Operator(&'static str),
    /// インスタンスのコンストラクタ（`.ctor`）
    Ctor,
}

/// 公開メンバー 1 つ。
pub(crate) struct CsMemberModel {
    pub(crate) role: MemberRole,
    /// メタデータの名前（`Add` / `get_Pi` / `op_Addition` / `.ctor`）
    pub(crate) cs_name: String,
    /// スタブの名前（`Add` / `getPi` / `__add__` / `__init__`）。export の名前もこれから作る
    pub(crate) arrow_name: String,
    pub(crate) is_static: bool,
    pub(crate) generic_param_names: Vec<String>,
    #[cfg_attr(feature = "editor", allow(dead_code))]
    pub(crate) ret: CsTy,
    /// 戻り値の Arrow の型（nullable 注釈を当てた後。setter とコンストラクタは `None`）
    pub(crate) ret_ann: String,
    pub(crate) params: Vec<CsParamModel>,
    /// シグネチャの引数の数（setter は `params` に先頭しか出さないので、添字つきの setter の見分けに使う）
    #[cfg_attr(feature = "editor", allow(dead_code))]
    pub(crate) sig_len: usize,
    /// 今の Arrow と C# の間で受け渡せない型を使っていれば、その理由（[`cs_unsupported_reason`]）
    pub(crate) unsupported: Option<String>,
}

/// メンバーの引数 1 つ。
pub(crate) struct CsParamModel {
    /// スタブの引数名（演算子は `p0` …・setter は `value`）
    pub(crate) name: String,
    #[cfg_attr(feature = "editor", allow(dead_code))]
    pub(crate) ty: CsTy,
    /// Arrow の型（nullable 注釈を当てた後）
    pub(crate) ann: String,
    /// `ref` / `out`（ELEMENT_TYPE_BYREF）
    pub(crate) byref: bool,
}

const TD_ABSTRACT: u32 = 0x80;

/// 型スタブにクラスとして出す型か（公開・`<Module>` でない・コンパイラが作ったものでない）。インターフェースを含む。
pub(crate) fn is_stub_type(td: &CsTypeDef) -> bool {
    let vis = td.flags & 0x07;
    (vis == TD_PUBLIC || vis == TD_NESTED_PUBLIC)
        && !td.name.is_empty()
        && td.name != "<Module>"
        && !td.name.starts_with('<')
        && !td.name.starts_with('_')
}

/// アセンブリの公開型と公開メンバーの一覧。
pub(crate) fn assembly_model(pa: &ParsedAssembly) -> Vec<CsClassModel> {
    let classes = stub_class_names(&pa.typedefs);
    let mut out = Vec::new();
    for (td_i, td) in pa.typedefs.iter().enumerate() {
        let td_row = td_i as u32 + 1;
        if !is_stub_type(td) {
            continue;
        }
        let mut members = Vec::new();
        for md_1 in td.method_list_start as usize..td.method_list_end as usize {
            let Some(m) = md_1.checked_sub(1).and_then(|i| pa.all_methods.get(i)) else { continue };
            // 公開でないもの・`[UnmanagedCallersOnly]` の手書きの export（C ABI の入口で、マネージドのコードからも
            // Arrow のブリッジ経由でも呼べない。⚠ 以前は型スタブにメソッドとして出ていた）は出さない
            if !m.is_public || pa.nullable.unmanaged_entries.contains(&(md_1 as u32)) {
                continue;
            }
            let role = match &m.property_role {
                // イベントの add / remove は出さない
                Some(PropertyRole::EventAdder) => continue,
                Some(PropertyRole::Getter(prop)) => MemberRole::Getter(prop.clone()),
                Some(PropertyRole::Setter(prop)) => MemberRole::Setter(prop.clone()),
                None if m.flags & MD_SPECIAL_NAME != 0 => {
                    if m.name == ".ctor" && !m.is_static {
                        MemberRole::Ctor
                    } else if let Some(op) = operator_name(&m.name) {
                        MemberRole::Operator(op)
                    } else {
                        continue; // `.cctor` などほかの特別な名前
                    }
                }
                None => MemberRole::Method,
            };
            out_member(pa, td, td_row, md_1 as u32, m, role, &classes, &mut members);
        }
        out.push(CsClassModel {
            name: td.name.clone(),
            cs_name: cs_source_name(pa, td_row),
            is_interface: td.flags & TD_INTERFACE != 0,
            is_abstract: td.flags & TD_ABSTRACT != 0,
            generic_param_names: td.generic_param_names.clone(),
            interface_names: td.interface_names.clone(),
            members,
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn out_member(
    pa: &ParsedAssembly,
    td: &CsTypeDef,
    td_row: u32,
    md_row: u32,
    m: &CsMethod,
    role: MemberRole,
    classes: &HashSet<String>,
    members: &mut Vec<CsMemberModel>,
) {
    let ((ret, ret_ann), sig) = decode_member_sig(pa, m, md_row, td_row, &td.generic_param_names);
    let sig_len = sig.len();
    let arrow_name = match &role {
        MemberRole::Getter(prop) => format!("get{prop}"),
        MemberRole::Setter(prop) => format!("set{prop}"),
        MemberRole::Operator(op) => op.to_string(),
        MemberRole::Ctor => "__init__".to_string(),
        MemberRole::Method => m.name.clone(),
    };
    // 引数名は Param の表から（無ければ `p{i}`）。演算子は `p{i}`、setter は先頭の 1 つだけを `value` として出す
    // （⚠ 添字つきの setter `set_Item(index, value)` でも先頭を出している・以前のスタブと同じ）。
    let named: Vec<&CsParam> = pa
        .all_params
        .iter()
        .skip((m.param_list_start as usize).saturating_sub(1))
        .take(((m.param_list_end - m.param_list_start) as usize).min(pa.all_params.len()))
        .filter(|p| p.sequence > 0)
        .collect();
    let params: Vec<CsParamModel> = match &role {
        MemberRole::Setter(_) => sig
            .into_iter()
            .take(1)
            .map(|(ty, ann, byref)| CsParamModel { name: "value".to_string(), ty, ann, byref })
            .collect(),
        MemberRole::Operator(_) => sig
            .into_iter()
            .enumerate()
            .map(|(i, (ty, ann, byref))| CsParamModel { name: format!("p{i}"), ty, ann, byref })
            .collect(),
        _ => sig
            .into_iter()
            .enumerate()
            .map(|(i, (ty, ann, byref))| CsParamModel {
                name: named.get(i).map(|p| sanitize_param_name(&p.name)).unwrap_or_else(|| format!("p{i}")),
                ty,
                ann,
                byref,
            })
            .collect(),
    };
    let ret_ann = match &role {
        MemberRole::Setter(_) | MemberRole::Ctor => "None".to_string(),
        _ => ret_ann,
    };
    let anns: Vec<(&str, &str)> = params.iter().map(|p| (p.name.as_str(), p.ann.as_str())).collect();
    let unsupported =
        cs_unsupported_reason(&ret_ann, &anns, classes, &td.generic_param_names, &m.generic_param_names);
    members.push(CsMemberModel {
        role,
        cs_name: m.name.clone(),
        arrow_name,
        is_static: m.is_static,
        generic_param_names: m.generic_param_names.clone(),
        ret,
        ret_ann,
        params,
        sig_len,
        unsupported,
    });
}

/// メソッドのシグネチャを、C# の型と Arrow の型の組で読む（`string?` / `Person?` は `Option[T]`・2-1）。
/// 戻り値は `((戻り値の型, その Arrow の型), [(引数の型, その Arrow の型, BYREF か)])`。
fn decode_member_sig(
    pa: &ParsedAssembly,
    m: &CsMethod,
    method_row: u32,
    td_row: u32,
    type_params: &[String],
) -> ((CsTy, String), Vec<(CsTy, String, bool)>) {
    let blob = read_blob(&pa.data, pa.streams.blob_off, m.sig_blob_idx);
    let mut reader = SigReader {
        data: blob,
        pos: 0,
        type_names: &pa.type_names,
        type_params,
        method_params: &m.generic_param_names,
    };
    let ((ret, ret_is_ref), sig) = reader.parse_method_sig_tys();
    let flag = |seq: u16| pa.nullable.flag(m, method_row, td_row, &pa.all_params, seq);
    let ret_ann = if ret_is_ref { apply_nullable(ret.arrow(), flag(0)) } else { ret.arrow() };
    let sig = sig
        .into_iter()
        .enumerate()
        .map(|(i, (ty, byref, is_ref))| {
            let ann = if is_ref { apply_nullable(ty.arrow(), flag(i as u16 + 1)) } else { ty.arrow() };
            (ty, ann, byref)
        })
        .collect();
    ((ret, ret_ann), sig)
}

/// 型の C# のソースでの名前（名前空間・外側の型つき）。
fn cs_source_name(pa: &ParsedAssembly, td_row: u32) -> String {
    let Some(td) = (td_row as usize).checked_sub(1).and_then(|i| pa.typedefs.get(i)) else { return String::new() };
    match pa.nullable.enclosing.get(&td_row) {
        Some(outer) if *outer != td_row => format!("{}.{}", cs_source_name(pa, *outer), td.name),
        _ if td.namespace.is_empty() => td.name.clone(),
        _ => format!("{}.{}", td.namespace, td.name),
    }
}
