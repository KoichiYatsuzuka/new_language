// resolver.rs — Phase R / R1: ローカル読み取りの slot 解決パス。
//
// 型検査後・実行前に **メインプログラムのトップレベル関数** を走査し、確実に
// 「関数 base スコープ（実行時は `scopes[frame_floor]`）」へ解決できる `Expr::Ident` を
// `Expr::LocalRef { name, slot }` に書き換える。実行時はスコープ遡り＋文字列ハッシュが
// 配列 index 1回に置き換わる（[eval/core.rs] の `eval_local_ref`）。
//
// ## なぜ base スコープが `scopes[frame_floor]` に決まるのか
// `exec_fn_evaled` は呼び出しのたびに `frame_floor = scopes.len()` を記録して base を push する。
// よって関数実行中、その関数の base スコープは必ず `scopes[frame_floor]` に来る（0=グローバル）。
// 入れ子ブロック（if/for/while/block/match）はさらに `frame_floor+1..` に積まれる。
// slot 番号（宣言順）は base スコープ内の相対 index なので frame_floor の値に依存しない。
//
// ## 健全性（保守的解決）
// - 解決対象はメインプログラム直下の `fn` / `gen` 定義のみ（メソッド=クラス内、入れ子関数、
//   テンプレート関数は対象外）。これらは capture_env が空になることが保証され、
//   base スコープの宣言順が AST から決定的に定まる。
// - base 宣言順 = [仮引数（宣言順）] + [本体直下の宣言（Let/Const/Mut/LetTuple/Static および
//   入れ子定義の名前）を出現順]。`bind_args` は仮引数を宣言順で束縛する（[functions/args.rs]）。
// - シャドウイング禁止（型検査が保証）なので、base スコープに宣言された名前はその関数内で一意。
//   したがって本体のどこで読んでも同じ base slot を指す（入れ子ブロックの深さは無関係）。
// - 本体直下に「未対応の宣言的文（import 等）」があれば、その関数の解決を丸ごと諦める（bail）。
// - 入れ子定義（fn/class/…）の本体・デコレータ・デフォルト式には踏み込まない
//   （キャプチャ変数は名前引きのまま＝正しさは保たれ、最適化されないだけ）。
//
// デバッグビルドでは `eval_local_ref` が slot と名前の一致を検証するため、
// 解決ロジックのずれがテストで即座に露見する。

use std::collections::{HashMap, HashSet};

use crate::ast::{CallArg, Expr, ExceptHandler, MatchArm, MatchPattern, Param, Stmt, TupleTarget};

/// メインプログラムのトップレベル文列を走査し、解決可能な関数本体を書き換える。
pub(crate) fn resolve_program(stmts: &mut [Stmt]) {
    // プログラム最上位で宣言される名前（＝グローバル）を先に集める（R2-b）。
    // 型検査が**グローバル名の再宣言を禁止**している（入れ子ブロックでも
    // `variable 'x' is already declared in an accessible scope`）ため、
    // ここに載った名前は関数内でシャドウされないと保証できる。
    let globals = collect_program_globals(stmts);
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::FnDef {
                template_params,
                params,
                body,
                ..
            }
            | Stmt::GenDef {
                template_params,
                params,
                body,
                ..
            } if template_params.is_empty() => {
                resolve_function(params, body, &globals);
            }
            // クラスのメソッド・入れ子定義・モジュール本体は対象外（別途拡張予定）。
            _ => {}
        }
    }
}

/// プログラム最上位で宣言される名前を集める（R2-b の `GlobalRef` 判定用）。
fn collect_program_globals(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stmts {
        match stmt {
            Stmt::Let(n, _, _)
            | Stmt::Const(n, _, _)
            | Stmt::Mut(n, _, _)
            | Stmt::Static(n, _, _) => {
                out.insert(n.clone());
            }
            Stmt::LetTuple { targets, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                            out.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
            }
            Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::ProtocolDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::Import { module, alias, .. } => {
                let bind = alias.clone().or_else(|| module.last().cloned());
                if let Some(b) = bind {
                    out.insert(b);
                }
            }
            Stmt::FromImport { names, .. } => {
                for (orig, alias) in names {
                    out.insert(alias.clone().unwrap_or_else(|| orig.clone()));
                }
            }
            _ => {}
        }
    }
    out
}

