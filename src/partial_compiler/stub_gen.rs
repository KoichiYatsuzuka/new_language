/// `.ars` stub file generator.
///
/// Walks the top-level AST and emits type-only declarations with `...` bodies.
/// The output is valid `.ar` syntax and is used by the type checker and VS Code
/// extension to inspect a compiled module without its implementation.
use crate::ast::{Accessibility, FieldKind, Param, Stmt, TemplateParam};

/// **VS Code 拡張へ渡すスタブ**（`--emit-stubs`）。[`generate_stub`] の宣言に加えて、
/// モジュールの**グローバル変数を型と属性つきで**書く（`enum_member_type_plan.md` 6-3）。
///
/// ```text
/// const LIMIT: int = Undefined
/// let TAGS: list[str] = Undefined
/// mut total: int = Undefined
/// ```
///
/// ⚠⚠ 拡張は import 先を読まない（スタブだけを読む）。以前はスタブが変数を落としていたので、
///   拡張では import したモジュールのグローバル変数の型も属性（`const` / `let` / `mut`）も分からず、
///   `g.LIMIT = 5` も `let s: bool = g.LABEL` も何も言わなかった。
/// ⚠ 値は運ばない（`Undefined`）。型は**必ず注釈で書く**: 型検査は注釈つきの宣言を注釈の型で
///   束縛するので、拡張でも CLI と同じ型になる（スタブの本体の誤りは拡張が捨てる・`annotate_module_body`）。
///   注釈が無いと `Undefined` の型になってしまうので、**型の分からない変数は書かない**（誤った型を
///   付けるより、分からないままの方がよい）。
/// ⚠ 型は、宣言の注釈があればそれ、無ければ CLI の型検査が推論した型（`globals`・
///   `TypeChecker::module_globals`）。推論した型は注釈として読み戻して同じ型になるものだけ使い、
///   このモジュールの名前（`module.`）は外す（スタブの中ではモジュール自身の名前で書く）。
/// ⚠ 部分コンパイル（`.arc` の `.ars`）と js-proc のスタブは [`generate_stub`] のまま（変数を書かない）。
pub fn generate_editor_stub(
    stmts: &[Stmt],
    module: &str,
    globals: Option<&std::collections::HashMap<String, (crate::type_check::InferredType, crate::type_check::VarAttr)>>,
) -> String {
    use crate::type_check::{InferredType, VarAttr};
    let var_decl = |name: &str, ann: Option<&str>, kw_default: VarAttr| -> Option<String> {
        let (inferred, attr) = match globals.and_then(|g| g.get(name)) {
            Some((ty, attr)) => (Some(ty), *attr),
            None => (None, kw_default),
        };
        let ty_text = match ann {
            Some(a) => a.to_string(),
            None => {
                let ty = inferred?;
                if matches!(
                    ty,
                    InferredType::Unresolved | InferredType::Any | InferredType::Undefined | InferredType::Never
                ) {
                    return None;
                }
                let shown = ty.to_string();
                // 注釈として読み戻して同じ型になるものだけ（関数型などの表示は注釈の綴りと限らない）。
                if InferredType::from_ann(&shown).as_ref() != Some(ty) {
                    return None;
                }
                shown.replace(&format!("{module}."), "")
            }
        };
        let kw = match attr {
            VarAttr::Const => "const",
            VarAttr::Let | VarAttr::Imported { .. } => "let",
            VarAttr::Mut => "mut",
        };
        Some(format!("{kw} {name}: {ty_text} = Undefined\n"))
    };
    let mut out = String::new();
    let mut first = true;
    for stmt in stmts {
        let text = match stmt {
            Stmt::Const(name, ann, ..) => var_decl(name, ann.as_deref(), VarAttr::Const),
            Stmt::Let(name, ann, ..) => var_decl(name, ann.as_deref(), VarAttr::Let),
            Stmt::Mut(name, ann, ..) => var_decl(name, ann.as_deref(), VarAttr::Mut),
            Stmt::Static(name, ..) => var_decl(name, None, VarAttr::Mut),
            Stmt::LetTuple { targets, .. } => {
                let lines: String = targets
                    .iter()
                    .filter_map(|t| match t {
                        crate::ast::TupleTarget::Let(n) => var_decl(n, None, VarAttr::Let),
                        crate::ast::TupleTarget::Mut(n) => var_decl(n, None, VarAttr::Mut),
                        _ => None,
                    })
                    .collect();
                (!lines.is_empty()).then_some(lines)
            }
            other => top_level_stub(other),
        };
        if let Some(s) = text {
            if !first {
                out.push('\n');
            }
            out.push_str(&s);
            first = false;
        }
    }
    out
}

