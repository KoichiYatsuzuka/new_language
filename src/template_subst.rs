// template_subst.rs — テンプレートの型変数を具体型へ置き換える AST 置換（タスク 2-7）。
//
// ## なぜ独立したファイルなのか
//
// `subst_*` は **`Value` にも `Interpreter` にも触れない純粋な AST→AST 変換**。以前は
// `src/interpreter/templates.rs` の私有関数で、実行時のテンプレート具体化からしか呼べなかった。
// 単相化を展開時に行う（設計書 D35 / D36・タスク 2-8）には、展開器・型検査・wasm フロントエンドの
// すべてから**同じ置換**を呼べる必要がある。
//
// ⚠⚠ **置換の実装は 1 つだけにする。** 実行時と型検査で別の置換を書くと「実行時は通るのに
// 型検査が落ちる（逆も）」が構造的に起きる（type_check_redesign.md フェーズ10 10-2 の指摘）。
// ⚠ 2-7 は**移しただけ**（中身は同じ・A/B で確認）。2-8 以降で置換の取りこぼし（合成型の中の
//   型変数・変数の型注釈・`gen` の仮引数と産出型・`async` の戻り値型・`mustbe` の型）を直した。

use std::collections::HashMap;

use crate::ast::{CallArg, ExceptHandler, Expr, MatchArm, MatchPattern, Param, Stmt};

// ---------------------------------------------------------------------------
// AST 置換ヘルパー（テンプレート実体化用）
// ---------------------------------------------------------------------------
// `type_map` は型変数名 → 具体型名のマップ。
// `subst_*` 関数群は AST ノードを再帰的に走査し、型変数名を具体型名に書き換えた新しい AST を返す。
// コードのロジック自体は変更せず、型アノテーション部分のみを置換する。

/// 型名文字列を置換する。型注釈の中に現れる**型変数名**を具体型名に置き換える。
///
/// ⚠⚠ **合成型の中も置換する**（タスク 2-8）。以前は注釈が型変数**そのもの**（`T`）のときしか
/// 置換せず、`list[T]` / `Option[T]` / `dict[K, V]` / `Box[T]` の中の `T` がそのまま残った。
/// 実行時は注釈の大半が検査に使われないので表に出ていなかったが、単相化した宣言の中の
/// `Box[T]` が具体化されない（`Box[int]` にならない）。
/// ⚠ 置換の単位は**識別子**（英数字と `_` の並び）。`Tag` の中の `T` のような部分一致はしない。
pub(crate) fn subst_type(type_name: &str, type_map: &HashMap<String, String>) -> String {
    SEEN_ANNOTATIONS.with(|c| {
        if let Some(seen) = &mut *c.borrow_mut() {
            seen.push(type_name.to_string());
        }
    });
    if let Some(t) = type_map.get(type_name) {
        return t.clone();
    }
    let mut out = String::with_capacity(type_name.len());
    let mut ident = String::new();
    let flush = |ident: &mut String, out: &mut String| {
        if !ident.is_empty() {
            match type_map.get(ident.as_str()) {
                Some(t) => out.push_str(t),
                None => out.push_str(ident),
            }
            ident.clear();
        }
    };
    for c in type_name.chars() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
        } else {
            flush(&mut ident, &mut out);
            out.push(c);
        }
    }
    flush(&mut ident, &mut out);
    out
}

/// 具体化の名前（`Box` と `[int]` から `"Box[int]"`・タスク 2-8）。
///
/// ⚠⚠ **展開器（宣言を置く側）と実行時（登録する側）が同じ綴りを使う。** 実行時は定義した
/// 宣言の名前を [`split_instance_name`] で分けて具体化のキャッシュへ登録する（タスク 2-15）。
/// 綴りがずれると登録の鍵が式の型引数と合わず、黙って従来の実行時具体化へ戻る（二重に実体化する）。
pub(crate) fn instance_name(base: &str, type_args: &[String]) -> String {
    format!("{base}[{}]", type_args.join(", "))
}

