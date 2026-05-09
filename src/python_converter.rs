// python_converter.rs — rustpython-parser の AST を tl の AST に変換する

use rustpython_parser::{ast as py, Parse};

use crate::ast::{BinOp, CallArg, Expr, FieldKind, Param, Stmt, UnaryOp};
use crate::token::Span;

// ---------------------------------------------------------------------------
// 公開エントリポイント
// ---------------------------------------------------------------------------

pub fn convert_python_source(source: &str, filename: &str) -> Result<Vec<Stmt>, String> {
    let ast = py::Suite::parse(source, filename)
        .map_err(|e| format!("{filename}: {e}"))?;
    convert_stmts(&ast, filename)
}

// ---------------------------------------------------------------------------
// スパン生成ヘルパー
// ---------------------------------------------------------------------------

fn make_span(filename: &str) -> Span {
    Span { file: filename.into(), line: 0, col: 0 }
}

// ---------------------------------------------------------------------------
// 文変換
// ---------------------------------------------------------------------------

fn convert_stmts(stmts: &[py::Stmt], filename: &str) -> Result<Vec<Stmt>, String> {
    convert_stmts_with_hoist(stmts, filename, false)
}

/// 関数本体用: if ブランチ内で代入される変数を事前宣言（ホイスト）してスコープ問題を回避する。
fn convert_stmts_fn_body(stmts: &[py::Stmt], filename: &str) -> Result<Vec<Stmt>, String> {
    convert_stmts_with_hoist(stmts, filename, true)
}

fn convert_stmts_with_hoist(
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
        result.push(Stmt::Mut(var.clone(), Expr::None));
    }
    for stmt in stmts {
        if let Some(s) = convert_stmt_in_hoist_ctx(stmt, filename, &hoisted)? {
            result.push(s);
        }
    }
    Ok(result)
}

/// ホイストコンテキストを保ちながら文を変換する。
fn convert_stmt_in_hoist_ctx(
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
        return Ok(Some(Stmt::If { branches, else_body }));
    }

    convert_stmt(stmt, filename)
}

fn convert_stmts_hoisted_branch(
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
fn collect_if_branch_assigns(
    i: &py::StmtIf,
    out: &mut std::collections::HashSet<String>,
) {
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
fn convert_stmt(stmt: &py::Stmt, filename: &str) -> Result<Option<Stmt>, String> {
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
            let return_type = f.returns.as_deref().map(|e| convert_annotation(e));
            let body = convert_stmts_fn_body(&f.body, filename)?;
            Ok(Some(Stmt::FnDef {
                name: f.name.to_string(),
                template_params: vec![],
                params,
                return_type,
                body,
                is_abstract: false,
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
            let expr = r.value.as_deref().map(|e| convert_expr(e, filename)).transpose()?;
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
                    Ok(Some(Stmt::Mut(n.id.to_string(), val)))
                }
                py::Expr::Attribute(_) => {
                    let target_expr = convert_expr(target, filename)?;
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(Stmt::AttrAssign { target: target_expr, value: val }))
                }
                py::Expr::Tuple(_) | py::Expr::List(_) => Err(format!(
                    "{filename}: tuple/list unpacking in assignment is not supported"
                )),
                _ => Err(format!(
                    "{filename}: unsupported assignment target"
                )),
            }
        }

        // ----- 型アノテーション付き代入: `x: int = 5` -----
        py::Stmt::AnnAssign(a) => {
            match &*a.target {
                py::Expr::Name(n) => {
                    if let Some(val_expr) = &a.value {
                        let val = convert_expr(val_expr, filename)?;
                        Ok(Some(Stmt::Mut(n.id.to_string(), val)))
                    } else {
                        Ok(None)
                    }
                }
                py::Expr::Attribute(_) => {
                    if let Some(val_expr) = &a.value {
                        let target_expr = convert_expr(&a.target, filename)?;
                        let val = convert_expr(val_expr, filename)?;
                        Ok(Some(Stmt::AttrAssign { target: target_expr, value: val }))
                    } else {
                        Ok(None)
                    }
                }
                _ => Ok(None),
            }
        }

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
                    }))
                }
                py::Expr::Attribute(_) => {
                    let target_expr = convert_expr(&a.target, filename)?;
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(Stmt::AttrCompoundAssign { target: target_expr, op, value: val }))
                }
                _ => Err(format!("{filename}: unsupported augmented assignment target")),
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

            Ok(Some(Stmt::If { branches, else_body }))
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
                _ => return Err(format!(
                    "{filename}: tuple unpacking in for-loop target is not supported"
                )),
            };
            let iter = convert_expr(&f.iter, filename)?;
            let body = convert_stmts(&f.body, filename)?;
            Ok(Some(Stmt::For { target, iter, body }))
        }

        // ----- with（未サポート） -----
        py::Stmt::With(_) => Err(format!(
            "{filename}: 'with' statement is not supported"
        )),
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
                handlers.push(ExceptHandler { exc_type, name, body: hbody });
            }
            let finally_body = if t.finalbody.is_empty() {
                None
            } else {
                Some(convert_stmts(&t.finalbody, filename)?)
            };
            Ok(Some(Stmt::Try { body, handlers, finally_body }))
        }

        // ----- raise -----
        py::Stmt::Raise(r) => {
            let exc = r.exc.as_deref().map(|e| convert_expr(e, filename)).transpose()?;
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
        py::Stmt::Match(_) => Err(format!(
            "{filename}: 'match' statement is not supported"
        )),

        // ----- type alias（Python 3.12+） -----
        py::Stmt::TypeAlias(_) => Ok(None),

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python statement")),
    }
}

