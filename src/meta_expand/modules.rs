// meta_expand/modules.rs — `import` した `.ar` モジュールのメタ関数（タスク 2-12）。
//
// ## 規則（設計書 §1.7・2026-09-25 確定）
//
//   * モジュールは**初めて読み込まれるとき**に、**モジュールごとに**展開する。
//   * **別のモジュールのメタ関数も使える** —— `import m` なら `m.greet()` / `!m.deco`、
//     `from m import greet` なら `greet()` / `!greet`。
//
// ## 形は実行時の `exec_module` に揃えてある
//
// パーサは `import` のたびにモジュールの本体を AST に抱えている（同じモジュールなら同じ
// 本体の写し）。実行時は `exec_module` が**最初の 1 回だけ**本体を走らせ、以後はキャッシュを返す。
// 展開も同じにする:
//
//   1. 最初の `import` で、スコープを 1 つ積んだ**展開の枠**の中で本体を展開する。
//      枠の中では、そのモジュールのメタ関数・`^` の宣言表・展開時型推論の前提が
//      **そのモジュールだけ**のものになる（呼び出し側の分は退避して戻す）。
//   2. 枠で定義した名前（メタ関数・`const`）を名前空間にまとめ、`import` の束縛名で
//      展開用インタプリタに束縛する。実行時の `import` と同じ名前で引ける。
//   3. 展開済みの本体をキャッシュし、同じモジュールの `import` はすべてその本体に差し替える。
//
// ⚠ メタ関数は枠の中で定義されるので、それより前にあるモジュールの名前を**閉包として**
//   捕まえる（`exec_fn_def` の `capture_env`）。⇒ 呼び出し側に同じ名前があっても、自分の
//   モジュールの名前を引く。実行時のモジュール関数と同じ性質（実測で確認）。

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::Stmt;
use crate::interpreter::value::Value;

use super::{expand_stmts, ordinary, Context, Expander};

/// 展開済みのモジュール（タスク 2-12）。
pub(super) struct ExpandedModule {
    /// 展開済みの本体。同じモジュールのすべての `import` をこれに差し替える。
    pub(super) body: Vec<Stmt>,
    /// 公開するメタ関数（名前 → 配置メタ関数か）。
    pub(super) metafns: HashMap<String, bool>,
    /// 展開の枠で定義した名前と値（メタ関数・`const`）。名前空間の中身になる。
    pub(super) members: HashMap<String, Value>,
}

/// 展開の枠を移るときに退避する、**モジュールごと**の状態。
struct FrameState {
    metafns: HashMap<String, bool>,
    modules: HashMap<String, HashMap<String, bool>>,
    runtime_names: std::collections::HashSet<String>,
    decls: HashMap<String, crate::interpreter::value::MetaDecl>,
    prefix: Vec<Stmt>,
    templates: super::monomorph::Templates,
}

/// モジュールを（初めてなら）展開して返す（タスク 2-12）。
///
/// ⚠ キャッシュの鍵は実行時の `exec_module` と同じ `(lang, モジュールパス)`。
/// ⚠ 循環 import はパーサが先に弾いているので、ここで展開中のモジュールに再び出会うことは無い。
pub(super) fn expand_module(
    ex: &mut Expander,
    lang: &str,
    module: &[String],
    body: Vec<Stmt>,
) -> Result<Rc<ExpandedModule>, String> {
    let key = format!("{lang}:{}", module.join("/"));
    if let Some(m) = ex.module_cache.get(&key) {
        return Ok(Rc::clone(m));
    }
    let shown = module.join(".");

    // ── 呼び出し側の状態を退避し、モジュールの枠を開く ──
    let prefix_handle = ex.interp.meta_prefix_handle();
    let saved = FrameState {
        metafns: std::mem::take(&mut ex.metafns),
        modules: std::mem::take(&mut ex.modules),
        runtime_names: std::mem::take(&mut ex.runtime_names),
        decls: ex.interp.meta_swap_decls(HashMap::new()),
        prefix: std::mem::take(&mut *prefix_handle.borrow_mut()),
        templates: std::mem::take(&mut ex.mono.templates),
    };
    super::prescan_runtime_names(ex, &body);
    ex.interp.meta_push_module_frame();

    let expanded = expand_stmts(ex, body, Context::TopLevel, Rc::new(Vec::new()))
        .and_then(|out| {
            ordinary::check(
                &out,
                &ordinary::MetaNames { plain: &ex.metafns, modules: &ex.modules },
            )?;
            Ok(out)
        });

    // ── 枠を閉じて呼び出し側の状態を戻す（失敗しても戻す） ──
    let members = ex.interp.meta_pop_module_frame();
    let metafns = std::mem::replace(&mut ex.metafns, saved.metafns);
    ex.modules = saved.modules;
    ex.runtime_names = saved.runtime_names;
    ex.mono.templates = saved.templates;
    ex.interp.meta_swap_decls(saved.decls);
    *prefix_handle.borrow_mut() = saved.prefix;

    let body = expanded.map_err(|e| format!("{e}\n  while importing module '{shown}'"))?;
    let m = Rc::new(ExpandedModule { body, metafns, members });
    ex.module_cache.insert(key, Rc::clone(&m));
    Ok(m)
}

/// `import m` の結果を今の枠に束縛する（タスク 2-12）。
///
/// ⚠ 展開用インタプリタには名前空間として束縛する。`m.CONST` や `!m.deco` は
/// これを普通の式として評価して引く。
pub(super) fn bind_import(ex: &mut Expander, bind_name: &str, module: &[String], m: &ExpandedModule) {
    let ns = crate::interpreter::value::NamespaceData {
        name: module.join("."),
        members: m.members.clone(),
    };
    ex.interp.meta_bind(bind_name, Value::Namespace(Rc::new(ns)));
    ex.modules.insert(bind_name.to_string(), m.metafns.clone());
}

/// `from m import a, b as c` の結果を今の枠に束縛する（タスク 2-12）。
///
/// ⚠ メタ関数は**素の名前のメタ関数**として登録する（`greet()` がそのまま配置呼び出しになる）。
pub(super) fn bind_from_import(
    ex: &mut Expander,
    names: &[(String, Option<String>)],
    m: &ExpandedModule,
) {
    for (orig, alias) in names {
        let local = alias.as_ref().unwrap_or(orig);
        if let Some(placing) = m.metafns.get(orig) {
            ex.metafns.insert(local.clone(), *placing);
        }
        if let Some(v) = m.members.get(orig) {
            ex.interp.meta_bind(local, v.clone());
        }
    }
}
