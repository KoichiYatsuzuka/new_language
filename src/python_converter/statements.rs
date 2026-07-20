// python_converter/statements.rs — 文の変換: make_span、文リスト変換・巻き上げ処理・個別文変換。

use {
    rustpython_parser::ast as py,
    crate::ast::{Expr, Stmt},
    crate::token::Span,
};
use super::*;

// ---------------------------------------------------------------------------
// スパン生成ヘルパー
// ---------------------------------------------------------------------------

/// 指定ファイル名を持つダミースパン（行・列 0）を生成する。
pub(crate) fn make_span(filename: &str) -> Span {
    Span {
        file: filename.into(),
        line: 0,
        col: 0,
    }
}

// ---------------------------------------------------------------------------
// 文変換
// ---------------------------------------------------------------------------

/// Python 文のスライスを tl の `Stmt` リストに変換する（ホイストなし）。
pub(crate) fn convert_stmts(stmts: &[py::Stmt], filename: &str) -> Result<Vec<Stmt>, String> {
    convert_stmts_with_hoist(stmts, filename, false)
}

/// 関数本体用: if ブランチ内で代入される変数を事前宣言（ホイスト）してスコープ問題を回避する。
pub(crate) fn convert_stmts_fn_body(stmts: &[py::Stmt], filename: &str) -> Result<Vec<Stmt>, String> {
    convert_stmts_with_hoist(stmts, filename, true)
}

/// ホイストフラグを指定して Python 文のスライスを tl の `Stmt` リストに変換する。
pub(crate) fn convert_stmts_with_hoist(
    stmts: &[py::Stmt],
    filename: &str,
    hoist: bool,
) -> Result<Vec<Stmt>, String> {
    let hoisted: std::collections::HashSet<String> = if hoist {
        let mut set = std::collections::HashSet::new();
        for stmt in stmts {
            if let py::Stmt::If(i) = stmt {
                collect_if_branch_assigns(i, &mut set);
            }
        }
        set
    } else {
        std::collections::HashSet::new()
    };

    let mut result = Vec::new();
    for var in &hoisted {
        result.push(Stmt::Mut(var.clone(), None, Expr::None));
    }
    for stmt in stmts {
        if let Some(s) = convert_stmt_in_hoist_ctx(stmt, filename, &hoisted)? {
            result.push(s);
        }
    }
    Ok(result)
}

/// ホイストコンテキストを保ちながら文を変換する。
pub(crate) fn convert_stmt_in_hoist_ctx(
    stmt: &py::Stmt,
    filename: &str,
    hoisted: &std::collections::HashSet<String>,
) -> Result<Option<Stmt>, String> {
    if !hoisted.is_empty() {
        if let py::Stmt::Assign(a) = stmt {
            if a.targets.len() == 1 {
                if let py::Expr::Name(n) = &a.targets[0] {
                    let name = n.id.to_string();
                    if hoisted.contains(&name) {
                        let val = convert_expr(&a.value, filename)?;
                        return Ok(Some(Stmt::Assign {
                            name,
                            value: val,
                            span: make_span(filename),
                            slot: Default::default(),
                        }));
                    }
                }
            }
        }
    }

    if let py::Stmt::If(i) = stmt {
        if is_main_guard(&i.test) {
            return Ok(None);
        }
        let cond = convert_expr(&i.test, filename)?;
        let then_body = convert_stmts_hoisted_branch(&i.body, filename, hoisted)?;

        let mut branches = vec![(cond, then_body)];
        let mut else_body: Option<Vec<Stmt>> = None;
        let mut orelse = &i.orelse;
        loop {
            if orelse.is_empty() {
                break;
            }
            if orelse.len() == 1 {
                if let py::Stmt::If(elif) = &orelse[0] {
                    if is_main_guard(&elif.test) {
                        break;
                    }
                    let c = convert_expr(&elif.test, filename)?;
                    let b = convert_stmts_hoisted_branch(&elif.body, filename, hoisted)?;
                    branches.push((c, b));
                    orelse = &elif.orelse;
                    continue;
                }
            }
            else_body = Some(convert_stmts_hoisted_branch(orelse, filename, hoisted)?);
            break;
        }
        return Ok(Some(Stmt::If {
            branches,
            else_body,
        }));
    }

    convert_stmt(stmt, filename)
}

/// ホイストコンテキストを引き継いで if/elif/else ブランチ内の文群を変換する。
pub(crate) fn convert_stmts_hoisted_branch(
    stmts: &[py::Stmt],
    filename: &str,
    hoisted: &std::collections::HashSet<String>,
) -> Result<Vec<Stmt>, String> {
    let mut result = Vec::new();
    for stmt in stmts {
        if let Some(s) = convert_stmt_in_hoist_ctx(stmt, filename, hoisted)? {
            result.push(s);
        }
    }
    Ok(result)
}

