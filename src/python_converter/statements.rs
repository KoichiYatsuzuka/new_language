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

// ---------------------------------------------------------------------------
// スコープ変換（INF-A: 再代入対応）
// ---------------------------------------------------------------------------
//
// Python は「関数（またはモジュール）内で一度でも代入された名前は、そのスコープ全体で
// 同じ 1 つの変数」というスコープ規則を持つ。一方 Arrow は宣言（`Stmt::Mut`）と
// 再代入（`Stmt::Assign`）が別ノードで、同じ名前を 2 回宣言すると
// `NameError: variable 'x' is already declared` になる。
//
// そこで **スコープ単位の完全巻き上げ**を行う:
//   1. スコープ内で `=` によって単純名前に代入される名前をすべて再帰収集する。
//   2. スコープ先頭で `mut name = None` として**一度だけ**宣言する。
//   3. 以降の `x = expr` は**すべて** `Stmt::Assign`（再代入）に変換する。
//
// ⚠ 以前は「トップレベルの `if` のブランチ内代入だけ」を巻き上げていたため、
//   `for` / `while` / `try` の本体に降りた時点で巻き上げ集合が失われ、
//   同じ名前がまた `Stmt::Mut` になって落ちていた（＝ネストで壊れるドリフト）。
//   現在は `declared` をスコープ内の全ネストへ引き回す。

/// スコープ内で `=` によって単純名前に代入される変数を、**出現順**に再帰収集する。
///
/// - 入れ子の `def` / `class` は**別スコープ**なので降りない。
/// - `for` のループ変数・`except ... as e` は `=` による代入ではないので**収集しない**。
///   これらは `Stmt::For` / ハンドラ側が自前で宣言するため、巻き上げると二重になる。
///   ⚠ その結果、ループ変数を**ループの外で参照**する Python 特有の書き方
///   （`for i in xs: pass` のあとに `i` を読む）は従来どおり `NameError` になる。
/// - `x += 1`（`AugAssign`）も収集しない。Python でも事前の束縛が必要なので、
///   その束縛元の `=` から拾われる。
///
/// `seen` にはあらかじめパラメータ名を入れておく（＝宣言済みなので巻き上げ不要）。
/// 戻り値の `out` は**巻き上げるべき名前だけ**が出現順に並ぶ。
/// ⚠ 順序を固定するのは、生成 AST → バイトコード / IR を**バイト単位で比較**する
///   ゲートがあるため（`HashSet` の反復順に依存させない）。
pub(crate) fn collect_assigned_names(
    stmts: &[py::Stmt],
    out: &mut Vec<String>,
    seen: &mut std::collections::HashSet<String>,
) {
    let push = |name: String, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        if seen.insert(name.clone()) {
            out.push(name);
        }
    };
    for stmt in stmts {
        match stmt {
            py::Stmt::Assign(a) if a.targets.len() == 1 => {
                if let py::Expr::Name(n) = &a.targets[0] {
                    push(n.id.to_string(), out, seen);
                }
            }
            py::Stmt::AnnAssign(a) if a.value.is_some() => {
                if let py::Expr::Name(n) = &*a.target {
                    push(n.id.to_string(), out, seen);
                }
            }
            py::Stmt::If(i) => {
                // ⚠ `if __name__ == "__main__":` は `convert_stmt` が**丸ごと捨てる**ので、
                //   その中の代入を巻き上げてはいけない。巻き上げると「代入されないのに
                //   `mut name = None` だけ残る」モジュール変数が生まれ、
                //   取り込み側と名前が衝突して `already declared` になる
                //   （`test_modules/py_calculator.py` の `c = Calculator(...)` で実際に踏んだ）。
                if is_main_guard(&i.test) {
                    continue;
                }
                collect_assigned_names(&i.body, out, seen);
                collect_assigned_names(&i.orelse, out, seen);
            }
            py::Stmt::For(f) => {
                collect_assigned_names(&f.body, out, seen);
                collect_assigned_names(&f.orelse, out, seen);
            }
            py::Stmt::While(w) => {
                collect_assigned_names(&w.body, out, seen);
                collect_assigned_names(&w.orelse, out, seen);
            }
            py::Stmt::Try(t) => {
                collect_assigned_names(&t.body, out, seen);
                for h in &t.handlers {
                    let py::ExceptHandler::ExceptHandler(eh) = h;
                    collect_assigned_names(&eh.body, out, seen);
                }
                collect_assigned_names(&t.orelse, out, seen);
                collect_assigned_names(&t.finalbody, out, seen);
            }
            py::Stmt::With(w) => collect_assigned_names(&w.body, out, seen),
            // `def` / `class` の本体は別スコープ。降りない。
            _ => {}
        }
    }
}

/// **スコープ**（モジュール本体 / 関数本体）を変換する。
///
/// `params` にはパラメータ名を渡す（既に宣言済みなので巻き上げないが、代入は再代入になる）。
pub(crate) fn convert_scope(
    stmts: &[py::Stmt],
    filename: &str,
    params: &[String],
) -> Result<Vec<Stmt>, String> {
    let mut hoisted: Vec<String> = Vec::new();
    // `seen` はパラメータで初期化する ⇒ パラメータは `hoisted` に入らないが `declared` には入る。
    let mut declared: std::collections::HashSet<String> = params.iter().cloned().collect();
    collect_assigned_names(stmts, &mut hoisted, &mut declared);

    let mut result: Vec<Stmt> = hoisted
        .into_iter()
        .map(|name| Stmt::Mut(name, None, Expr::None))
        .collect();
    for stmt in stmts {
        if let Some(s) = convert_stmt(stmt, filename, &declared)? {
            result.push(s);
        }
    }
    Ok(result)
}

