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
//   * 型検査: 置いた宣言を**普通のクラス・関数として検査する**（2026-09-26・設計書 2-13）。
//     具体化の型（`GenericInstance`）からは、レジストリの `instance_class` で具体クラスを引く。
//     ⇒ `Box[str]` の `self.v = 0` やテンプレートのメソッド呼び出しの型違いが静的エラーになる。
//     エディタは展開しないが、単相化だけは [`monomorphize`] で行う。
//   * 値の名前はテンプレートの名前のまま（`<Box object>`）。束縛名だけが `Box[int]`。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{Expr, Stmt};


/// 1 つのプログラムで作ってよい具体化の数の上限（タスク 2-9）。
///
/// ⚠ 実際のプログラムで届く数ではない。届いたら暴走（下の深さの上限をすり抜ける形）なので止める。
pub(super) const INSTANCE_BUDGET: usize = 1_000;

/// 具体化の**入れ子の深さ**の上限（タスク 2-9）。
///
/// ⚠⚠ **再帰的な具体化は終わらない。** `f[T]` の本体が `f[Box[T]]` を呼ぶと、`f[int]` を作るのに
/// `f[Box[int]]` が要り、それを作るのに `f[Box[Box[int]]]` が要り……と型引数が伸び続ける。
/// 実行時の具体化は**呼ばれたときに**作るので、再帰が途中で止まれば終わっていたが、展開時の単相化は
/// **書かれた具体化をすべて先に作る**ので原理的に終わらない（設計書 参考P のリスク 2）。
/// ⚠ 以前（2-8）は上限まで黙って作り続けてから実行時に任せていた（名前が伸びるので 2 秒以上かかった・実測）。
/// ⚠ 64 段は、書いた型の入れ子（`Box[Box[int]]` など）では届かない深さ。
pub(super) const INSTANCE_DEPTH: usize = 64;

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
    /// それまでに宣言されたクラスの基底（クラス名 → 基底の trait 名）。制約の判定に使う（タスク 2-14）。
    class_bases: HashMap<String, Vec<String>>,
    /// **テンプレートの宣言より前**に書かれた具体化（タスク 2-14）。宣言に出会ったら作る。
    pending: Vec<(String, Vec<String>)>,
}

/// 単相化の状態（タスク 2-8）。展開器が持つほか、エディタは [`monomorphize`] で単独に使う。
pub(super) struct Mono {
    /// 今の枠のテンプレートと作った具体化。**枠ごと**（モジュールの展開で入れ替える）。
    pub(super) templates: Templates,
    /// このプログラムで作った具体化の数（[`INSTANCE_BUDGET`] の判定用・全体で 1 つ）。
    count: usize,
    /// プログラム全体の node-id カウンタ。写しの node-id を振り直すのに使う（タスク 2-13）。
    counter: std::rc::Rc<std::cell::Cell<u32>>,
    /// このプログラム（か、読み込んだモジュール）がテンプレートを宣言しているか。
    ///
    /// ⚠ 偽なら具体化の場所を探さない（全文の走査を省く）。テンプレートの宣言より前の具体化も
    /// 保留しておく必要がある（タスク 2-14）ので、「まだテンプレートに出会っていない」では判定できない。
    pub(super) enabled: bool,
}

/// 最上位にテンプレートの宣言があるか（[`Mono::enabled`] の判定・読むだけ）。
pub(super) fn declares_template(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| {
        matches!(s,
            Stmt::ClassDef { template_params, .. }
            | Stmt::FnDef { template_params, .. }
            | Stmt::GenDef { template_params, .. } if !template_params.is_empty())
    })
}

impl Mono {
    /// ⚠ `counter` は本体をパースしたパーサと**同じ**カウンタ（設計書 §0.3）。
    pub(super) fn new(counter: std::rc::Rc<std::cell::Cell<u32>>) -> Self {
        Self { templates: Templates::default(), count: 0, counter, enabled: false }
    }
}