// ---------------------------------------------------------------------------
// クラス変換
// ---------------------------------------------------------------------------

fn convert_class(c: &py::StmtClassDef, filename: &str) -> Result<Stmt, String> {
    let class_name = c.name.to_string();
    let bases: Vec<String> = c.bases.iter().map(|b| expr_to_name(b)).collect();

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
                                });
                            }
                        }
                    }
                }
            }
            py::Stmt::FunctionDef(f) => {
                if !f.decorator_list.is_empty() {
                    return Err(format!(
                        "{filename}: decorators are not yet implemented (method '{}.{}')",
                        class_name,
                        f.name.as_str()
                    ));
                }
                let params = convert_params(&f.args, filename)?;
                let return_type = f.returns.as_deref().map(|e| convert_annotation(e));
                let body = convert_stmts_fn_body(&f.body, filename)?;
                methods.push(Stmt::FnDef {
                    name: f.name.to_string(),
                    template_params: vec![],
                    params,
                    return_type,
                    body,
                    is_abstract: false,
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
    })
}

/// `__init__` 本体（ネスト含む）を再帰探索して `self.field = ...` の代入を収集する。
fn collect_self_fields(stmts: &[py::Stmt], out: &mut Vec<(String, String)>) {
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
fn extract_param_types(args: &py::Arguments) -> std::collections::HashMap<String, String> {
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

fn convert_params(args: &py::Arguments, _filename: &str) -> Result<Vec<Param>, String> {
    let mut params: Vec<Param> = Vec::new();

    for arg in args.posonlyargs.iter().chain(args.args.iter()) {
        let type_ann = arg.def.annotation.as_deref().map(|e| convert_annotation(e));
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
        });
    }

    if let Some(_vararg) = &args.vararg {
        params.push(Param {
            name: "*args".to_string(),
            mutable: true,
            type_ann: Some("list[Any]".to_string()),
        });
    }

    for arg in &args.kwonlyargs {
        let type_ann = arg.def.annotation.as_deref().map(|e| convert_annotation(e));
        params.push(Param {
            name: arg.def.arg.to_string(),
            mutable: true,
            type_ann,
        });
    }

    // **kwargs はパラメータリストに含めない。
    // 呼び出し時に余分なキーワード引数が kwargs dict として自動注入される。

    Ok(params)
}

