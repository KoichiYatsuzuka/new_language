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

        py::Expr::Name(n) => Ok(Expr::Ident { name: n.id.to_string(), node_id: 0, res: Resolution::Unresolved }),

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

        py::Expr::Compare(c) => {
            if c.ops.len() != 1 || c.comparators.len() != 1 {
                return Err(format!("{filename}: chained comparisons are not supported"));
            }
            let left = convert_expr(&c.left, filename)?;
            let right = convert_expr(&c.comparators[0], filename)?;
            let span = make_span(filename);

            // ★ `is` / `is not` は `convert_cmpop` に置けない。
            //   - Python の `is` は**識別比較**なので Arrow の `===`（`BinOp::RefEq`）に対応する。
            //     ⚠⚠ Arrow にも `is` キーワードがあるが**型ガード**（`x is int`）で**別物**。
            //   - Arrow に `!==` が無いので `is not` は `Not(RefEq)` でラップする
            //     （`convert_cmpop` は `BinOp` しか返せないのでここで組む）。
            let is_op = matches!(c.ops[0], py::CmpOp::Is | py::CmpOp::IsNot);
            if is_op {
                let eq = Expr::BinOp {
                    op: BinOp::RefEq,
                    left: Box::new(left),
                    right: Box::new(right),
                    span,
                    node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
                };
                return Ok(if matches!(c.ops[0], py::CmpOp::IsNot) {
                    Expr::UnaryOp {
                        op: UnaryOp::Not,
                        operand: Box::new(eq),
                    }
                } else {
                    eq
                });
            }

            let op = convert_cmpop(&c.ops[0], filename)?;
            Ok(Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
                node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
            })
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

        py::Expr::List(l) => {
            let items: Result<Vec<Expr>, _> =
                l.elts.iter().map(|e| convert_expr(e, filename)).collect();
            Ok(Expr::List(items?))
        }

        py::Expr::Tuple(t) => {
            let items: Result<Vec<Expr>, _> =
                t.elts.iter().map(|e| convert_expr(e, filename)).collect();
            Ok(Expr::Tuple(items?))
        }

        py::Expr::Dict(d) => {
            let mut pairs: Vec<(Expr, Expr)> = Vec::new();
            for (k, v) in d.keys.iter().zip(d.values.iter()) {
                let Some(k) = k else {
                    return Err(format!(
                        "{filename}: **dict unpacking in dict literal is not supported"
                    ));
                };
                pairs.push((convert_expr(k, filename)?, convert_expr(v, filename)?));
            }
            Ok(Expr::Dict(pairs))
        }

        // 集合内包は**セットリテラルとは別ノード**。リテラル `{1, 2}` は項目 22 で通るが、
        // `{x for x in xs}` は内包表記（項目 17）が要る。取り違えやすいので専用の文言にする。
        py::Expr::SetComp(_) => Err(format!(
            "{filename}: set comprehension is not supported (set literals like `{{1, 2}}` are supported)"
        )),

        py::Expr::ListComp(_) | py::Expr::DictComp(_) | py::Expr::GeneratorExp(_) => {
            Err(format!("{filename}: comprehensions are not supported"))
        }

        py::Expr::Lambda(_) => Err(format!("{filename}: lambda is not supported")),

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

        py::Expr::NamedExpr(_) => Err(format!("{filename}: walrus operator ':=' is not supported")),

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
            let then_val = convert_expr(&ifexp.body, filename)?;
            let else_val = convert_expr(&ifexp.orelse, filename)?;
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
        py::Expr::Set(st) => {
            let items: Result<Vec<Expr>, _> =
                st.elts.iter().map(|e| convert_expr(e, filename)).collect();
            Ok(Expr::Set(items?))
        }

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
fn constant_value_to_expr(value: &py::Constant, filename: &str) -> Result<Expr, String> {
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
            Ok(Expr::Tuple(elts?))
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

