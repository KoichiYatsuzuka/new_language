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

// ---------------------------------------------------------------------------
// ジェネレータ判定（項目 9）
// ---------------------------------------------------------------------------

/// **このフレーム**（＝ある `def` の本体）に文としての `yield` があるか。
///
/// ⚠⚠ **入れ子の `def` / `class` には降りない**。Python では内側の `def` は
/// **別の関数**で、そこの `yield` は内側をジェネレータにするだけ。外側は
/// ジェネレータにならない。
/// ⇒ 各 `def` を「自分のフレームだけ」で判定すれば、入れ子のジェネレータは
/// 入れ子の `Stmt::GenDef` として自然に落ちる（Arrow も入れ子 `gen` に対応済み・B13 段階E）。
///
/// ⚠ 値位置の `yield`（`x = yield`）はここでは拾わない。`convert_expr` の
/// `py::Expr::Yield` アームが明示エラーにする。
pub(crate) fn frame_has_yield(stmts: &[py::Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        py::Stmt::Expr(e) => {
            matches!(&*e.value, py::Expr::Yield(_) | py::Expr::YieldFrom(_))
        }
        py::Stmt::If(i) => frame_has_yield(&i.body) || frame_has_yield(&i.orelse),
        py::Stmt::For(f) => frame_has_yield(&f.body) || frame_has_yield(&f.orelse),
        py::Stmt::While(w) => frame_has_yield(&w.body) || frame_has_yield(&w.orelse),
        py::Stmt::With(w) => frame_has_yield(&w.body),
        py::Stmt::Try(t) => {
            frame_has_yield(&t.body)
                || t.handlers.iter().any(|h| {
                    let py::ExceptHandler::ExceptHandler(eh) = h;
                    frame_has_yield(&eh.body)
                })
                || frame_has_yield(&t.orelse)
                || frame_has_yield(&t.finalbody)
        }
        // `def` / `class` は別フレーム。降りない。
        _ => false,
    })
}

/// 戻り値注釈から `yield` の型を取り出す（`Generator[T, ...]` / `Iterator[T]` / `Iterable[T]`）。
///
/// これら以外の注釈（`-> None` など）や注釈なしは `None`（型を付けない）。
/// ⚠ `Generator[Y, S, R]` は 3 引数だが Arrow が持てるのは産出型だけなので**先頭だけ**使う。
pub(crate) fn extract_yield_type(returns: Option<&py::Expr>) -> Option<String> {
    let py::Expr::Subscript(s) = returns? else {
        return None;
    };
    let base = match &*s.value {
        py::Expr::Name(n) => n.id.to_string(),
        py::Expr::Attribute(a) => a.attr.to_string(),
        _ => return None,
    };
    if !matches!(base.as_str(), "Generator" | "Iterator" | "Iterable") {
        return None;
    }
    // `Generator[int, None, None]` の先頭要素、`Iterator[int]` はそれ自体。
    let first = match &*s.slice {
        py::Expr::Tuple(t) => t.elts.first()?,
        other => other,
    };
    Some(convert_annotation(first))
}

/// 名前参照の `Expr::Ident` を作る短縮形。
pub(crate) fn ident_expr(name: &str) -> Expr {
    Expr::Ident {
        name: name.to_string(),
        node_id: 0,
        res: crate::ast::Resolution::Unresolved,
    }
}