// ---------------------------------------------------------------------------
// 式変換
// ---------------------------------------------------------------------------

fn convert_expr(expr: &py::Expr, filename: &str) -> Result<Expr, String> {
    match expr {
        py::Expr::Constant(c) => convert_constant(c, filename),

        py::Expr::Name(n) => Ok(Expr::Ident(n.id.to_string())),

        py::Expr::Attribute(a) => {
            let obj = convert_expr(&a.value, filename)?;
            Ok(Expr::Attr { object: Box::new(obj), attr: a.attr.to_string() })
        }

        py::Expr::BinOp(b) => {
            let op = convert_binop(&b.op, filename)?;
            let left = convert_expr(&b.left, filename)?;
            let right = convert_expr(&b.right, filename)?;
            let span = make_span(filename);
            Ok(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span })
        }

        py::Expr::UnaryOp(u) => {
            let op = match &u.op {
                py::UnaryOp::USub => UnaryOp::Neg,
                py::UnaryOp::Not  => UnaryOp::Not,
                py::UnaryOp::Invert => UnaryOp::BitNot,
                py::UnaryOp::UAdd => {
                    return convert_expr(&u.operand, filename);
                }
            };
            let operand = convert_expr(&u.operand, filename)?;
            Ok(Expr::UnaryOp { op, operand: Box::new(operand) })
        }

        py::Expr::BoolOp(b) => {
            let op = match &b.op {
                py::BoolOp::And => BinOp::And,
                py::BoolOp::Or  => BinOp::Or,
            };
            let mut values = b.values.iter();
            let first = convert_expr(values.next().unwrap(), filename)?;
            let mut result = first;
            for val in values {
                let right = convert_expr(val, filename)?;
                let span = Span::unknown();
                result = Expr::BinOp { op: op.clone(), left: Box::new(result), right: Box::new(right), span };
            }
            Ok(result)
        }

        py::Expr::Compare(c) => {
            if c.ops.len() != 1 || c.comparators.len() != 1 {
                return Err(format!(
                    "{filename}: chained comparisons are not supported"
                ));
            }
            let op = convert_cmpop(&c.ops[0], filename)?;
            let left = convert_expr(&c.left, filename)?;
            let right = convert_expr(&c.comparators[0], filename)?;
            let span = make_span(filename);
            Ok(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span })
        }

        py::Expr::Call(c) => {
            let func = convert_expr(&c.func, filename)?;
            let mut args: Vec<CallArg> = Vec::new();
            for arg in &c.args {
                args.push(CallArg::Positional(convert_expr(arg, filename)?));
            }
            for kw in &c.keywords {
                let name = kw.arg.as_ref().map(|a| a.to_string()).unwrap_or_default();
                if name.is_empty() {
                    return Err(format!(
                        "{filename}: **kwargs unpacking in call is not supported"
                    ));
                }
                args.push(CallArg::Keyword {
                    name,
                    value: convert_expr(&kw.value, filename)?,
                });
            }
            Ok(Expr::Call { func: Box::new(func), args })
        }

        py::Expr::Subscript(s) => {
            let obj = convert_expr(&s.value, filename)?;
            let idx = convert_expr(&s.slice, filename)?;
            Ok(Expr::Subscript { object: Box::new(obj), index: Box::new(idx) })
        }

        py::Expr::List(l) => {
            let items: Result<Vec<Expr>, _> = l.elts.iter().map(|e| convert_expr(e, filename)).collect();
            Ok(Expr::List(items?))
        }

        py::Expr::Tuple(t) => {
            let items: Result<Vec<Expr>, _> = t.elts.iter().map(|e| convert_expr(e, filename)).collect();
            Ok(Expr::Tuple(items?))
        }

        py::Expr::Dict(d) => {
            let mut pairs: Vec<(Expr, Expr)> = Vec::new();
            for (k, v) in d.keys.iter().zip(d.values.iter()) {
                let Some(k) = k else {
                    return Err(format!("{filename}: **dict unpacking in dict literal is not supported"));
                };
                pairs.push((convert_expr(k, filename)?, convert_expr(v, filename)?));
            }
            Ok(Expr::Dict(pairs))
        }

        py::Expr::ListComp(_) | py::Expr::SetComp(_) | py::Expr::DictComp(_) | py::Expr::GeneratorExp(_) => {
            Err(format!("{filename}: comprehensions are not supported"))
        }

        py::Expr::Lambda(_) => Err(format!("{filename}: lambda is not supported")),

        py::Expr::JoinedStr(_) => Err(format!("{filename}: f-strings are not supported")),

        py::Expr::Await(_) => Err(format!("{filename}: 'await' is not supported")),

        py::Expr::Yield(_) | py::Expr::YieldFrom(_) => Err(format!(
            "{filename}: yield expression in Python is not supported"
        )),

        py::Expr::NamedExpr(_) =>
            Err(format!("{filename}: walrus operator ':=' is not supported")),

        py::Expr::IfExp(_) =>
            Err(format!("{filename}: inline 'if' expression is not supported")),

        py::Expr::Starred(_) => Err(format!(
            "{filename}: starred expression is not supported in this context"
        )),

        py::Expr::Set(_) => Err(format!("{filename}: set literal is not supported")),

        py::Expr::Slice(_) =>
            Err(format!("{filename}: slice expression is not supported")),

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python expression")),
    }
}

