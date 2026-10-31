// imports/editor_kinds.rs — `editor` 専用: import で束縛した名前の**正体**（種類・シグネチャ・定義の場所）を
// 宣言表へ書き足す（VSIX_update_plan.md 4-2 / 4-4）。
//
// # なぜ要るのか
//
// `parse_from_import_stmt` は `from X import Y` の `Y` を、import 先を読み込む**前**に宣言表へ控える
// （位置は名前を読んだその場でしか取れない）。その時点では `Y` が関数かクラスか分からないので、
// 以前は一律に `Module` で控えていた。その結果、拡張は `Y` をモジュールの色で塗り、ホバーには
// `from X import Y` しか出さず、定義へのジャンプは import 文に飛んで終わっていた。
//
// ここは読み込んだ**後**に、import 先の本体（CLI と同じ AST）から `Y` を引いて書き足す。
// 「その文がどの名前を束縛するか」は [`crate::decl_names::each_declared_name`]（唯一の定義）に任せ、
// ここは束縛した文の種類を宣言表の種類へ写すだけ。推し量らない。
//
// ⚠ 引けなかった名前は `DeclKind::Unresolved`（黙って `Module` などにしない）。import が読めなかった・
//   `X` に `Y` が無い、のどちらも誤りは診断が別に出している。

use crate::ast::Stmt;
use crate::decl_names::{each_declared_name, DeclOrigin};
use crate::parser::editor_hooks::{docstring_of, render_fn_signature};
use crate::parser::editor_index::{Decl, DeclKind, DefLoc, EditorIndex};

/// 再エクスポート（`b` が `from a import x` した `x` を `from b import x` で取り込む…）をたどる深さの上限。
/// 循環 import は CLI が止めるが、エディタは読めなかった import を飛ばして続けるので念のため。
const MAX_REEXPORT_DEPTH: usize = 16;

/// 取り込んだ名前の正体。宣言表の該当欄へそのまま写す。
struct Imported {
    kind: DeclKind,
    signature: Option<String>,
    doc: Option<String>,
    bases: Vec<String>,
    type_ann: Option<String>,
    mutability: Option<&'static str>,
    def: Option<DefLoc>,
}

impl Imported {
    fn new(kind: DeclKind, def: Option<DefLoc>) -> Self {
        Imported { kind, signature: None, doc: None, bases: Vec::new(), type_ann: None, mutability: None, def }
    }

    /// 宣言表へ書き足す。⚠ 分からない欄は**元の値を残す**（`from X import Y` のシグネチャなど）。
    fn apply(self, d: &mut Decl) {
        d.kind = self.kind;
        if self.signature.is_some() {
            d.signature = self.signature;
        }
        if self.doc.is_some() {
            d.doc = self.doc;
        }
        if !self.bases.is_empty() {
            d.bases = self.bases;
        }
        if self.type_ann.is_some() {
            d.type_ann = self.type_ann;
        }
        if self.mutability.is_some() {
            d.mutability = self.mutability;
        }
        d.def = self.def;
        d.imported = true;
    }
}

/// `body`（モジュールの最上位）で `name` を束縛している文から、その正体を決める。
///
/// ⚠ `decl_names::module_exports` と同じく**宣言が import より勝つ**。同じ側の中では後の束縛が勝つ
///   （モジュールを上から実行した結果の値）。
fn lookup_in_body(body: &[Stmt], name: &str, depth: usize) -> Option<Imported> {
    let mut defined: Option<(&Stmt, DeclOrigin)> = None;
    let mut imported: Option<(&Stmt, DeclOrigin)> = None;
    for st in body {
        each_declared_name(st, &mut |n, origin, _| {
            if n != name {
                return;
            }
            match origin {
                DeclOrigin::Import | DeclOrigin::FromImport => imported = Some((st, origin)),
                DeclOrigin::Let
                | DeclOrigin::Mut
                | DeclOrigin::Static
                | DeclOrigin::TupleLet
                | DeclOrigin::TupleMut
                | DeclOrigin::Fn
                | DeclOrigin::Gen
                | DeclOrigin::Class
                | DeclOrigin::Trait
                | DeclOrigin::Protocol
                | DeclOrigin::Enum
                | DeclOrigin::NewType => defined = Some((st, origin)),
            }
        });
    }
    let (st, origin) = defined.or(imported)?;
    describe(st, origin, name, depth)
}

