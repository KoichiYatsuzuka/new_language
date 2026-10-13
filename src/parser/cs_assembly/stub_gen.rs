// cs_assembly/stub_gen.rs — Arrow スタブ生成関数: .ars テキスト描画、スタブ構築、シグネチャ復号、演算子名/パラメータ名処理。

use {
    std::collections::{HashMap, HashSet},
    crate::ast::{Accessibility, Expr, Param, Stmt, TemplateParam},
};
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
    let nullable   = &pa.nullable;

    let mut out   = String::new();
    let mut first = true;

    for (td_i, td) in typedefs.iter().enumerate() {
        let td_row = td_i as u32 + 1;
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
            // ⚠ `->{n}` は付けない。旧・正規表現版拡張向けの印で Arrow の構文では
            //   なく、生成した `.ars` が読み返せなくなる
            //   （`partial_compiler::stub_gen::class_stub` の注記と同じ理由）。
            out.push_str(&format!("class {n}{tparams}{bases_str}:\n"));
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
            if let Some(PropertyRole::EventAdder) = &m.property_role { continue; }

            let is_accessor = matches!(&m.property_role,
                Some(PropertyRole::Getter(_)) | Some(PropertyRole::Setter(_)));
            if m.flags & MD_SPECIAL_NAME != 0 && !is_accessor {
                if m.name == ".ctor" || m.name == ".cctor" { continue; }
                if let Some(op) = operator_name(&m.name) {
                    let (ret, sig_params) = decode_method_sig(
                        data, streams, layout, m, params, type_names, &td.generic_param_names,
                        nullable, md_1 as u32, td_row);
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
                data, streams, layout, m, params, type_names, &td.generic_param_names,
                nullable, md_1 as u32, td_row);

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

/// C# の `ref`/`out`（ELEMENT_TYPE_BYREF）パラメータは `mutable=true` で渡すこと。
/// Arrow の `mut` パラメータとして型検査され、`let` 変数を渡す誤りを静的に捕捉する
/// （`Param::bridge` 参照 — VS Code 拡張の cs_assembly.ts `isByRef → mut` と同じ規則）。
/// 制限: C# の `in`（読み取り専用参照）もシグネチャ上は BYREF のため `mut` 扱いになる
/// （過剰に厳しい側へ倒れる — `mut` 変数を渡せば通る）。
pub(crate) fn make_param(name: &str, type_ann: &str, mutable: bool) -> Param {
    Param::bridge(name, Some(type_ann.to_string()), mutable)
}


pub(crate) fn make_fn_stub(
    name: &str,
    params: Vec<Param>,
    ret: &str,
    is_static: bool,
    is_abstract: bool,
    template_params: Vec<TemplateParam>,
) -> Stmt {
    Stmt::FnDef { src: None,
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
    nullable: &NullableInfo,
) -> Result<Vec<Stmt>, String> {
    let mut stmts: Vec<Stmt> = Vec::new();
    let classes = stub_class_names(typedefs);

    for (td_i, td) in typedefs.iter().enumerate() {
        let td_row = td_i as u32 + 1;
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
            if let Some(PropertyRole::EventAdder) = &m.property_role {
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
                        nullable, md_1 as u32, td_row,
                    );
                    let mut arrow_params = vec![make_param("self", "Self", false)];
                    for (i, (ty, is_byref)) in sig_params.iter().enumerate() {
                        let pname = format!("p{i}");
                        arrow_params.push(make_param(&pname, ty, *is_byref));
                    }
                    let unsupported = cs_unsupported_reason(
                        &ret, &arrow_params, &classes, &td.generic_param_names, &m.generic_param_names);
                    body_stmts.push(mark_unsupported(
                        make_fn_stub(op, arrow_params, &ret, false, is_interface, vec![]),
                        unsupported,
                    ));
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
                nullable, md_1 as u32, td_row,
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
                if let Some((_, (ty, is_byref))) = param_iter.next() {
                    arrow_params.push(make_param("value", ty, *is_byref));
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

                for (i, (ty, is_byref)) in sig_params.iter().enumerate() {
                    let pname = method_params.get(i)
                        .map(|p| sanitize_param_name(&p.name))
                        .unwrap_or_else(|| format!("p{i}"));
                    arrow_params.push(make_param(&pname, ty, *is_byref));
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

            let unsupported = cs_unsupported_reason(
                &eff_ret, &arrow_params, &classes, &td.generic_param_names, &m.generic_param_names);
            body_stmts.push(mark_unsupported(
                make_fn_stub(&arrow_name, arrow_params, &eff_ret, m.is_static, is_interface, tmpl),
                unsupported,
            ));
        }

        if is_interface {
            stmts.push(Stmt::TraitDef { src: None,
                name: td.name.clone(),
                template_params,
                body: body_stmts,
            });
        } else {
            stmts.push(Stmt::ClassDef { src: None,
                name: td.name.clone(),
                template_params,
                // ⚠ C# スタブの基底は型引数を持たせていない（タスク 9.9）。
                //   持たせるなら `bases` と同じ並びで詰めること。
                base_args: vec![Vec::new(); bases.len()],
                bases,
                decorators: vec![],
                body: body_stmts,
            });
        }
    }

    Ok(stmts)
}


// Decode a method's signature blob → (return_type_arrow, [(param_type, is_byref)])
/// null になりうる参照型（`string?` / `Person?`）は `Option[T]` にする（nullable 注釈・2-1）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_method_sig(
    data: &[u8],
    streams: &Streams,
    _layout: &TildeLayout,
    m: &CsMethod,
    params: &[CsParam],
    type_names: &HashMap<u32, String>,
    type_params: &[String],
    nullable: &NullableInfo,
    method_row: u32,
    td_row: u32,
) -> (String, Vec<(String, bool)>) {
    let blob = read_blob(data, streams.blob_off, m.sig_blob_idx);
    let mut reader = SigReader {
        data: blob,
        pos: 0,
        type_names,
        type_params,
        method_params: &m.generic_param_names,
    };
    let ((ret, ret_is_ref), sig_params) = reader.parse_method_sig_refs(&m.generic_param_names);
    let flag = |seq: u16| nullable.flag(m, method_row, td_row, params, seq);
    let ret = if ret_is_ref { apply_nullable(ret, flag(0)) } else { ret };
    let sig_params = sig_params
        .into_iter()
        .enumerate()
        .map(|(i, (ty, byref, is_ref))| {
            let ty = if is_ref { apply_nullable(ty, flag(i as u16 + 1)) } else { ty };
            (ty, byref)
        })
        .collect();
    (ret, sig_params)
}


/// nullable の値が 2（null になりうる）なら `Option[T]` にする（2-1）。`object`（`Any`）は `None` も含むのでそのまま。
/// ⚠ 0（注釈なし）は `T` のまま。実行時に null が来たら例外にする（`cs_values`）。Kotlin がプラットフォーム型の値を
///   null にならない型へ入れるときに検査するのと同じ扱い。
fn apply_nullable(ty: String, flag: u8) -> String {
    if flag == 2 && ty != "Any" && !ty.starts_with("Option[") {
        format!("Option[{ty}]")
    } else {
        ty
    }
}


/// 型スタブにクラスとして出す型の名前（公開・インターフェースでない・コンパイラが作ったものでない）。
/// メソッドが受け渡す型がこの中にあれば、実行時に `CsObject` として扱える（2-1）。
fn stub_class_names(typedefs: &[CsTypeDef]) -> HashSet<String> {
    typedefs
        .iter()
        .filter(|td| {
            let vis = td.flags & 0x07;
            (vis == TD_PUBLIC || vis == TD_NESTED_PUBLIC)
                && td.flags & TD_INTERFACE == 0
                && !td.name.is_empty()
                && td.name != "<Module>"
                && !td.name.starts_with('<')
                && !td.name.starts_with('_')
        })
        .map(|td| td.name.clone())
        .collect()
}


/// C# のメソッドが、今の Arrow と C# の間で受け渡せない型を使っていれば理由を返す（2-1）。
///
/// 受け渡せるのは `int` / `float` / `bool` / `str` / `None` / `Any`（`object`）と、このアセンブリの公開クラス
/// （`CsObject`）、およびそれらの `Option[T]`。コレクション・デリゲート・タプル・型変数・他のアセンブリの型・
/// インターフェースは、使った箇所を静的エラーにする（`TypeErrorKind::FfiUnsupported`）。
/// ⚠ 以前はコレクションやオブジェクトの戻り値が整数（ホスト側のハンドル番号）になり、リストの引数は C# に
///   `null` が届いていた（型スタブは `list[str]` / `Person` と宣言していた）。
pub(crate) fn cs_unsupported_reason(
    ret: &str,
    params: &[Param],
    classes: &HashSet<String>,
    class_type_params: &[String],
    method_type_params: &[String],
) -> Option<String> {
    // 型が受け渡せなければ、その型を説明する句（`it returns `T`, …` / `parameter `p` is `T`, …` に続ける）
    let check = |ty: &str| -> Option<&'static str> {
        let inner = ty.strip_prefix("Option[").and_then(|s| s.strip_suffix(']')).unwrap_or(ty);
        let head = inner.split('[').next().unwrap_or(inner);
        match head {
            "None" | "int" | "float" | "bool" | "str" | "Any" | "Self" => None,
            "list" | "dict" | "set" => Some("a C# collection, which cannot cross the bridge yet"),
            "tuple" => Some("a C# tuple, which cannot cross the bridge yet"),
            "function" => Some("a C# delegate, which cannot cross the bridge yet"),
            _ if class_type_params.iter().chain(method_type_params).any(|t| t == head) => {
                Some("a generic type parameter, which cannot cross the bridge")
            }
            _ if inner.contains('[') => Some("a generic C# type, which cannot cross the bridge yet"),
            _ if classes.contains(inner) => None,
            _ => Some("which is not a public class of this assembly"),
        }
    };
    if let Some(why) = check(ret) {
        return Some(format!("it returns `{ret}`, {why}"));
    }
    for p in params {
        if let Some(ty) = p.type_ann.as_deref() {
            if let Some(why) = check(ty) {
                return Some(format!("parameter `{}` is `{ty}`, {why}", p.name));
            }
        }
    }
    None
}


/// 未対応のメソッドの本体に、理由を書いた文字列（`FFI_UNSUPPORTED_MARK` で始まる docstring）を置く（2-1）。
/// 型検査がそれを読み、使った箇所を静的エラーにする。
/// ⚠ C/C++ の型スタブ（`parser/imports/cpp.rs`）はデコレータで印を付けるが、C# のスタブはクラスの本体として
///   実行される（`exec_class_def` がメソッドのデコレータを適用する）ので、文字列のデコレータは置けない。
///   本体はブリッジへ呼び出しが行くので実行されない。
fn mark_unsupported(stub: Stmt, reason: Option<String>) -> Stmt {
    let Some(reason) = reason else { return stub };
    match stub {
        Stmt::FnDef { src, name, template_params, params, return_type, mut body, is_abstract, is_static,
                      is_class_method, decorators, access } => {
            body.insert(0, Stmt::Expr(Expr::Str(
                format!("{}{reason}", crate::cpp_header::FFI_UNSUPPORTED_MARK).into(),
            )));
            Stmt::FnDef { src, name, template_params, params, return_type, body, is_abstract, is_static,
                          is_class_method, decorators, access }
        }
        other => other,
    }
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
