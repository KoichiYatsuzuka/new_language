// vm/compiler/decls.rs — **slot 採番**と、そのための AST 走査。
//
// ⚠⚠ **採番はリゾルバと同順・同数**（`push_base` される名前は `slots` に入れなくても
// 必ず 1 slot 消費する）。ずれると `LoadLocal` が範囲外を読む。
// ⚠ **`compile_stmt` に文種別を足したら必ず `collect_nested_decls` も見る**（#27-c で 2 回踏んだ）。


use std::collections::{HashMap, HashSet};

use crate::ast::{
    CallArg, Expr, Param, Stmt, TupleTarget,
};


/// 本体の**入れ子 `fn` が自由変数として参照する名前**をすべて集める（#27-d 段階 2b）。
///
/// この中で「自分の**可変**ローカル」に当たるものは、ツリーウォークだと `capture_env` が
/// `Var::Mutable` → `Var::Cell` へ昇格して**外側と `Rc<RefCell<Value>>` を共有**する。
/// VM でも slot ではなくセルに置かないと、クロージャ内の書き込みが外側へ返らない。
///
/// ⚠ **入れ子の入れ子まで拾える**（`collect_referenced_names` が `Stmt::FnDef` の本体へ
/// 降りるので、内側の `fn` の自由変数も中間の `fn` の参照に含まれる）。
pub(super) fn nested_fn_free_names(body: &[Stmt]) -> HashSet<String> {
    pub(super) fn walk(stmts: &[Stmt], out: &mut HashSet<String>) {
        for s in stmts {
            if let Stmt::FnDef { params, body, .. } = s {
                let mut own: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
                crate::interpreter::collect_declared_names(body, &mut own);
                let mut referenced: HashSet<String> = HashSet::new();
                crate::interpreter::collect_referenced_names(body, &mut referenced);
                out.extend(referenced.into_iter().filter(|n| !own.contains(n)));
                continue; // 本体へは降りない（その中の `fn` は上の `referenced` に含まれる）
            }
            // 制御フローの中に置かれた `fn` も拾う。
            match s {
                Stmt::If { branches, else_body } => {
                    for (_, b) in branches {
                        walk(b, out);
                    }
                    if let Some(eb) = else_body {
                        walk(eb, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => walk(body, out),
                Stmt::Block(b) => walk(b, out),
                Stmt::Match { arms, .. } => {
                    for a in arms {
                        walk(&a.body, out);
                    }
                }
                Stmt::Try { body, handlers, finally_body } => {
                    walk(body, out);
                    for h in handlers {
                        walk(&h.body, out);
                    }
                    if let Some(fb) = finally_body {
                        walk(fb, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashSet::new();
    walk(body, &mut out);
    out
}

/// param または非 `for` 宣言と名前衝突する `for` ループ変数（式形含む）の集合（#27）。
///
/// Arrow の `for` 変数はブロックスコープで、ループを抜けると外側の同名変数が戻る。
/// flat-slot モデルは名前ごとに 1 slot なので、素直に採番すると外側の値を壊す。
/// ⇒ **ここに挙がった名前だけ、ループ本体のコンパイル中に専用 slot へ差し替える**
/// （`compile_stmt` の `Stmt::For`）。以前はこの集合が空でなければ関数ごと諦めていた。
pub(super) fn for_target_shadows(params: &[Param], body: &[Stmt]) -> HashSet<String> {
    let mut for_names: HashSet<String> = HashSet::new();
    let mut decl_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
    scan_shadow_stmts(body, &mut for_names, &mut decl_names);
    for_names.intersection(&decl_names).cloned().collect()
}

pub(super) fn scan_shadow_stmts(
    stmts: &[Stmt],
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    for s in stmts {
        match s {
            Stmt::Let(n, _, e) | Stmt::Const(n, _, e) | Stmt::Mut(n, _, e) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                            decl_names.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::Static(n, e, _) => {
                decl_names.insert(n.clone());
                scan_shadow_expr(e, for_names, decl_names);
            }
            Stmt::For { targets, iter, body } => {
                for t in targets {
                    for_names.insert(t.clone());
                }
                scan_shadow_expr(iter, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e))
            | Stmt::Assign { value: e, .. }
            | Stmt::CompoundAssign { value: e, .. } => scan_shadow_expr(e, for_names, decl_names),
            Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
                scan_shadow_expr(target, for_names, decl_names);
                scan_shadow_expr(value, for_names, decl_names);
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    scan_shadow_expr(c, for_names, decl_names);
                    scan_shadow_stmts(b, for_names, decl_names);
                }
                if let Some(eb) = else_body {
                    scan_shadow_stmts(eb, for_names, decl_names);
                }
            }
            Stmt::While { cond, body } => {
                scan_shadow_expr(cond, for_names, decl_names);
                scan_shadow_stmts(body, for_names, decl_names);
            }
            Stmt::Match { subject, arms, .. } => {
                scan_shadow_expr(subject, for_names, decl_names);
                for a in arms {
                    scan_shadow_stmts(&a.body, for_names, decl_names);
                }
            }
            Stmt::Block(b) => scan_shadow_stmts(b, for_names, decl_names),
            Stmt::Try { body, handlers, finally_body } => {
                scan_shadow_stmts(body, for_names, decl_names);
                for h in handlers {
                    if let Some(alias) = &h.name {
                        decl_names.insert(alias.clone());
                    }
                    scan_shadow_stmts(&h.body, for_names, decl_names);
                }
                if let Some(fb) = finally_body {
                    scan_shadow_stmts(fb, for_names, decl_names);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn scan_shadow_expr(
    e: &Expr,
    for_names: &mut HashSet<String>,
    decl_names: &mut HashSet<String>,
) {
    macro_rules! rec {
        ($x:expr) => {
            scan_shadow_expr($x, for_names, decl_names)
        };
    }
    match e {
        Expr::Block { stmts, .. } => scan_shadow_stmts(stmts, for_names, decl_names),
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                scan_shadow_stmts(b, for_names, decl_names);
            }
            if let Some(eb) = else_body {
                scan_shadow_stmts(eb, for_names, decl_names);
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for a in arms {
                scan_shadow_stmts(&a.body, for_names, decl_names);
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            for_names.insert(target.clone());
            rec!(iter);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            scan_shadow_stmts(body, for_names, decl_names);
        }
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        _ => {}
    }
}

/// slot テーブルへ1つ宣言を追加する（既出名・`_` はスキップ）。
pub(super) fn add_decl(
    name: &str,
    ty: &Option<String>,
    mutable: bool,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    if name != "_" && !slots.contains_key(name) {
        slots.insert(name.to_string(), *n);
        slot_mut.push(mutable);
        slot_type.push(ty.clone());
        *n = n.checked_add(1)?;
    }
    Some(())
}

/// ネストしたブロック内の `let`/`const`/`mut` 宣言に平坦 slot を割り当てる（再帰）。
/// コンパイラが本体をコンパイルできる構文（if/while/match/for/try）と、**ブロック式**
/// （式の中の `block:`/if/while/for/match 式の本体宣言）にも踏み込む。
/// 既出名（トップレベル decl・別ブロックの同名）はスキップ（slot 再利用）。
pub(super) fn collect_nested_decls(
    body: &[Stmt],
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, e) | Stmt::Const(name, ty, e) => {
                add_decl(name, ty, false, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Mut(name, ty, e) => {
                add_decl(name, ty, true, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            // 入れ子の `let a, b = t`（#27-c）。ツリーウォークはスコープを push するので
            // **反復ごとに宣言し直せる**が、slot を割り当てないとグローバル宣言に落ちて
            // 2 周目で「already declared」になる（`built_in.ar` の `zx` が実例）。
            // ここで slot に載せることで、最上位の `let a, b = t` 文（slot 無し）と
            // 入れ子（slot 有り）をコンパイラが区別できる。
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        crate::ast::TupleTarget::Wildcard => {}
                        crate::ast::TupleTarget::Let(name)
                        | crate::ast::TupleTarget::Bare(name) => {
                            add_decl(name, &None, false, slots, slot_mut, slot_type, n)?
                        }
                        crate::ast::TupleTarget::Mut(name) => {
                            add_decl(name, &None, true, slots, slot_mut, slot_type, n)?
                        }
                    }
                }
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Expr(e)
            | Stmt::BlockReturn(e, _)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::Return(Some(e)) => collect_expr_decls(e, slots, slot_mut, slot_type, n)?,
            Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => {
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?
            }
            // 入れ子ブロック（`block:` 式・if/while の本体など）の中の `fn` 定義（#27-c）。
            // 名前は**そのブロックのローカル**なので slot を振る（関数本体直下の `fn` を
            // base slot に採番するのと同じ扱い）。⚠ **本体には踏み込まない**（別フレーム）。
            // ⚠ ここが抜けていたため `alias.ar` の `block->function` が
            // 「slot にもグローバルにも無い識別子」として bail していた
            //   — 採番 walker と `compile_stmt` の walker がずれていた典型例。
            Stmt::FnDef { name, .. } => {
                add_decl(name, &None, false, slots, slot_mut, slot_type, n)?;
            }
            Stmt::AttrAssign { target, value }
            | Stmt::AttrCompoundAssign { target, value, .. } => {
                collect_expr_decls(target, slots, slot_mut, slot_type, n)?;
                collect_expr_decls(value, slots, slot_mut, slot_type, n)?;
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    collect_expr_decls(c, slots, slot_mut, slot_type, n)?;
                    collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
                }
                if let Some(eb) = else_body {
                    collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::While { cond, body } => {
                collect_expr_decls(cond, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            Stmt::Match { subject, arms, .. } => {
                collect_expr_decls(subject, slots, slot_mut, slot_type, n)?;
                for arm in arms {
                    collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
                }
            }
            Stmt::For { targets, iter, body } => {
                // ループ変数は可変（tree-walk は Var::new(item, true)）。型注釈なし。
                for t in targets {
                    add_decl(t, &None, true, slots, slot_mut, slot_type, n)?;
                }
                collect_expr_decls(iter, slots, slot_mut, slot_type, n)?;
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
            }
            // `block: <stmts>` 文の中の宣言も slot を持つ（#27-c）。
            // ⚠ ここが抜けていたため、最上位の `block:` 内の `let` が slot に載らず
            // 「slot にもグローバルにも無い識別子」として bail していた。
            // `block_body_bails` は元から `Stmt::Block` を降りており、**2 つの walker が不整合**だった。
            Stmt::Block(b) => collect_nested_decls(b, slots, slot_mut, slot_type, n)?,
            Stmt::Raise { exc: Some(e), .. } => {
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?
            }
            Stmt::Try { body, handlers, finally_body } => {
                collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
                for h in handlers {
                    // `except E as e:` の別名は不変束縛（tree-walk は Var::new(exc, false)）。
                    if let Some(alias) = &h.name {
                        add_decl(alias, &None, false, slots, slot_mut, slot_type, n)?;
                    }
                    collect_nested_decls(&h.body, slots, slot_mut, slot_type, n)?;
                }
                if let Some(fb) = finally_body {
                    collect_nested_decls(fb, slots, slot_mut, slot_type, n)?;
                }
            }
            // その他（未対応構文）には踏み込まない。compile_stmt が到達時に bail する。
            _ => {}
        }
    }
    Some(())
}

/// 式の中の**ブロック式**（`block:`/if/while/for/match 式）の本体宣言に slot を割り当てる（再帰）。
/// ブロック式でない部分式も辿り、入れ子のブロック式を漏れなく採番する。
pub(super) fn collect_expr_decls(
    e: &Expr,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    macro_rules! rec {
        ($x:expr) => {
            collect_expr_decls($x, slots, slot_mut, slot_type, n)?
        };
    }
    match e {
        // ── ブロック式（本体に宣言を持つ） ──
        Expr::Block { stmts, .. } => collect_nested_decls(stmts, slots, slot_mut, slot_type, n)?,
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                rec!(c);
                collect_nested_decls(b, slots, slot_mut, slot_type, n)?;
            }
            if let Some(eb) = else_body {
                collect_nested_decls(eb, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rec!(subject);
            for arm in arms {
                collect_nested_decls(&arm.body, slots, slot_mut, slot_type, n)?;
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            add_decl(target, &None, true, slots, slot_mut, slot_type, n)?;
            rec!(iter);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        Expr::WhileExpr { cond, body, .. } => {
            rec!(cond);
            collect_nested_decls(body, slots, slot_mut, slot_type, n)?;
        }
        // ── 部分式を辿る（入れ子のブロック式を探す） ──
        Expr::BinOp { left, right, .. } => {
            rec!(left);
            rec!(right);
        }
        Expr::UnaryOp { operand, .. } => rec!(operand),
        Expr::Call { func, args, .. } => {
            rec!(func);
            for a in args {
                match a {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => rec!(x),
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            rec!(x);
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => rec!(object),
        Expr::Subscript { object, index, .. } => {
            rec!(object);
            rec!(index);
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                rec!(x);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                rec!(x);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                rec!(k);
                rec!(v);
            }
        }
        Expr::TemplateInstantiate { base, .. } => rec!(base),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => rec!(expr),
        Expr::Cast { object, .. } => rec!(object),
        // リテラル・Ident 等は宣言を含まない。
        _ => {}
    }
    Some(())
}

/// `finally` 本体の複製がネストできる上限（#40）。経路ごとに複製されるので、
/// これを超えるとコード量が指数的に膨らむ。超えたら bail する（＝`VmForceError` で停止・#33）。
///
/// ⚠ #51 まで、ここには**削除済みの `has_escape` の doc**（`include_return` 引数の説明）が
/// 前置されたまま残っていた。関数を消すときは doc も一緒に消すこと。
pub(super) const MAX_FINALLY_NEST: usize = 4;

/// ブロック式の本体が VM コンパイル不能な脱出を含むかを判定する。
/// `return` は常に不可（ブロック式内 return は構文エラー）。
/// `block_return`/`loop_yield` は当該ブロック式が扱うので許容。
///
/// ⚠ **`break`/`continue` はここでは見ない**（#34）。跳び先も、跳ぶ前に捨てるオペランド数も
/// `Stmt::Break` のコンパイル時に `loops` / `stmt_base` から確定するので、
/// **同じ木を歩く 2 つ目の walker を持たない**（ずれの温床になる）。
pub(super) fn block_body_bails(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        Stmt::Return(_) => true,
        Stmt::While { body, .. } | Stmt::For { body, .. } => block_body_bails(body),
        Stmt::If { branches, else_body } => {
            branches.iter().any(|(_, b)| block_body_bails(b))
                || else_body.as_ref().is_some_and(|eb| block_body_bails(eb))
        }
        Stmt::Match { arms, .. } => arms.iter().any(|a| block_body_bails(&a.body)),
        Stmt::Block(b) => block_body_bails(b),
        Stmt::Try { body, handlers, finally_body } => {
            block_body_bails(body)
                || handlers.iter().any(|h| block_body_bails(&h.body))
                || finally_body.as_ref().is_some_and(|fb| block_body_bails(fb))
        }
        _ => false,
    })
}