/// if 文のすべてのブランチ内で単純名前代入される変数を収集する（再帰あり）。
pub(crate) fn collect_if_branch_assigns(i: &py::StmtIf, out: &mut std::collections::HashSet<String>) {
    for stmt in i.body.iter().chain(i.orelse.iter()) {
        match stmt {
            py::Stmt::Assign(a) if a.targets.len() == 1 => {
                if let py::Expr::Name(n) = &a.targets[0] {
                    out.insert(n.id.to_string());
                }
            }
            py::Stmt::AnnAssign(a) if a.value.is_some() => {
                if let py::Expr::Name(n) = &*a.target {
                    out.insert(n.id.to_string());
                }
            }
            py::Stmt::If(nested) => collect_if_branch_assigns(nested, out),
            _ => {}
        }
    }
}

/// 単一の Python 文を tl の Stmt に変換する。
pub(crate) fn convert_stmt(stmt: &py::Stmt, filename: &str) -> Result<Option<Stmt>, String> {
    match stmt {
        // ----- 関数定義 -----
        py::Stmt::FunctionDef(f) => {
            if !f.decorator_list.is_empty() {
                return Err(format!(
                    "{filename}: decorators are not yet implemented (function '{}')",
                    f.name.as_str()
                ));
            }
            let params = convert_params(&f.args, filename)?;
            let return_type = f.returns.as_deref().map(convert_annotation);
            let body = convert_stmts_fn_body(&f.body, filename)?;
            Ok(Some(Stmt::FnDef {
                name: f.name.to_string(),
                template_params: vec![],
                params,
                return_type,
                body,
                is_abstract: false,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: crate::ast::Accessibility::Public,
            }))
        }

        // ----- 非同期関数定義（未サポート） -----
        py::Stmt::AsyncFunctionDef(f) => Err(format!(
            "{filename}: async def is not supported (function '{}')",
            f.name.as_str()
        )),

        // ----- クラス定義 -----
        py::Stmt::ClassDef(c) => {
            if !c.decorator_list.is_empty() {
                return Err(format!(
                    "{filename}: decorators are not yet implemented (class '{}')",
                    c.name.as_str()
                ));
            }
            convert_class(c, filename).map(Some)
        }

        // ----- return -----
        py::Stmt::Return(r) => {
            let expr = r
                .value
                .as_deref()
                .map(|e| convert_expr(e, filename))
                .transpose()?;
            Ok(Some(Stmt::Return(expr)))
        }

        // ----- 代入: 単純な `x = expr` -----
        py::Stmt::Assign(a) => {
            if a.targets.len() != 1 {
                return Err(format!(
                    "{filename}: multiple assignment targets are not supported"
                ));
            }
            let target = &a.targets[0];
            match target {
                py::Expr::Name(n) => {
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(Stmt::Mut(n.id.to_string(), None, val)))
                }
                py::Expr::Attribute(_) => {
                    let target_expr = convert_expr(target, filename)?;
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(Stmt::AttrAssign {
                        target: target_expr,
                        value: val,
                    }))
                }
                py::Expr::Tuple(_) | py::Expr::List(_) => Err(format!(
                    "{filename}: tuple/list unpacking in assignment is not supported"
                )),
                _ => Err(format!("{filename}: unsupported assignment target")),
            }
        }

        // ----- 型アノテーション付き代入: `x: int = 5` -----
        py::Stmt::AnnAssign(a) => match &*a.target {
            py::Expr::Name(n) => {
                if let Some(val_expr) = &a.value {
                    let val = convert_expr(val_expr, filename)?;
                    Ok(Some(Stmt::Mut(n.id.to_string(), None, val)))
                } else {
                    Ok(None)
                }
            }
            py::Expr::Attribute(_) => {
                if let Some(val_expr) = &a.value {
                    let target_expr = convert_expr(&a.target, filename)?;
                    let val = convert_expr(val_expr, filename)?;
                    Ok(Some(Stmt::AttrAssign {
                        target: target_expr,
                        value: val,
                    }))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        },

        // ----- 拡張代入: `x += expr` -----
        py::Stmt::AugAssign(a) => {
            let op = convert_augop(&a.op, filename)?;
            match &*a.target {
                py::Expr::Name(n) => {
                    let val = convert_expr(&a.value, filename)?;
                    let span = make_span(filename);
                    Ok(Some(Stmt::CompoundAssign {
                        name: n.id.to_string(),
                        op,
                        value: val,
                        span,
                        slot: Default::default(),
                    }))
                }
                py::Expr::Attribute(_) => {
                    let target_expr = convert_expr(&a.target, filename)?;
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(Stmt::AttrCompoundAssign {
                        target: target_expr,
                        op,
                        value: val,
                    }))
                }
                _ => Err(format!(
                    "{filename}: unsupported augmented assignment target"
                )),
            }
        }

        // ----- if 文（ホイストなし版: convert_stmt_in_hoist_ctx 経由でホイストあり版を使う） -----
        py::Stmt::If(i) => {
            if is_main_guard(&i.test) {
                return Ok(None);
            }
            let cond = convert_expr(&i.test, filename)?;
            let then_body = convert_stmts(&i.body, filename)?;

            let mut branches = vec![(cond, then_body)];
            let mut else_body: Option<Vec<Stmt>> = None;

            let mut orelse = &i.orelse;
            loop {
                if orelse.is_empty() {
                    break;
                }
                if orelse.len() == 1 {
                    if let py::Stmt::If(elif) = &orelse[0] {
                        if is_main_guard(&elif.test) {
                            break;
                        }
                        let c = convert_expr(&elif.test, filename)?;
                        let b = convert_stmts(&elif.body, filename)?;
                        branches.push((c, b));
                        orelse = &elif.orelse;
                        continue;
                    }
                }
                else_body = Some(convert_stmts(orelse, filename)?);
                break;
            }

            Ok(Some(Stmt::If {
                branches,
                else_body,
            }))
        }

        // ----- while -----
        py::Stmt::While(w) => {
            let cond = convert_expr(&w.test, filename)?;
            let body = convert_stmts(&w.body, filename)?;
            Ok(Some(Stmt::While { cond, body }))
        }

        // ----- for -----
        py::Stmt::For(f) => {
            let target = match &*f.target {
                py::Expr::Name(n) => n.id.to_string(),
                _ => {
                    return Err(format!(
                        "{filename}: tuple unpacking in for-loop target is not supported"
                    ))
                }
            };
            let iter = convert_expr(&f.iter, filename)?;
            let body = convert_stmts(&f.body, filename)?;
            Ok(Some(Stmt::For {
                targets: vec![target],
                iter,
                body,
            }))
        }

        // ----- with（未サポート） -----
        py::Stmt::With(_) => Err(format!("{filename}: 'with' statement is not supported")),
        py::Stmt::AsyncWith(_) => Err(format!(
            "{filename}: 'async with' statement is not supported"
        )),
        py::Stmt::AsyncFor(_) => Err(format!(
            "{filename}: 'async for' statement is not supported"
        )),

        // ----- try / except / finally -----
        py::Stmt::Try(t) => {
            use crate::ast::ExceptHandler;
            let body = convert_stmts(&t.body, filename)?;
            let mut handlers = Vec::new();
            for h in &t.handlers {
                let py::ExceptHandler::ExceptHandler(eh) = h;
                let exc_type = eh.type_.as_deref().map(|e| match e {
                    py::Expr::Name(n) => n.id.to_string(),
                    _ => "Exception".to_string(),
                });
                let name = eh.name.as_ref().map(|n| n.to_string());
                let hbody = convert_stmts(&eh.body, filename)?;
                handlers.push(ExceptHandler {
                    exc_type,
                    name,
                    body: hbody,
                });
            }
            let finally_body = if t.finalbody.is_empty() {
                None
            } else {
                Some(convert_stmts(&t.finalbody, filename)?)
            };
            Ok(Some(Stmt::Try {
                body,
                handlers,
                finally_body,
            }))
        }

        // ----- raise -----
        py::Stmt::Raise(r) => {
            let exc = r
                .exc
                .as_deref()
                .map(|e| convert_expr(e, filename))
                .transpose()?;
            let span = make_span(filename);
            Ok(Some(Stmt::Raise { exc, span }))
        }

        // ----- pass -----
        py::Stmt::Pass(_) => Ok(Some(Stmt::Pass)),

        // ----- break / continue -----
        py::Stmt::Break(_) => Ok(Some(Stmt::Break)),
        py::Stmt::Continue(_) => Ok(Some(Stmt::Continue)),

        // ----- 式文 -----
        py::Stmt::Expr(e) => {
            let expr = convert_expr(&e.value, filename)?;
            Ok(Some(Stmt::Expr(expr)))
        }

        // ----- global / nonlocal → 無視 -----
        py::Stmt::Global(_) | py::Stmt::Nonlocal(_) => Ok(None),

        // ----- import / from-import（モジュール本体内の import は無視） -----
        py::Stmt::Import(_) | py::Stmt::ImportFrom(_) => Ok(None),

        // ----- match（未サポート） -----
        py::Stmt::Match(_) => Err(format!("{filename}: 'match' statement is not supported")),

        // ----- type alias（Python 3.12+） -----
        py::Stmt::TypeAlias(_) => Ok(None),

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python statement")),
    }
}