/// 単相化で作った宣言の**実行時の表示名**（`"Box[int]"` → `"Box"`・タスク 2-8）。
///
/// ⚠ 実行時の具体化はクラス・関数にテンプレートの名前をそのまま付けていた
/// （`<Box object>`・traceback の `in ident`・型引数を省いた `Box` 注釈との照合）。
/// 束縛名は `Box[int]` でも、値の名前はそれに揃える。
pub(crate) fn display_name(name: &str) -> &str {
    match name.find('[') {
        Some(i) => &name[..i],
        None => name,
    }
}

/// [`instance_name`] の逆（`"Box[Pair[int, str], int]"` → `("Box", ["Pair[int, str]", "int"])`・タスク 2-15）。
///
/// 単相化した宣言を定義した時点で、**そのスコープのテンプレート**の具体化として登録するのに使う
/// （`Interpreter::register_mono_instance`）。具体化の名前でないときは `None`。
/// ⚠ 分けるのは**入れ子の外の** `, ` だけ（型引数そのものが `[..]` / `{..}` / `(..)` を含む）。
pub(crate) fn split_instance_name(name: &str) -> Option<(&str, Vec<String>)> {
    let open = name.find('[')?;
    let inner = name[open + 1..].strip_suffix(']')?;
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(inner[start..].trim().to_string());
    Some((&name[..open], args))
}

// ── 型注釈を見て回る（タスク 10-4） ───────────────────────────────────────

