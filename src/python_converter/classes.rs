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
                                // ⚠ Python のクラス属性は**可変・全インスタンス共有**なので
                                //   `StaticMut`（`static mut`）に対応する。`Const` にすると
                                //   `Counter.count = ...` が
                                //   `cannot assign to class variable (declared const)` で落ちる。
                                //   定数として使いたい属性と静的に区別できないため、
                                //   Python 側の意味に忠実な**可変**へ倒す。
                                kind: FieldKind::StaticMut,
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
                                    // 注釈つきクラス変数 `n: int = 5` も同じく共有可変。
                                    kind: FieldKind::StaticMut,
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
                // メソッド本体は**新しいスコープ**。パラメータ名（`self` 含む）を宣言済みとして渡す。
                let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                let body = convert_scope(&f.body, filename, &param_names)?;
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
///
/// 順序は Python の宣言順（posonly → 通常 → vararg → kwonly）をそのまま保つ。
///
/// ⚠ **位置専用マーカ `/` と キーワード専用マーカ `*`（bare `*`）は無視して平坦化する**（項目 24）。
/// rustpython はどちらもマーカ自体をノードとして持たず、`posonlyargs` / `kwonlyargs` という
/// **区分**として表現するので、区分をまたいで 1 本の `Param` 列に並べるだけで済む。
/// 　- `def f(a, *, b)` → `Param[a, b]`。`f(1, b=2)` も `f(1, 2)` も通る。
/// 　- **意味の緩和**: Python はキーワード専用引数を位置渡しできない（`TypeError`）が、
/// 　  平坦化後の Arrow は位置渡しも受け付ける。Arrow 側に「位置渡し禁止」を表す
/// 　  `Param` のフラグが無いため。**受け入れる Python コードが広がる方向**なので許容
/// 　  （`/` も同様に「キーワード渡しも通る」方向へ緩む）。
///
/// ⚠ **デフォルト値は「定義時に 1 回」ではなく「呼び出しごと」に評価される**（項目 1）。
/// rustpython は `ArgWithDefault.default` として引数ごとにデフォルト式を持つので写すだけだが、
/// Arrow の `exec_fn_evaled` は毎回 `self.eval(expr)` する（`functions/execution.rs` の
/// `evaluated_defaults`）。Python は `def` の実行時に 1 回だけ評価して**その値を共有**する。
/// 　- リテラル（`0` / `"hi"` / `None` / `True`）は**完全に同じ**。実用上ほぼこれ。
/// 　- ⚠ **可変デフォルト**（`def f(x=[])`）だけ意味が違う: Python は同じリストを呼び出し間で
/// 　  共有する（有名な罠）が、Arrow は毎回新しいリストを作る。**Arrow 側が「普通に期待される」
/// 　  挙動**で、その罠に依存したコードだけが差を踏む。
/// 　- ⚠ 名前を参照するデフォルト（`def f(x=CONST)`）も、Arrow は呼び出し時に読み直す。
pub(crate) fn convert_params(args: &py::Arguments, filename: &str) -> Result<Vec<Param>, String> {
    let mut params: Vec<Param> = Vec::new();

    // `/` より前（posonlyargs）と通常引数は区別せず同じ列に積む。
    for arg in args.posonlyargs.iter().chain(args.args.iter()) {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        let default = arg
            .default
            .as_deref()
            .map(|e| convert_expr(e, filename))
            .transpose()?;
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default,
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

    // bare `*` / `*args` より後ろのキーワード専用引数。通常引数として平坦化する（項目 24）。
    // ⚠ 上の vararg 分岐が走った場合（`def f(a, *rest, b)`）は、`*args` が不正な
    //   `Param` になっている（項目 6 未実装）ため、この列全体が壊れる。bare `*` 単体は無害。
    for arg in &args.kwonlyargs {
        let type_ann = arg.def.annotation.as_deref().map(convert_annotation);
        let default = arg
            .default
            .as_deref()
            .map(|e| convert_expr(e, filename))
            .transpose()?;
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
            default,
            variadic: false,
        });
    }

    // **kwargs はパラメータリストに含めない。
    // 呼び出し時に余分なキーワード引数が kwargs dict として自動注入される。

    Ok(params)
}

