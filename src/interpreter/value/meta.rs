// value/meta.rs — メタ情報の値（設計書 §1.5 / タスク 4-0）。
//
// `^対象` が返す値。**展開時にしか存在しない**（`Value::Code` と同じ立場）。
//
// ⚠⚠ **`parse_ar` が返す `Value::Namespace` の木とは別物にしてある。**
// あちらは「Arrow の AST を他言語へ出す」ための表現で、`std_tools/convert_to_python/`
// の `converter.ar`（1177 行）が `__type__` 文字列に依存している。同じ器を使い回すと、
// メタプログラミングの都合で converter が壊れる。役割が違うので分ける（設計書 4-5）。

use std::rc::Rc;

use crate::ast::Stmt;
use crate::type_check::MetaKind;

use super::core::Value;

/// `^対象` が返すメタ情報（設計書 §1.5）。
///
/// ⚠ 射影（`.name` / `.fields` / …）は**宣言そのもの**から引く。値を作る時点で
/// 全部を展開しておくと、使われない射影のために毎回 AST を舐めることになる。
#[derive(Debug, Clone)]
pub struct MetaValue {
    /// 種別。`meta_instance` / `meta_function` / `meta_class` / `meta_member` のどれか。
    ///
    /// ⚠ 型検査側の [`MetaKind`] と**同じ定義を使う**。種別が 2 か所にあると、
    /// 片方だけ増やしたときに黙ってずれる。
    pub kind: MetaKind,
    /// 対象の名前（`^Point` なら `"Point"`）。
    pub name: String,
    /// 対象の宣言。射影はここから引く。
    ///
    /// ⚠ `Rc` 共有。メタ値を配り回しても AST は複製されない。
    pub decl: Rc<Stmt>,
}

impl MetaValue {
    /// 宣言から種別を決める（設計書 §1.5）。
    ///
    /// ⚠⚠ **網羅 match にしない。** `Stmt` は 40 種類以上あり、メタ情報の対象になるのは
    /// ごく一部。代わりに**対象になるものを列挙**し、それ以外は `None` を返して
    /// 呼び出し側に「`^` を当てられない」と言わせる。
    pub fn kind_of(decl: &Stmt) -> Option<MetaKind> {
        Some(match decl {
            Stmt::ClassDef { .. } | Stmt::TraitDef { .. } | Stmt::EnumDef { .. } => MetaKind::Class,
            Stmt::FnDef { .. } | Stmt::GenDef { .. } => MetaKind::Function,
            Stmt::Field { .. } => MetaKind::Member,
            // 変数束縛は「値（インスタンス）のメタ情報」。
            Stmt::Let(..) | Stmt::Mut(..) | Stmt::Const(..) | Stmt::Static(..) => {
                MetaKind::Instance
            }
            _ => return None,
        })
    }
}

/// メタ情報への射影（設計書 §1.5 / D12・タスク 4-0）。
///
/// ⚠⚠ **射影の一覧はここ 1 箇所に集める**（D12）。後から型の変更・追加・削除が入る前提なので、
/// 散らばると「足したのに片方だけ古い」が起きる。増やすときはこの関数だけを直す。
///
/// ⚠ 未知の射影は `None` を返す。呼び出し側が「そんな射影は無い」と言う。
/// **黙って `None` 値を返さないこと** —— 綴り間違いが気づかれずに通ってしまう。
///
/// ⚠ 参考B の 8 項目（`params` / `fields` / `methods` …）は**タスク 4-1**。
/// ここにあるのは種別に依らない土台だけ。
pub fn meta_projection(m: &Rc<MetaValue>, attr: &str) -> Option<Value> {
    match attr {
        // 対象の名前。
        "name" => Some(Value::str(m.name.as_str())),
        // 種別の名前（`meta_class` など）。分岐に使う。
        "kind" => Some(Value::str(kind_name(m.kind.clone()))),
        _ => None,
    }
}

/// 種別の表示名。⚠ **網羅 match**。種別を足したらここが止まる。
fn kind_name(kind: MetaKind) -> &'static str {
    match kind {
        MetaKind::Instance => "meta_instance",
        MetaKind::Function => "meta_function",
        MetaKind::Class => "meta_class",
        MetaKind::Member => "meta_member",
    }
}
