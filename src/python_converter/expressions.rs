// python_converter/expressions.rs — 式・定数・演算子の変換: convert_expr / convert_constant / convert_binop / convert_augop / convert_cmpop。

use crate::ast::Resolution;
use std::rc::Rc;
use {
    rustpython_parser::ast as py,
    crate::ast::{BinOp, CallArg, Expr, Stmt, UnaryOp},
    crate::token::Span,
};
use super::*;

// ---------------------------------------------------------------------------
// 式変換
// ---------------------------------------------------------------------------

/// 比較 1 つ分（`left <op> right`）を Arrow の式にする。
///
/// ★ `is` / `is not` は `convert_cmpop` に置けないのでここで組む。
///   - Python の `is` は**識別比較**なので Arrow の `===`（`BinOp::RefEq`）に対応する。
///     ⚠⚠ Arrow にも `is` キーワードがあるが**型ガード**（`x is int`）で**別物**。
///   - Arrow に `!==` が無いので `is not` は `Not(RefEq)` でラップする
///     （`convert_cmpop` は `BinOp` しか返せない）。
fn build_comparison(
    left: Expr,
    op: &py::CmpOp,
    right: Expr,
    filename: &str,
    span: Span,
) -> Result<Expr, String> {
    if matches!(op, py::CmpOp::Is | py::CmpOp::IsNot) {
        let eq = Expr::BinOp {
            op: BinOp::RefEq,
            left: Box::new(left),
            right: Box::new(right),
            span,
            node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
        };
        return Ok(if matches!(op, py::CmpOp::IsNot) {
            Expr::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(eq),
            }
        } else {
            eq
        });
    }
    Ok(Expr::BinOp {
        op: convert_cmpop(op, filename)?,
        left: Box::new(left),
        right: Box::new(right),
        span,
        node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
    })
}

/// 式を**読むだけで副作用が無く、途中の呼び出しで値も変わらない**か（D6）。
///
/// 連鎖比較の中間オペランドを「2 回読んでよいか」の判定に使う。
///
/// ⚠ 真を返してよいのは **`Name` と定数**だけ。`obj.f` は Python では property が
/// 走りうるし、`d[k]` は `__getitem__`、`a + b` は `__add__` が走る。
///
/// ⭐ **名前が「途中で変わらない」と言い切れる**のは、変換器が `global` / `nonlocal` を
/// 明示エラーにしているから（群5）。関数呼び出しが呼び出し元のローカル名を
/// 書き換える経路が無いので、`a < f() < b` の `a` を後ろで読んでも同じ値になる。
/// ⇒ 順序を保つために純粋なオペランドまで一時変数へ退避する必要が無い。
fn is_side_effect_free(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Ident { .. }
            | Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::None
    )
}

/// 連鎖比較を**早期脱出のブロック式**へ落とす（D6）。
///
/// ```text
/// a < f() < b < g()
///   ↓
/// block->bool:
///     mut __py_cmp_0 = f()          # 中間オペランドは 1 回だけ評価
///     if not (a < __py_cmp_0):
///         block_return False        # ← ここで抜けるので b / g() は評価されない
///     mut __py_cmp_1 = b
///     if not (__py_cmp_0 < __py_cmp_1):
///         block_return False
///     block_return __py_cmp_1 < g()
/// ```
///
/// ⚠⚠ **ブロック式はその場に埋め込む**（文へ持ち上げない）。持ち上げると
/// `while 0 < f() < 10:` が 1 回しか評価されない・`p and (0 < f() < 10)` で
/// `p` が偽でも `f()` が走る、という形で**黙って評価回数が変わる**。
/// `Expr::Block` は VM コンパイラの一般の式ディスパッチが扱うので、
/// 条件式の中でも内包表記の中でも、そのまま置ける。
///
/// ⚠ 一時変数を作るのは**中間オペランドのうち副作用を持つものだけ**。先頭・末尾は
/// 1 回しか読まれないのでその場で評価すればよく、素の名前・定数は
/// [`is_side_effect_free`] の理由で退避が要らない。
///   ⇒ 余計な束縛を作らないので、**識別子の同一性（`is`）も動かない**。
fn chained_compare_block(
    operands: Vec<Expr>,
    ops: &[py::CmpOp],
    filename: &str,
) -> Result<Expr, String> {
    let span = make_span(filename);
    let last = operands.len() - 1;

    // 各オペランドを「ブロックの中でどう参照するか」。
    let mut refs: Vec<Option<Expr>> = vec![None; operands.len()];
    let mut stmts: Vec<Stmt> = Vec::new();

    let mut operands = operands;
    for (i, op) in ops.iter().enumerate() {
        // i 番目の比較に要るオペランドを、まだ用意していなければここで評価する。
        // ⚠ `i + 1` を**ループの中で**評価するのが短絡の肝（前の比較が偽なら来ない）。
        materialize(i, last, &mut operands, &mut refs, &mut stmts);
        materialize(i + 1, last, &mut operands, &mut refs, &mut stmts);

        let left = refs[i].clone().expect("materialize 済み");
        let right = refs[i + 1].clone().expect("materialize 済み");
        let pair = build_comparison(left, op, right, filename, span.clone())?;

        if i + 1 == ops.len() {
            // 最後の比較がそのまま答え。
            stmts.push(Stmt::BlockReturn(pair, span.clone()));
        } else {
            // ★ 1 つでも偽なら**即座に False**（CPython の連鎖比較と同じ）。
            stmts.push(Stmt::If {
                branches: vec![(
                    Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(pair) },
                    vec![Stmt::BlockReturn(Expr::Bool(false), span.clone())],
                )],
                else_body: None,
            });
        }
    }

    Ok(Expr::Block { stmts, return_type: Some("bool".to_string()) })
}