/// タプル/リストのアンパック代入 `a, b = <value>` を**添字アクセスへ脱糖**する（U2）。
///
/// ```text
/// a, b = t        →  __py_tmp_N = t ; a = __py_tmp_N[0] ; b = __py_tmp_N[1]
/// a, *rest = t    →  __py_tmp_N = t ; a = __py_tmp_N[0] ; rest = __py_tmp_N[1:]
/// ```
///
/// ⚠⚠ **`Stmt::LetTuple` は使えない**。Arrow の `let a, let b = t` は**宣言**で、
/// しかも**宣言済みの名前への `a, b = t` という構文自体が存在しない**
/// （実測: `ParseError: unexpected token: ','`）。項目 2 のスコープ巻き上げが
/// 全代入名を先に宣言して以降を `Stmt::Assign` にする以上、変換器から `LetTuple` は出せない。
///
/// ⚠ 一時変数は**必須**。`a = t[0]; b = t[1]` と直接書くと右辺が 2 回評価される
/// （`a, b = f()` で `f` が 2 回走る）。
/// ⚠ `*rest` は**末尾だけ**。途中に置く形（`a, *m, b = t`）は末尾からの逆算が要るので拒否する。
fn unpack_assign(
    elts: &[py::Expr],
    value: Expr,
    filename: &str,
    declared: &std::collections::HashSet<String>,
) -> Result<Vec<Stmt>, String> {
    let tmp = next_temp_name("tmp");
    if !hoist_emit(Stmt::Mut(tmp.clone(), None, value)) {
        return Err(format!("{filename}: internal error: no hoist buffer"));
    }
    let mut out = Vec::new();
    for (i, elt) in elts.iter().enumerate() {
        let is_last = i + 1 == elts.len();
        match elt {
            py::Expr::Name(n) => {
                let idx = Expr::Subscript {
                    object: Box::new(ident_expr(&tmp)),
                    index: Box::new(Expr::Int(i as i64)),
                    node_id: 0,
                };
                out.push(assign_or_declare(n.id.to_string(), idx, filename, declared));
            }
            // `*rest` は残り全部。末尾のときだけスライス（項目 4）で受ける。
            py::Expr::Starred(s) if is_last => {
                let py::Expr::Name(n) = &*s.value else {
                    return Err(format!(
                        "{filename}: only a simple name is supported after `*` in unpacking"
                    ));
                };
                let rest = Expr::Subscript {
                    object: Box::new(ident_expr(&tmp)),
                    index: Box::new(Expr::Slice {
                        begin: Some(Box::new(Expr::Int(i as i64))),
                        end: None,
                        step: None,
                    }),
                    node_id: 0,
                };
                out.push(assign_or_declare(n.id.to_string(), rest, filename, declared));
            }
            py::Expr::Starred(_) => {
                return Err(format!(
                    "{filename}: `*rest` must be the last target in unpacking"
                ))
            }
            py::Expr::Tuple(_) | py::Expr::List(_) => {
                return Err(format!(
                    "{filename}: nested unpacking (`a, (b, c) = ...`) is not supported"
                ))
            }
            // `o.a, d[k] = t` のような属性/添字ターゲット。
            py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
                let idx = Expr::Subscript {
                    object: Box::new(ident_expr(&tmp)),
                    index: Box::new(Expr::Int(i as i64)),
                    node_id: 0,
                };
                out.push(Stmt::AttrAssign {
                    target: convert_expr(elt, filename)?,
                    value: idx,
                });
            }
            _ => return Err(format!("{filename}: unsupported unpacking target")),
        }
    }
    Ok(out)
}

/// `for ... else:` / `while ... else:` の明示エラー文言（2 箇所で同じものを出す）。
///
/// ⚠ Python のループ `else` は「**`break` せずに回り切ったときだけ**走る」節。
/// 「ループが終わったら走る」ではないので、黙って本体の後ろに繋ぐ変換もできない。
fn loop_else_error(filename: &str, kw: &str) -> String {
    format!(
        "{filename}: `{kw} ... else:` is not supported (Arrow has no loop `else`); \
         the `else` body runs only when the loop finishes without `break` — \
         use a flag variable instead"
    )
}

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
        convert_one_into(stmt, filename, &declared, &mut result)?;
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
        convert_one_into(stmt, filename, declared, &mut result)?;
    }
    Ok(result)
}

