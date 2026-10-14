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
/// ⚠ `.ars` はどのブリッジで読むか分からないので cs-dll の判定で書く（印は出さない）。型は両方で同じ。
pub(crate) fn render_cs_ars_text(pa: &ParsedAssembly, docs: &HashMap<String, String>) -> String {
    let mut out   = String::new();
    let mut first = true;

    for c in assembly_model(pa, CsBridge::Dll) {
        let tparams = if c.generic_param_names.is_empty() {
            String::new()
        } else {
            format!("[{}]", c.generic_param_names.join(", "))
        };
        let bases_str = if c.interface_names.is_empty() {
            String::new()
        } else {
            format!("({})", c.interface_names.join(", "))
        };

        if !first { out.push('\n'); }
        first = false;

        if c.is_interface {
            out.push_str(&format!("trait {}{tparams}:\n", c.name));
        } else {
            let n = &c.name;
            // ⚠ `->{n}` は付けない。旧・正規表現版拡張向けの印で Arrow の構文では
            //   なく、生成した `.ars` が読み返せなくなる
            //   （`partial_compiler::stub_gen::class_stub` の注記と同じ理由）。
            out.push_str(&format!("class {n}{tparams}{bases_str}:\n"));
        }

        // Class-level docstring
        if let Some(doc) = docs.get(&format!("T:{}", c.name)) {
            out.push_str(&format!("    \"\"\"{doc}\"\"\"\n"));
        }

        // __init__: one per public constructor (5-1), or a bare one when there is none
        if !c.is_interface {
            let ctors: Vec<&CsMemberModel> = c.members.iter().filter(|m| m.role == MemberRole::Ctor).collect();
            if ctors.is_empty() {
                out.push_str("    fn __init__(self: Self) -> None:\n        ...\n");
            }
            for m in ctors {
                let mut p_parts = vec!["self: Self".to_string()];
                p_parts.extend(m.params.iter().map(|p| format!("{}: {}", p.name, p.ann)));
                out.push_str(&format!("    fn __init__({}) -> None:\n        ...\n", p_parts.join(", ")));
            }
        }

        for m in &c.members {
            let p_self = match m.role {
                MemberRole::Ctor => continue,
                MemberRole::Operator(_) => true,
                _ => !m.is_static,
            };
            let mut p_parts: Vec<String> = if p_self { vec!["self: Self".to_string()] } else { vec![] };
            p_parts.extend(m.params.iter().map(|p| format!("{}: {}", p.name, p.ann)));
            let tmpl = if m.generic_param_names.is_empty() {
                String::new()
            } else {
                format!("[{}]", m.generic_param_names.join(", "))
            };
            out.push_str(&format!("    fn {}{tmpl}({}) -> {}:\n", m.arrow_name, p_parts.join(", "), m.ret_ann));
            if matches!(m.role, MemberRole::Operator(_)) {
                out.push_str("        ...\n");
                continue;
            }

            // Method docstring — look up by original C# name, fall back to property key
            let mkey = format!("M:{}.{}", c.name, m.cs_name);
            let pkey = match &m.role {
                MemberRole::Getter(prop) | MemberRole::Setter(prop) => Some(format!("P:{}.{}", c.name, prop)),
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


pub(crate) fn generate_stubs(pa: &ParsedAssembly, bridge: CsBridge) -> Result<Vec<Stmt>, String> {
    let mut stmts: Vec<Stmt> = Vec::new();

    for c in assembly_model(pa, bridge) {
        let template_params: Vec<TemplateParam> = c
            .generic_param_names
            .iter()
            .map(|n| TemplateParam { name: n.clone(), constraints: vec![] })
            .collect();
        let mut body_stmts: Vec<Stmt> = Vec::new();

        // `__init__`: 公開のコンストラクタごとに 1 つ（5-1・実行時は引数の個数で選んで、この型で並べる）。
        // コンストラクタが無ければ（`static class`）引数の無いもの。
        // ⚠ 型検査は外部言語のクラスのコンストラクタの引数を見ない（`call_check.rs` の `arrow_class_names`）。
        // ⚠ 未対応の印はメソッドの名前ごとに集める（`registry/builder.rs`）ので、全部のコンストラクタが未対応のときだけ
        //   付ける。1 つでも付けると、そのクラスは構築できなくなる（使えるコンストラクタがあっても）。未対応の個数で
        //   呼べば、実行時に「その個数の export が無い」か `decl_of` のエラーになる。
        if !c.is_interface {
            let ctors: Vec<&CsMemberModel> = c.members.iter().filter(|m| m.role == MemberRole::Ctor).collect();
            if ctors.is_empty() {
                body_stmts.push(make_fn_stub("__init__", vec![make_param("self", "Self", false)], "None", false, false, vec![]));
            }
            let all_unsupported = ctors.iter().all(|m| m.unsupported.is_some());
            for m in ctors {
                let mut ps = vec![make_param("self", "Self", false)];
                ps.extend(m.params.iter().map(|p| make_param(&p.name, &p.ann, p.byref)));
                let mark = if all_unsupported { m.unsupported.clone() } else { None };
                body_stmts.push(mark_unsupported(make_fn_stub("__init__", ps, "None", false, false, vec![]), mark));
            }
        }

        for m in &c.members {
            // 演算子は `self` を取る形で出す（C# では static）
            let (p_self, is_static) = match m.role {
                MemberRole::Ctor => continue,
                MemberRole::Operator(_) => (true, false),
                _ => (!m.is_static, m.is_static),
            };
            let mut ps: Vec<Param> = if p_self { vec![make_param("self", "Self", false)] } else { vec![] };
            ps.extend(m.params.iter().map(|p| make_param(&p.name, &p.ann, p.byref)));
            let tmpl: Vec<TemplateParam> = m
                .generic_param_names
                .iter()
                .map(|n| TemplateParam { name: n.clone(), constraints: vec![] })
                .collect();
            body_stmts.push(mark_unsupported(
                make_fn_stub(&m.arrow_name, ps, &m.ret_ann, is_static, c.is_interface, tmpl),
                m.unsupported.clone(),
            ));
        }

        if c.is_interface {
            stmts.push(Stmt::TraitDef { src: None,
                name: c.name,
                template_params,
                body: body_stmts,
            });
        } else {
            stmts.push(Stmt::ClassDef { src: None,
                name: c.name,
                template_params,
                // ⚠ C# スタブの基底は型引数を持たせていない（タスク 9.9）。
                //   持たせるなら `bases` と同じ並びで詰めること。
                base_args: vec![Vec::new(); c.interface_names.len()],
                bases: c.interface_names,
                decorators: vec![],
                body: body_stmts,
            });
        }
    }

    Ok(stmts)
}


/// nullable の値が 2（null になりうる）なら `Option[T]` にする（2-1）。`object`（`Any`）は `None` も含むのでそのまま。
/// ⚠ 0（注釈なし）は `T` のまま。実行時に null が来たら例外にする（`cs_values`）。Kotlin がプラットフォーム型の値を
///   null にならない型へ入れるときに検査するのと同じ扱い。
pub(crate) fn apply_nullable(ty: String, flag: u8) -> String {
    if flag == 2 && ty != "Any" && !ty.starts_with("Option[") {
        format!("Option[{ty}]")
    } else {
        ty
    }
}


/// 型スタブにクラスとして出す型の名前（公開・インターフェースでない・コンパイラが作ったものでない）。
/// メソッドが受け渡す型がこの中にあれば、実行時に `CsObject` として扱える（2-1）。
pub(crate) fn stub_class_names(typedefs: &[CsTypeDef]) -> HashSet<String> {
    typedefs
        .iter()
        .filter(|td| is_stub_type(td) && td.flags & TD_INTERFACE == 0)
        .map(|td| td.name.clone())
        .collect()
}


/// C# のメソッドが、今の Arrow と C# の間で受け渡せない型を使っていれば理由を返す（2-1・5-2）。
///
/// 受け渡せるのは `int` / `float` / `bool` / `str` / `None` / `Any`（`object`）と、このアセンブリの公開クラス
/// （`CsObject`）、およびそれらの `Option[T]`。cs-proc はさらにコレクション（`list` / `tuple` / `dict` / `set`・中身も
/// 受け渡せる型のもの）と `bytes` をコピーで運ぶ（5-2）。デリゲート・型変数・他のアセンブリの型・インターフェース
/// （と cs-dll のコレクション）は、使った箇所を静的エラーにする（`TypeErrorKind::FfiUnsupported`）。
/// ⚠ 以前はコレクションやオブジェクトの戻り値が整数（ホスト側のハンドル番号）になり、リストの引数は C# に
///   `null` が届いていた（型スタブは `list[str]` / `Person` と宣言していた）。
pub(crate) fn cs_unsupported_reason(
    ret: &str,
    params: &[(&str, &str)],
    classes: &HashSet<String>,
    class_type_params: &[String],
    method_type_params: &[String],
    bridge: CsBridge,
) -> Option<String> {
    let ctx = TypeCtx { classes, type_params: class_type_params.iter().chain(method_type_params).collect(), bridge };
    if let Some(why) = type_reason(ret, &ctx) {
        return Some(format!("it returns `{ret}`, {why}"));
    }
    for (name, ty) in params {
        if let Some(why) = type_reason(ty, &ctx) {
            return Some(format!("parameter `{name}` is `{ty}`, {why}"));
        }
    }
    None
}

struct TypeCtx<'a> {
    classes: &'a HashSet<String>,
    type_params: Vec<&'a String>,
    bridge: CsBridge,
}

/// 型が受け渡せなければ、その型を説明する句（`it returns `T`, …` / `parameter `p` is `T`, …` に続ける）。
fn type_reason(ty: &str, ctx: &TypeCtx) -> Option<String> {
    let inner = ty.strip_prefix("Option[").and_then(|s| s.strip_suffix(']')).unwrap_or(ty);
    let (head, args) = match inner.split_once('[') {
        Some((h, rest)) => (h, rest.strip_suffix(']')),
        None => (inner, None),
    };
    let dll = ctx.bridge == CsBridge::Dll;
    match head {
        "None" | "int" | "float" | "bool" | "str" | "Any" | "Self" => None,
        "bytes" if dll => Some("a byte array, which cannot cross the cs-dll bridge yet (use cs-proc)".to_string()),
        "bytes" => None,
        "tuple" if dll => Some("a C# tuple, which cannot cross the cs-dll bridge yet (use cs-proc)".to_string()),
        "list" | "dict" | "set" if dll => {
            Some("a C# collection, which cannot cross the cs-dll bridge yet (use cs-proc)".to_string())
        }
        // cs-proc: 中身も受け渡せるなら運ぶ（5-2）
        "list" | "dict" | "set" | "tuple" => match args {
            Some(a) => split_type_args(a)
                .into_iter()
                .find_map(|t| type_reason(t, ctx).map(|why| format!("a C# collection of `{t}`, {why}"))),
            None => Some("a C# collection without element types".to_string()),
        },
        "function" => Some("a C# delegate, which cannot cross the bridge yet".to_string()),
        _ if ctx.type_params.iter().any(|t| t.as_str() == head) => {
            Some("a generic type parameter, which cannot cross the bridge".to_string())
        }
        _ if args.is_some() => Some("a generic C# type, which cannot cross the bridge yet".to_string()),
        _ if ctx.classes.contains(inner) => None,
        _ => Some("which is not a public class of this assembly".to_string()),
    }
}

/// `int,dict[str,int]` を最上位のカンマで分ける。
fn split_type_args(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
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