/// 関数本体の**どこかで束縛される名前**をすべて集める（R2-b の安全条件）。
///
/// `base`（引数＋本体直下の宣言）だけでは足りない。**for ループのターゲットは
/// `base` に入らないのにグローバルをシャドウできる**（型検査は `let`/`mut` の再宣言は
/// 禁じるが、for ターゲットは許す）。実際 `collection.ar` の
/// `let x = {1,2,3}`（最上位・set）を `fn sum_ints` の `for x in items:` が覆っており、
/// これを見落として `GlobalRef` に書き換えたところ set を読んで落ちた。
///
/// そこで入れ子ブロック・ブロック式の中まで含め、束縛の可能性がある名前を保守的に集め、
/// **1 つでも該当したらその名前は `GlobalRef` にしない**。
fn collect_bound_names(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Let(n, _, e) | Stmt::Const(n, _, e) | Stmt::Mut(n, _, e) => {
                out.insert(n.clone());
                collect_bound_in_expr(e, out);
            }
            Stmt::Static(n, e, _) => {
                out.insert(n.clone());
                collect_bound_in_expr(e, out);
            }
            Stmt::LetTuple { targets, value, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                            out.insert(n.clone());
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
                collect_bound_in_expr(value, out);
            }
            Stmt::For { targets, iter, body } => {
                for t in targets {
                    out.insert(t.clone());
                }
                collect_bound_in_expr(iter, out);
                collect_bound_names(body, out);
            }
            Stmt::AsyncAssign { target, stmts, .. } => {
                out.insert(target.clone());
                collect_bound_names(stmts, out);
            }
            Stmt::FnDef { name, body, .. }
            | Stmt::GenDef { name, body, .. }
            | Stmt::ClassDef { name, body, .. }
            | Stmt::TraitDef { name, body, .. } => {
                out.insert(name.clone());
                collect_bound_names(body, out);
            }
            Stmt::ProtocolDef { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::If { branches, else_body } => {
                for (c, b) in branches {
                    collect_bound_in_expr(c, out);
                    collect_bound_names(b, out);
                }
                if let Some(b) = else_body {
                    collect_bound_names(b, out);
                }
            }
            Stmt::While { cond, body } => {
                collect_bound_in_expr(cond, out);
                collect_bound_names(body, out);
            }
            Stmt::Block(b) => collect_bound_names(b, out),
            Stmt::Match { subject, arms, .. } => {
                collect_bound_in_expr(subject, out);
                for a in arms {
                    collect_bound_names(&a.body, out);
                }
            }
            Stmt::Try { body, handlers, finally_body } => {
                collect_bound_names(body, out);
                for h in handlers {
                    if let Some(n) = &h.name {
                        out.insert(n.clone());
                    }
                    collect_bound_names(&h.body, out);
                }
                if let Some(f) = finally_body {
                    collect_bound_names(f, out);
                }
            }
            Stmt::Expr(e)
            | Stmt::LoopYield(e)
            | Stmt::Yield(e)
            | Stmt::BlockReturn(e, _) => collect_bound_in_expr(e, out),
            Stmt::Return(Some(e)) => collect_bound_in_expr(e, out),
            Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => {
                collect_bound_in_expr(value, out)
            }
            _ => {}
        }
    }
}

