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
// ⚠ 事前走査も反復もしない（参考P）。**宣言より前に書いた具体化**（関数の本体の中など）は
//   保留し、テンプレートの宣言に出会った時点で作って宣言の直後に置く（タスク 2-14）。
// ⚠ `import` したモジュールのテンプレート（`m.Box[int]` / `from m import Box`）は、その `import` 文の
//   本体の末尾に置く（タスク 2-15・[`instantiate_imported`]）。
//
// ## 実行時・型検査との分担（この段階）
//
//   * 実行時: **具体化は作らない**（D36・タスク 2-16）。ここで置いた宣言が定義された時点で
//     `(テンプレート, 型引数)` の組で登録され（`register_mono_instance`）、具体化の式
//     （`Op::CallTemplate`）はそれを呼ぶだけ（`templates.rs`）。REPL はブロックごとに展開する
//     （`meta_expand::Session`）。型引数の数違い・制約違反は展開時のエラー。
//     ⚠ デバッガは展開しないので、プログラムが作っていない具体化は使えない。
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
    /// `import` したモジュールのテンプレートの呼び名（タスク 2-15）。
    /// `m.Box`（`import m`）や `Box` / `B`（`from m import Box` / `Box as B`）→
    /// （[`Self::modules`] の添字, モジュールの中での名前）。
    imports: HashMap<String, (usize, String)>,
    /// `import` したモジュールごとの枠（タスク 2-15）。`.0` は `import` 文の `out` の中の位置で、
    /// 具体化は**その本体の末尾**に置く（モジュールの名前はモジュールの中で引く必要がある）。
    modules: Vec<(usize, Templates)>,
    /// `lang:モジュールパス` → [`Self::modules`] の添字（同じモジュールを 1 つの枠にまとめる・2-16）。
    module_ids: HashMap<String, usize>,
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
///
/// ⚠ 最上位で `import` した Arrow のモジュールが宣言しているものも数える（タスク 2-15）。
pub(super) fn declares_template(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } => !template_params.is_empty(),
        Stmt::Import { lang, body, .. } | Stmt::FromImport { lang, body, .. } => {
            is_arrow_source(lang) && body.iter().any(is_template_decl)
        }
        _ => false,
    })
}

fn is_template_decl(s: &Stmt) -> bool {
    matches!(s,
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } if !template_params.is_empty())
}

/// Arrow のソースを読み込む `import` か（型検査のレジストリの `is_arrow_source_lang` と同じ）。
///
/// ⚠ 外部言語のスタブ（C# の `.ars` など）の本体には具体化を置かない。実行時の意味が違う。
fn is_arrow_source(lang: &str) -> bool {
    crate::module_path::is_arrow_source_lang(lang)
}

/// 本体か、そこで `import` したモジュールが（入れ子も含めて）テンプレートを宣言しているか。
fn mentions_templates(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Import { lang, body, .. } | Stmt::FromImport { lang, body, .. } => {
            is_arrow_source(lang) && mentions_templates(body)
        }
        _ => is_template_decl(s),
    })
}

/// `import` したモジュールの本体の中の具体化を、その本体の中で単相化する（タスク 2-15）。
///
/// ⚠⚠ 具体化はモジュールの中に普通の宣言として置く（モジュールの他の宣言と同じくモジュールの大域で
///   名前を引く）。実行時の具体化は廃止した（D36・タスク 2-16）ので、モジュール自身の中の具体化も
///   ここで作らないとどこにも無い。
/// ⚠ メタ関数を持つモジュールは読み込むときに展開され、そこで単相化も済んでいる（2-12）。
///   既に置かれた具体化は「作った」ことにする（二重に置かない）。
fn monomorphize_module_body(m: &mut Mono, body: &mut Vec<Stmt>) -> Result<(), String> {
    if !mentions_templates(body) {
        return Ok(());
    }
    let mut frame = Templates::default();
    for st in body.iter() {
        if let Stmt::ClassDef { name, template_params, .. }
        | Stmt::FnDef { name, template_params, .. }
        | Stmt::GenDef { name, template_params, .. } = st
        {
            if template_params.is_empty() && name.ends_with(']') {
                frame.made.insert(name.clone());
            }
        }
    }
    std::mem::swap(&mut m.templates, &mut frame);
    let was_enabled = std::mem::replace(&mut m.enabled, true);
    let stmts = std::mem::take(body);
    let mut out: Vec<Stmt> = Vec::with_capacity(stmts.len());
    let mut result = Ok(());
    for st in stmts {
        if let Err(e) = instantiate_sites(m, &st, &mut out) {
            result = Err(e);
            break;
        }
        let at = out.len();
        out.push(st);
        if let Err(e) = note_decls(m, &mut out, at) {
            result = Err(e);
            break;
        }
    }
    m.enabled = was_enabled;
    std::mem::swap(&mut m.templates, &mut frame);
    if result.is_ok() {
        *body = out;
    }
    result
}