/// オペランド `k` を必要なら一時変数へ退避して、ブロック内での参照式を確定する
/// （[`chained_compare_block`] の補助）。
///
/// ⚠ **呼ぶ順序がそのまま評価順序**になる（Python と同じ左→右）。
/// ⚠ 退避するのは「2 回読まれる ＝ 中間」かつ「読むと副作用がある」ものだけ。
fn materialize(
    k: usize,
    last: usize,
    operands: &mut [Expr],
    refs: &mut [Option<Expr>],
    stmts: &mut Vec<Stmt>,
) {
    if refs[k].is_some() {
        return;
    }
    let e = std::mem::replace(&mut operands[k], Expr::None);
    // 末尾は「使う直前に評価する」位置そのものなので、その場で使ってよい。
    // 副作用の無い式も退避が要らない（[`is_side_effect_free`] の理由）。
    //
    // ⚠⚠ **先頭を「1 回しか読まないから」で素通しすると評価順序が壊れる**（実測）。
    //   `v("a") < v("b") < v("c")` で先頭を式のまま置くと、先に積まれる
    //   `mut __py_cmp = v("b")` が**先に走って** `b a c` の順になる。
    //   ⇒ 先頭も副作用があるなら退避する（この経路には必ず temp が 1 つ以上できるので、
    //     「退避しなければ順序が保たれる」状況は無い）。
    if k == last || is_side_effect_free(&e) {
        refs[k] = Some(e);
        return;
    }
    let tmp = next_temp_name("cmp");
    stmts.push(Stmt::Mut(tmp.clone(), None, e));
    refs[k] = Some(ident_expr(&tmp));
}


/// 内包表記の `for ... in ... if ...` 節を [`crate::ast::ComprehensionClause`] に変換する。
///
/// ⚠ タプル展開（`for k, v in d.items()`）は未対応。`Stmt::For` 側と同じ制限なので同じ形で拒否する。
/// ⚠ `async for` は Arrow の非同期モデル（`mng <- async->T:`）と別体系なので明示エラー。
fn convert_comprehension_clauses(
    generators: &[py::Comprehension],
    filename: &str,
    kind: &str,
) -> Result<Vec<crate::ast::ComprehensionClause>, String> {
    let mut clauses = Vec::new();
    for gen in generators {
        if gen.is_async {
            return Err(format!(
                "{filename}: `async for` in a {kind} comprehension is not supported"
            ));
        }
        // ★ `[k for k, v in d.items()]` — 多ターゲットも写せる（Arrow 側の
        //   `ComprehensionClause.targets` / `ForExpr.targets` を `Vec<String>` にした）。
        let targets: Vec<String> = match &gen.target {
            py::Expr::Name(n) => vec![n.id.to_string()],
            py::Expr::Tuple(t) => {
                let mut names = Vec::with_capacity(t.elts.len());
                for elt in &t.elts {
                    match elt {
                        py::Expr::Name(n) => names.push(n.id.to_string()),
                        _ => {
                            return Err(format!(
                                "{filename}: only simple names are supported in a {kind}                                  comprehension target (nested unpacking is not)"
                            ))
                        }
                    }
                }
                names
            }
            _ => {
                return Err(format!(
                    "{filename}: unsupported {kind} comprehension target"
                ))
            }
        };
        // ⚠ 2 本目以降の `for` の iter と、すべてのフィルタは**要素ごとに評価**される。
        //   持ち上げ禁止の位置（項目 23）。
        let iter = {
            let _unsafe_guard = (!clauses.is_empty()).then(UnsafeHoistGuard::enter);
            convert_expr(&gen.iter, filename)?
        };
        let ifs: Result<Vec<Expr>, String> = {
            let _unsafe_guard = UnsafeHoistGuard::enter();
            gen.ifs.iter().map(|c| convert_expr(c, filename)).collect()
        };
        clauses.push(crate::ast::ComprehensionClause {
            targets,
            iter,
            ifs: ifs?,
        });
    }
    Ok(clauses)
}

