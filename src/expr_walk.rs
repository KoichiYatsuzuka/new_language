// expr_walk.rs — 「この式の**直下**には何がぶら下がっているか」の**唯一の定義**（#81）。
//
// ## なぜ要るのか
//
// 同じ `match expr` を複製して部分式を辿る walker が **4 本**あり、いずれも `_ => {}` で
// 終わっていた。`Expr` に variant を足しても**何も強制されない**ので、足した人が 4 本すべてを
// 直したかどうかは誰にも分からなかった。
//
// 実際にずれた（**#75 の実バグ**）:
// - `exec::collect_refs_expr` が `Set` / `Block` / `IfExpr` / `ForExpr` / `WhileExpr` /
//   `MatchExpr` / `Cast` / `MustBe` を落としていた
// - 結果、**式形式の制御構文の中だけで外側変数を参照するクロージャが `NameError`**
//   （正しいプログラムが 6 形で動かない。参照実装との差分で確定）
//
// #59（[`crate::decl_names`]）が `Stmt` の「束縛される名前」に対してやったことを、
// `Expr` の「直下に何があるか」に対してやるのがこのモジュール。
//
// ## どう強制するか（**この 2 段が仕掛けの全部**）
//
// 1. [`each_subpart`] の `match expr` は **exhaustive**（`_` を書かない）。
//    ⇒ `Expr` に variant を足すと**まずここがコンパイルエラーになる**。
// 2. [`SubPart`] を消費側は**すべて exhaustive match** で受ける。
//    ⇒ 部分式の種類を足した瞬間、**全 walker がコンパイルエラーになり**、
//    「この walker ではどう扱うのか」を必ず決めさせられる。
//
// ⚠ **`_ => {}` を書き足してこの仕掛けを無効化しないこと。** 降りないなら
// **降りない理由を書いてバリアントを列挙する**。
//
// ## ⚠ ここが答えないこと（意図的に walker 側へ残した差）
//
// - **文の本体へどう降りるか。** [`SubPart::Body`] として**渡すだけ**で、降りるかどうか・
//   どの関数で降りるかは walker が決める。降り方は walker ごとに本当に違う
//   （`collect_bound_names` は入れ子定義の本体まで／`collect_refs_expr` は参照だけ）。
//   これは #59 が `decl_names` で下したのと同じ判断。
// - **`ForExpr` の `target` をどう扱うか。** [`SubPart::ForTarget`] として渡すだけ。
//   **束縛であって参照ではなく、しかも採番順に意味がある**（プランの「採番はリゾルバと
//   同順・同数」）。⇒ 「拾う／拾わない」は walker が決める。
//
// ## ⚠⚠ 列挙の順序を変えないこと
//
// slot の採番はこの順序に依存する（`collect_expr_decls` がここを歩いて `add_decl` する）。
// 並べ替えると **`LoadLocal` が別の変数を読む**。

use crate::ast::{CallArg, Expr, Stmt};

