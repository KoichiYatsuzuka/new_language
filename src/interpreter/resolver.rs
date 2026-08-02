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

use std::collections::HashMap;

use crate::ast::{CallArg, Expr, ExceptHandler, MatchArm, MatchPattern, Param, Stmt, TupleTarget};

/// メインプログラムのトップレベル文列を走査し、解決可能な関数本体を書き換える。
pub(crate) fn resolve_program(stmts: &mut [Stmt]) {
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
                resolve_function(params, body);
            }
            // クラスのメソッド・入れ子定義・モジュール本体は対象外（別途拡張予定）。
            _ => {}
        }
    }
}

/// 単一関数の base スコープを解決して本体の読み取りを書き換える。
fn resolve_function(params: &[Param], body: &mut [Stmt]) {
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

    rewrite_stmts(body, &base);
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

fn rewrite_stmts(body: &mut [Stmt], base: &HashMap<String, u32>) {
    for stmt in body.iter_mut() {
        rewrite_stmt(stmt, base);
    }
}

fn rewrite_stmt(stmt: &mut Stmt, base: &HashMap<String, u32>) {
    match stmt {
        Stmt::Expr(e)
        | Stmt::Let(_, _, e)
        | Stmt::Const(_, _, e)
        | Stmt::Mut(_, _, e)
        | Stmt::Static(_, e, _)
        | Stmt::LoopYield(e)
        | Stmt::Yield(e)
        | Stmt::BlockReturn(e, _) => rewrite_expr(e, base),

        Stmt::LetTuple { value, .. } => rewrite_expr(value, base),

        // 代入先の変数名は String フィールドで Expr::Ident ではないため書き換え対象外。
        // 右辺（読み取り）のみ書き換える。
        Stmt::Assign { value, .. } | Stmt::CompoundAssign { value, .. } => rewrite_expr(value, base),

        Stmt::AttrAssign { target, value } => {
            rewrite_expr(target, base);
            rewrite_expr(value, base);
        }
        Stmt::AttrCompoundAssign { target, value, .. } => {
            rewrite_expr(target, base);
            rewrite_expr(value, base);
        }

        Stmt::If {
            branches,
            else_body,
        } => {
            for (cond, b) in branches.iter_mut() {
                rewrite_expr(cond, base);
                rewrite_stmts(b, base);
            }
            if let Some(eb) = else_body {
                rewrite_stmts(eb, base);
            }
        }
        Stmt::Match { subject, arms, .. } => {
            rewrite_expr(subject, base);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, base);
            }
        }
        Stmt::While { cond, body } => {
            rewrite_expr(cond, base);
            rewrite_stmts(body, base);
        }
        Stmt::For { iter, body, .. } => {
            // ループ変数 target は入れ子スコープの名前（base ではない）ため、
            // 本体内でのその読み取りは base に無いので書き換わらない（正しい）。
            rewrite_expr(iter, base);
            rewrite_stmts(body, base);
        }
        Stmt::Block(b) => rewrite_stmts(b, base),
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => {
            rewrite_stmts(body, base);
            for h in handlers.iter_mut() {
                rewrite_except_handler(h, base);
            }
            if let Some(fb) = finally_body {
                rewrite_stmts(fb, base);
            }
        }
        Stmt::Return(Some(e)) => rewrite_expr(e, base),
        Stmt::Raise { exc: Some(e), .. } => rewrite_expr(e, base),

        // 入れ子定義には踏み込まない（キャプチャは名前引きのまま＝正しさ維持）。
        // その他（import/async/event/leaf 文など）も書き換え不要。
        _ => {}
    }
}

fn rewrite_match_arm(arm: &mut MatchArm, base: &HashMap<String, u32>) {
    if let MatchPattern::Case(e) = &mut arm.pattern {
        rewrite_expr(e, base);
    }
    rewrite_stmts(&mut arm.body, base);
}

fn rewrite_except_handler(h: &mut ExceptHandler, base: &HashMap<String, u32>) {
    rewrite_stmts(&mut h.body, base);
}

fn rewrite_call_arg(arg: &mut CallArg, base: &HashMap<String, u32>) {
    match arg {
        CallArg::Positional(e) | CallArg::Keyword { value: e, .. } => rewrite_expr(e, base),
        CallArg::Variadic(exprs) => {
            for e in exprs.iter_mut() {
                rewrite_expr(e, base);
            }
        }
    }
}

fn rewrite_expr(expr: &mut Expr, base: &HashMap<String, u32>) {
    match expr {
        Expr::Ident(name) => {
            if let Some(&slot) = base.get(name) {
                *expr = Expr::LocalRef {
                    name: name.clone(),
                    slot,
                };
            }
        }

        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => {
            rewrite_expr(object, base)
        }
        Expr::BinOp { left, right, .. } => {
            rewrite_expr(left, base);
            rewrite_expr(right, base);
        }
        Expr::UnaryOp { operand, .. } => rewrite_expr(operand, base),
        Expr::Call { func, args, .. } => {
            rewrite_expr(func, base);
            for a in args.iter_mut() {
                rewrite_call_arg(a, base);
            }
        }
        Expr::TemplateInstantiate { base: b, .. } => rewrite_expr(b, base),
        Expr::Subscript { object, index, .. } => {
            rewrite_expr(object, base);
            rewrite_expr(index, base);
        }
        Expr::Slice { begin, end, step } => {
            if let Some(e) = begin {
                rewrite_expr(e, base);
            }
            if let Some(e) = end {
                rewrite_expr(e, base);
            }
            if let Some(e) = step {
                rewrite_expr(e, base);
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for e in items.iter_mut() {
                rewrite_expr(e, base);
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs.iter_mut() {
                rewrite_expr(k, base);
                rewrite_expr(v, base);
            }
        }
        Expr::Block { stmts, .. } => rewrite_stmts(stmts, base),
        Expr::IfExpr {
            branches,
            else_body,
            ..
        } => {
            for (cond, b) in branches.iter_mut() {
                rewrite_expr(cond, base);
                rewrite_stmts(b, base);
            }
            if let Some(eb) = else_body {
                rewrite_stmts(eb, base);
            }
        }
        Expr::ForExpr { iter, body, .. } => {
            rewrite_expr(iter, base);
            rewrite_stmts(body, base);
        }
        Expr::WhileExpr { cond, body, .. } => {
            rewrite_expr(cond, base);
            rewrite_stmts(body, base);
        }
        Expr::MatchExpr { subject, arms, .. } => {
            rewrite_expr(subject, base);
            for arm in arms.iter_mut() {
                rewrite_match_arm(arm, base);
            }
        }
        Expr::IsType { expr: e, .. }
        | Expr::MustBe { expr: e, .. }
        | Expr::Cast { object: e, .. } => rewrite_expr(e, base),

        // リテラル・leaf（Int/Float/Str/Bool/None/Undefined/DebugVar/LocalVar/
        // LocalRef/ImaginaryLit）は書き換え不要。
        _ => {}
    }
}