/// Python の列要素（`e` / `*e`）を Arrow の [`SeqEntry`] へ写す（U3）。
///
/// ⚠⚠ **以前は連結へ脱糖していた**（`[0, *a, 9]` → `[0] + a + [9]`）。Arrow に
/// splat が無かったため。その形は**展開元がリストに限られる**（`list()` 組込みは
/// 意図的に非公開）という穴があり、タプルやセットを展開すると `TypeError` になった。
/// ⇒ **Arrow 側に `*other` を入れた**ので 1 対 1 で写せるようになり、
///   展開元の種類を選ばなくなった。
fn convert_seq_entries(
    elts: &[py::Expr],
    filename: &str,
) -> Result<Vec<crate::ast::SeqEntry>, String> {
    let mut out = Vec::with_capacity(elts.len());
    for e in elts {
        match e {
            py::Expr::Starred(st) => {
                out.push(crate::ast::SeqEntry::Spread(convert_expr(&st.value, filename)?))
            }
            other => out.push(crate::ast::SeqEntry::Item(convert_expr(other, filename)?)),
        }
    }
    Ok(out)
}

/// 式が引数なしの `super()` 呼び出しかどうかを判定する。
///
/// ⚠ Python 2 形式の `super(Cls, self)` は**対象外**（引数ありなので false を返し、
/// 通常の呼び出しとして扱われた結果 `super` が未定義でエラーになる）。
fn is_zero_arg_super(e: &py::Expr) -> bool {
    matches!(e, py::Expr::Call(c)
        if c.args.is_empty()
            && c.keywords.is_empty()
            && matches!(&*c.func, py::Expr::Name(n) if n.id.as_str() == "super"))
}

