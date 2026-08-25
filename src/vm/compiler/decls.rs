// vm/compiler/decls.rs — **slot 採番**と、そのための AST 走査。
//
// ⚠⚠ **採番はリゾルバと同順・同数**（`push_base` される名前は `slots` に入れなくても
// 必ず 1 slot 消費する）。ずれると `LoadLocal` が範囲外を読む。
// ⚠ **`compile_stmt` に文種別を足したら必ず `collect_nested_decls` も見る**（#27-c で 2 回踏んだ）。


use std::collections::{HashMap, HashSet};

use crate::ast::{
    Expr, Param, Stmt, TupleTarget,
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
    // 部分式の構造は 1 箇所（#81）。⚠ **`_ => {}` を書かない** — `SubPart` に
    // 種類が増えるとここが止まり、「この walker ではどう扱うか」を決めさせられる。
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) => scan_shadow_expr(x, for_names, decl_names),
            P::Body(b) => scan_shadow_stmts(b, for_names, decl_names),
            // `for` ターゲットは**衝突候補**として集める（この walker 固有の判断）。
            P::ForTarget(t) => {
                for_names.insert(t.to_string());
            }
            // ⚠ パターンは宣言を含まない（#81 以前も見ていない）。
            P::MatchPattern(_) => {}
        }
    });
}

/// slot テーブルへ1つ宣言を追加する（既出名・`_` はスキップ）。
///
/// ⚠ `ty` は `Option<&String>`（#59）。[`crate::decl_names::each_declared_name`] が
/// この形で型注釈を渡すので、変換を挟まずそのまま流せるようにしてある。
pub(super) fn add_decl(
    name: &str,
    ty: Option<&String>,
    mutable: bool,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    if name != "_" && !slots.contains_key(name) {
        slots.insert(name.to_string(), *n);
        slot_mut.push(mutable);
        slot_type.push(ty.cloned());
        *n = n.checked_add(1)?;
    }
    Some(())
}