/// `import` したモジュールの枠の添字（テンプレートを宣言していなければ `None`）。
///
/// ⚠⚠ **同じモジュールは 1 つの枠にまとめる**（タスク 2-16）。実行時にモジュールの本体が走るのは
///   最初の `import` だけ（2 回目からは読み込み済みの名前空間を返す）なので、`import m as c` と
///   `from m import Box` の両方がある形で、2 つめの `import` 文の本体に具体化を置くと**一度も
///   定義されなかった**（以前は実行時の具体化が肩代わりしていたので表に出なかった）。
///   ⇒ 具体化は最初の `import` 文の本体に置く。
fn module_frame(m: &mut Mono, lang: &str, module: &[String], body: &[Stmt], at: usize) -> Option<usize> {
    let key = format!("{lang}:{}", module.join("/"));
    if let Some(&idx) = m.templates.module_ids.get(&key) {
        return Some(idx);
    }
    let t = module_templates(body);
    if t.decls.is_empty() {
        return None;
    }
    let idx = m.templates.modules.len();
    m.templates.modules.push((at, t));
    m.templates.module_ids.insert(key, idx);
    Some(idx)
}

/// `import` したモジュールの本体から、テンプレートの枠を作る（タスク 2-15）。
///
/// ⚠ 本体に既に置かれている具体化（メタ関数を持つモジュールは読み込むときに展開され、自分の中の
///   具体化はそこで作られている・2-12）は「作った」ことにする（二重に置かない）。
fn module_templates(body: &[Stmt]) -> Templates {
    let mut t = Templates::default();
    for st in body {
        match st {
            Stmt::ClassDef { name, template_params, bases, .. } if template_params.is_empty() => {
                if name.contains('[') {
                    t.made.insert(name.clone());
                } else {
                    t.class_bases.insert(name.clone(), bases.clone());
                }
            }
            Stmt::FnDef { name, template_params, .. } | Stmt::GenDef { name, template_params, .. }
                if template_params.is_empty() && name.contains('[') =>
            {
                t.made.insert(name.clone());
            }
            _ if is_template_decl(st) => {
                if let Stmt::ClassDef { name, .. } | Stmt::FnDef { name, .. } | Stmt::GenDef { name, .. } = st {
                    t.decls.insert(name.clone(), Rc::new(st.clone()));
                }
            }
            _ => {}
        }
    }
    t
}

impl Mono {
    /// ⚠ `counter` は本体をパースしたパーサと**同じ**カウンタ（設計書 §0.3）。
    pub(super) fn new(counter: std::rc::Rc<std::cell::Cell<u32>>) -> Self {
        Self { templates: Templates::default(), count: 0, counter, enabled: false }
    }