/// 単一の Python 式を tl の `Expr` に変換する。
pub(crate) fn convert_expr(expr: &py::Expr, filename: &str) -> Result<Expr, String> {
    match expr {
        py::Expr::Constant(c) => convert_constant(c, filename),

        // ⚠ `*args` / `**kwargs` の Python 名は Arrow 側の参照（`local::args` / `kwargs`）へ
        //   差し替える（`param_rewrite.rs`。関数本体の変換中だけ有効）。
        py::Expr::Name(n) => Ok(renamed_ident(n.id.as_str()).unwrap_or_else(|| Expr::Ident {
            name: n.id.to_string(),
            node_id: 0,
            res: Resolution::Unresolved,
        })),

        py::Expr::Attribute(a) => {
            let obj = convert_expr(&a.value, filename)?;
            Ok(Expr::Attr {
                object: Box::new(obj),
                attr: a.attr.to_string(),
                span: make_span(filename),
                cache: Default::default(),
                node_id: 0, // #16: 合成/変換コード（注釈対象外）
            })
        }

        py::Expr::BinOp(b) => {
            let op = convert_binop(&b.op, filename)?;
            let left = convert_expr(&b.left, filename)?;
            let right = convert_expr(&b.right, filename)?;
            let span = make_span(filename);
            Ok(Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
                node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
            })
        }

        py::Expr::UnaryOp(u) => {
            let op = match &u.op {
                py::UnaryOp::USub => UnaryOp::Neg,
                py::UnaryOp::Not => UnaryOp::Not,
                py::UnaryOp::Invert => UnaryOp::BitNot,
                py::UnaryOp::UAdd => {
                    return convert_expr(&u.operand, filename);
                }
            };
            let operand = convert_expr(&u.operand, filename)?;
            Ok(Expr::UnaryOp {
                op,
                operand: Box::new(operand),
            })
        }

        py::Expr::BoolOp(b) => {
            let op = match &b.op {
                py::BoolOp::And => BinOp::And,
                py::BoolOp::Or => BinOp::Or,
            };
            let mut values = b.values.iter();
            let first = convert_expr(values.next().unwrap(), filename)?;
            let mut result = first;
            // ⚠ 2 つ目以降は**短絡で評価されないことがある**。ここで補助文を持ち上げると
            //   評価回数が変わるので、walrus を禁じる位置として深さを上げる（項目 23）。
            let _unsafe_guard = UnsafeHoistGuard::enter();
            for val in values {
                let right = convert_expr(val, filename)?;
                let span = Span::unknown();
                result = Expr::BinOp {
                    op: op.clone(),
                    left: Box::new(result),
                    right: Box::new(right),
                    span,
                    node_id: 0, // #16: py-converter は未採番
                };
            }
            Ok(result)
        }

        // 比較。Python は**連鎖比較**（`a < b < c`）を書けるので、隣接ペアを連結する。
        py::Expr::Compare(c) => {
            if c.ops.len() != c.comparators.len() || c.ops.is_empty() {
                return Err(format!("{filename}: malformed comparison"));
            }
            // オペランドは `left, comparators[0], comparators[1], ...`。
            // ⚠ **1 回だけ変換して clone で使い回す**。同じ式を 2 回変換すると
            //   （将来ノード ID を振るようになったとき）別ノードになってしまう。
            let mut operands: Vec<Expr> = Vec::with_capacity(c.comparators.len() + 1);
            operands.push(convert_expr(&c.left, filename)?);
            for cmp in &c.comparators {
                operands.push(convert_expr(cmp, filename)?);
            }

            // ★ **中間オペランドに副作用があるか**（D6）。
            //   中間オペランド ＝ `operands[1 ..= len-2]` は 2 つの比較にまたがるので、
            //   素朴に `and` で連結すると**2 回評価**される。副作用があると
            //   CPython と食い違う（`0 < mid() < 100` で `mid()` が 2 回走る）。
            //   ⇒ そのときだけ**早期脱出のブロック式**へ落とす（`chained_compare_block`）。
            //   ⚠ **素の名前・定数しか無い連鎖（`0 < x < 10` など圧倒的多数）は従来どおり**。
            //     2 回読んでも観測できないので、命令を増やす理由が無い。
            let intermediates = &operands[1..operands.len().saturating_sub(1)];
            if intermediates.iter().any(|e| !is_side_effect_free(e)) {
                return chained_compare_block(operands, &c.ops, filename);
            }

            let span = make_span(filename);
            let mut result: Option<Expr> = None;
            for (i, op) in c.ops.iter().enumerate() {
                let pair = build_comparison(
                    operands[i].clone(),
                    op,
                    operands[i + 1].clone(),
                    filename,
                    span.clone(),
                )?;
                result = Some(match result {
                    // `a < b < c` → `(a < b) and (b < c)`。
                    // ⚠ Arrow の `and` も短絡するので、`a < b` が偽なら `c` 側は評価されない
                    //   （Python と同じ）。中間オペランドを 2 回読むが、ここへ来るのは
                    //   **読んでも副作用が無い形だけ**（上の分岐で振り分け済み）。
                    Some(acc) => Expr::BinOp {
                        op: BinOp::And,
                        left: Box::new(acc),
                        right: Box::new(pair),
                        span: span.clone(),
                        node_id: 0, // #16: py-converter は未採番
                    },
                    None => pair,
                });
            }
            Ok(result.expect("ops is non-empty"))
        }

        py::Expr::Call(c) => {
            // ★ `super().m(args)` の脱糖（`import[py]` 限定のクラス継承サポートの一部）。
            //    Arrow に `super` は無いので、変換時に `<第1基底>.m(self, args)` へ書き換える。
            //    受け側（クラス経由のアンバウンド呼び出し）は `classes/class_methods.rs` が
            //    `FnValue::is_python` 限定で許可している。
            if let py::Expr::Attribute(attr) = &*c.func {
                if is_zero_arg_super(&attr.value) {
                    let base = current_super_base().ok_or_else(|| {
                        format!(
                            "{filename}: `super()` is only supported inside a method of a class that has a base class"
                        )
                    })?;
                    let mut args: Vec<CallArg> = vec![CallArg::Positional(Expr::Ident {
                        name: "self".to_string(),
                        node_id: 0,
                        res: Resolution::Unresolved,
                    })];
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
                    let base_expr = Expr::Ident {
                        name: base,
                        node_id: 0,
                        res: Resolution::Unresolved,
                    };
                    return Ok(Expr::Call {
                        func: Box::new(Expr::Attr {
                            object: Box::new(base_expr),
                            attr: attr.attr.to_string(),
                            span: make_span(filename),
                            cache: Default::default(),
                            node_id: 0,
                        }),
                        args,
                        span: crate::token::Span::unknown(),
                        cache: Default::default(),
                        node_id: 0,
                    });
                }
            }
            // `super()` を「メソッドを呼ぶ」以外の形で使うのは未対応（明示エラー）。
            if is_zero_arg_super(expr) {
                return Err(format!(
                    "{filename}: bare `super()` is only supported as `super().method(...)`"
                ));
            }
            let func = convert_expr(&c.func, filename)?;
            let mut args: Vec<CallArg> = Vec::new();
            for arg in &c.args {
                // ★ `f(*xs)` — Arrow 側にも同じ構文を入れたので 1 対 1 で写せる。
                if let py::Expr::Starred(st) = arg {
                    args.push(CallArg::Spread(convert_expr(&st.value, filename)?));
                    continue;
                }
                args.push(CallArg::Positional(convert_expr(arg, filename)?));
            }
            for kw in &c.keywords {
                // ★ `f(**d)` — キーワード名が無い（`arg` が `None`）のが `**` の印。
                let Some(name) = kw.arg.as_ref().map(|a| a.to_string()) else {
                    args.push(CallArg::KwSpread(convert_expr(&kw.value, filename)?));
                    continue;
                };
                args.push(CallArg::Keyword {
                    name,
                    value: convert_expr(&kw.value, filename)?,
                });
            }
            Ok(Expr::Call {
                func: Box::new(func),
                args,
                span: crate::token::Span::unknown(),
                cache: Default::default(),
                node_id: 0, // #16: py-converter は未採番
            })
        }

        py::Expr::Subscript(s) => {
            let obj = convert_expr(&s.value, filename)?;
            let idx = convert_expr(&s.slice, filename)?;
            Ok(Expr::Subscript {
                object: Box::new(obj),
                index: Box::new(idx),
                node_id: 0, // #16: py-converter は未採番
            })
        }

        // リスト表示。`*a` の展開（U3）を含むときは**連結**へ脱糖する:
        //   `[0, *a, 9]` → `[0] + a + [9]`
        // ⚠ Arrow に splat が無いので連結で表す。B5 で `list` の `+` が入ったので成立する。
        py::Expr::List(l) => Ok(Expr::List(convert_seq_entries(&l.elts, filename)?)),

        // ⚠ タプル表示の `*` は**連結できない**（Arrow のタプルは `+` で繋げるが、
        //   展開元がリストだと型が混ざる）。`Starred` アームの明示エラーに任せる。
        py::Expr::Tuple(t) => Ok(Expr::Tuple(convert_seq_entries(&t.elts, filename)?)),

        // 辞書リテラル。`{**other}`（キーが `None`）は Arrow の `DictEntry::Spread` へ。
        // ⚠ Arrow 側にも同じ構文を入れたので、そのまま 1 対 1 で写せる。
        py::Expr::Dict(d) => {
            let mut entries: Vec<crate::ast::DictEntry> = Vec::new();
            for (k, v) in d.keys.iter().zip(d.values.iter()) {
                match k {
                    Some(k) => entries.push(crate::ast::DictEntry::Pair(
                        convert_expr(k, filename)?,
                        convert_expr(v, filename)?,
                    )),
                    None => entries
                        .push(crate::ast::DictEntry::Spread(convert_expr(v, filename)?)),
                }
            }
            Ok(Expr::Dict(entries))
        }

        // リスト内包表記 → `for` 式 + `loop_yield`。
        // ⚠ 脱糖は `ast::build_list_comprehension` に集約してある。**Arrow のネイティブ構文
        //   （`parse_comprehension_tail`）と同じ関数**を通すので、生成される AST は必ず同一。
        py::Expr::ListComp(lc) => {
            // ⚠ 要素式は**要素ごと**に評価され、ループ変数がここでだけ見える。
            //   持ち上げ禁止の位置にする（walrus・lambda ともに外へ出すと壊れる）。
            let elt = {
                let _unsafe_guard = UnsafeHoistGuard::enter();
                convert_expr(&lc.elt, filename)?
            };
            let clauses = convert_comprehension_clauses(&lc.generators, filename, "list")?;
            crate::ast::build_list_comprehension(elt, clauses)
                .ok_or_else(|| format!("{filename}: list comprehension needs at least one `for` clause"))
        }

        // 集合内包 → リスト内包の結果を `set(...)` に通す（Arrow の `set()` はリストを受け取れる）。
        py::Expr::SetComp(sc) => {
            // ⚠ リスト内包と同じ理由で持ち上げ禁止。
            let elt = {
                let _unsafe_guard = UnsafeHoistGuard::enter();
                convert_expr(&sc.elt, filename)?
            };
            let clauses = convert_comprehension_clauses(&sc.generators, filename, "set")?;
            let list_expr = crate::ast::build_list_comprehension(elt, clauses)
                .ok_or_else(|| format!("{filename}: set comprehension needs at least one `for` clause"))?;
            Ok(Expr::Call {
                func: Box::new(Expr::Ident {
                    name: "set".to_string(),
                    node_id: 0,
                    res: Resolution::Unresolved,
                }),
                args: vec![CallArg::Positional(list_expr)],
                span: make_span(filename),
                cache: Default::default(),
                node_id: 0, // #16: py-converter は未採番
            })
        }

        // ⚠ 辞書内包は未対応。Arrow に「ペアのリストから dict を作る」手段が無い
        //   （`dict(pairs)` は `'dict' object is not callable`）。
        py::Expr::DictComp(_) => Err(format!(
            "{filename}: dict comprehension is not supported (list and set comprehensions are)"
        )),

        // ⚠ ジェネレータ式は**遅延評価**。リスト内包と同じ脱糖にすると先行評価になり、
        //   無限ジェネレータや副作用の回数が変わる。黙って変えないため明示エラーにする。
        py::Expr::GeneratorExp(_) => Err(format!(
            "{filename}: generator expression is not supported (it is lazy; use a list comprehension `[...]` instead)"
        )),

        // ★ lambda を**名前付き関数へ持ち上げる**（lambda lifting・項目 26）。
        //
        //   sorted(xs, key=lambda x: -x)
        //     ->  fn __py_lambda_N(x):
        //             return -x
        //         sorted(xs, key=__py_lambda_N)
        //
        // ⚠ 戻り型は **`None`（注釈なし）**。`-> Any` にすると `Any` が伝染して
        //   `cannot apply '+' to 'Any'` になる（実測）。py 由来の関数はどれも注釈が
        //   無いので、それに揃えるのが正しい。
        //
        // ⚠⚠ **入れ子の lambda は内側を外側の本体へ入れる**。同じバッファへ積むと
        //   `lambda x: (lambda y: x + y)` の内側が外側の外に出てしまい、`x` が見えなくなる。
        //   ⇒ 本体の変換だけ別バッファで囲い、そこで持ち上がった定義を
        //     **持ち上げ先 `fn` の本体の先頭**に置く。
        py::Expr::Lambda(l) => {
            // ⚠ 束縛を新しく作る位置（内包表記のループ変数など）へ持ち上げると、
            //   本体が参照する名前がスコープ外になる。評価回数のガードを流用して止める。
            if !hoist_is_safe() {
                return Err(format!(
                    "{filename}: a lambda here cannot be lifted (it is inside a comprehension,                      an `and`/`or` operand, a conditional expression, or a `while` test);                      define a named function instead"
                ));
            }
            let name = next_temp_name("lambda");
            let (params, renames) = convert_params(&l.args, filename)?;
            let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            // 本体は別バッファで変換する（入れ子 lambda の定義を内側に閉じ込めるため）。
            hoist_push();
            let body_expr = {
                let _rename_guard = ParamRenameGuard::push(renames, &param_names);
                convert_expr(&l.body, filename)
            };
            let mut body = hoist_pop();
            let body_expr = body_expr?;
            body.push(Stmt::Return(Some(body_expr)));
            if !hoist_emit(Stmt::FnDef {
                name: name.clone(),
                template_params: vec![],
                params,
                return_type: None,
                body,
                is_abstract: false,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: crate::ast::Accessibility::Public,
            }) {
                return Err(format!("{filename}: internal error: no hoist buffer"));
            }
            Ok(ident_expr(&name))
        }

        // f-string。`desugar_fstring`（`src/parser/exprs.rs`）と**同形**に脱糖する:
        // リテラル片はそのまま、埋め込み式は `str(...)` で包み、左結合の `+` で連結する。
        //
        // ⚠ Arrow に「書式指定」に相当する構文が無いので、`{x:.2f}` のような
        //   **format_spec 付きは明示エラー**（FUTURE_FEATURE.md に残してある）。
        // ⚠ 変換フラグは `!s` → `str()`、`!r` → `repr()` に写せる（どちらも Arrow に組込がある）。
        //   `!a`（ascii）は相当する組込が無いので明示エラー。
        py::Expr::JoinedStr(j) => {
            let span = make_span(filename);
            let mut parts: Vec<Expr> = Vec::new();
            for v in &j.values {
                match v {
                    // リテラル片（`f"hi {n}"` の `"hi "` の部分）。
                    py::Expr::Constant(c) => parts.push(convert_constant(c, filename)?),
                    py::Expr::FormattedValue(fv) => {
                        if fv.format_spec.is_some() {
                            return Err(format!(
                                "{filename}: f-string format specifier (e.g. `{{x:.2f}}`) is not supported"
                            ));
                        }
                        let func_name = match fv.conversion.to_char() {
                            None | Some('s') => "str",
                            Some('r') => "repr",
                            Some(other) => {
                                return Err(format!(
                                    "{filename}: f-string conversion `!{other}` is not supported (only `!s` and `!r`)"
                                ))
                            }
                        };
                        let inner = convert_expr(&fv.value, filename)?;
                        parts.push(Expr::Call {
                            func: Box::new(Expr::Ident {
                                name: func_name.to_string(),
                                node_id: 0,
                                res: Resolution::Unresolved,
                            }),
                            args: vec![CallArg::Positional(inner)],
                            span: span.clone(),
                            cache: Default::default(),
                            node_id: 0, // #16: py-converter は未採番
                        });
                    }
                    // 仕様上ここには来ないが、来たら文字列化して連結する（黙って落とさない）。
                    other => {
                        let inner = convert_expr(other, filename)?;
                        parts.push(Expr::Call {
                            func: Box::new(Expr::Ident {
                                name: "str".to_string(),
                                node_id: 0,
                                res: Resolution::Unresolved,
                            }),
                            args: vec![CallArg::Positional(inner)],
                            span: span.clone(),
                            cache: Default::default(),
                            node_id: 0,
                        });
                    }
                }
            }
            // `f""` は空文字列。
            let mut iter = parts.into_iter();
            let Some(first) = iter.next() else {
                return Ok(Expr::Str(Rc::from("")));
            };
            Ok(iter.fold(first, |acc, e| Expr::BinOp {
                op: BinOp::Add,
                left: Box::new(acc),
                right: Box::new(e),
                span: span.clone(),
                node_id: 0, // #16: 合成連結（注釈対象外）
            }))
        }

        py::Expr::Await(_) => Err(format!("{filename}: 'await' is not supported")),

        py::Expr::Yield(_) | py::Expr::YieldFrom(_) => Err(format!(
            "{filename}: yield expression in Python is not supported"
        )),

        // ★ walrus `(x := expr)`（項目 23）— 補助文 `x = expr` を持ち上げ、式は `x` を返す。
        //
        // ⚠⚠ **持ち上げると評価回数が変わる位置では拒否する**。`while` の条件・
        //   `and` / `or` の右辺・三項の腕・内包表記の中は、条件付き／反復評価なので
        //   「文の直前に 1 回」へ移すと**黙って意味が変わる**（`UnsafeHoistGuard`）。
        py::Expr::NamedExpr(ne) => {
            let py::Expr::Name(target) = &*ne.target else {
                return Err(format!(
                    "{filename}: only a simple name is supported on the left of `:=`"
                ));
            };
            if !hoist_is_safe() {
                return Err(format!(
                    "{filename}: `:=` here would change how many times it is evaluated                      (it is inside a `while` test, an `and`/`or` operand, a conditional                      expression, or a comprehension); assign before the statement instead"
                ));
            }
            let name = target.id.to_string();
            let value = convert_expr(&ne.value, filename)?;
            // ⚠ 宣言か再代入かは項目 2 の巻き上げが決める。walrus の名前も
            //   `collect_assigned_names` が拾うので、ここは常に再代入で足りる。
            if !hoist_emit(Stmt::Assign {
                name: name.clone(),
                value,
                span: make_span(filename),
                slot: Default::default(),
            }) {
                return Err(format!("{filename}: internal error: no hoist buffer"));
            }
            Ok(ident_expr(&name))
        }

        // Python の三項式 `a if cond else b` を Arrow の `if` 式へ。
        //
        // Arrow の `if` 式は分岐本体が**文の列**で、値は `block_return` で返す形なので、
        // 各腕を `BlockReturn(<値>)` 1 文だけのブロックにする。
        // ⚠ `return_type: None`（`-> T` 注釈なし）でも式として評価できる（実機確認済み）。
        // ⚠ Python の三項式は選ばれた腕しか評価しない。Arrow の `if` 式も同じなので、
        //   副作用の回数（`f() if c else g()`）まで一致する。
        py::Expr::IfExp(ifexp) => {
            let span = make_span(filename);
            let cond = convert_expr(&ifexp.test, filename)?;
            // ⚠ **腕は選ばれた側しか評価されない**ので、持ち上げ禁止の位置（項目 23）。
            //   `test` は必ず評価されるので対象外。
            let (then_val, else_val) = {
                let _unsafe_guard = UnsafeHoistGuard::enter();
                (
                    convert_expr(&ifexp.body, filename)?,
                    convert_expr(&ifexp.orelse, filename)?,
                )
            };
            Ok(Expr::IfExpr {
                branches: vec![(cond, vec![Stmt::BlockReturn(then_val, span.clone())])],
                else_body: Some(vec![Stmt::BlockReturn(else_val, span)]),
                return_type: None,
            })
        }

        py::Expr::Starred(_) => Err(format!(
            "{filename}: starred expression is not supported in this context"
        )),

        // セットリテラル `{1, 2, 3}`。Arrow にも `Expr::Set` が実在する。
        // ⚠ 空セットは Python でも `set()`（`{}` は空辞書）なので、ここには来ない。
        // ⚠ 集合内包 `{x for x in xs}` は `SetComp` で別ノード。項目 17（内包表記）の担当。
        // セット表示。`*a` の展開（U3）を含むときは `set(...)` + `.union(...)` へ脱糖する:
        //   `{1, *a, 2}` → `set([1, 2]).union(a)`
        // ⚠ **リストの `[*a]` と違い、展開元がリストでもセットでも通る**
        //   （`set()` は任意のイテラブルを受け、`union` もリストを受けるため）。
        //   セットは順序を持たないので、リテラル分を先に集めてから union しても意味は同じ。
        py::Expr::Set(st) => Ok(Expr::Set(convert_seq_entries(&st.elts, filename)?)),

        // スライス `a[1:3]` / `a[::2]`。rustpython は 3 要素とも `Option` で持ち、
        // 省略（`a[:2]` の begin など）は `None` になる。Arrow の `Expr::Slice` も同じ形。
        // ⚠ 負のインデックス・負のステップ（`a[::-1]`）は Arrow 側が既に対応済み。
        py::Expr::Slice(sl) => {
            let conv = |e: &Option<Box<py::Expr>>| -> Result<Option<Box<Expr>>, String> {
                match e {
                    Some(inner) => Ok(Some(Box::new(convert_expr(inner, filename)?))),
                    None => Ok(None),
                }
            };
            Ok(Expr::Slice {
                begin: conv(&sl.lower)?,
                end: conv(&sl.upper)?,
                step: conv(&sl.step)?,
            })
        }

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python expression")),
    }
}