/// 式の内部（ブロック式・for 式など）に現れる束縛も集める。
fn collect_bound_in_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Block { stmts, .. } => collect_bound_names(stmts, out),
        Expr::IfExpr { branches, else_body, .. } => {
            for (c, b) in branches {
                collect_bound_in_expr(c, out);
                collect_bound_names(b, out);
            }
            if let Some(b) = else_body {
                collect_bound_names(b, out);
            }
        }
        Expr::ForExpr { target, iter, body, .. } => {
            out.insert(target.clone());
            collect_bound_in_expr(iter, out);
            collect_bound_names(body, out);
        }
        Expr::WhileExpr { cond, body, .. } => {
            collect_bound_in_expr(cond, out);
            collect_bound_names(body, out);
        }
        Expr::MatchExpr { subject, arms, .. } => {
            collect_bound_in_expr(subject, out);
            for a in arms {
                collect_bound_names(&a.body, out);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_bound_in_expr(left, out);
            collect_bound_in_expr(right, out);
        }
        Expr::UnaryOp { operand, .. } => collect_bound_in_expr(operand, out),
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            collect_bound_in_expr(object, out)
        }
        Expr::Call { func, args, .. } => {
            collect_bound_in_expr(func, out);
            for a in args {
                match a {
                    CallArg::Positional(e) | CallArg::Keyword { value: e, .. } => {
                        collect_bound_in_expr(e, out)
                    }
                    CallArg::Variadic(es) => {
                        for e in es {
                            collect_bound_in_expr(e, out);
                        }
                    }
                }
            }
        }
        Expr::Subscript { object, index, .. } => {
            collect_bound_in_expr(object, out);
            collect_bound_in_expr(index, out);
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for e in items {
                collect_bound_in_expr(e, out);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                collect_bound_in_expr(k, out);
                collect_bound_in_expr(v, out);
            }
        }
        Expr::Cast { object, .. } => collect_bound_in_expr(object, out),
        Expr::MustBe { expr, .. } | Expr::IsType { expr, .. } => collect_bound_in_expr(expr, out),
        _ => {}
    }
}

/// 単一関数の base スコープを解決して本体の読み取りを書き換える。
fn resolve_function(params: &[Param], body: &mut [Stmt], globals: &HashSet<String>) {
    // 可変長パラメータがあると base の並びが `local::args` を含んで複雑になるため諦める。
    if params.iter().any(|p| p.variadic) {
        return;
    }

    // base 宣言順を組み立てる: まず仮引数、続いて本体直下の宣言。
    let mut order: Vec<String> = Vec::new();
    for p in params {
        push_base(&p.name, &mut order);
    }
    if !collect_base_decls(body, &mut order) {
        return; // 未対応の宣言的文を検出 → この関数は解決しない
    }

    let base: HashMap<String, u32> = order
        .into_iter()
        .enumerate()
        .map(|(i, n)| (n, i as u32))
        .collect();
    if base.is_empty() {
        return;
    }

    // for ターゲット等、`base` に載らない束縛がグローバルを覆うことがあるので差し引く（R2-b）。
    let mut bound: HashSet<String> = HashSet::new();
    for p in params {
        bound.insert(p.name.clone());
    }
    collect_bound_names(body, &mut bound);
    let visible_globals: HashSet<String> =
        globals.difference(&bound).cloned().collect();

    rewrite_stmts(body, &base, &visible_globals);
}

/// base スコープに名前を追加する（`_` と重複は無視）。slot 番号は追加順。
fn push_base(name: &str, order: &mut Vec<String>) {
    if name == "_" {
        return;
    }
    if !order.iter().any(|n| n == name) {
        order.push(name.to_string());
    }
}

