// meta_expand/monomorph.rs — 展開時の単相化（インライン単相化・設計書 D35 / 参考P・タスク 2-8）。
//
// ## 何をするか
//
// 最上位の文を前から順に歩く展開器（§1.7）の中で、テンプレートの具体化
// （`Box[int](3)` / `ident[str]("s")` / `^Box[int]`）に**出会った時点で**実体を作る:
//
//   1. テンプレートの宣言を `subst_*`（`crate::template_subst`）で置換した**普通の宣言**を作る。
//      名前は `Box[int]`（`template_subst::instance_name`）。
//   2. その宣言を、具体化が書かれた**最上位の文の直前**に置く（式の途中で出会っても文へ持ち上げる）。
//   3. `(テンプレート, 型引数)` でメモ化する。置いた宣言の中の具体化（`Box[Box[int]]` の中の
//      `Box[int]` など）も同じ経路で拾う。
//
// ⚠ 事前走査も反復もしない（参考P）。テンプレートの宣言は具体化より前にある（前方のみ参照）。
//   **宣言より前に書いた具体化**（関数の本体の中など）は実体を作らず、実行時の具体化に任せる
//   （従来どおり動く。例題 `type_args_not_dropped.ar` の注釈はこの形）。
//
// ## 実行時・型検査との分担（この段階）
//
//   * 実行時: 具体化の式（`Op::CallTemplate`）は、ここで置いた宣言があればそれを使う
//     （`templates.rs`）。無いときだけ従来どおり実行時に作る —— REPL・デバッガ・宣言より前の
//     具体化・**制約付きのテンプレート**（制約の検査は実行時が持っている）。
//   * 型検査: 置いた宣言は**見ない**（`template_subst::is_instance_decl`）。テンプレートは従来どおり
//     `GenericInstance` として検査する。具体化した本体を検査すると、今は通るプログラムが
//     弾かれる（受け付ける範囲が変わる）ので別の判断にしてある。
//   * 値の名前はテンプレートの名前のまま（`<Box object>`）。束縛名だけが `Box[int]`。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{Expr, Stmt};

use super::Expander;

/// 1 つのプログラムで作ってよい具体化の数の上限（タスク 2-8 の暫定の柵・本番の診断は 2-9）。
///
/// ⚠ 再帰的な具体化（`f[T]` が `f[Box[T]]` を呼ぶ形）は原理的に終わらない。
pub(super) const INSTANCE_BUDGET: usize = 1_000;

/// 展開器が今の枠で知っているテンプレートと、作った具体化（タスク 2-8）。
///
/// ⚠ **枠ごと**（モジュールごと）に持つ。別のモジュールの同名のテンプレートと混ぜない。
#[derive(Default)]
pub(super) struct Templates {
    /// テンプレート名 → 宣言（型パラメータ付きの `class` / `fn` / `gen`）。
    decls: HashMap<String, Rc<Stmt>>,
    /// 作った（あるいは作っている最中の）具体化の名前。「実体化中」の印を兼ねる
    /// （型レベルの再帰 `Node[T]` の中の `Node[T]` で無限に潜らないように）。
    made: HashSet<String>,
}

/// テンプレートの宣言なら覚える（最上位に置かれた文を出すときに呼ぶ）。
pub(super) fn note_template(ex: &mut Expander, stmt: &Stmt) {
    let name = match stmt {
        Stmt::ClassDef { name, template_params, .. }
        | Stmt::FnDef { name, template_params, .. }
        | Stmt::GenDef { name, template_params, .. }
            if !template_params.is_empty() =>
        {
            name
        }
        _ => return,
    };
    ex.templates.decls.insert(name.clone(), Rc::new(stmt.clone()));
}

/// `stmt` の中の具体化を探し、まだ作っていなければ実体を作って `out` に積む（タスク 2-8）。
///
/// ⚠ **テンプレートの宣言そのものの中は見ない**（型変数のままの `Box[T]` は、そのテンプレートを
/// 具体化したときに置換されて現れる）。メタ関数の本体も見ない（展開時に走るコード）。
pub(super) fn instantiate_sites(ex: &mut Expander, stmt: &Stmt, out: &mut Vec<Stmt>) {
    if ex.templates.decls.is_empty() {
        return;
    }
    let mut sites: Vec<(String, Vec<String>)> = Vec::new();
    collect_sites_stmt(stmt, &mut sites);
    for (base, args) in sites {
        instantiate(ex, &base, &args, out);
    }
}