/// 文を 1 つ変換し、**式から持ち上がった補助文を前に置いて**から `out` へ並べる（INF-B / INF-C）。
///
/// ⚠⚠ 補助文は**必ず本体より前**。順序を逆にすると walrus も一時変数も未定義参照になる。
/// ⚠ バッファは文ごとに積む。`if` の本体などは内側の `convert_stmts` が自分のぶんを
///   積むので、外側の文の補助文と混ざらない。
fn convert_one_into(
    stmt: &py::Stmt,
    filename: &str,
    declared: &std::collections::HashSet<String>,
    out: &mut Vec<Stmt>,
) -> Result<(), String> {
    hoist_push();
    // ⚠ `?` で早期 return するとバッファが残るが、変換エラーはモジュールごと中断するので
    //   実害は無い（`convert_python_source` の入口の `reset_hoist()` が消す）。
    let converted = convert_stmt(stmt, filename, declared)?;
    let aux = hoist_pop();
    out.extend(aux);
    out.extend(converted);
    Ok(())
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
) -> Result<Vec<Stmt>, String> {
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
            let (params, renames) = convert_params(&f.args, filename)?;
            let return_type = f.returns.as_deref().map(convert_annotation);
            // 関数本体は**新しいスコープ**。パラメータ名を宣言済みとして渡す。
            let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let body = {
                // `*args` / `**kwargs` の識別子差し替えは**この本体の変換中だけ**有効。
                let _rename_guard = ParamRenameGuard::push(renames, &param_names);
                convert_scope(&f.body, filename, &param_names)?
            };
            // ★ 本体に文としての `yield` があればジェネレータ（項目 9）。
            //   判定は**自分のフレームだけ**なので、入れ子の `def` が持つ `yield` は
            //   内側を `GenDef` にするだけで外側には影響しない（Python と同じ規則）。
            if frame_has_yield(&f.body) {
                // ⚠ `Stmt::GenDef` は `decorators` を持たない（Arrow の `gen` に
                //   デコレータ構文が無い）。捨てると黙って効かなくなるので明示エラー。
                if !dec.decorators.is_empty() {
                    return Err(format!(
                        "{filename}: a decorator on a generator function is not supported \
                         (function '{}'); Arrow's `gen` has no decorator form",
                        f.name.as_str()
                    ));
                }
                return Ok(vec![Stmt::GenDef {
                    name: f.name.to_string(),
                    template_params: vec![],
                    params,
                    yield_type: extract_yield_type(f.returns.as_deref()),
                    body,
                    access: crate::ast::Accessibility::Public,
                }]);
            }
            Ok(vec![Stmt::FnDef {
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
            }])
        }

        // ----- 非同期関数定義（未サポート） -----
        py::Stmt::AsyncFunctionDef(f) => Err(format!(
            "{filename}: async def is not supported (function '{}')",
            f.name.as_str()
        )),

        // ----- クラス定義 -----
        // デコレータは `convert_class` 側で処理する（クラス名が必要なため）。
        py::Stmt::ClassDef(c) => convert_class(c, filename).map(|s| vec![s]),

        // ----- return -----
        py::Stmt::Return(r) => {
            let expr = r
                .value
                .as_deref()
                .map(|e| convert_expr(e, filename))
                .transpose()?;
            Ok(vec![Stmt::Return(expr)])
        }

        // ----- 代入: `x = expr` / `a = b = expr`（項目 15）/ `a, b = t`（U2） -----
        py::Stmt::Assign(a) => {
            let val = convert_expr(&a.value, filename)?;
            // ★ **複数ターゲット**（`a = b = c`・項目 15）。
            //   ⚠ `a = c; b = c` と 2 回書くと**右辺が 2 回評価される**（`a = b = f()` で
            //     `f` が 2 回走り、`a = b = []` は別オブジェクトになる）。
            //     一時変数へ 1 回だけ退避してから配る。
            let (source, mut out) = if a.targets.len() > 1 {
                let tmp = next_temp_name("tmp");
                if !hoist_emit(Stmt::Mut(tmp.clone(), None, val)) {
                    return Err(format!("{filename}: internal error: no hoist buffer"));
                }
                (ident_expr(&tmp), Vec::new())
            } else {
                (val, Vec::new())
            };
            for target in &a.targets {
                // ⚠ ターゲットが複数のときは毎回 `source`（一時変数への参照）を複製する。
                let value = source.clone();
                match target {
                    py::Expr::Name(n) => {
                        out.push(assign_or_declare(n.id.to_string(), value, filename, declared));
                    }
                    // 属性代入 `o.a = v` と添字代入 `a[i] = v` / `d[k] = v` は
                    // どちらも Arrow では `Stmt::AttrAssign`（target に代入先の**式**を置く形）。
                    // ⚠ `d["k"][0] = v` のような入れ子も、target が入れ子の `Expr::Subscript` に
                    //   なるだけでそのまま通る。
                    py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
                        out.push(Stmt::AttrAssign {
                            target: convert_expr(target, filename)?,
                            value,
                        });
                    }
                    // ★ **タプル/リストのアンパック**（`a, b = t` / `a, *rest = t`・U2）。
                    py::Expr::Tuple(_) | py::Expr::List(_) => {
                        let elts = match target {
                            py::Expr::Tuple(t) => &t.elts,
                            py::Expr::List(l) => &l.elts,
                            _ => unreachable!("直前の match で絞り込み済み"),
                        };
                        out.extend(unpack_assign(elts, value, filename, declared)?);
                    }
                    _ => return Err(format!("{filename}: unsupported assignment target")),
                }
            }
            Ok(out)
        }

        // ----- 型アノテーション付き代入: `x: int = 5` -----
        py::Stmt::AnnAssign(a) => match &*a.target {
            py::Expr::Name(n) => {
                if let Some(val_expr) = &a.value {
                    let name = n.id.to_string();
                    let val = convert_expr(val_expr, filename)?;
                    Ok(vec![assign_or_declare(name, val, filename, declared)])
                } else {
                    // 値なしの `x: int` は Python でも束縛を作らないので何も出さない。
                    Ok(vec![])
                }
            }
            // `o.a: T = v` / `d[k]: T = v`（Python は注釈つき添字代入も許す）。注釈は捨てる。
            py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
                if let Some(val_expr) = &a.value {
                    let target_expr = convert_expr(&a.target, filename)?;
                    let val = convert_expr(val_expr, filename)?;
                    Ok(vec![Stmt::AttrAssign {
                        target: target_expr,
                        value: val,
                    }])
                } else {
                    Ok(vec![])
                }
            }
            _ => Ok(vec![]),
        },

        // ----- 拡張代入: `x += expr` -----
        py::Stmt::AugAssign(a) => {
            let op = convert_augop(&a.op, filename)?;
            match &*a.target {
                py::Expr::Name(n) => {
                    let val = convert_expr(&a.value, filename)?;
                    let span = make_span(filename);
                    Ok(vec![Stmt::CompoundAssign {
                        name: n.id.to_string(),
                        op,
                        value: val,
                        span,
                        slot: Default::default(),
                        node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
                    }])
                }
                // `o.a += v` と `a[i] += v` はどちらも `Stmt::AttrCompoundAssign`。
                py::Expr::Attribute(_) | py::Expr::Subscript(_) => {
                    let target_expr = convert_expr(&a.target, filename)?;
                    let val = convert_expr(&a.value, filename)?;
                    Ok(vec![Stmt::AttrCompoundAssign {
                        target: target_expr,
                        op,
                        value: val,
                    }])
                }
                _ => Err(format!(
                    "{filename}: unsupported augmented assignment target"
                )),
            }
        }

        // ----- if 文（ホイストなし版: convert_stmt_in_hoist_ctx 経由でホイストあり版を使う） -----
        py::Stmt::If(i) => {
            if is_main_guard(&i.test) {
                return Ok(vec![]);
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

            Ok(vec![Stmt::If {
                branches,
                else_body,
            }])
        }

        // ----- while -----
        py::Stmt::While(w) => {
            // ⚠ `while ... else:` は Arrow に相当構文が無い。**黙って捨てると
            //   「break しなかったときだけ走る」節が消えて意味が変わる**ので明示エラー。
            if !w.orelse.is_empty() {
                return Err(loop_else_error(filename, "while"));
            }
            let cond = convert_expr(&w.test, filename)?;
            let body = convert_stmts(&w.body, filename, declared)?;
            Ok(vec![Stmt::While { cond, body }])
        }

        // ----- for -----
        py::Stmt::For(f) => {
            // ⚠ `for ... else:` も同上（`while` と同じ意味論）。
            if !f.orelse.is_empty() {
                return Err(loop_else_error(filename, "for"));
            }
            // ループ変数。Arrow の `Stmt::For.targets` は元から `Vec<String>` で
            // **多ターゲットに対応済み**なので、単純名を並べるだけで通る（U1）。
            //   `for k, v in pairs:` / `for i, v in enumerate(xs):` /
            //   `for k, v in d.items():` がそのまま動く。
            // ⚠ 入れ子タプル（`for a, (b, c) in ...`）と `*rest` は Arrow 側に受け皿が
            //   無いので明示エラーのまま。
            let targets: Vec<String> = match &*f.target {
                py::Expr::Name(n) => vec![n.id.to_string()],
                py::Expr::Tuple(t) => {
                    let mut names = Vec::with_capacity(t.elts.len());
                    for elt in &t.elts {
                        match elt {
                            py::Expr::Name(n) => names.push(n.id.to_string()),
                            py::Expr::Starred(_) => {
                                return Err(format!(
                                    "{filename}: `*rest` in a for-loop target is not supported"
                                ))
                            }
                            _ => {
                                return Err(format!(
                                    "{filename}: only simple names are supported in a for-loop \
                                     target (nested unpacking is not)"
                                ))
                            }
                        }
                    }
                    names
                }
                _ => {
                    return Err(format!(
                        "{filename}: unsupported for-loop target"
                    ))
                }
            };
            let iter = convert_expr(&f.iter, filename)?;
            let body = convert_stmts(&f.body, filename, declared)?;
            Ok(vec![Stmt::For {
                targets,
                iter,
                body,
            }])
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
            // ⚠ `try ... else:` は「例外が起きなかったときだけ走る」節。Arrow に無く、
            //   黙って捨てると本体が丸ごと消える（`finally` とも違う）ので明示エラー。
            if !t.orelse.is_empty() {
                return Err(format!(
                    "{filename}: `try ... else:` is not supported \
                     (Arrow has no `else` clause on `try`); move the `else` body to the end \
                     of the `try` body"
                ));
            }
            let body = convert_stmts(&t.body, filename, declared)?;
            let mut handlers = Vec::new();
            for h in &t.handlers {
                let py::ExceptHandler::ExceptHandler(eh) = h;
                // ⚠⚠ **以前は単純名以外を黙って `"Exception"` に潰していた**。
                //   `except (A, B):` は Tuple、`except mod.Err:` は Attribute で来るので、
                //   どちらも「何でも捕まえるハンドラ」に化けていた（＝捕まえすぎる誤変換）。
                //   型を捨てる変換は危険なので明示エラーにする。
                let exc_type = match eh.type_.as_deref() {
                    None => None,
                    Some(py::Expr::Name(n)) => Some(n.id.to_string()),
                    Some(py::Expr::Tuple(_)) => {
                        return Err(format!(
                            "{filename}: `except (A, B):` (multiple exception types) is not supported; \
                             write one `except` clause per type"
                        ))
                    }
                    Some(_) => {
                        return Err(format!(
                            "{filename}: only a simple exception name is supported in `except` \
                             (e.g. `except ValueError:`); qualified or computed types are not"
                        ))
                    }
                };
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
            Ok(vec![Stmt::Try {
                body,
                handlers,
                finally_body,
            }])
        }

        // ----- raise -----
        py::Stmt::Raise(r) => {
            // ⚠ `raise X from Y` の `from Y`（例外連鎖）は Arrow に無い。黙って捨てると
            //   原因情報が消えるので明示エラー。
            if r.cause.is_some() {
                return Err(format!(
                    "{filename}: `raise ... from ...` (exception chaining) is not supported; \
                     Arrow has no `__cause__`"
                ));
            }
            let exc = r
                .exc
                .as_deref()
                .map(|e| convert_expr(e, filename))
                .transpose()?;
            let span = make_span(filename);
            Ok(vec![Stmt::Raise { exc, span }])
        }

        // ----- assert（未サポート・専用の文言） -----
        // ⚠ 以前は末尾の catch-all で `unsupported Python statement` になっていた。
        //   どの構文で落ちたのか判らないので専用アームにする。
        py::Stmt::Assert(_) => Err(format!(
            "{filename}: `assert` is not supported; \
             use `if not <cond>: raise ...` instead"
        )),

        // ----- del -----
        //
        // ⚠ **ターゲットの種類で扱いを分ける**（coverage 項目 14 の決定）:
        //   - `del x`（名前）… 束縛の削除。Arrow はスコープ退出で破棄するので
        //     **警告して無視**する。残る差は「削除後に読むと Python は `NameError`」だけ。
        //   - `del d[k]` / `del o.a` … **意味のある削除**。無視すると結果が変わるので明示エラー。
        py::Stmt::Delete(d) => {
            for target in &d.targets {
                match target {
                    py::Expr::Name(n) => {
                        eprintln!(
                            "Warning: {filename}: `del {}` is ignored \
                             (Arrow drops bindings at scope exit)",
                            n.id.as_str()
                        );
                    }
                    py::Expr::Subscript(_) => {
                        return Err(format!(
                            "{filename}: `del <expr>[<key>]` is not supported \
                             (it would silently do nothing); remove the entry another way"
                        ))
                    }
                    py::Expr::Attribute(a) => {
                        return Err(format!(
                            "{filename}: `del <expr>.{}` is not supported \
                             (Arrow cannot remove an attribute)",
                            a.attr.as_str()
                        ))
                    }
                    _ => return Err(format!("{filename}: unsupported `del` target")),
                }
            }
            Ok(vec![])
        }

        // ----- pass -----
        py::Stmt::Pass(_) => Ok(vec![Stmt::Pass]),

        // ----- break / continue -----
        py::Stmt::Break(_) => Ok(vec![Stmt::Break]),
        py::Stmt::Continue(_) => Ok(vec![Stmt::Continue]),

        // ----- 式文 -----
        py::Stmt::Expr(e) => {
            // 文としての `...`（`def f(): ...` などスタブ本体のプレースホルダ）は
            // 「何もしない」以上の意味を持たないので `pass` に読み替える。
            //
            // ⚠ **値位置の `...`（`x = ...` / `a[...]`）は対象外**。そちらは
            //   `convert_constant` が `Expr::None` にする（承認済みの仕様）。
            //   ⇒ CPython は `Ellipsis` オブジェクトを返すので**そこだけ表示が違う**が、
            //     Arrow に `Ellipsis` 値が無く、副作用も無いため許容している。
            if matches!(&*e.value, py::Expr::Constant(c) if matches!(c.value, py::Constant::Ellipsis))
            {
                return Ok(vec![Stmt::Pass]);
            }
            // ★ 文位置の `yield x` → `Stmt::Yield`（項目 9）。
            //   ⚠ ここを通るのは `frame_has_yield` が真になった `def` の本体だけ
            //     （そうでなければ `Stmt::GenDef` にならないので、型検査の
            //     `YieldOutsideGenerator` が拾う）。
            if let py::Expr::Yield(y) = &*e.value {
                // `yield`（値なし）は `yield None` 相当。
                let val = match y.value.as_deref() {
                    Some(v) => convert_expr(v, filename)?,
                    None => Expr::None,
                };
                return Ok(vec![Stmt::Yield(val)]);
            }
            // ⚠ `yield from xs` は「内側のジェネレータへ委譲する」構文で、Arrow に
            //   相当するものが無い。`for v in xs: yield v` へ機械的に書き換えると
            //   `.send()` / 戻り値の意味が変わるので、黙って変えず明示エラーにする。
            if matches!(&*e.value, py::Expr::YieldFrom(_)) {
                return Err(format!(
                    "{filename}: `yield from` is not supported; \
                     write `for v in <iterable>: yield v` instead"
                ));
            }
            let expr = convert_expr(&e.value, filename)?;
            Ok(vec![Stmt::Expr(expr)])
        }

        // ----- global / nonlocal -----
        //
        // ⚠⚠ **以前は黙って無視していた**（`Ok(vec![])`）。その結果、内側の関数からの
        //   代入が**外側に届かないまま静かに動く**（＝内側のローカルを書き換えるだけ）
        //   という誤変換になっていた。Arrow は外側の変数を `mut` で宣言すれば内側から
        //   書き換えられるので、代替手段はある。
        py::Stmt::Global(g) => Err(format!(
            "{filename}: `global {}` is not supported; \
             declare the outer variable with `mut` instead (Arrow has no `global`)",
            g.names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
        )),
        py::Stmt::Nonlocal(n) => Err(format!(
            "{filename}: `nonlocal {}` is not supported; \
             declare the outer variable with `mut` instead (Arrow has no `nonlocal`)",
            n.names.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")
        )),

        // ----- import / from-import（モジュール本体内の import は無視） -----
        py::Stmt::Import(_) | py::Stmt::ImportFrom(_) => Ok(vec![]),

        // ----- match（値 / ワイルドカードのサブセット・項目 8） -----
        //
        // Arrow の `match` は **値の等価比較**（`MatchPattern::Case`）か
        // **型検査**（`IsType`）のどちらか一方しか持たず、構造的パターンマッチングは無い。
        // ⇒ Python の `case <リテラル/値>:` と `case _:` だけを写し、
        //   構造を分解するパターンは**明示エラー**にする（黙って一部だけ合わせない）。
        //
        // ⚠ 意味論は一致する（実機確認済み）: アームは**フォールスルーしない**、
        //   どれにも当たらなければ何もしない、`_` は常にマッチ。
        py::Stmt::Match(m) => {
            use crate::ast::{MatchArm, MatchPattern};
            let subject = convert_expr(&m.subject, filename)?;
            let mut arms: Vec<MatchArm> = Vec::new();
            for case in &m.cases {
                // ⚠ ガード（`case 1 if cond:`）は Arrow のアームに置き場が無い。
                //   落とすと条件を無視して当たってしまうので明示エラー。
                if case.guard.is_some() {
                    return Err(format!(
                        "{filename}: a guard in `case ... if ...:` is not supported; \
                         use an `if` inside the case body instead"
                    ));
                }
                let pattern = match &case.pattern {
                    // `case 1:` / `case "a":` / `case CONST:`
                    py::Pattern::MatchValue(v) => {
                        MatchPattern::Case(convert_expr(&v.value, filename)?)
                    }
                    // `case None:` / `case True:` / `case False:`
                    py::Pattern::MatchSingleton(s) => {
                        MatchPattern::Case(constant_value_to_expr(&s.value, filename)?)
                    }
                    // `case _:`（`pattern` も `name` も無い `MatchAs` がワイルドカード）
                    py::Pattern::MatchAs(a) if a.pattern.is_none() && a.name.is_none() => {
                        MatchPattern::Case(Expr::Ident {
                            name: "_".to_string(),
                            node_id: 0,
                            res: crate::ast::Resolution::Unresolved,
                        })
                    }
                    // `case x:`（キャプチャ）は「常にマッチして名前に束縛する」形。
                    // Arrow に束縛付きアームが無く、`_` に潰すと**束縛が黙って消える**。
                    py::Pattern::MatchAs(_) => {
                        return Err(format!(
                            "{filename}: a capture pattern (`case <name>:`) is not supported; \
                             only literal values and `case _:` are"
                        ))
                    }
                    py::Pattern::MatchOr(_) => {
                        return Err(format!(
                            "{filename}: an or-pattern (`case A | B:`) is not supported; \
                             write one `case` per value"
                        ))
                    }
                    py::Pattern::MatchSequence(_) | py::Pattern::MatchStar(_) => {
                        return Err(format!(
                            "{filename}: a sequence pattern (`case [a, b]:`) is not supported \
                             (Arrow's `match` compares values, it does not destructure)"
                        ))
                    }
                    py::Pattern::MatchMapping(_) => {
                        return Err(format!(
                            "{filename}: a mapping pattern (`case {{...}}:`) is not supported \
                             (Arrow's `match` compares values, it does not destructure)"
                        ))
                    }
                    py::Pattern::MatchClass(_) => {
                        return Err(format!(
                            "{filename}: a class pattern (`case C(...):`) is not supported \
                             (Arrow's `match` compares values, it does not destructure)"
                        ))
                    }
                };
                arms.push(MatchArm {
                    pattern,
                    body: convert_stmts(&case.body, filename, declared)?,
                });
            }
            Ok(vec![Stmt::Match {
                subject,
                arms,
                span: make_span(filename),
            }])
        }

        // ----- type alias（Python 3.12+） -----
        // ----- 型エイリアス `type X = T`（Python 3.12+・項目 10） -----
        //
        // ⚠⚠ **Arrow の `alias` は AST に出せない**（パース時に `parser.aliases` へ
        //   登録され `Stmt::Pass` になる構文）。`new_type` は名目的別型なので
        //   `type V = list[int]` を `list[int]` として使えなくなり不適。
        //   ⇒ **変換器が自前の表を持ち、型注釈の解決時に透過展開する**。
        // ⚠ 文としては何も出さない（Arrow 側に対応する宣言が無い）。
        // ⚠ 型パラメータ付き（`type L[T] = list[T]`）は展開先に型変数が残るので拒否する。
        py::Stmt::TypeAlias(t) => {
            if !t.type_params.is_empty() {
                return Err(format!(
                    "{filename}: a parameterized type alias (`type X[T] = ...`) is not supported"
                ));
            }
            let py::Expr::Name(n) = &*t.name else {
                return Err(format!("{filename}: unsupported type alias target"));
            };
            // 右辺は**登録前に**解決する（連鎖は効き、自己参照は無限再帰にならない）。
            register_type_alias(n.id.to_string(), convert_annotation(&t.value));
            Ok(vec![])
        }

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python statement")),
    }
}