/// 本体の直下（base スコープ）に宣言される名前を出現順に収集する。
///
/// 未対応の宣言的文（import 等、base に名前を導入しうるが並びを保証できない文）を見つけたら
/// `false` を返して関数全体の解決を諦めさせる。純粋に非宣言的な文は無視して先へ進む。
fn collect_base_decls(body: &[Stmt], order: &mut Vec<String>) -> bool {
    for stmt in body {
        match stmt {
            // --- base に名前を導入する文 ---
            Stmt::Let(n, _, _) | Stmt::Const(n, _, _) | Stmt::Mut(n, _, _) => push_base(n, order),
            Stmt::Static(n, _, _) => push_base(n, order),
            Stmt::LetTuple { targets, .. } => {
                for t in targets {
                    match t {
                        TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                            push_base(n, order)
                        }
                        TupleTarget::Wildcard => {}
                    }
                }
            }
            // 入れ子定義は現スコープに「名前」を束縛する（本体には踏み込まない）。
            Stmt::FnDef { name, .. }
            | Stmt::GenDef { name, .. }
            | Stmt::ClassDef { name, .. }
            | Stmt::TraitDef { name, .. }
            | Stmt::ProtocolDef { name, .. }
            | Stmt::NewTypeDef { name, .. }
            | Stmt::EnumDef { name, .. } => push_base(name, order),

            // --- base に名前を導入しない文（無視して継続） ---
            Stmt::Expr(_)
            | Stmt::Assign { .. }
            | Stmt::AttrAssign { .. }
            | Stmt::AttrCompoundAssign { .. }
            | Stmt::CompoundAssign { .. }
            | Stmt::If { .. }
            | Stmt::Match { .. }
            | Stmt::While { .. }
            | Stmt::For { .. }
            | Stmt::Block(_)
            | Stmt::Try { .. }
            | Stmt::Return(_)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Pass
            | Stmt::BlockReturn(_, _)
            | Stmt::LoopYield(_)
            | Stmt::Yield(_)
            | Stmt::Freeze(_, _)
            | Stmt::Raise { .. }
            | Stmt::BreakPoint { .. } => {}

            // 上記以外（import / from-import / async / event 等）は base への影響が
            // 読み切れないため、この関数の解決自体を諦める。
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Rewrite: Ident → LocalRef（base 名のみ）。入れ子定義には踏み込まない。
// ---------------------------------------------------------------------------

fn rewrite_stmts(body: &mut [Stmt], base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    for stmt in body.iter_mut() {
        rewrite_stmt(stmt, base, globals);
    }
}

fn rewrite_stmt(stmt: &mut Stmt, base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    match stmt {
        Stmt::Expr(e)
        | Stmt::Let(_, _, e)
        | Stmt::Const(_, _, e)
        | Stmt::Mut(_, _, e)
        | Stmt::Static(_, e, _)
        | Stmt::LoopYield(e)
        | Stmt::Yield(e)
        | Stmt::BlockReturn(e, _) => rewrite_expr(e, base, globals),

        Stmt::LetTuple { value, .. } => rewrite_expr(value, base, globals),

        // 代入先の変数名は String フィールドで Expr::Ident ではないため書き換え対象外。
        // 右辺（読み取り）のみ書き換える。
        Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => rewrite_expr(value, base, globals),

        Stmt::AttrAssign { target, value } => {
            rewrite_expr(target, base, globals);
            rewrite_expr(value, base, globals);
        }
        Stmt::AttrCompoundAssign { target, value, .. } => {
            rewrite_expr(target, base, globals);
            rewrite_expr(value, base, globals);
        }

        Stmt::If {
            branches,
            else_body,
        } => {
            for (cond, b) in branches.iter_mut() {
                rewrite_expr(cond, base, globals);
                rewrite_stmts(b, base, globals);
            }
            if let Some(eb) = else_body {
                rewrite_stmts(eb, base, globals);
            }
        }
        Stmt::Match { subject, arms, .. } => {
            rewrite_expr(subject, base, globals);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, base, globals);
            }
        }
        Stmt::While { cond, body } => {
            rewrite_expr(cond, base, globals);
            rewrite_stmts(body, base, globals);
        }
        Stmt::For { iter, body, .. } => {
            // ループ変数 target は入れ子スコープの名前（base ではない）ため、
            // 本体内でのその読み取りは base に無いので書き換わらない（正しい）。
            rewrite_expr(iter, base, globals);
            rewrite_stmts(body, base, globals);
        }
        Stmt::Block(b) => rewrite_stmts(b, base, globals),
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => {
            rewrite_stmts(body, base, globals);
            for h in handlers.iter_mut() {
                rewrite_except_handler(h, base, globals);
            }
            if let Some(fb) = finally_body {
                rewrite_stmts(fb, base, globals);
            }
        }
        Stmt::Return(Some(e)) => rewrite_expr(e, base, globals),
        Stmt::Raise { exc: Some(e), .. } => rewrite_expr(e, base, globals),

        // 入れ子定義には踏み込まない（キャプチャは名前引きのまま＝正しさ維持）。
        // その他（import/async/event/leaf 文など）も書き換え不要。
        _ => {}
    }
}