/// 式の直下にぶら下がっているものの種類（#81）。**消費側は必ず exhaustive に match する。**
///
/// ⚠ ここに 1 つ足すと**全消費者がコンパイルエラーになる**。それが狙いなので、
/// 消費側で `_ => {}` を書いて黙らせないこと。
pub enum SubPart<'a> {
    /// 純粋な部分式（演算子の項・引数・要素・添字…）。
    /// **式を辿る walker は基本的にそのまま降りる**。
    Plain(&'a Expr),
    /// 式形式の制御構文が持つ**式**（`if`/`while` の条件・`match` の subject・`for` の iter）。
    /// 値としては [`SubPart::Plain`] と同じだが、**本体（`Body`）と対になっている**ことが分かる。
    Control(&'a Expr),
    /// 式形式の制御構文が持つ**文の本体**。⚠ **降り方は walker ごとに違う**。
    Body(&'a [Stmt]),
    /// `ForExpr` の `target`。⚠ **参照ではなく束縛**で、採番順に意味がある。
    ForTarget(&'a str),
    /// `match` 式アームの `case <expr>` パターン。
    ///
    /// ⚠⚠ **参照を集める walker だけが降りる**（#75 でクロージャ捕捉のために追加した経路）。
    /// 宣言・slot 採番の walker は**降りない** — 降りるとパターン内のブロック式が
    /// slot を取り、**採番がずれる**。⇒ 「降りる／降りない」は walker が決める。
    MatchPattern(&'a Expr),
}

/// `expr` の直下の部分を**出現順に**列挙する（#81）。
///
/// ⚠⚠ `match` は **exhaustive**。`Expr` に variant を足すとここが止まる。
/// ⚠⚠ **順序を変えない**（slot 採番が依存している）。
pub fn each_subpart(expr: &Expr, f: &mut impl FnMut(SubPart<'_>)) {
    match expr {
        // ── 式形式の制御構文（式と本体が対になる） ──
        Expr::Block { stmts, .. } => f(SubPart::Body(stmts)),
        Expr::IfExpr {
            branches,
            else_body,
            ..
        } => {
            for (cond, body) in branches {
                f(SubPart::Control(cond));
                f(SubPart::Body(body));
            }
            if let Some(body) = else_body {
                f(SubPart::Body(body));
            }
        }
        Expr::MatchExpr { subject, arms, .. } => {
            f(SubPart::Control(subject));
            for arm in arms {
                // ⚠ パターン → 本体 の順（#75 の `collect_refs_expr` と同じ順序）。
                if let crate::ast::MatchPattern::Case(pat) = &arm.pattern {
                    f(SubPart::MatchPattern(pat));
                }
                f(SubPart::Body(&arm.body));
            }
        }
        // ⚠ `target` → `iter` → `body` の順を変えないこと（採番が依存）。
        Expr::ForExpr {
            target, iter, body, ..
        } => {
            f(SubPart::ForTarget(target));
            f(SubPart::Control(iter));
            f(SubPart::Body(body));
        }
        Expr::WhileExpr { cond, body, .. } => {
            f(SubPart::Control(cond));
            f(SubPart::Body(body));
        }

        // ── 純粋な部分式を持つもの ──
        Expr::BinOp { left, right, .. } => {
            f(SubPart::Plain(left));
            f(SubPart::Plain(right));
        }
        Expr::UnaryOp { operand, .. } => f(SubPart::Plain(operand)),
        Expr::Call { func, args, .. } => {
            f(SubPart::Plain(func));
            for arg in args {
                match arg {
                    CallArg::Positional(x) | CallArg::Keyword { value: x, .. } => {
                        f(SubPart::Plain(x))
                    }
                    CallArg::Variadic(xs) => {
                        for x in xs {
                            f(SubPart::Plain(x));
                        }
                    }
                }
            }
        }
        Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } => f(SubPart::Plain(object)),
        Expr::Subscript { object, index, .. } => {
            f(SubPart::Plain(object));
            f(SubPart::Plain(index));
        }
        Expr::Slice { begin, end, step } => {
            for x in [begin, end, step].into_iter().flatten() {
                f(SubPart::Plain(x));
            }
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for x in items {
                f(SubPart::Plain(x));
            }
        }
        Expr::Dict(pairs) => {
            for (k, v) in pairs {
                f(SubPart::Plain(k));
                f(SubPart::Plain(v));
            }
        }
        Expr::TemplateInstantiate { base, .. } => f(SubPart::Plain(base)),
        Expr::IsType { expr, .. } | Expr::MustBe { expr, .. } => f(SubPart::Plain(expr)),
        Expr::Cast { object, .. } => f(SubPart::Plain(object)),

        // ── ここから下は「直下に何も持たない」バリアント。
        //    ⚠ 理由を消さずに残すこと（`_ => {}` にしない・#59/#75/#81）──
        // リテラル。
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::ImaginaryLit(_)
        | Expr::Str(_)
        | Expr::Bool(_)
        | Expr::None
        | Expr::Undefined => {}
        // 名前そのもの。部分式を持たない（名前をどう扱うかは walker の仕事）。
        Expr::Ident { .. } | Expr::DebugVar(_) | Expr::LocalVar(_) => {}
    }
}