/// `stmt` が**直接束縛する名前**のうち、VM がフレーム slot を持たせるものだけ採番する（#59）。
///
/// 束縛の一覧は [`crate::decl_names::each_declared_name`] が持つ唯一の定義から取り、
/// ここが決めるのは「**その種類に slot を振るか**」だけ。
/// ⚠ `DeclOrigin` が増えるとこの `match` が壊れる — それが #59 の仕掛けなので
/// `_ => {}` を足して黙らせないこと（#68 の実バグはこの強制が無かったせい）。
///
/// ⚠ **`for` ターゲットと `except ... as` の別名はここでは振らない**。
/// `collect_nested_decls` の `Try` アームは「try 本体 → 別名 → handler 本体」の順に振るので、
/// ここで先に振ると **slot 番号がずれる**（`decl_names` のモジュール doc 参照）。
fn declare_stmt_slots(
    stmt: &Stmt,
    slots: &mut HashMap<String, u16>,
    slot_mut: &mut Vec<bool>,
    slot_type: &mut Vec<Option<String>>,
    n: &mut u16,
) -> Option<()> {
    // ⚠ クロージャからは `?` で抜けられないので、slot 数の溢れはフラグで持ち帰る。
    let mut overflow = false;
    crate::decl_names::each_declared_name(stmt, &mut |name, origin, ty| {
        use crate::decl_names::DeclOrigin as D;
        let mutable = match origin {
            D::Let | D::TupleLet => false,
            D::Mut | D::TupleMut => true,
            // 入れ子ブロック（`block:` 式・if/while の本体など）の `fn` / `enum`（#27-c / #68）。
            // 名前は**そのブロックのローカル**なので slot を振る（関数本体直下の定義を
            // base slot に採番するのと同じ扱い）。⚠ **本体には踏み込まない**（別フレーム）。
            // ⚠ `fn` が抜けていたため `alias.ar` の `block->function` が、`enum` が抜けていたため
            // `if:` の中の `enum` が、それぞれ「slot にもグローバルにも無い識別子」として
            // bail していた — **採番 walker と `compile_stmt` の walker がずれていた**典型例。
            D::Fn | D::Enum => false,
            // ⚠ **slot を振らないもの**（振っても使われない or 振ってはいけない）:
            // - `static mut` は記憶域が `Interpreter::static_cells`（span がキー）で slot を持たない
            // - `gen`/`class`/`trait`/`protocol`/`new_type`/`import` は `compile_stmt` に
            //   アームが無く、到達すれば bail する
            D::Static
            | D::Gen
            | D::Class
            | D::Trait
            | D::Protocol
            | D::NewType
            | D::Import
            | D::FromImport => return,
        };
        if add_decl(name, ty, mutable, slots, slot_mut, slot_type, n).is_none() {
            overflow = true;
        }
    });
    if overflow {
        None
    } else {
        Some(())
    }
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
        // ① この 1 文が直接束縛する名前に slot を振る（判断は [`crate::decl_names`]・#59）。
        //
        // ⚠⚠ **②より先に呼ぶこと。** 従来の採番順は「その文の宣言 → 部分式 → 本体」で、
        // `each_declared_name` は宣言順に報告するので、この順序でのみ **slot 番号が一致する**
        // （ずれると `LoadLocal` が別の変数を読む — プランの「採番はリゾルバと同順・同数」）。
        declare_stmt_slots(stmt, slots, slot_mut, slot_type, n)?;

        // ② どこへ降りるか＋入れ子スコープの束縛（**この walker 固有**）。
        match stmt {
            // 初期化式の中のブロック式にも宣言がありうる。名前は①が振り済み。
            Stmt::Let(_, _, e) | Stmt::Const(_, _, e) | Stmt::Mut(_, _, e) => {
                collect_expr_decls(e, slots, slot_mut, slot_type, n)?;
            }
            // 入れ子の `let a, b = t`（#27-c）。ツリーウォークはスコープを push するので
            // **反復ごとに宣言し直せる**が、slot を割り当てないとグローバル宣言に落ちて
            // 2 周目で「already declared」になる（`built_in.ar` の `zx` が実例）。
            // ここで slot に載せることで、最上位の `let a, b = t` 文（slot 無し）と
            // 入れ子（slot 有り）をコンパイラが区別できる。
            Stmt::LetTuple { value, .. } => {
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
                    add_decl(t, None, true, slots, slot_mut, slot_type, n)?;
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
                        add_decl(alias, None, false, slots, slot_mut, slot_type, n)?;
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
    // 部分式の構造は 1 箇所（#81）。⚠ **`_ => {}` を書かない**。
    //
    // ⚠⚠ **列挙の順序＝採番の順序**。`each_subpart` は `ForExpr` を
    // `target` → `iter` → `body` の順に返す（#81 以前の手書きと同じ順序）。
    // ここを崩すと `LoadLocal` が別の変数を読む（プランの「採番はリゾルバと同順・同数」）。
    //
    // ⚠ `add_decl` は slot が溢れると `None` を返して**打ち切る**。クロージャからは `?` で
    // 抜けられないので、`ok` が落ちたら以降は**何もしない**（＝ 以後 slot を増やさない）。
    // #81 以前の `?` による早期 return と同じ効果。
    let mut ok = Some(());
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        if ok.is_none() {
            return;
        }
        ok = match part {
            P::Plain(x) | P::Control(x) => collect_expr_decls(x, slots, slot_mut, slot_type, n),
            P::Body(b) => collect_nested_decls(b, slots, slot_mut, slot_type, n),
            // `for` ターゲットはこの式が宣言する名前（この walker 固有の判断）。
            P::ForTarget(t) => add_decl(t, None, true, slots, slot_mut, slot_type, n),
            // ⚠⚠ パターンへは**降りない**。降りるとパターン内のブロック式が slot を取り、
            // **採番がずれる**（#81 以前も見ていない。挙動不変を優先）。
            P::MatchPattern(_) => Some(()),
        };
    });
    ok
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