    /// 写しの node-id を振るカウンタを差し替える（REPL はブロックごとにパーサが違う・タスク 2-16）。
    pub(super) fn set_counter(&mut self, counter: std::rc::Rc<std::cell::Cell<u32>>) {
        self.counter = counter;
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
    // ⚠ `import` したモジュールは、先に**モジュール自身の中の具体化**を単相化する（タスク 2-15）。
    for st in out[from..].iter_mut() {
        if let Stmt::Import { lang, body, .. } | Stmt::FromImport { lang, body, .. } = st {
            if is_arrow_source(lang) {
                monomorphize_module_body(m, body)?;
            }
        }
    }
    let mut noted: Vec<String> = Vec::new();
    let mut noted_classes: Vec<String> = Vec::new();
    for (i, st) in out[from..].iter().enumerate() {
        match st {
            // ⚠ `import` したモジュールのテンプレートも具体化できるようにする（タスク 2-15）。
            //   以前はメインが宣言したテンプレートしか知らず、`m.Box[int]` / `from m import Box` の
            //   `Box[int]` は具体化されなかった ⇒ そのメソッド呼び出し・コンストラクタの検査が丸ごと抜けていた。
            // ⚠ 束縛は AST の `bind`（CPython 準拠・2026-10-02）。`import a.b` は `a` を束縛するので、
            //   `a.Box[int]`（パッケージ `a` のテンプレート）も `a.b.Box[int]` も書ける。連鎖の各モジュールの
            //   テンプレートを、束縛名からの綴り（`a` / `a.b`）で登録する。パッケージの枠は、パーサが
            //   手前に足した束縛しない文（`bind: None`）を処理したときに作ってある。
            Stmt::Import { lang, module, body, bind, .. } if is_arrow_source(lang) => {
                let _ = module_frame(m, lang, module, body, from + i);
                let Some(b) = bind else { continue };
                for k in b.module.len()..=module.len() {
                    let path = &module[..k];
                    let Some(&idx) = m.templates.module_ids.get(&format!("{lang}:{}", path.join("/"))) else {
                        continue;
                    };
                    let spelled: Vec<&str> = std::iter::once(b.name.as_str())
                        .chain(path[b.module.len().min(k)..].iter().map(String::as_str))
                        .collect();
                    let spelled = spelled.join(".");
                    let names: Vec<String> = m.templates.modules[idx].1.decls.keys().cloned().collect();
                    for name in names {
                        let key = format!("{spelled}.{name}");
                        m.templates.imports.insert(key.clone(), (idx, name));
                        noted.push(key);
                    }
                }
            }
            Stmt::FromImport { lang, module, names, body, .. } if is_arrow_source(lang) => {
                let Some(idx) = module_frame(m, lang, module, body, from + i) else { continue };
                for (orig, alias) in names {
                    if m.templates.modules[idx].1.decls.contains_key(orig) {
                        let key = alias.clone().unwrap_or_else(|| orig.clone());
                        m.templates.imports.insert(key.clone(), (idx, orig.clone()));
                        noted.push(key);
                    }
                }
            }
            Stmt::ClassDef { name, template_params, bases, .. } => {
                if template_params.is_empty() {
                    m.templates.class_bases.insert(name.clone(), bases.clone());
                    // ⚠ このクラスを型引数にして保留していた制約付きの具体化を作れる（2-16）。
                    noted_classes.push(name.clone());
                } else {
                    note_template(m, name, st);
                    noted.push(name.clone());
                }
            }
            Stmt::FnDef { name, template_params, .. } | Stmt::GenDef { name, template_params, .. }
                if !template_params.is_empty() =>
            {
                note_template(m, name, st);
                noted.push(name.clone());
            }
            _ => {}
        }
    }
    if (noted.is_empty() && noted_classes.is_empty()) || m.templates.pending.is_empty() {
        return Ok(());
    }
    let (ready, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut m.templates.pending)
        .into_iter()
        .partition(|(base, args)| {
            noted.contains(base)
                || args.iter().any(|a| noted_classes.iter().any(|c| mentions_name(a, c)))
        });
    m.templates.pending = rest;
    for (base, args) in ready {
        instantiate(m, &base, &args, out, &mut Vec::new())?;
    }
    Ok(())
}

/// テンプレートの宣言を覚える。
///
/// ⚠ 同じ名前で宣言し直したとき（REPL で書き直したブロックなど・タスク 2-16）は、前の宣言から
///   作った具体化を「作っていない」ことに戻す。実行時は新しいテンプレートの値に結び付けるので、
///   作り直さないと新しいテンプレートの具体化がどこにも無くなる。
fn note_template(m: &mut Mono, name: &str, st: &Stmt) {
    if m.templates.decls.insert(name.to_string(), Rc::new(st.clone())).is_some() {
        let prefix = format!("{name}[");
        m.templates.made.retain(|c| !c.starts_with(&prefix));
    }
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
    // ⚠ `import` したモジュールのテンプレート（タスク 2-15）はモジュールの枠で作る。
    //   ⚠ この枠が同じ名前のテンプレートを宣言していればそちらが勝つ（後から隠した形）。
    if !m.templates.decls.contains_key(base) {
        if let Some((mi, name)) = m.templates.imports.get(base).cloned() {
            return instantiate_imported(m, mi, &name, args, out, chain);
        }
    }
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
    // ⚠⚠ 型引数の数違い・制約違反は**展開時のエラー**（D36・タスク 2-16）。実行時の具体化を
    //   廃止したので、実行時に作り直して `TemplateError` を出す経路はもう無い。文面は実行時
    //   （`check_template_constraints`）と同じ。
    if params.len() != args.len() {
        return Err(format!(
            "TemplateError: expected {} type argument(s), got {} (in '{concrete}')",
            params.len(),
            args.len()
        ));
    }
    // ⚠⚠ 制約付き（`class Box[T: Printable]`）の判定は実行時の `type_satisfies_trait` と同じ
    //   ——「型引数がクラスで、その基底にその trait がある」（タスク 2-14）。
    for (p, a) in params.iter().zip(args) {
        let Some(first) = p.constraints.first() else { continue };
        match m.templates.class_bases.get(a) {
            Some(bases) => {
                if let Some(c) = p.constraints.iter().find(|c| !bases.contains(c)) {
                    return Err(constraint_violation(a, c, &p.name));
                }
            }
            // ⚠ まだ宣言されていないクラス（関数の本体の中の具体化など）。そのクラスの宣言に
            //   出会ったら作る（[`note_decls`]）。最後まで宣言されなければ作られず、実行時に
            //   `NameError: type ... is not defined` になる（実行時の判定と同じ）。
            None if may_be_a_class(a) => {
                if !m.templates.pending.iter().any(|(b, x)| b == base && x == args) {
                    m.templates.pending.push((base.to_string(), args.to_vec()));
                }
                return Ok(());
            }
            // 組み込みの型（`int` / `list[int]` …）は trait を実装しない。
            None => return Err(constraint_violation(a, first, &p.name)),
        }
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

/// `import` したモジュールのテンプレートを具体化する（タスク 2-15）。
///
/// ⚠⚠ 具体化は**その `import` 文の本体の末尾**に置く（メインには置かない）。クラスの本体が
///   引く名前（モジュールの定数・補助関数・同じモジュールの別のテンプレート）は、モジュールの中で
///   引かなければならない。メインに置くとメインの名前を引いてしまう。
/// ⚠ 実行時は従来どおり動く: 呼び出し側（メイン）には `Box[int]` という名前が無いので、
///   `m.Box[int](..)` は実行時の具体化に落ちる（`templates.rs`）。置いた宣言は型検査が使う
///   （レジストリは `import` の本体の宣言も集める）。
fn instantiate_imported(
    m: &mut Mono,
    mi: usize,
    name: &str,
    args: &[String],
    out: &mut [Stmt],
    chain: &mut Vec<String>,
) -> Result<(), String> {
    let at = m.templates.modules[mi].0;
    let mut frame = std::mem::take(&mut m.templates.modules[mi].1);
    // ⚠ 型引数は呼び出し側のクラスでもよい（`m.Holder[Cat]`）。制約の判定に呼び出し側の基底も見せる
    //   （モジュールの名前が先・同名なら隠す）。
    for (k, v) in &m.templates.class_bases {
        frame.class_bases.entry(k.clone()).or_insert_with(|| v.clone());
    }
    std::mem::swap(&mut m.templates, &mut frame);
    let r = match &mut out[at] {
        Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => {
            instantiate(m, name, args, body, chain)
        }
        _ => Ok(()),
    };
    std::mem::swap(&mut m.templates, &mut frame);
    m.templates.modules[mi].1 = frame;
    r
}

/// 制約違反の文面（実行時の `check_template_constraints` と同じ・タスク 2-16）。
fn constraint_violation(type_name: &str, constraint: &str, param: &str) -> String {
    format!(
        "TemplateError: type `{type_name}` does not satisfy trait `{constraint}` \
         (required for template parameter `{param}`)"
    )
}

/// 型引数が（まだ宣言されていない）クラスでありうるか。組み込みの型なら偽。
fn may_be_a_class(type_name: &str) -> bool {
    matches!(
        crate::type_check::InferredType::from_ann(type_name),
        Some(crate::type_check::InferredType::NamedInstance(_))
    )
}

/// 型引数の文字列が名前 `name` を（識別子として）含むか（`list[Cat]` は `Cat` を含む）。
fn mentions_name(type_arg: &str, name: &str) -> bool {
    type_arg
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|tok| tok == name)
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
    // ⚠⚠ **型注釈の中の具体化も作る**（タスク 10-4）。`fn take(let b: Box[int])` のように
    //   型注釈にしか現れない具体化は、以前は作られず、型検査がテンプレートの宣言を型引数で置き換えて
    //   読む特例（`class_and_subst` の置換表）で扱っていた。作っておけば、具体的な型引数の具体化は
    //   いつも普通のクラスとして引ける。⚠ テンプレートの宣言の中（型変数のまま）は下で見送る。
    if !matches!(stmt,
        Stmt::ClassDef { template_params, .. }
        | Stmt::FnDef { template_params, .. }
        | Stmt::GenDef { template_params, .. } if !template_params.is_empty())
        && !matches!(stmt, Stmt::MetaFnDef { .. })
        // ⚠ `import` の本体（モジュールの中身）へは降りない。置換の走査は降りるので、そのままだと
        //   モジュールのテンプレートの宣言の中の `Box[V]`（型変数のまま）を拾って具体化してしまった
        //   （実測）。モジュールの中の具体化はモジュールの枠で作る（`monomorphize_module_body`）。
        && !matches!(stmt, Stmt::Import { .. } | Stmt::FromImport { .. })
    {
        for ann in crate::template_subst::annotations_of(stmt) {
            collect_ann_sites(&ann, sites);
        }
    }
    collect_expr_sites_stmt(stmt, sites);
}

/// 型注釈の文字列の中の具体化（`Name[args]`・入れ子も）を集める（タスク 10-4）。
///
/// ⚠ 組み込みの型（`list[..]` / `dict[..]` / `Option[..]` …）は具体化ではないので見送る。
///   テンプレートでない名前（trait のテンプレート・未知の名前）は、作れないので保留に残るだけ。
fn collect_ann_sites(ann: &str, sites: &mut Vec<(String, Vec<String>)>) {
    const BUILTIN: [&str; 13] = [
        "list", "dict", "set", "tuple", "fixed_list", "list_like", "Option", "Result", "Union",
        "Intersection", "function", "type", "Signal",
    ];
    let chars: Vec<char> = ann.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '[' {
            continue;
        }
        let name: String = chars[start..i].iter().collect();
        // 対応する `]` と、入れ子の外の `,` で型引数を分ける。
        let open = i;
        let mut depth = 0i32;
        let mut close = None;
        for (j, c) in chars.iter().enumerate().skip(open) {
            match c {
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { return };
        let inner: String = chars[open + 1..close].iter().collect();
        let args = split_top_level(&inner);
        if !BUILTIN.contains(&name.as_str()) && !args.is_empty() {
            sites.push((name, args.clone()));
        }
        for a in &args {
            collect_ann_sites(a, sites);
        }
        i = close + 1;
    }
}

/// 入れ子の外の `,` で分ける（前後の空白は落とす）。
fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn collect_expr_sites_stmt(stmt: &Stmt, sites: &mut Vec<(String, Vec<String>)>) {
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
                    collect_expr_sites_stmt(s, sites);
                }
            }
            P::FnBody { body, .. } => {
                for s in body {
                    collect_expr_sites_stmt(s, sites);
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
        match &**base {
            Expr::Ident { name, .. } => sites.push((name.clone(), type_args.clone())),
            // `m.Box[int]`（`import` したモジュールのテンプレート・タスク 2-15）。
            Expr::Attr { object, attr, .. } => {
                if let Expr::Ident { name, .. } = &**object {
                    sites.push((format!("{name}.{attr}"), type_args.clone()));
                }
            }
            _ => {}
        }
    }
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => collect_sites_expr(x, sites),
            P::Body(b) => {
                for s in b {
                    collect_expr_sites_stmt(s, sites);
                }
            }
            P::ForTarget(_) => {}
        }
    });
}