// ---------------------------------------------------------------------------
// 定数変換
// ---------------------------------------------------------------------------

/// Python のリテラル定数を tl の `Expr` に変換する。
pub(crate) fn convert_constant(c: &py::ExprConstant, filename: &str) -> Result<Expr, String> {
    constant_value_to_expr(&c.value, filename)
}

/// 生の `py::Constant` を Arrow の `Expr` に変換する。
///
/// `Constant::Tuple` が入れ子の `Constant` を持つため、`convert_constant` から分離して
/// **再帰できる**形にしてある。
pub(crate) fn constant_value_to_expr(value: &py::Constant, filename: &str) -> Result<Expr, String> {
    match value {
        py::Constant::Int(n) => {
            let v: i64 = n.try_into().unwrap_or(i64::MAX);
            Ok(Expr::Int(v))
        }
        py::Constant::Float(f) => Ok(Expr::Float(*f)),
        py::Constant::Str(s) => Ok(Expr::Str(Rc::from(s.as_str()))),
        py::Constant::Bool(b) => Ok(Expr::Bool(*b)),
        py::Constant::None => Ok(Expr::None),
        py::Constant::Bytes(_) => Err(format!("{filename}: bytes literals are not supported")),
        py::Constant::Ellipsis => Ok(Expr::None),
        // 定数タプル。要素も `Constant` なので再帰する。
        //
        // ⚠ **このアームは現在の構成では到達しない**。`Constant::Tuple` を作るのは
        //   rustpython の `ConstantOptimizer` だけで、それは `constant-optimization`
        //   フィーチャ有効時にしか実装されず、`Suite::parse` は畳み込みを行わない。
        //   通常のタプル `(1, 2)` は**常に** `py::Expr::Tuple` として来る（そちらは対応済み）。
        //   将来 rustpython の畳み込みを有効にしたときに黙って壊れないよう、正しく変換しておく。
        py::Constant::Tuple(items) => {
            let elts: Result<Vec<Expr>, _> = items
                .iter()
                .map(|x| constant_value_to_expr(x, filename))
                .collect();
            Ok(Expr::Tuple(
                elts?.into_iter().map(crate::ast::SeqEntry::Item).collect(),
            ))
        }
        py::Constant::Complex { .. } => {
            Err(format!("{filename}: complex numbers are not supported"))
        }
    }
}