/// **展開器を通さずに単相化だけを行う**（エディタ用・タスク 2-8 段階 2）。
///
/// ⚠⚠ 単相化は評価器の要らない**純粋な AST 操作**。エディタはメタ関数を展開しない（5-0）が、
/// 単相化だけはここで行う —— しないと具体化した本体の誤り（`Box[str]` の `self.v = 0`・
/// テンプレートのメソッド呼び出しの型違い）が**エディタにだけ出ない**。
/// ⚠ 具体化が終わらない（2-9）ときは単相化せずに元の文を返す（CLI が展開時エラーで止める）。
/// ⚠ メタ関数が置いたコードの中の具体化はエディタでは見えない（展開しないので）。そこは
///   CLI より少なく報告するだけで、多く報告することは無い。
#[allow(dead_code)] // CLI は展開器の中で単相化する（エディタ専用の入口）
pub fn monomorphize(stmts: Vec<Stmt>, counter: std::rc::Rc<std::cell::Cell<u32>>) -> Vec<Stmt> {
    let mut m = Mono::new(counter);
    m.enabled = declares_template(&stmts);
    if !m.enabled {
        return stmts;
    }
    let mut out: Vec<Stmt> = Vec::with_capacity(stmts.len());
    for st in &stmts {
        if instantiate_sites(&mut m, st, &mut out).is_err() {
            return stmts;
        }
        let at = out.len();
        out.push(st.clone());
        if note_decls(&mut m, &mut out, at).is_err() {
            return stmts;
        }
    }
    out
}

/// `out[from..]` に置かれた宣言を覚える（最上位に置かれた文を出すときに呼ぶ）。
///
/// - テンプレートの宣言は覚え、**それより前に書かれていた具体化**（関数の本体の中など）を
///   ここで作って `out` の末尾（テンプレートの宣言の後ろ）に積む（タスク 2-14）。
///   関数の本体が実際に呼ばれるのはもっと後なので、実行時の意味は変わらない。
/// - クラスの宣言は基底を覚える（制約付きテンプレートの判定に使う）。
pub(super) fn note_decls(m: &mut Mono, out: &mut Vec<Stmt>, from: usize) -> Result<(), String> {
    let mut noted: Vec<String> = Vec::new();
    for st in &out[from..] {
        match st {
            Stmt::ClassDef { name, template_params, bases, .. } => {
                if template_params.is_empty() {
                    m.templates.class_bases.insert(name.clone(), bases.clone());
                } else {
                    m.templates.decls.insert(name.clone(), Rc::new(st.clone()));
                    noted.push(name.clone());
                }
            }
            Stmt::FnDef { name, template_params, .. } | Stmt::GenDef { name, template_params, .. }
                if !template_params.is_empty() =>
            {
                m.templates.decls.insert(name.clone(), Rc::new(st.clone()));
                noted.push(name.clone());
            }
            _ => {}
        }
    }
    if noted.is_empty() || m.templates.pending.is_empty() {
        return Ok(());
    }
    let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut m.templates.pending)
        .into_iter()
        .partition(|(base, _)| noted.contains(base));
    m.templates.pending = rest;
    for (base, args) in ready {
        instantiate(m, &base, &args, out, &mut Vec::new())?;
    }
    Ok(())
}

/// `stmt` の中の具体化を探し、まだ作っていなければ実体を作って `out` に積む（タスク 2-8）。
///
/// ⚠ **テンプレートの宣言そのものの中は見ない**（型変数のままの `Box[T]` は、そのテンプレートを
/// 具体化したときに置換されて現れる）。メタ関数の本体も見ない（展開時に走るコード）。
/// ⚠ 具体化が終わらないとき（再帰的な具体化・タスク 2-9）は `Err`。
/// ⚠ 置いた宣言は `out` の末尾に積むだけ。展開器は積まれた分を `^` の宣言表へ登録する。
pub(super) fn instantiate_sites(
    m: &mut Mono,
    stmt: &Stmt,
    out: &mut Vec<Stmt>,
) -> Result<(), String> {
    instantiate_sites_in(m, stmt, out, &mut Vec::new())
}

/// `chain` は今作っている具体化の連鎖（外側から）。深さの上限と診断に使う（タスク 2-9）。
fn instantiate_sites_in(
    m: &mut Mono,
    stmt: &Stmt,
    out: &mut Vec<Stmt>,
    chain: &mut Vec<String>,
) -> Result<(), String> {
    if !m.enabled {
        return Ok(());
    }
    let mut sites: Vec<(String, Vec<String>)> = Vec::new();
    collect_sites_stmt(stmt, &mut sites);
    for (base, args) in sites {
        instantiate(m, &base, &args, out, chain)?;
    }
    Ok(())
}

