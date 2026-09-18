// python_converter/annotations.rs — 型注釈の変換: convert_annotation / subscript スライス / map_type_name。

use rustpython_parser::ast as py;

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
            let parts: Vec<String> = t.elts.iter().map(convert_annotation).collect();
            parts.join(", ")
        }
        _ => "Any".to_string(),
    }
}

/// 添字スライス式（タプルまたは単一要素）を型文字列に変換する。
pub(crate) fn convert_annotation_subscript_slice(expr: &py::Expr) -> String {
    match expr {
        py::Expr::Tuple(t) => {
            let parts: Vec<String> = t.elts.iter().map(convert_annotation).collect();
            parts.join(", ")
        }
        _ => convert_annotation(expr),
    }
}

/// Python の型名を tl の型名にマッピングする（例: `"List"` → `"list"`）。
pub(crate) fn map_type_name(name: &str) -> String {
    // ★ 変換器内の型エイリアス（項目 10）を最優先で展開する。
    if let Some(expanded) = lookup_type_alias(name) {
        return expanded;
    }
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


// ---------------------------------------------------------------------------
// 型エイリアス（項目 10）
// ---------------------------------------------------------------------------
//
// ★ Python 3.12 の `type X = <型式>` を**変換器の中で透過展開**する。
//
// ⚠⚠ **Arrow の `alias` は AST に出せない**。`alias name: RHS` はパース時構文で、
//   パーサが `parser.aliases` に登録して `Stmt::Pass` を返す（＝変換器から
//   `Stmt` として生成できない）。`new_type` へ落とす案もあるが、あちらは
//   **名目的別型**なので `type V = list[int]` を `list[int]` として使えなくなる。
//   ⇒ 変換器が自前の表を持ち、型注釈の解決時に展開する。
//
// ⚠ 表はモジュール 1 本の変換中だけ有効（`convert_python_source` が入口で空にする）。
//   変換器は 1 スレッドで動くのでスレッドローカルで持つ（`supers.rs` と同じ方式）。

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// `type X = T` の `X` → 展開後の Arrow 型文字列。
    static TYPE_ALIASES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// モジュールの変換を始めるときにエイリアス表を空にする。
pub(crate) fn reset_type_aliases() {
    TYPE_ALIASES.with(|t| t.borrow_mut().clear());
}

/// `type X = T` を登録する。
///
/// ⚠ **登録時に右辺を展開してから入れる**ので、`type A = int` → `type B = list[A]` と
/// 連鎖しても `B` は `list[int]` になる。自己参照（`type T = list[T]`）は、自分が
/// まだ登録されていない時点で右辺を解決するため無限再帰にならない。
pub(crate) fn register_type_alias(name: String, expanded: String) {
    TYPE_ALIASES.with(|t| {
        t.borrow_mut().insert(name, expanded);
    });
}

/// 登録済みエイリアスなら展開後の型文字列を返す。
pub(crate) fn lookup_type_alias(name: &str) -> Option<String> {
    TYPE_ALIASES.with(|t| t.borrow().get(name).cloned())
}