fn rewrite_match_arm(arm: &mut MatchArm, base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    if let MatchPattern::Case(e) = &mut arm.pattern {
        rewrite_expr(e, base, globals);
    }
    rewrite_stmts(&mut arm.body, base, globals);
}

fn rewrite_except_handler(h: &mut ExceptHandler, base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    rewrite_stmts(&mut h.body, base, globals);
}

fn rewrite_call_arg(arg: &mut CallArg, base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    match arg {
        CallArg::Positional(e) | CallArg::Keyword { value: e, .. } => rewrite_expr(e, base, globals),
        CallArg::Variadic(exprs) => {
            for e in exprs.iter_mut() {
                rewrite_expr(e, base, globals);
            }
        }
    }
}

fn rewrite_expr(expr: &mut Expr, base: &HashMap<String, u32>,
    globals: &HashSet<String>) {
    match expr {
        Expr::Ident { name, node_id } => {
            // node_id は書き換え先へ引き継ぐ（#15b）。型検査はリゾルバより前に走るので、
            // ここで落とすと解決済み参照から注釈を引けなくなる。
            let node_id = *node_id;
            if let Some(&slot) = base.get(name) {
                *expr = Expr::LocalRef {
                    name: name.clone(),
                    slot,
                    node_id,
                };
            } else if globals.contains(name) {
                // base（引数＋本体直下宣言）に無く、最上位で宣言された名前 → グローバル確定。
                // 型検査がグローバル名の再宣言を禁じているのでシャドウの心配はない。
                *expr = Expr::GlobalRef {
                    name: name.clone(),
                    cache: crate::ast::SlotCache::default(),
                    node_id,
                };
            }
        }

        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            rewrite_expr(object, base, globals)
        }
        Expr::BinOp { left, right, .. } => {
            rewrite_expr(left, base, globals);
            rewrite_expr(right, base, globals);
        }
        Expr::UnaryOp { operand, .. } => rewrite_expr(operand, base, globals),
        Expr::Call { func, args, .. } => {
            rewrite_expr(func, base, globals);
            for a in args.iter_mut() {
                rewrite_call_arg(a, base, globals);
            }
        }
        Expr::TemplateInstantiate { base: b, .. } => rewrite_expr(b, base, globals),
        Expr::Subscript { object, index, .. } => {
            rewrite_expr(object, base, globals);
            rewrite_expr(index, base, globals);
        }
        Expr::Slice { begin, end, step } => {
            if let Some(e) = begin {
                rewrite_expr(e, base, globals);
            }
            if let Some(e) = end {
                rewrite_expr(e, base, globals);
            }
            if let Some(e) = step {
                rewrite_expr(e, base, globals);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for e in items.iter_mut() {
                rewrite_expr(e, base, globals);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs.iter_mut() {
                rewrite_expr(k, base, globals);
                rewrite_expr(v, base, globals);
            }
        }
        Expr::Block { stmts, .. } => rewrite_stmts(stmts, base, globals),
        Expr::IfExpr {
            branches,
            else_body,
            ..
        } => {
            for (cond, b) in branches.iter_mut() {
                rewrite_expr(cond, base, globals);
                rewrite_stmts(b, base, globals);
            }
            if let Some(eb) = else_body {
                rewrite_stmts(eb, base, globals);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            rewrite_expr(iter, base, globals);
            rewrite_stmts(body, base, globals);
        }
        Expr::WhileExpr { cond, body, .. } => {
            rewrite_expr(cond, base, globals);
            rewrite_stmts(body, base, globals);
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rewrite_expr(subject, base, globals);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, base, globals);
            }
        }
        Expr::IsType { expr: e, .. }
        | Expr::MustBe { expr: e, .. }
        | Expr::Cast { object: e, .. } => rewrite_expr(e, base, globals),

        // リテラル・leaf（Int/Float/Str/Bool/None/Undefined/DebugVar/LocalVar/
        // LocalRef/ImaginaryLit）は書き換え不要。
        _ => {}
    }
}
