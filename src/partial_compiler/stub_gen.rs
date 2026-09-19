/// `.ars` stub file generator.
///
/// Walks the top-level AST and emits type-only declarations with `...` bodies.
/// The output is valid `.ar` syntax and is used by the type checker and VS Code
/// extension to inspect a compiled module without its implementation.
use crate::ast::{Accessibility, FieldKind, Param, Stmt, TemplateParam};

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

        Stmt::TraitDef {
            name,
            template_params,
            body,
        } => Some(trait_stub(name, template_params, body)),

        Stmt::NewTypeDef { name, original } => Some(format!("new_type {name}: {original}\n")),

        Stmt::EnumDef { name, variants } => Some(enum_stub(name, variants)),

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
        Stmt::Field {
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