/// Generate a `.ars` stub string from a parsed program's top-level statements.
pub fn generate_stub(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    let mut first = true;
    for stmt in stmts {
        if let Some(s) = top_level_stub(stmt) {
            if !first {
                out.push('\n');
            }
            out.push_str(&s);
            first = false;
        }
    }
    out
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// 指定インデントレベルに対応するスペース文字列を返す（4スペース単位）。
fn ind(level: usize) -> String {
    "    ".repeat(level)
}

/// テンプレートパラメータリストを `[T, U: Constraint]` 形式の文字列に変換する。空の場合は空文字列。
fn template_params_str(params: &[TemplateParam]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|p| {
            if p.constraints.is_empty() {
                p.name.clone()
            } else {
                format!("{}: {}", p.name, p.constraints.join(" and "))
            }
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// パラメータリストをカンマ区切りの文字列に変換する。型アノテーションとデフォルト値 (`= ...`) を含む。
fn params_str(params: &[Param]) -> String {
    params
        .iter()
        .map(|p| {
            let mut s = if p.mutable {
                "mut ".to_string()
            } else {
                String::new()
            };
            s.push_str(&p.name);
            if let Some(t) = &p.type_ann {
                s.push_str(": ");
                s.push_str(t);
            }
            if p.default.is_some() {
                s.push_str(" = ...");
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ── top-level dispatch ────────────────────────────────────────────────────────

/// トップレベル文からスタブ文字列を生成する。対象外の文（変数宣言など）には `None` を返す。
fn top_level_stub(stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::FnDef {
            name,
            template_params,
            params,
            return_type,
            is_abstract,
            ..
        } => Some(fn_stub(
            name,
            template_params,
            params,
            return_type.as_deref(),
            0,
            *is_abstract,
            false, // トップレベル関数に static は無い
        )),

        Stmt::GenDef {
            name,
            template_params,
            params,
            yield_type,
            ..
        } => Some(gen_stub(
            name,
            template_params,
            params,
            yield_type.as_deref(),
            0,
        )),

        Stmt::ClassDef {
            name,
            template_params,
            bases,
            body,
            ..
        } => Some(class_stub(name, template_params, bases, body)),

        Stmt::TraitDef { src: _,
            name,
            template_params,
            body,
        } => Some(trait_stub(name, template_params, body)),

        Stmt::NewTypeDef { name, original } => Some(format!("new_type {name}: {original}\n")),

        Stmt::EnumDef { src: _, name, variants } => Some(enum_stub(name, variants)),

        _ => None,
    }
}

// ── function / generator stubs ───────────────────────────────────────────────

/// 関数定義のスタブ文字列（`fn name(...) -> T:\n    ...`）を生成する。
fn fn_stub(
    name: &str,
    template_params: &[TemplateParam],
    params: &[Param],
    return_type: Option<&str>,
    indent_level: usize,
    _is_abstract: bool,
    is_static: bool,
) -> String {
    let i = ind(indent_level);
    let tparams = template_params_str(template_params);
    let pstr = params_str(params);
    let ret = return_type.map(|r| format!(" -> {r}")).unwrap_or_default();
    let body_i = ind(indent_level + 1);
    let stat = if is_static { "static " } else { "" };
    format!("{i}{stat}fn {name}{tparams}({pstr}){ret}:\n{body_i}...\n")
}

/// ジェネレータ定義のスタブ文字列（`gen name(...) -> T:\n    ...`）を生成する。
fn gen_stub(
    name: &str,
    template_params: &[TemplateParam],
    params: &[Param],
    yield_type: Option<&str>,
    indent_level: usize,
) -> String {
    let i = ind(indent_level);
    let tparams = template_params_str(template_params);
    let pstr = params_str(params);
    let ret = yield_type.map(|t| format!(" -> {t}")).unwrap_or_default();
    let body_i = ind(indent_level + 1);
    format!("{i}gen {name}{tparams}({pstr}){ret}:\n{body_i}...\n")
}

// ── class stub ────────────────────────────────────────────────────────────────

/// クラス定義のスタブ文字列を生成する。`->Name` コンストラクタ戻り値アノテーション付き。
fn class_stub(
    name: &str,
    template_params: &[TemplateParam],
    bases: &[String],
    body: &[Stmt],
) -> String {
    let tparams = template_params_str(template_params);
    let bases_str = if bases.is_empty() {
        String::new()
    } else {
        format!("({})", bases.join(", "))
    };
    // ⚠⚠ かつてここは `class Name(...)->Name:` と書いていた。`->Name` は
    //    **旧・正規表現版の VS Code 拡張**向けにコンストラクタの戻り値型を
    //    伝える印で、Arrow の構文ではない。現行の拡張は `.ars` を**本物のパーサ**
    //    で読むので、この形だと "expected :, got ->" で構文エラーになり、
    //    **生成した `.ars` を読み返せない**（`import[js-proc]` の `.ars` 経路も同じ罠）。
    //    現行拡張にこの印を見る箇所は 1 つも無いので落とした。
    let mut out = format!("class {name}{tparams}{bases_str}:\n");

    let body_text = class_or_trait_body_stubs(body, 1);
    if body_text.is_empty() {
        out.push_str("    ...\n");
    } else {
        out.push_str(&body_text);
    }
    out.push('\n');
    out
}

// ── trait stub ────────────────────────────────────────────────────────────────

/// trait 定義のスタブ文字列を生成する。
fn trait_stub(name: &str, template_params: &[TemplateParam], body: &[Stmt]) -> String {
    let tparams = template_params_str(template_params);
    let mut out = format!("trait {name}{tparams}:\n");

    let body_text = class_or_trait_body_stubs(body, 1);
    if body_text.is_empty() {
        out.push_str("    ...\n");
    } else {
        out.push_str(&body_text);
    }
    out.push('\n');
    out
}

/// Renders class/trait body items grouped by accessibility section.
///
/// Emits `public:` / `private:` / `protected:` section headers only when the
/// access level changes and at least one item exists in that section.
fn class_or_trait_body_stubs(body: &[Stmt], indent_level: usize) -> String {
    let mut out = String::new();

    // Split items into (access, stub_text) pairs
    let items: Vec<(Accessibility, String)> = body
        .iter()
        .filter_map(|s| class_body_item_stub(s, indent_level))
        .collect();

    if items.is_empty() {
        return out;
    }

    // Check whether any non-public items exist; if all public, suppress headers
    let all_public = items.iter().all(|(acc, _)| *acc == Accessibility::Public);

    let mut current_access: Option<&Accessibility> = None;
    let sec_indent = ind(indent_level.saturating_sub(1));

    for (acc, text) in &items {
        if !all_public {
            let changed = current_access != Some(acc);
            if changed {
                let header = match acc {
                    Accessibility::Public => format!("{sec_indent}public:\n"),
                    Accessibility::Private => format!("{sec_indent}private:\n"),
                    Accessibility::Protected => format!("{sec_indent}protected:\n"),
                };
                out.push_str(&header);
                current_access = Some(acc);
            }
        }
        out.push_str(text);
    }

    out
}

/// Returns `(access, stub_text)` for a class body statement, or `None` to skip it.
fn class_body_item_stub(stmt: &Stmt, indent_level: usize) -> Option<(Accessibility, String)> {
    match stmt {
        Stmt::Field { src: _,
            name,
            kind,
            type_ann,
            default,
            access,
        } => {
            let i = ind(indent_level);
            let kw = match kind {
                FieldKind::Mut => "mut",
                FieldKind::Let => "let",
                FieldKind::Const => "const",
                FieldKind::StaticMut => "static mut",
            };
            let default_str = if default.is_some() { " = ..." } else { "" };
            Some((
                access.clone(),
                format!("{i}{kw} {name}: {type_ann}{default_str}\n"),
            ))
        }

        Stmt::FnDef {
            name,
            template_params,
            params,
            return_type,
            is_abstract,
            is_static,
            access,
            ..
        } => {
            // ⚠⚠ **`static` を落とさないこと。** 落とすと `self` を取らないメソッドが
            //    インスタンスメソッドとして読み直され、先頭の仮引数が `self` の席に
            //    吸収されて**引数の数が 1 つずれる**。C# の static メソッドで実測した症状は
            //    `'WpfApp.show' takes 0 argument(s) but 1 were given` — スタブを置いたせいで
            //    **エディタだけが偽のエラーを出す**形になる。
            let text = fn_stub(
                name,
                template_params,
                params,
                return_type.as_deref(),
                indent_level,
                *is_abstract,
                *is_static,
            );
            Some((access.clone(), text))
        }

        Stmt::GenDef {
            name,
            template_params,
            params,
            yield_type,
            access,
            ..
        } => {
            let text = gen_stub(
                name,
                template_params,
                params,
                yield_type.as_deref(),
                indent_level,
            );
            Some((access.clone(), text))
        }

        _ => None,
    }
}

// ── enum stub ─────────────────────────────────────────────────────────────────

/// enum 定義のスタブ文字列を生成する。
fn enum_stub(name: &str, variants: &[(String, Option<crate::ast::Expr>)]) -> String {
    let mut out = format!("enum {name}:\n");
    for (variant, value) in variants {
        match value {
            Some(crate::ast::Expr::Int(n)) => {
                out.push_str(&format!("    {variant} = {n}\n"));
            }
            Some(_) => {
                out.push_str(&format!("    {variant} = ...\n"));
            }
            None => {
                out.push_str(&format!("    {variant}\n"));
            }
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_check::{InferredType, VarAttr};
    use std::collections::HashMap;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        crate::parser::Parser::new(tokens, None).parse_program().expect("parse")
    }

    /// 拡張へ渡すスタブにグローバル変数を型と属性つきで書く（`enum_member_type_plan.md` 6-3）。
    #[test]
    fn editor_stub_writes_globals_with_type_and_attribute() {
        let body = parse(concat!(
            "class P:
",
            "    mut x: int
",
            "const A: int = 1
",
            "let B = \"s\"
",
            "mut C: list[int] = [1]
",
            "mut D = 0
",
            "let O = P(1)
",
        ));
        let mut globals = HashMap::new();
        globals.insert("A".to_string(), (InferredType::Int, VarAttr::Const));
        globals.insert("B".to_string(), (InferredType::Str, VarAttr::Let));
        // 型の分からない変数は書かない（`Undefined` の型が付いてしまうので）。
        globals.insert("D".to_string(), (InferredType::Unresolved, VarAttr::Mut));
        // モジュール自身の名前（`m.`）は外す。
        globals.insert("O".to_string(), (InferredType::NamedInstance("m.P".to_string()), VarAttr::Let));
        let text = generate_editor_stub(&body, "m", Some(&globals));
        assert!(text.contains("const A: int = Undefined"), "{text}");
        assert!(text.contains("let B: str = Undefined"), "{text}");
        assert!(text.contains("mut C: list[int] = Undefined"), "{text}");
        assert!(text.contains("let O: P = Undefined"), "{text}");
        assert!(!text.contains("mut D"), "a global of unknown type must be left out: {text}");
        // 読み戻せる（拡張はこのテキストをパースする）。
        parse(&text);
        // 部分コンパイル・js-proc のスタブは変数を書かない（従来どおり）。
        assert!(!generate_stub(&body).contains("const A"));
    }
}