/// `name` を束縛する文 `st`（束縛の出どころは `origin`）から正体を作る。
fn describe(st: &Stmt, origin: DeclOrigin, name: &str, depth: usize) -> Option<Imported> {
    // `!装飾子` は包み（`each_declared_name` も透かす）。中身の宣言で決める。
    if let Stmt::MetaDecorated { target, .. } = st {
        return describe(target, origin, name, depth);
    }
    let def = DefLoc::of_decl(st, name);
    let variable = |mutability: &'static str, def: Option<DefLoc>| {
        let mut v = Imported::new(DeclKind::Variable, def);
        v.mutability = Some(mutability);
        v.type_ann = match st {
            Stmt::Let(_, ty, ..) | Stmt::Const(_, ty, ..) | Stmt::Mut(_, ty, ..) => ty.clone(),
            _ => None,
        };
        v
    };
    Some(match origin {
        DeclOrigin::Let | DeclOrigin::TupleLet => {
            variable(if matches!(st, Stmt::Const(..)) { "const" } else { "let" }, def)
        }
        DeclOrigin::Mut | DeclOrigin::TupleMut => variable("mut", def),
        DeclOrigin::Static => variable("static mut", def),
        DeclOrigin::Fn | DeclOrigin::Gen => {
            let kind = if origin == DeclOrigin::Gen { DeclKind::Generator } else { DeclKind::Function };
            let mut v = Imported::new(kind, def);
            let (keyword, params, ret, body) = match st {
                Stmt::FnDef { params, return_type, body, .. } => ("fn", params, return_type, Some(body)),
                Stmt::GenDef { params, yield_type, body, .. } => ("gen", params, yield_type, Some(body)),
                Stmt::MetaFnDef { params, return_type, .. } => ("exprconst fn", params, return_type, None),
                // 関数を束縛する文はこの 3 つだけ（`decl_names`）。それ以外は種類だけ写す。
                _ => return Some(v),
            };
            v.signature = Some(render_fn_signature(keyword, name, params, ret.as_deref()));
            v.bases = params.iter().map(|p| p.name.clone()).collect();
            v.doc = body.and_then(|b| docstring_of(b)).map(str::to_string);
            v
        }
        DeclOrigin::Class => {
            let mut v = Imported::new(DeclKind::Class, def);
            if let Stmt::ClassDef { bases, body, .. } = st {
                v.bases = bases.clone();
                v.doc = docstring_of(body).map(str::to_string);
            }
            v
        }
        DeclOrigin::Trait => {
            let mut v = Imported::new(DeclKind::Trait, def);
            if let Stmt::TraitDef { body, .. } = st {
                v.doc = docstring_of(body).map(str::to_string);
            }
            v
        }
        DeclOrigin::Protocol => {
            let mut v = Imported::new(DeclKind::Protocol, def);
            if let Stmt::ProtocolDef { body, .. } = st {
                v.doc = docstring_of(body).map(str::to_string);
            }
            v
        }
        DeclOrigin::Enum => Imported::new(DeclKind::Enum, def),
        DeclOrigin::NewType => {
            let mut v = Imported::new(DeclKind::NewType, def);
            if let Stmt::NewTypeDef { original, .. } = st {
                v.type_ann = Some(original.clone());
            }
            v
        }
        DeclOrigin::Import => module_of(st)?,
        // 再エクスポート: 取り込み元のモジュールで元の名前を引き直す。
        DeclOrigin::FromImport => {
            let Stmt::FromImport { names, body, .. } = st else { return None };
            let (orig, _) = names.iter().find(|(o, a)| a.as_deref().unwrap_or(o) == name)?;
            if depth >= MAX_REEXPORT_DEPTH {
                return None;
            }
            return lookup_in_body(body, orig, depth + 1);
        }
    })
}

/// `import` 文 `st` が束縛するモジュール（定義の場所はモジュールのファイルの先頭）。
fn module_of(st: &Stmt) -> Option<Imported> {
    let Stmt::Import { lang, module, source_module, body, .. } = st else { return None };
    let mut v = Imported::new(DeclKind::Module, DefLoc::of_module(body));
    let spelled = source_module.clone().unwrap_or_else(|| module.join("."));
    v.signature = Some(format!("import[{lang}] {spelled}"));
    Some(v)
}

/// `from X import …` の 1 つの名前（元の名前 `orig`・束縛名 `bound`）の正体を、`from` 文が作った文から引く。
///
/// `stmts` は `from` 文そのもの（`FromImport`）と、パーサが一緒に作った `Import`（サブモジュールを読み込む文・
/// `from . import x` の `x`）。
fn resolve_bound(stmts: &[&Stmt], orig: &str, bound: &str) -> Option<Imported> {
    // `from . import x`: サブモジュール `x` を `x` に束縛する `Import` を作っている（`from_dots_import`）。
    for st in stmts {
        if let Stmt::Import { bind: Some(b), .. } = st {
            if b.name == bound {
                return module_of(st);
            }
        }
    }
    for st in stmts {
        let Stmt::FromImport { names, body, module, .. } = st else { continue };
        if !names.iter().any(|(o, a)| a.as_deref().unwrap_or(o) == bound) {
            continue;
        }
        // 名前空間の名前（宣言・再エクスポート）。
        if let Some(v) = lookup_in_body(body, orig, 0) {
            return Some(v);
        }
        // サブモジュール（`from a import b` の `b` が `a.b`）。パーサは `a.b` を読み込む文（束縛なし）を
        // 足している（`from_import_submodules`）。
        for sub in stmts {
            if let Stmt::Import { bind: None, module: m, .. } = sub {
                if m.len() == module.len() + 1 && m.starts_with(module) && m.last().map(String::as_str) == Some(orig) {
                    return module_of(sub);
                }
            }
        }
    }
    None
}

/// `from X import …` の各名前（`handles` = (宣言表のハンドル, 元の名前, 束縛名)）の正体を書き足す。
/// `stmts` は [`resolve_bound`] と同じ。import が読めなかったときは空（全部 `Unresolved`）。
///
/// ⚠ `&mut Parser` ではなく索引だけを受け取る。呼び出し側は `stmts` に `Parser::pending_stmts` を
///   借りたまま呼ぶ（別の欄なので同時に借りられる）。
pub(crate) fn note_from_import_kinds(index: &mut EditorIndex, handles: &[(usize, String, String)], stmts: &[&Stmt]) {
    for (h, orig, bound) in handles {
        let found = resolve_bound(stmts, orig, bound).unwrap_or_else(|| Imported::new(DeclKind::Unresolved, None));
        index.with_decl(*h, |d| found.apply(d));
    }
}

/// `import m` の `m`（宣言表のハンドル `h`）に、モジュールのファイルを定義の場所として書き足す。
pub(crate) fn note_module_def(index: &mut EditorIndex, h: usize, body: &[Stmt]) {
    if let Some(l) = DefLoc::of_module(body) {
        index.with_decl(h, |d| d.def = Some(l));
    }
}
