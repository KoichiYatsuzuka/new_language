// python_converter/expressions.rs — 式・定数・演算子の変換: convert_expr / convert_constant / convert_binop / convert_augop / convert_cmpop。

use crate::ast::Resolution;
use std::rc::Rc;
use {
    rustpython_parser::ast as py,
    crate::ast::{BinOp, CallArg, Expr, UnaryOp},
    crate::token::Span,
};
use super::*;

// ---------------------------------------------------------------------------
// 式変換
// ---------------------------------------------------------------------------

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
            let op = convert_cmpop(&c.ops[0], filename)?;
            let left = convert_expr(&c.left, filename)?;
            let right = convert_expr(&c.comparators[0], filename)?;
            let span = make_span(filename);
            Ok(Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
                node_id: 0, // #16: py-converter は未採番（0=注釈対象外）
            })
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

        py::Expr::ListComp(_)
        | py::Expr::SetComp(_)
        | py::Expr::DictComp(_)
        | py::Expr::GeneratorExp(_) => Err(format!("{filename}: comprehensions are not supported")),

        py::Expr::Lambda(_) => Err(format!("{filename}: lambda is not supported")),

        py::Expr::JoinedStr(_) => Err(format!("{filename}: f-strings are not supported")),

        py::Expr::Await(_) => Err(format!("{filename}: 'await' is not supported")),

        py::Expr::Yield(_) | py::Expr::YieldFrom(_) => Err(format!(
            "{filename}: yield expression in Python is not supported"
        )),

        py::Expr::NamedExpr(_) => Err(format!("{filename}: walrus operator ':=' is not supported")),

        py::Expr::IfExp(_) => Err(format!(
            "{filename}: inline 'if' expression is not supported"
        )),

        py::Expr::Starred(_) => Err(format!(
            "{filename}: starred expression is not supported in this context"
        )),

        py::Expr::Set(_) => Err(format!("{filename}: set literal is not supported")),

        py::Expr::Slice(_) => Err(format!("{filename}: slice expression is not supported")),

        #[allow(unreachable_patterns)]
        _ => Err(format!("{filename}: unsupported Python expression")),
    }
}

// ---------------------------------------------------------------------------
// 定数変換
// ---------------------------------------------------------------------------

/// Python のリテラル定数を tl の `Expr` に変換する。
pub(crate) fn convert_constant(c: &py::ExprConstant, filename: &str) -> Result<Expr, String> {
    match &c.value {
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
        py::Constant::Tuple(_) => Err(format!("{filename}: constant tuple is not supported")),
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
        py::CmpOp::In => {
            return Err(format!(
                "{filename}: 'in' operator is not supported in expression context"
            ))
        }
        py::CmpOp::NotIn => {
            return Err(format!(
                "{filename}: 'not in' operator is not supported in expression context"
            ))
        }
        py::CmpOp::Is => return Err(format!("{filename}: 'is' operator is not supported")),
        py::CmpOp::IsNot => return Err(format!("{filename}: 'is not' operator is not supported")),
    })
}

