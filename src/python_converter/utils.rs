// python_converter/utils.rs — ユーティリティ: is_self / is_main_guard / expr_to_name。

use rustpython_parser::ast as py;

// ---------------------------------------------------------------------------
// ユーティリティ
// ---------------------------------------------------------------------------

/// 式が `self` という名前の識別子かどうかを判定する。
pub(crate) fn is_self(expr: &py::Expr) -> bool {
    matches!(expr, py::Expr::Name(n) if n.id.as_str() == "self")
}

/// 式が `if __name__ == "__main__":` ガード条件かどうかを判定する。
pub(crate) fn is_main_guard(expr: &py::Expr) -> bool {
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

/// Python の式からクラス継承基底名などの名前文字列を取り出す。属性アクセスは `"a.b"` 形式に展開する。
pub(crate) fn expr_to_name(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Name(n) => n.id.to_string(),
        py::Expr::Attribute(a) => {
            let base = expr_to_name(&a.value);
            format!("{}.{}", base, a.attr)
        }
        _ => "Any".to_string(),
    }
}
