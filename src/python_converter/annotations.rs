// python_converter/annotations.rs — 型注釈の変換: convert_annotation / subscript スライス / map_type_name。

#[allow(unused_imports)]
use {
    rustpython_parser::{ast as py, Parse},
    crate::ast::{BinOp, CallArg, Expr, FieldKind, Param, Stmt, UnaryOp},
    crate::token::Span,
};
#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// 型アノテーション変換
// ---------------------------------------------------------------------------

/// Python の型アノテーション式を tl の型文字列（例: `"list[int]"`）に変換する。
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
                "Option" => format!("Option[{arg}]"),
                "Union" => format!("Union[{arg}]"),
                "list" => format!("list[{arg}]"),
                "dict" => format!("dict[{arg}]"),
                "tuple" => format!("tuple[{arg}]"),
                "Optional" => format!("Option[{arg}]"),
                other => format!("{other}[{arg}]"),
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

/// 添字スライス式（タプルまたは単一要素）を型文字列に変換する。
pub(crate) fn convert_annotation_subscript_slice(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Tuple(t) => {
            let parts: Vec<String> = t.elts.iter().map(|e| convert_annotation(e)).collect();
            parts.join(", ")
        }
        _ => convert_annotation(expr),
    }
}

/// Python の型名を tl の型名にマッピングする（例: `"List"` → `"list"`）。
pub(crate) fn map_type_name(name: &str) -> String {
    match name {
        "int" => "int".to_string(),
        "str" => "str".to_string(),
        "float" => "float".to_string(),
        "bool" => "bool".to_string(),
        "None" => "None".to_string(),
        "NoneType" => "None".to_string(),
        "list" => "list".to_string(),
        "List" => "list".to_string(),
        "dict" => "dict".to_string(),
        "Dict" => "dict".to_string(),
        "tuple" => "tuple".to_string(),
        "Tuple" => "tuple".to_string(),
        "Optional" => "Option".to_string(),
        "Union" => "Union".to_string(),
        "Any" => "Any".to_string(),
        other => other.to_string(),
    }
}