thread_local! {
    /// [`annotations_of`] の最中だけ立てる「見た型注釈」の控え。
    static SEEN_ANNOTATIONS: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// 文の中の**すべての型注釈**を返す（仮引数・戻り値・変数・フィールド・`mustbe`・`=>`・`async` …）。
///
/// ⚠⚠ 型注釈の置き場所を別に数え上げない。置換（[`subst_stmt`]）は型注釈を**すべて**置き換える
///   必要があり、その網羅は 2-8 で取りこぼしを直してある。同じ走査を空の置換表で回し、
///   [`subst_type`] に渡ってきた注釈を控える。置き場所を足したら置換の側を直せば両方に効く。
pub(crate) fn annotations_of(stmt: &Stmt) -> Vec<String> {
    SEEN_ANNOTATIONS.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let _ = subst_stmt(stmt, &HashMap::new());
    SEEN_ANNOTATIONS.with(|c| c.borrow_mut().take().unwrap_or_default())
}

// ── 写しの node-id の振り直し（タスク 2-13） ───────────────────────────────

thread_local! {
    /// 置換の最中だけ立てる「新しい node-id を振る」カウンタ（[`subst_stmt_renumbered`]）。
    static FRESH_IDS: std::cell::RefCell<Option<std::rc::Rc<std::cell::Cell<u32>>>> =
        const { std::cell::RefCell::new(None) };
}

/// 写しのノードの node-id。[`subst_stmt_renumbered`] の最中なら新しく振り、そうでなければ引き継ぐ。
fn fresh_node_id(id: u32) -> u32 {
    FRESH_IDS.with(|c| match &*c.borrow() {
        // ⚠ パーサの `next_node_id` と同じ振り方（先に 1 足してから使う）。
        Some(counter) => {
            let next = counter.get() + 1;
            counter.set(next);
            next
        }
        None => id,
    })
}

/// `subst_stmt` と同じだが、**写しのすべてのノードに新しい node-id を振る**（単相化用・タスク 2-13）。
///
/// ⚠⚠ **型検査にかける写しは node-id を共有してはならない**（設計書 §0.3）。型検査の注釈は node-id で
/// 引かれるので、`add_all[int]` と `add_all[float]` が同じ node-id を持つと、後に検査した具体化の型
/// （float の加算）が**すべての具体化に効く**（`add_all[int]` の `a + b` が `FBIN` になった・実測）。
/// ⚠ 実行時の具体化（`templates.rs`）は写しを型検査にかけないので従来どおり引き継ぐ（[`subst_stmt`]）。
/// ⚠ カウンタはプログラム全体の node-id カウンタを渡すこと（別のカウンタだと本体と衝突する）。
pub(crate) fn subst_stmt_renumbered(
    stmt: &Stmt,
    type_map: &HashMap<String, String>,
    counter: &std::rc::Rc<std::cell::Cell<u32>>,
) -> Stmt {
    FRESH_IDS.with(|c| *c.borrow_mut() = Some(std::rc::Rc::clone(counter)));
    let out = subst_stmt(stmt, type_map);
    FRESH_IDS.with(|c| *c.borrow_mut() = None);
    out
}

/// 仮引数リストの型アノテーションを置換した新しいリストを返す。
pub(crate) fn subst_params(params: &[Param], type_map: &HashMap<String, String>) -> Vec<Param> {
    params
        .iter()
        .map(|p| Param {
            name: p.name.clone(),
            mutable: p.mutable,
            type_ann: p.type_ann.as_ref().map(|t| subst_type(t, type_map)),
            default: p.default.clone(),
            variadic: p.variadic,
        })
        .collect()
}

/// 呼び出し引数の式部分を置換した新しい `CallArg` を返す。
pub(crate) fn subst_call_arg(arg: &CallArg, type_map: &HashMap<String, String>) -> CallArg {
    match arg {
        CallArg::Positional(e) => CallArg::Positional(subst_expr(e, type_map)),
        CallArg::Keyword { name, value } => CallArg::Keyword {
            name: name.clone(),
            value: subst_expr(value, type_map),
        },
        CallArg::Variadic(exprs) => {
            CallArg::Variadic(exprs.iter().map(|e| subst_expr(e, type_map)).collect())
        }
        CallArg::Spread(e) => CallArg::Spread(subst_expr(e, type_map)),
        CallArg::KwSpread(e) => CallArg::KwSpread(subst_expr(e, type_map)),
    }
}

/// 式内の型変数名を具体型名に置換した新しい `Expr` を返す。
/// リテラル・識別子などは変更せず、再帰的にサブ式を置換する。
pub(crate) fn subst_expr(expr: &Expr, type_map: &HashMap<String, String>) -> Expr {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::ImaginaryLit(_)
        | Expr::Str(_) | Expr::Bool(_) | Expr::None | Expr::Undefined => expr.clone(),
        // テンプレート関数はリゾルバの対象外なので `res` は通常 `Unresolved` だが、
        // 網羅性のためそのまま複製する。`Resolution::Global` の `SlotCache::clone` は
        // 空キャッシュを返すので、実体化ごとに解決し直される。
        // ⚠ `code:` の**地の文**はトークン列で、型注釈の置換対象にならない（未パース）。
        //   ⇒ そのまま複製する。単相化（D35）で扱うのは展開**後**の AST。
        //   ただし `<! !>` の中は普通の式なので、ここは置換して降りる。
        Expr::CodeBlock(lines) => Expr::CodeBlock(
            lines
                .iter()
                .map(|line| crate::ast::CodeLine {
                    pieces: line
                        .pieces
                        .iter()
                        .map(|p| match p {
                            crate::ast::CodePiece::Token(t) => {
                                crate::ast::CodePiece::Token(t.clone())
                            }
                            crate::ast::CodePiece::Splice(e) => {
                                crate::ast::CodePiece::Splice(subst_expr(e, type_map))
                            }
                        })
                        .collect(),
                    indent: line.indent,
                    span: line.span.clone(),
                })
                .collect(),
        ),
        // `^対象` の対象は普通の式なので、型注釈の置換はそのまま降りる。
        Expr::MetaInfo(target) => Expr::MetaInfo(Box::new(subst_expr(target, type_map))),
        Expr::Ident { name, node_id, res } =>
            Expr::Ident { name: name.clone(), node_id: fresh_node_id(*node_id), res: res.clone() },
        Expr::List(items) => Expr::List(
            items
                .iter()
                .map(|x| match x {
                    crate::ast::SeqEntry::Item(e) => {
                        crate::ast::SeqEntry::Item(subst_expr(e, type_map))
                    }
                    crate::ast::SeqEntry::Spread(e) => {
                        crate::ast::SeqEntry::Spread(subst_expr(e, type_map))
                    }
                })
                .collect(),
        ),
        Expr::Attr { object, attr, span, node_id, .. } => Expr::Attr {
            object: Box::new(subst_expr(object, type_map)),
            attr: attr.clone(),
            span: span.clone(),
            cache: Default::default(),
            node_id: fresh_node_id(*node_id),
        },
        Expr::TraitAccess {
            object,
            trait_name,
            attr,
        } => Expr::TraitAccess {
            object: Box::new(subst_expr(object, type_map)),
            trait_name: trait_name.clone(),
            attr: attr.clone(),
        },
        Expr::BinOp {
            op,
            left,
            right,
            span,
            node_id,
        } => Expr::BinOp {
            op: op.clone(),
            left: Box::new(subst_expr(left, type_map)),
            right: Box::new(subst_expr(right, type_map)),
            span: span.clone(),
            node_id: fresh_node_id(*node_id),
        },
        Expr::UnaryOp { op, operand } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(subst_expr(operand, type_map)),
        },
        Expr::Call { func, args, span, node_id, .. } => Expr::Call {
            func: Box::new(subst_expr(func, type_map)),
            args: args.iter().map(|a| subst_call_arg(a, type_map)).collect(),
            span: span.clone(),
            cache: Default::default(),
            node_id: fresh_node_id(*node_id),
        },
        Expr::TemplateInstantiate { base, type_args } => Expr::TemplateInstantiate {
            base: Box::new(subst_expr(base, type_map)),
            type_args: type_args.iter().map(|t| subst_type(t, type_map)).collect(),
        },
        Expr::Subscript { object, index, node_id } => Expr::Subscript {
            object: Box::new(subst_expr(object, type_map)),
            index: Box::new(subst_expr(index, type_map)),
            node_id: fresh_node_id(*node_id),
        },
        Expr::Slice { begin, end, step } => Expr::Slice {
            begin: begin.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
            end: end.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
            step: step.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
        },
        Expr::Dict(entries) => Expr::Dict(
            entries
                .iter()
                .map(|e| match e {
                    crate::ast::DictEntry::Pair(k, v) => crate::ast::DictEntry::Pair(
                        subst_expr(k, type_map),
                        subst_expr(v, type_map),
                    ),
                    crate::ast::DictEntry::Spread(src) => {
                        crate::ast::DictEntry::Spread(subst_expr(src, type_map))
                    }
                })
                .collect(),
        ),
        Expr::Tuple(items) => Expr::Tuple(
            items
                .iter()
                .map(|x| match x {
                    crate::ast::SeqEntry::Item(e) => {
                        crate::ast::SeqEntry::Item(subst_expr(e, type_map))
                    }
                    crate::ast::SeqEntry::Spread(e) => {
                        crate::ast::SeqEntry::Spread(subst_expr(e, type_map))
                    }
                })
                .collect(),
        ),
        Expr::IsType {
            expr,
            negated,
            type_name,
            span,
            node_id,
        } => Expr::IsType {
            expr: Box::new(subst_expr(expr, type_map)),
            negated: *negated,
            type_name: subst_type(type_name, type_map),
            span: span.clone(),
            node_id: fresh_node_id(*node_id),
        },
        Expr::Block { stmts, return_type } => Expr::Block {
            stmts: subst_stmts(stmts, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::IfExpr {
            branches,
            else_body,
            return_type,
        } => Expr::IfExpr {
            branches: branches
                .iter()
                .map(|(c, b)| (subst_expr(c, type_map), subst_stmts(b, type_map)))
                .collect(),
            else_body: else_body.as_ref().map(|b| subst_stmts(b, type_map)),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::ForExpr {
            targets,
            iter,
            body,
            return_type,
        } => Expr::ForExpr {
            targets: targets.clone(),
            iter: Box::new(subst_expr(iter, type_map)),
            body: subst_stmts(body, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::WhileExpr {
            cond,
            body,
            return_type,
        } => Expr::WhileExpr {
            cond: Box::new(subst_expr(cond, type_map)),
            body: subst_stmts(body, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::MatchExpr {
            subject,
            arms,
            return_type,
        } => Expr::MatchExpr {
            subject: Box::new(subst_expr(subject, type_map)),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern: match &arm.pattern {
                        MatchPattern::Case(e) => MatchPattern::Case(subst_expr(e, type_map)),
                        MatchPattern::IsType(t) => MatchPattern::IsType(subst_type(t, type_map)),
                    },
                    body: subst_stmts(&arm.body, type_map),
                })
                .collect(),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::Set(items) => Expr::Set(
            items
                .iter()
                .map(|x| match x {
                    crate::ast::SeqEntry::Item(e) => {
                        crate::ast::SeqEntry::Item(subst_expr(e, type_map))
                    }
                    crate::ast::SeqEntry::Spread(e) => {
                        crate::ast::SeqEntry::Spread(subst_expr(e, type_map))
                    }
                })
                .collect(),
        ),
        Expr::Cast {
            object,
            type_name,
            span,
            node_id,
        } => Expr::Cast {
            object: Box::new(subst_expr(object, type_map)),
            type_name: subst_type(type_name, type_map),
            span: span.clone(),
            node_id: fresh_node_id(*node_id),
        },
        Expr::DebugVar(name) => Expr::DebugVar(name.clone()),
        Expr::LocalVar(name) => Expr::LocalVar(name.clone()),
        Expr::MustBe { expr, guard_type, span, node_id } => Expr::MustBe {
            expr: Box::new(subst_expr(expr, type_map)),
            guard_type: subst_type(guard_type, type_map),
            span: span.clone(),
            // テンプレ実体化のクローン: node_id を引き継ぐ（テンプレ対応は #16 次段）。
            node_id: fresh_node_id(*node_id),
        },
    }
}

/// 文リスト全体を再帰的に置換した新しいリストを返す。
pub(crate) fn subst_stmts(stmts: &[Stmt], type_map: &HashMap<String, String>) -> Vec<Stmt> {
    stmts.iter().map(|s| subst_stmt(s, type_map)).collect()
}

/// 文内の型変数名を具体型名に置換した新しい `Stmt` を返す。
/// 各バリアントを再帰的に処理し、型アノテーション・式・サブ文をすべて置換する。
pub(crate) fn subst_stmt(stmt: &Stmt, type_map: &HashMap<String, String>) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(subst_expr(e, type_map)),
        // ⚠⚠ 変数の型注釈も置換する（タスク 2-8 段階 2）。以前は複製するだけで、
        //   テンプレートの本体の `let x: T = ..` が具体化しても `T` のまま残った。
        Stmt::Let(name, ann, e, sp) => Stmt::Let(
            name.clone(),
            ann.as_ref().map(|t| subst_type(t, type_map)),
            subst_expr(e, type_map),
            sp.clone(),
        ),
        Stmt::Const(name, ann, e, sp) => Stmt::Const(
            name.clone(),
            ann.as_ref().map(|t| subst_type(t, type_map)),
            subst_expr(e, type_map),
            sp.clone(),
        ),
        Stmt::Mut(name, ann, e, sp) => Stmt::Mut(
            name.clone(),
            ann.as_ref().map(|t| subst_type(t, type_map)),
            subst_expr(e, type_map),
            sp.clone(),
        ),
        Stmt::LetTuple {
            targets,
            value,
            span,
        } => Stmt::LetTuple {
            targets: targets.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::Assign { name, value, span, .. } => Stmt::Assign {
            name: name.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
            slot: Default::default(),
        },
        Stmt::AttrAssign { target, value, span } => Stmt::AttrAssign {
            target: subst_expr(target, type_map),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::AttrCompoundAssign { target, op, value, span } => Stmt::AttrCompoundAssign {
            target: subst_expr(target, type_map),
            op: op.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::CompoundAssign {
            name,
            op,
            value,
            span,
            node_id,
            ..
        } => Stmt::CompoundAssign {
            name: name.clone(),
            op: op.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
            slot: Default::default(),
            // node_id は原型から引き継ぐ（他ノードと同じ規約）。実体化後は型変数が具体型に
            // 置き換わるため注釈は原型のものを指すが、VM 側は slot 型からの導出で補う。
            node_id: fresh_node_id(*node_id),
        },
        Stmt::If {
            branches,
            else_body,
            span,
        } => Stmt::If {
            branches: branches
                .iter()
                .map(|(cond, body)| (subst_expr(cond, type_map), subst_stmts(body, type_map)))
                .collect(),
            else_body: else_body.as_ref().map(|b| subst_stmts(b, type_map)),
            span: span.clone(),
        },
        Stmt::While { cond, body, span } => Stmt::While {
            cond: subst_expr(cond, type_map),
            body: subst_stmts(body, type_map),
            span: span.clone(),
        },
        Stmt::For {
            targets,
            iter,
            body,
            span,
        } => Stmt::For {
            targets: targets.clone(),
            iter: subst_expr(iter, type_map),
            body: subst_stmts(body, type_map),
            span: span.clone(),
        },
        Stmt::Block(body) => Stmt::Block(subst_stmts(body, type_map)),
        Stmt::Return(e, sp) => Stmt::Return(e.as_ref().map(|e| subst_expr(e, type_map)), sp.clone()),
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::Pass => Stmt::Pass,
        Stmt::BlockReturn(e, span) => Stmt::BlockReturn(subst_expr(e, type_map), span.clone()),
        Stmt::LoopYield(e, sp) => Stmt::LoopYield(subst_expr(e, type_map), sp.clone()),
        Stmt::Yield(e, sp) => Stmt::Yield(subst_expr(e, type_map), sp.clone()),
        Stmt::GenDef { src,
            name,
            template_params,
            params,
            yield_type,
            body,
            access,
        } => Stmt::GenDef { src: src.clone(),
            name: name.clone(),
            template_params: template_params.clone(),
            // ⚠⚠ 仮引数と産出型も置換する（タスク 2-8 段階 2）。以前は複製するだけで、
            //   単相化した `gen take[int]` が `list[T]` の仮引数を持ったままになった（実測）。
            //   実行時の具体化は `subst_params(&tmpl.params)` を別に呼んでいたので表に出なかった。
            params: subst_params(params, type_map),
            yield_type: yield_type.as_ref().map(|t| subst_type(t, type_map)),
            body: subst_stmts(body, type_map),
            access: access.clone(),
        },
        // ⚠ メタ関数はテンプレート本体には書けない（展開はテンプレート実体化より前）。
        //   到達しないが、網羅性のために複製だけしておく。
        Stmt::MetaDecorated { decorators, spans, target } => Stmt::MetaDecorated {
            decorators: decorators.iter().map(|d| subst_expr(d, type_map)).collect(),
            spans: spans.clone(),
            target: Box::new(subst_stmt(target, type_map)),
        },
        Stmt::MetaFnDef { name, params, return_type, body, is_placing } => Stmt::MetaFnDef {
            name: name.clone(),
            params: subst_params(params, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
            body: subst_stmts(body, type_map),
            is_placing: *is_placing,
        },
        Stmt::Quote(e) => Stmt::Quote(subst_expr(e, type_map)),
        Stmt::FnDef { src,
            name,
            template_params,
            params,
            return_type,
            body,
            is_abstract,
            is_static,
            is_class_method,
            decorators,
            access,
        } => Stmt::FnDef { src: src.clone(),
            name: name.clone(),
            template_params: template_params.clone(),
            params: subst_params(params, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
            body: subst_stmts(body, type_map),
            is_abstract: *is_abstract,
            is_static: *is_static,
            is_class_method: *is_class_method,
            decorators: decorators.clone(),
            access: access.clone(),
        },
        Stmt::ClassDef { src,
            name,
            template_params,
            bases,
            base_args,
            body,
            decorators,
        } => Stmt::ClassDef { src: src.clone(),
            name: name.clone(),
            template_params: template_params.clone(),
            bases: bases.clone(),
            // ⚠ 基底 trait の型引数も置換する（タスク 9.9）。
            //   `class Outer[T](Holder[T])` を `Outer[int]` として実体化したら
            //   基底は `Holder[int]` でなければならない。
            base_args: base_args
                .iter()
                .map(|args| {
                    // ⚠ 合成型の中も置換する（`Holder[list[T]]` の `T`・タスク 2-8 段階 2）。
                    args.iter().map(|a| subst_type(a, type_map)).collect()
                })
                .collect(),
            body: subst_stmts(body, type_map),
            decorators: decorators.clone(),
        },
        Stmt::TraitDef { src,
            name,
            template_params,
            body,
        } => Stmt::TraitDef { src: src.clone(),
            name: name.clone(),
            template_params: template_params.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::ProtocolDef { name, body } => Stmt::ProtocolDef {
            name: name.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::Field { src,
            name,
            kind,
            type_ann,
            default,
            access,
        } => Stmt::Field { src: src.clone(),
            name: name.clone(),
            kind: kind.clone(),
            type_ann: subst_type(type_ann, type_map),
            default: default.as_ref().map(|e| subst_expr(e, type_map)),
            access: access.clone(),
        },
        Stmt::Freeze(name, span) => Stmt::Freeze(name.clone(), span.clone()),
        Stmt::Static(name, e, span) => {
            Stmt::Static(name.clone(), subst_expr(e, type_map), span.clone())
        }
        Stmt::NewTypeDef { name, original } => Stmt::NewTypeDef {
            name: name.clone(),
            original: subst_type(original, type_map),
        },
        Stmt::EnumDef { src, name, variants } => Stmt::EnumDef { src: src.clone(),
            name: name.clone(),
            variants: variants
                .iter()
                .map(|(vname, vexpr)| {
                    (
                        vname.clone(),
                        vexpr.as_ref().map(|e| subst_expr(e, type_map)),
                    )
                })
                .collect(),
        },
        Stmt::Try {
            body,
            handlers,
            finally_body,
            span,
        } => Stmt::Try {
            span: span.clone(),
            body: subst_stmts(body, type_map),
            handlers: handlers
                .iter()
                .map(|h| ExceptHandler {
                    exc_type: h.exc_type.clone(),
                    name: h.name.clone(),
                    body: subst_stmts(&h.body, type_map),
                })
                .collect(),
            finally_body: finally_body.as_ref().map(|b| subst_stmts(b, type_map)),
        },
        Stmt::Raise { exc, span } => Stmt::Raise {
            exc: exc.as_ref().map(|e| subst_expr(e, type_map)),
            span: span.clone(),
        },
        // Import 文は型変数置換の対象外（body はパース時に解決済み）
        Stmt::Import {
            lang,
            module,
            source_module,
            alias,
            body,
            origin,
            bind,
        } => Stmt::Import {
            lang: lang.clone(),
            module: module.clone(),
            source_module: source_module.clone(),
            alias: alias.clone(),
            body: subst_stmts(body, type_map),
            origin: origin.clone(),
            bind: bind.clone(),
        },
        Stmt::FromImport {
            lang,
            module,
            source_module,
            names,
            body,
            origin,
        } => Stmt::FromImport {
            lang: lang.clone(),
            module: module.clone(),
            source_module: source_module.clone(),
            names: names.clone(),
            body: subst_stmts(body, type_map),
            origin: origin.clone(),
        },
        Stmt::Match {
            subject,
            arms,
            span,
        } => Stmt::Match {
            subject: subst_expr(subject, type_map),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern: match &arm.pattern {
                        MatchPattern::Case(e) => MatchPattern::Case(subst_expr(e, type_map)),
                        MatchPattern::IsType(t) => MatchPattern::IsType(subst_type(t, type_map)),
                    },
                    body: subst_stmts(&arm.body, type_map),
                })
                .collect(),
            span: span.clone(),
        },
        Stmt::AsyncAssign {
            target,
            return_type,
            stmts,
        } => Stmt::AsyncAssign {
            target: target.clone(),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
            stmts: subst_stmts(stmts, type_map),
        },
        Stmt::BreakPoint { span } => Stmt::BreakPoint { span: span.clone() },
        Stmt::DebugLet(name, e) => Stmt::DebugLet(name.clone(), subst_expr(e, type_map)),
        Stmt::EventSubscribe {
            source,
            handler,
            is_once,
            is_async,
            span,
        } => Stmt::EventSubscribe {
            source: subst_expr(source, type_map),
            handler: subst_expr(handler, type_map),
            is_once: *is_once,
            is_async: *is_async,
            span: span.clone(),
        },
        Stmt::EventUnsubscribe {
            source,
            handler,
            span,
        } => Stmt::EventUnsubscribe {
            source: subst_expr(source, type_map),
            handler: subst_expr(handler, type_map),
            span: span.clone(),
        },
    }
}