// ---------------------------------------------------------------------------
// 定数変換
// ---------------------------------------------------------------------------

fn convert_constant(c: &py::ExprConstant, filename: &str) -> Result<Expr, String> {
    match &c.value {
        py::Constant::Int(n) => {
            let v: i64 = n.try_into().unwrap_or(i64::MAX);
            Ok(Expr::Int(v))
        }
        py::Constant::Float(f) => Ok(Expr::Float(*f)),
        py::Constant::Str(s) => Ok(Expr::Str(s.to_string())),
        py::Constant::Bool(b) => Ok(Expr::Bool(*b)),
        py::Constant::None => Ok(Expr::None),
        py::Constant::Bytes(_) => Err(format!("{filename}: bytes literals are not supported")),
        py::Constant::Ellipsis => Ok(Expr::None),
        py::Constant::Tuple(_) => Err(format!("{filename}: constant tuple is not supported")),
        py::Constant::Complex { .. } => Err(format!("{filename}: complex numbers are not supported")),
    }
}

// ---------------------------------------------------------------------------
// 演算子変換
// ---------------------------------------------------------------------------

fn convert_binop(op: &py::Operator, filename: &str) -> Result<BinOp, String> {
    Ok(match op {
        py::Operator::Add      => BinOp::Add,
        py::Operator::Sub      => BinOp::Sub,
        py::Operator::Mult     => BinOp::Mul,
        py::Operator::Div      => BinOp::Div,
        py::Operator::FloorDiv => BinOp::FloorDiv,
        py::Operator::Mod      => BinOp::Mod,
        py::Operator::Pow      => BinOp::Pow,
        py::Operator::BitAnd   => BinOp::BitAnd,
        py::Operator::BitOr    => BinOp::BitOr,
        py::Operator::BitXor   => BinOp::BitXor,
        py::Operator::LShift   => BinOp::LShift,
        py::Operator::RShift   => BinOp::RShift,
        py::Operator::MatMult  => return Err(format!("{filename}: '@' matrix multiply is not supported")),
    })
}

fn convert_augop(op: &py::Operator, filename: &str) -> Result<BinOp, String> {
    convert_binop(op, filename)
}