// ---------------------------------------------------------------------------
// 演算子変換
// ---------------------------------------------------------------------------

/// Python の二項演算子 (`py::Operator`) を tl の `BinOp` に変換する。
pub(crate) fn convert_binop(op: &py::Operator, filename: &str) -> Result<BinOp, String> {
    Ok(match op {
        py::Operator::Add => BinOp::Add,
        py::Operator::Sub => BinOp::Sub,
        py::Operator::Mult => BinOp::Mul,
        py::Operator::Div => BinOp::Div,
        py::Operator::FloorDiv => BinOp::FloorDiv,
        py::Operator::Mod => BinOp::Mod,
        py::Operator::Pow => BinOp::Pow,
        py::Operator::BitAnd => BinOp::BitAnd,
        py::Operator::BitOr => BinOp::BitOr,
        py::Operator::BitXor => BinOp::BitXor,
        py::Operator::LShift => BinOp::LShift,
        py::Operator::RShift => BinOp::RShift,
        py::Operator::MatMult => {
            return Err(format!("{filename}: '@' matrix multiply is not supported"))
        }
    })
}

/// Python の拡張代入演算子を tl の `BinOp` に変換する（`convert_binop` の別名）。
pub(crate) fn convert_augop(op: &py::Operator, filename: &str) -> Result<BinOp, String> {
    convert_binop(op, filename)
}

/// Python の比較演算子 (`py::CmpOp`) を tl の `BinOp` に変換する。
pub(crate) fn convert_cmpop(op: &py::CmpOp, filename: &str) -> Result<BinOp, String> {
    Ok(match op {
        py::CmpOp::Eq => BinOp::Eq,
        py::CmpOp::NotEq => BinOp::NotEq,
        py::CmpOp::Lt => BinOp::Lt,
        py::CmpOp::LtE => BinOp::LtEq,
        py::CmpOp::Gt => BinOp::Gt,
        py::CmpOp::GtE => BinOp::GtEq,
        // メンバシップ。Arrow の `in` / `not in` は list / dict / str / tuple / set の
        // どれにも効き、Python と同じ真偽を返す（実機確認済み）。
        py::CmpOp::In => BinOp::In,
        py::CmpOp::NotIn => BinOp::NotIn,
        // ⚠ `is` / `is not` は `Compare` アーム側で処理する（`is not` は `Not` ラップが要り、
        //   `BinOp` 1 個では表せないため）。ここには到達しない。
        py::CmpOp::Is | py::CmpOp::IsNot => {
            return Err(format!(
                "{filename}: internal error: 'is'/'is not' must be handled by the Compare arm"
            ))
        }
    })
}