/// 1 つの具体化を作る（作れないときは何もしない ＝ 実行時に任せる）。
fn instantiate(ex: &mut Expander, base: &str, args: &[String], out: &mut Vec<Stmt>) {
    let concrete = crate::template_subst::instance_name(base, args);
    if ex.templates.made.contains(&concrete) {
        return;
    }
    let Some(decl) = ex.templates.decls.get(base).cloned() else { return };
    let params = match &*decl {
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } => template_params,
        _ => return,
    };
    // ⚠ 型引数の数違いは実行時がそう言う（ここで黙って作らない）。
    // ⚠ 制約付き（`class Box[T: Printable]`）は実行時に任せる。制約の検査は実行時が持っている。
    if params.len() != args.len() || params.iter().any(|p| !p.constraints.is_empty()) {
        return;
    }
    if ex.instance_count >= INSTANCE_BUDGET {
        return;
    }
    ex.instance_count += 1;
    // ⚠ 「実体化中」の印を**置換の前に**付ける。中に自分自身が現れても潜り直さない。
    ex.templates.made.insert(concrete.clone());

    let type_map: HashMap<String, String> =
        params.iter().map(|p| p.name.clone()).zip(args.iter().cloned()).collect();
    let mut inst = crate::template_subst::subst_stmt(&decl, &type_map);
    match &mut inst {
        Stmt::ClassDef { name, template_params, .. }
        | Stmt::FnDef { name, template_params, .. }
        | Stmt::GenDef { name, template_params, .. } => {
            *name = concrete;
            template_params.clear();
        }
        _ => unreachable!("checked above"),
    }
    // 置いた宣言の中の具体化（`Box[Box[int]]` のフィールド型の `Box[int]` など）も作る。
    // ⚠ 使う側より**前**に置く（定義の順序が実行時の順序になる）。
    instantiate_sites(ex, &inst, out);
    if ex.has_metafns {
        super::register_decl(ex, &inst);
    }
    out.push(inst);
}

// ── 具体化の場所を集める（読むだけ） ────────────────────────────────────────

fn collect_sites_stmt(stmt: &Stmt, sites: &mut Vec<(String, Vec<String>)>) {
    match stmt {
        // テンプレートの宣言の中は型変数のまま。メタ関数の本体は展開時に走るコード。
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. }
            if !template_params.is_empty() =>
        {
            return
        }
        Stmt::MetaFnDef { .. } => return,
        _ => {}
    }
    crate::stmt_walk::each_subpart(stmt, &mut |part| {
        use crate::stmt_walk::StmtPart as P;
        match part {
            P::Expr(e) | P::MatchPattern(e) => collect_sites_expr(e, sites),
            P::Control(b) | P::GenBody(b) | P::TypeBody(b) | P::AsyncBody(b) => {
                for s in b {
                    collect_sites_stmt(s, sites);
                }
            }
            P::FnBody { body, .. } => {
                for s in body {
                    collect_sites_stmt(s, sites);
                }
            }
            // シグネチャだけ・別モジュール（それぞれの枠で扱う）・束縛名。
            P::ProtocolBody(_)
            | P::ModuleBody(_)
            | P::ForTarget(_)
            | P::ExceptAlias(_)
            | P::TargetName(_) => {}
        }
    });
}

fn collect_sites_expr(e: &Expr, sites: &mut Vec<(String, Vec<String>)>) {
    if let Expr::TemplateInstantiate { base, type_args } = e {
        if let Expr::Ident { name, .. } = &**base {
            sites.push((name.clone(), type_args.clone()));
        }
    }
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => collect_sites_expr(x, sites),
            P::Body(b) => {
                for s in b {
                    collect_sites_stmt(s, sites);
                }
            }
            P::ForTarget(_) => {}
        }
    });
}
