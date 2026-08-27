// python_converter/decorators.rs — `@decorator` の変換: Arrow のデコレータ式と、
// Arrow 側でフラグとして表現されるメソッド属性（static / class_method / abstract）への振り分け。

use {
    rustpython_parser::ast as py,
    crate::ast::Expr,
};
use super::*;

/// Python のデコレータリストを変換した結果。
///
/// Python の `@staticmethod` / `@classmethod` / `@abstractmethod` は「呼び出して包む
/// デコレータ」ではなく**定義の種別マーカ**であり、Arrow 側では対応する組込み関数が
/// 存在しない（そのまま `decorators` に積むと実行時 `NameError` になる）。
/// Arrow はこれらを `static fn` / `class_method fn` / 抽象メソッドという
/// `Stmt::FnDef` のフラグで表現するため、ここでフラグへ振り替える。
pub(crate) struct DecoratorInfo {
    /// Arrow の `FnDef.decorators` / `ClassDef.decorators` にそのまま載せる式。
    pub decorators: Vec<Expr>,
    pub is_static: bool,
    pub is_class_method: bool,
    pub is_abstract: bool,
}

/// デコレータ式から「デコレータ名」を取り出す。
/// `@a.b` は `"a.b"`、`@f(x)`（デコレータファクトリ）は呼び出される側の名前を返す。
fn decorator_name(expr: &py::Expr) -> Option<String> {
    match expr {
        py::Expr::Name(_) | py::Expr::Attribute(_) => Some(expr_to_name(expr)),
        py::Expr::Call(c) => decorator_name(&c.func),
        _ => None,
    }
}

/// Python の `decorator_list` を Arrow のデコレータ式リスト＋メソッド属性フラグに変換する。
///
/// - `target`: エラーメッセージ用の対象表記（例 `"method 'C.m'"`）。
/// - `in_class`: クラス本体のメソッドかどうか。`false` のとき `@staticmethod` 等は
///   Python でも無意味なので明示エラーにする。
pub(crate) fn convert_decorators(
    decorator_list: &[py::Expr],
    filename: &str,
    target: &str,
    in_class: bool,
) -> Result<DecoratorInfo, String> {
    let mut info = DecoratorInfo {
        decorators: Vec::new(),
        is_static: false,
        is_class_method: false,
        is_abstract: false,
    };

    for dec in decorator_list {
        let name = decorator_name(dec).unwrap_or_default();
        // 末端の名前で判定する（`abc.abstractmethod` → `abstractmethod`）。
        let last = name.rsplit('.').next().unwrap_or("");
        match last {
            "staticmethod" | "classmethod" | "abstractmethod" if !in_class => {
                return Err(format!(
                    "{filename}: '@{name}' is only meaningful on a method ({target})"
                ));
            }
            "staticmethod" => info.is_static = true,
            "classmethod" => info.is_class_method = true,
            // `@abstractmethod` は本体を持ったままでも Python では単なるマーカ。
            // 本体はそのまま変換し、Arrow 側の抽象フラグだけを立てる。
            "abstractmethod" => info.is_abstract = true,
            // Arrow にプロパティ構文（getter/setter）は存在しない。
            // `@x.setter` 等は属性形のときだけ拾う（同名のユーザデコレータを誤検出しないため）。
            "property" | "cached_property" => {
                return Err(format!(
                    "{filename}: '@{name}' is not supported ({target}): \
                     Arrow has no property syntax; use a plain method instead"
                ));
            }
            "setter" | "getter" | "deleter" if name.contains('.') => {
                return Err(format!(
                    "{filename}: '@{name}' is not supported ({target}): \
                     Arrow has no property syntax; use a plain method instead"
                ));
            }
            _ => info.decorators.push(convert_expr(dec, filename)?),
        }
    }

    if info.is_static && info.is_class_method {
        return Err(format!(
            "{filename}: '@staticmethod' and '@classmethod' cannot be combined ({target})"
        ));
    }

    Ok(info)
}