fn convert_cmpop(op: &py::CmpOp, filename: &str) -> Result<BinOp, String> {
    Ok(match op {
        py::CmpOp::Eq    => BinOp::Eq,
        py::CmpOp::NotEq => BinOp::NotEq,
        py::CmpOp::Lt    => BinOp::Lt,
        py::CmpOp::LtE   => BinOp::LtEq,
        py::CmpOp::Gt    => BinOp::Gt,
        py::CmpOp::GtE   => BinOp::GtEq,
        py::CmpOp::In    => return Err(format!("{filename}: 'in' operator is not supported in expression context")),
        py::CmpOp::NotIn => return Err(format!("{filename}: 'not in' operator is not supported in expression context")),
        py::CmpOp::Is    => return Err(format!("{filename}: 'is' operator is not supported")),
        py::CmpOp::IsNot => return Err(format!("{filename}: 'is not' operator is not supported")),
    })
}

// ---------------------------------------------------------------------------
// 型アノテーション変換
// ---------------------------------------------------------------------------

pub fn convert_annotation(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Name(n) => map_type_name(n.id.as_str()),
        py::Expr::Attribute(a) => {
            let attr = a.attr.as_str();
            map_type_name(attr)
        }
        py::Expr::Subscript(s) => {
            let base = convert_annotation(&s.value);
            let arg = convert_annotation_subscript_slice(&s.slice);
            match base.as_str() {
                "Option"  => format!("Option[{arg}]"),
                "Union"   => format!("Union[{arg}]"),
                "list"    => format!("list[{arg}]"),
                "dict"    => format!("dict[{arg}]"),
                "tuple"   => format!("tuple[{arg}]"),
                "Optional" => format!("Option[{arg}]"),
                other     => format!("{other}[{arg}]"),
            }
        }
        py::Expr::Constant(c) if matches!(c.value, py::Constant::None) => "None".to_string(),
        py::Expr::Tuple(t) => {
            let parts: Vec<String> = t.elts.iter().map(|e| convert_annotation(e)).collect();
            parts.join(", ")
        }
        _ => "Any".to_string(),
    }
}

fn convert_annotation_subscript_slice(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Tuple(t) => {
            let parts: Vec<String> = t.elts.iter().map(|e| convert_annotation(e)).collect();
            parts.join(", ")
        }
        _ => convert_annotation(expr),
    }
}

fn map_type_name(name: &str) -> String {
    match name {
        "int"       => "int".to_string(),
        "str"       => "str".to_string(),
        "float"     => "float".to_string(),
        "bool"      => "bool".to_string(),
        "None"      => "None".to_string(),
        "NoneType"  => "None".to_string(),
        "list"      => "list".to_string(),
        "List"      => "list".to_string(),
        "dict"      => "dict".to_string(),
        "Dict"      => "dict".to_string(),
        "tuple"     => "tuple".to_string(),
        "Tuple"     => "tuple".to_string(),
        "Optional"  => "Option".to_string(),
        "Union"     => "Union".to_string(),
        "Any"       => "Any".to_string(),
        other       => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------------------

fn is_self(expr: &py::Expr) -> bool {
    matches!(expr, py::Expr::Name(n) if n.id.as_str() == "self")
}

fn is_main_guard(expr: &py::Expr) -> bool {
    if let py::Expr::Compare(c) = expr {
        if c.ops.len() == 1 && matches!(c.ops[0], py::CmpOp::Eq) {
            if let py::Expr::Name(n) = &*c.left {
                if n.id.as_str() == "__name__" {
                    if let Some(py::Expr::Constant(cv)) = c.comparators.first() {
                        if let py::Constant::Str(s) = &cv.value {
                            return s.as_str() == "__main__";
                        }
                    }
                }
            }
        }
    }
    false
}

fn expr_to_name(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Name(n) => n.id.to_string(),
        py::Expr::Attribute(a) => {
            let base = expr_to_name(&a.value);
            format!("{}.{}", base, a.attr)
        }
        _ => "Any".to_string(),
    }
}
