// python_converter/classes.rs — クラス・パラメータの変換: convert_class / self フィールド収集 / パラメータ型抽出 / convert_params。

use {
    rustpython_parser::ast as py,
    crate::ast::{FieldKind, Param, Stmt},
};
use super::*;

// ---------------------------------------------------------------------------
// クラス変換
// ---------------------------------------------------------------------------

/// Python クラス定義を tl の `Stmt::ClassDef` に変換する。
/// フィールドは `__init__` からの `self.x = ...` 代入と型アノテーションを元に収集する。
pub(crate) fn convert_class(c: &py::StmtClassDef, filename: &str) -> Result<Stmt, String> {
    let class_name = c.name.to_string();
    let bases: Vec<String> = c.bases.iter().map(expr_to_name).collect();
    // クラスデコレータ。`@staticmethod` 等はクラスには付かないので `in_class: false`。
    let class_dec = convert_decorators(
        &c.decorator_list,
        filename,
        &format!("class '{class_name}'"),
        false,
    )?;

    let mut fields: Vec<Stmt> = Vec::new();
    let mut methods: Vec<Stmt> = Vec::new();

    // __init__ を先に見つけてインスタンスフィールドを収集
    let mut init_fields: Vec<(String, String)> = Vec::new();
    for stmt in &c.body {
        if let py::Stmt::FunctionDef(f) = stmt {
            if f.name.as_str() == "__init__" {
                collect_self_fields(&f.body, &mut init_fields);
                let param_types = extract_param_types(&f.args);
                for (fname, ftype) in init_fields.iter_mut() {
                    if ftype == "Any" {
                        if let Some(t) = param_types.get(fname.as_str()) {
                            *ftype = t.clone();
                        }
                    }
                }
                break;
            }
        }
    }

    let mut seen_fields: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (fname, ftype) in &init_fields {
        if seen_fields.insert(fname.clone()) {
            fields.push(Stmt::Field {
                name: fname.clone(),
                kind: FieldKind::Mut,
                type_ann: ftype.clone(),
                default: None,
                access: crate::ast::Accessibility::Public,
            });
        }
    }

    for stmt in &c.body {
        match stmt {
            py::Stmt::Assign(a) if a.targets.len() == 1 => {
                if let py::Expr::Name(n) = &a.targets[0] {
                    let fname = n.id.to_string();
                    if !fname.starts_with("__") {
                        let default = convert_expr(&a.value, filename)?;
                        if seen_fields.insert(fname.clone()) {
                            fields.push(Stmt::Field {
                                name: fname,
                                kind: FieldKind::Const,
                                type_ann: "Any".to_string(),
                                default: Some(default),
                                access: crate::ast::Accessibility::Public,
                            });
                        }
                    }
                }
            }
            py::Stmt::AnnAssign(a) => {
                if let py::Expr::Name(n) = &*a.target {
                    let fname = n.id.to_string();
                    if !fname.starts_with("__") {
                        let type_ann = convert_annotation(&a.annotation);
                        if let Some(val_expr) = &a.value {
                            let default = convert_expr(val_expr, filename)?;
                            if seen_fields.insert(fname.clone()) {
                                fields.push(Stmt::Field {
                                    name: fname,
                                    kind: FieldKind::Const,
                                    type_ann,
                                    default: Some(default),
                                    access: crate::ast::Accessibility::Public,
                                });
                            }
                        }
                    }
                }
            }
            py::Stmt::FunctionDef(f) => {
                let dec = convert_decorators(
                    &f.decorator_list,
                    filename,
                    &format!("method '{}.{}'", class_name, f.name.as_str()),
                    true,
                )?;
                let params = convert_params(&f.args, filename)?;
                let return_type = f.returns.as_deref().map(convert_annotation);
                let body = convert_stmts_fn_body(&f.body, filename)?;
                methods.push(Stmt::FnDef {
                    name: f.name.to_string(),
                    template_params: vec![],
                    params,
                    return_type,
                    body,
                    is_abstract: dec.is_abstract,
                    is_static: dec.is_static,
                    is_class_method: dec.is_class_method,
                    decorators: dec.decorators,
                    access: crate::ast::Accessibility::Public,
                });
            }
            py::Stmt::Pass(_) => {}
            _ => {}
        }
    }

    let mut body = fields;
    body.extend(methods);

    Ok(Stmt::ClassDef {
        name: class_name,
        template_params: vec![],
        bases,
        body,
        decorators: class_dec.decorators,
    })
}

/// `__init__` 本体（ネスト含む）を再帰探索して `self.field = ...` の代入を収集する。
pub(crate) fn collect_self_fields(stmts: &[py::Stmt], out: &mut Vec<(String, String)>) {
    for stmt in stmts {
        match stmt {
            py::Stmt::Assign(a) => {
                for target in &a.targets {
                    if let py::Expr::Attribute(attr) = target {
                        if is_self(&attr.value) {
                            let fname = attr.attr.to_string();
                            if !out.iter().any(|(n, _)| n == &fname) {
                                out.push((fname, "Any".to_string()));
                            }
                        }
                    }
                }
            }
            py::Stmt::AnnAssign(a) => {
                if let py::Expr::Attribute(attr) = &*a.target {
                    if is_self(&attr.value) {
                        let fname = attr.attr.to_string();
                        let type_ann = convert_annotation(&a.annotation);
                        if !out.iter().any(|(n, _)| n == &fname) {
                            out.push((fname, type_ann));
                        }
                    }
                }
            }
            py::Stmt::If(i) => {
                collect_self_fields(&i.body, out);
                collect_self_fields(&i.orelse, out);
            }
            py::Stmt::While(w) => collect_self_fields(&w.body, out),
            py::Stmt::For(f) => collect_self_fields(&f.body, out),
            py::Stmt::Try(t) => {
                collect_self_fields(&t.body, out);
                for h in &t.handlers {
                    let py::ExceptHandler::ExceptHandler(eh) = h;
                    collect_self_fields(&eh.body, out);
                }
                collect_self_fields(&t.finalbody, out);
            }
            _ => {}
        }
    }
}

/// `__init__` 引数リストからパラメータ名 → 型アノテーション文字列のマップを作る。
pub(crate) fn extract_param_types(args: &py::Arguments) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for arg in &args.args {
        if let Some(ann) = &arg.def.annotation {
            map.insert(arg.def.arg.to_string(), convert_annotation(ann));
        }
    }
    for arg in args.posonlyargs.iter().chain(args.kwonlyargs.iter()) {
        if let Some(ann) = &arg.def.annotation {
            map.insert(arg.def.arg.to_string(), convert_annotation(ann));
        }
    }
    map
}

// ---------------------------------------------------------------------------
// パラメータ変換
// ---------------------------------------------------------------------------

/// Python の引数リスト（`py::Arguments`）を tl の `Param` リストに変換する。
pub(crate) fn convert_params(args: &py::Arguments, _filename: &str) -> Result<Vec<Param>, String> {
    let mut params: Vec<Param> = Vec::new();

    for arg in args.posonlyargs.iter().chain(args.args.iter()) {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default: None,
            variadic: false,
        });
    }

    if let Some(_vararg) = &args.vararg {
        params.push(Param {
            name: "*args".to_string(),
            mutable: true,
            type_ann: Some("list[Any]".to_string()),
            default: None,
            variadic: false,
        });
    }

    for arg in &args.kwonlyargs {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default: None,
            variadic: false,
        });
    }

    // **kwargs はパラメータリストに含めない。
    // 呼び出し時に余分なキーワード引数が kwargs dict として自動注入される。

    Ok(params)
}