/// 1 つの具体化を作る（作れないときは何もしない ＝ 実行時に任せる）。
fn instantiate(
    m: &mut Mono,
    base: &str,
    args: &[String],
    out: &mut Vec<Stmt>,
    chain: &mut Vec<String>,
) -> Result<(), String> {
    let concrete = crate::template_subst::instance_name(base, args);
    if m.templates.made.contains(&concrete) {
        return Ok(());
    }
    let Some(decl) = m.templates.decls.get(base).cloned() else {
        // ⚠ テンプレートの宣言より前に書かれた具体化（タスク 2-14）。宣言に出会ったら作る
        //   （[`note_decls`]）。組み込みの `dict[K, V](..)` などもここへ来るが、宣言が来ないので作られない。
        if !m.templates.pending.iter().any(|(b, a)| b == base && a == args) {
            m.templates.pending.push((base.to_string(), args.to_vec()));
        }
        return Ok(());
    };
    let params = match &*decl {
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } => template_params,
        _ => return Ok(()),
    };
    // ⚠ 型引数の数違いは実行時がそう言う（ここで黙って作らない）。
    if params.len() != args.len() {
        return Ok(());
    }
    // ⚠⚠ 制約付き（`class Box[T: Printable]`）は、**制約を満たすと確かめられたときだけ**作る
    //   （タスク 2-14）。判定は実行時の `type_satisfies_trait` と同じ ——「型引数がクラスで、
    //   その基底にその trait がある」。満たさない・判断できない（組み込み型・まだ宣言されて
    //   いないクラス）ときは作らず、実行時の `TemplateError` に任せる（従来どおり）。
    let satisfied = params.iter().zip(args).all(|(p, a)| {
        p.constraints.is_empty()
            || m.templates
                .class_bases
                .get(a)
                .is_some_and(|bases| p.constraints.iter().all(|c| bases.contains(c)))
    });
    if !satisfied {
        return Ok(());
    }
    // ⚠⚠ 終わらない具体化は止めて、**どう連鎖したか**を見せる（タスク 2-9）。
    if chain.len() >= INSTANCE_DEPTH {
        return Err(endless_instantiation(chain, &concrete));
    }
    if m.count >= INSTANCE_BUDGET {
        return Err(format!(
            "MetaError: too many template instantiations (more than {INSTANCE_BUDGET}) — the last one \
             was '{concrete}'; a template that keeps producing new type arguments cannot be \
             monomorphised"
        ));
    }
    m.count += 1;
    // ⚠ 「実体化中」の印を**置換の前に**付ける。中に自分自身が現れても潜り直さない。
    m.templates.made.insert(concrete.clone());

    let type_map: HashMap<String, String> =
        params.iter().map(|p| p.name.clone()).zip(args.iter().cloned()).collect();
    // ⚠⚠ 写しには**新しい node-id**を振る（型検査の注釈が具体化どうしで衝突しないように）。
    let mut inst = crate::template_subst::subst_stmt_renumbered(&decl, &type_map, &m.counter);
    match &mut inst {
        Stmt::ClassDef { name, template_params, .. }
        | Stmt::FnDef { name, template_params, .. }
        | Stmt::GenDef { name, template_params, .. } => {
            *name = concrete.clone();
            template_params.clear();
        }
        _ => unreachable!("checked above"),
    }
    // 置いた宣言の中の具体化（`Box[Box[int]]` のフィールド型の `Box[int]` など）も作る。
    // ⚠ 使う側より**前**に置く（定義の順序が実行時の順序になる）。
    chain.push(concrete);
    let nested = instantiate_sites_in(m, &inst, out, chain);
    chain.pop();
    nested?;
    out.push(inst);
    Ok(())
}

/// 終わらない具体化の文面（タスク 2-9）。連鎖の頭と、伸びていく様子が分かるところまでを見せる。
fn endless_instantiation(chain: &[String], next: &str) -> String {
    let shown: Vec<&str> = chain.iter().take(3).map(String::as_str).collect();
    // ⚠ 伸び続けた名前は数百文字になる（`f[Box[Box[…]]]`）。頭と尻尾だけ見せる。
    let next: String = if next.chars().count() > 60 {
        let head: String = next.chars().take(40).collect();
        let tail: String = next.chars().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{head}…{tail}")
    } else {
        next.to_string()
    };
    format!(
        "MetaError: template instantiation does not end — {} → … needs '{next}' (more than \
         {INSTANCE_DEPTH} levels deep). A template that uses itself with a growing type argument \
         (such as `f[T]` calling `f[Box[T]]`) has infinitely many instantiations, because every \
         instantiation written in the program is made before it runs; keep the type argument \
         fixed and pass the growing value instead",
        shown.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(" → ")
    )
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