/// **同一スコープ内**の文リスト（`if` / `for` / `while` / `try` の本体）を変換する。
/// 新しいスコープではないので巻き上げはせず、`declared` をそのまま引き回す。
pub(crate) fn convert_stmts(
    stmts: &[py::Stmt],
    filename: &str,
    declared: &std::collections::HashSet<String>,
) -> Result<Vec<Stmt>, String> {
    let mut result = Vec::new();
    for stmt in stmts {
        if let Some(s) = convert_stmt(stmt, filename, declared)? {
            result.push(s);
        }
    }
    Ok(result)
}

/// 単純名前への代入を、宣言済みなら再代入（`Stmt::Assign`）、そうでなければ宣言（`Stmt::Mut`）にする。
///
/// `convert_scope` がスコープ内の全代入名を巻き上げるので、通常は**必ず**再代入側に落ちる。
/// `Mut` 側は「巻き上げ集合の収集が届かない形が将来出てきたとき」の保険で、
/// 少なくとも従来（＝全部 `Mut`）より悪くはならない。
fn assign_or_declare(
    name: String,
    value: Expr,
    filename: &str,
    declared: &std::collections::HashSet<String>,
) -> Stmt {
    if declared.contains(&name) {
        Stmt::Assign {
            name,
            value,
            span: make_span(filename),
            slot: Default::default(),
        }
    } else {
        Stmt::Mut(name, None, value)
    }
}

/// 単一の Python 文を tl の Stmt に変換する。
pub(crate) fn convert_stmt(
    stmt: &py::Stmt,
    filename: &str,
    declared: &std::collections::HashSet<String>,
) -> Result<Option<Stmt>, String> {
    match stmt {
        // ----- 関数定義 -----
        py::Stmt::FunctionDef(f) => {
            // モジュール直下の関数なので `in_class: false`（`@staticmethod` 等は明示エラー）。
            let dec = convert_decorators(
                &f.decorator_list,
                filename,
                &format!("function '{}'", f.name.as_str()),
                false,
            )?;
            let params = convert_params(&f.args, filename)?;
            let return_type = f.returns.as_deref().map(convert_annotation);
            // 関数本体は**新しいスコープ**。パラメータ名を宣言済みとして渡す。
            let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let body = convert_scope(&f.body, filename, &param_names)?;
            Ok(Some(Stmt::FnDef {
                name: f.name.to_string(),
                template_params: vec![],
                params,
                return_type,
                body,
                is_abstract: false,
                is_static: false,
                is_class_method: false,
                decorators: dec.decorators,
                access: crate::ast::Accessibility::Public,
            }))
        }

        // ----- 非同期関数定義（未サポート） -----
        py::Stmt::AsyncFunctionDef(f) => Err(format!(
            "{filename}: async def is not supported (function '{}')",
            f.name.as_str()
        )),

        // ----- クラス定義 -----
        // デコレータは `convert_class` 側で処理する（クラス名が必要なため）。
        py::Stmt::ClassDef(c) => convert_class(c, filename).map(Some),

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
                    let name = n.id.to_string();
                    let val = convert_expr(&a.value, filename)?;
                    Ok(Some(assign_or_declare(name, val, filename, declared)))
                }
                // 属性代入 `o.a = v` と添字代入 `a[i] = v` / `d[k] = v` は
                // どちらも Arrow では `Stmt::AttrAssign`（target に代入先の**式**を置く形）。
                // ⚠ `d["k"][0] = v` のような入れ子も、target が入れ子の `Expr::Subscript` に
                //   なるだけでそのまま通る。
                py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
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
                    let name = n.id.to_string();
                    let val = convert_expr(val_expr, filename)?;
                    Ok(Some(assign_or_declare(name, val, filename, declared)))
                } else {
                    // 値なしの `x: int` は Python でも束縛を作らないので何も出さない。
                    Ok(None)
                }
            }
            // `o.a: T = v` / `d[k]: T = v`（Python は注釈つき添字代入も許す）。注釈は捨てる。
            py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
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
                        node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
                    }))
                }
                // `o.a += v` と `a[i] += v` はどちらも `Stmt::AttrCompoundAssign`。
                py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
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
            let then_body = convert_stmts(&i.body, filename, declared)?;

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
                        let b = convert_stmts(&elif.body, filename, declared)?;
                        branches.push((c, b));
                        orelse = &elif.orelse;
                        continue;
                    }
                }
                else_body = Some(convert_stmts(orelse, filename, declared)?);
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
            let body = convert_stmts(&w.body, filename, declared)?;
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
            let body = convert_stmts(&f.body, filename, declared)?;
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
            let body = convert_stmts(&t.body, filename, declared)?;
            let mut handlers = Vec::new();
            for h in &t.handlers {
                let py::ExceptHandler::ExceptHandler(eh) = h;
                let exc_type = eh.type_.as_deref().map(|e| match e {
                    py::Expr::Name(n) => n.id.to_string(),
                    _ => "Exception".to_string(),
                });
                let name = eh.name.as_ref().map(|n| n.to_string());
                let hbody = convert_stmts(&eh.body, filename, declared)?;
                handlers.push(ExceptHandler {
                    exc_type,
                    name,
                    body: hbody,
                });
            }
            let finally_body = if t.finalbody.is_empty() {
                None
            } else {
                Some(convert_stmts(&t.finalbody, filename, declared)?)
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

