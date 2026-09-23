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

    /// 宣言の名前（[`MetaValue::kind_of`] と**対**で使う）。
    ///
    /// ⚠⚠ **`crate::decl_names` は使えない。** あちらは「スコープに名前を作るか」を
    /// 答えるモジュールで、クラスのフィールドは**インスタンスのフィールド**であって
    /// スコープの名前ではないので**意図的に報告しない**（`decl_names` の doc）。
    /// メタ情報が欲しいのは「宣言に書かれた名前」なので、問いが違う。
    /// ⇒ 混ぜると `^T.fields[0].name` が**空文字列になる**（実測）。
    pub fn name_of(decl: &Stmt) -> Option<String> {
        Some(match decl {
            Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::EnumDef { name, .. }
            | Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::Field { name, .. } => name.clone(),
            Stmt::Let(name, ..) | Stmt::Mut(name, ..) | Stmt::Const(name, ..) => name.clone(),
            Stmt::Static(name, ..) => name.clone(),
            _ => return None,
        })
    }
}
