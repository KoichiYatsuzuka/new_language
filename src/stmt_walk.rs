// stmt_walk.rs — 「この文の**直下**には何がぶら下がっているか」の**唯一の定義**（#84）。
//
// ## なぜ要るのか
//
// #59（[`crate::decl_names`]）が `Stmt` の「**束縛される名前**」を、
// #81（[`crate::expr_walk`]）が `Expr` の「**直下に何があるか**」を強制化した。
// **残っていたのが「文の下にどの `Vec<Stmt>` / 部分式がぶら下がるか」**で、
// どちらのモジュールも doc で**意図的に範囲外**と明記していた 3 つ目の半分がここ。
//
// 同じ `match stmt` を複製して降りる walker が **7 本**あり、うち 6 本は `_ => {}` で
// 終わっていた（7 本目 `collect_referenced_names_stmt` は #75 で exhaustive 化済み）。`Stmt` に variant を足しても**何も強制されない**ので、足した人が
// 6 本すべてを直したかどうかは誰にも分からなかった。
//
// 実際にずれていた（**#84 の起票時に 8 walker の表を突き合わせて発見・3 件とも実バグ**）:
//
// - `exec::collect_declared_names` だけが **`Stmt::Match` のアーム本体へ降りていなかった**。
//   ⇒ `match` アームの中で宣言した名前が「自前の名前」から漏れ、**自由変数として捕捉**され、
//   採番側（`collect_nested_decls` は降りる）が振った slot と衝突して
//   `capture-slot-conflict` で `VmForceError`。#68 と**同じ壊れ方**。
// - `vm::compiler::decls::collect_nested_decls` だけが **`Stmt::Static` の初期化式へ
//   降りていなかった**。⇒ `static mut s = block ->T: …` の本体宣言が slot を取れず
//   `decl-no-slot` で `VmForceError`。
// - `resolver::collect_bound_names` が `AttrAssign` / `AttrCompoundAssign` / `Raise` の
//   部分式を見ていなかった（**束縛の取りこぼしは危険側**）。
//
// ⚠⚠ **3 件とも「参照実装 `impl_python` は正しく実行できる正しいプログラム」**だった。
//
// ## どう強制するか（**この 2 段が仕掛けの全部**）
//
// 1. [`each_subpart`] の `match stmt` は **exhaustive**（`_` を書かない）。
//    ⇒ `Stmt` に variant を足すと**まずここがコンパイルエラーになる**。
// 2. [`StmtPart`] を消費側は**すべて exhaustive match** で受ける。
//    ⇒ 部分の種類を足した瞬間、**全 walker がコンパイルエラーになり**、
//    「この walker ではどう扱うのか」を必ず決めさせられる。
//
// ⚠ **`_ => {}` を書き足してこの仕掛けを無効化しないこと。** 降りないなら
// **降りない理由を書いてバリアントを列挙する**。
//
// ⚠⚠ **1 段目（列挙の網羅）はコンパイラが保証しない**（#81 で学んだ）。
// 本体の種類を細かく分けてあるのは、**walker ごとに本当に判断が違う**からで、
// 「まとめられそう」に見えても畳むと**片方の判断が黙って消える**。
//
// ## ⚠ ここが答えないこと（意図的に walker 側へ残した差）
//
// - **降りるか降りないか。** 種類だけ渡す。`Control` へは 6 本すべてが降りるが、
//   `FnBody` へ降りるのは `collect_bound_names` だけ、といった差は walker が決める。
// - **文が直接束縛する名前。** それは [`crate::decl_names`] の担当（#59）。
//   ここが渡すのは **`for` ターゲット**と **`except ... as` の別名**だけで、
//   どちらも「その場のスコープ」ではなく**入れ子スコープ**の束縛なので #59 が意図的に
//   載せなかったもの。
// - **デコレータ・仮引数の既定値・型注釈。** **どの walker も見ていない**ので載せない。
//   ⚠ 載せると `collect_nested_decls` が採番を始めてしまう（挙動が動く）。
//
// ## ⚠⚠ 列挙の順序を変えないこと
//
// slot の採番はこの順序に依存する（`collect_nested_decls` がここを歩いて `add_decl` する）。
// 特に **`For` は「ターゲット → iter → 本体」**、**`Try` は「本体 → 別名 → handler 本体 →
// finally」**。並べ替えると **`LoadLocal` が別の変数を読む**
// （プランの「採番はリゾルバと同順・同数」）。
//
// ## ⚠ 変換しなかった walker と、その理由
//
// | walker | 変換しない理由 |
// |---|---|
// | `resolver::collect_base_decls` | **そもそも降りない**（関数本体の直下しか見ない）。答える問いは「**この文を理解できるか**」で、未知の文は `false` を返して解決を諦める＝**既定が安全側** |
// | （なし） | 降りる walker 7 本はすべて変換した |

use crate::ast::{Expr, MatchPattern, Param, Stmt};

/// 文の直下にぶら下がっているものの種類（#84）。**消費側は必ず exhaustive に match する。**
///
/// ⚠ ここに 1 つ足すと**全消費者がコンパイルエラーになる**。それが狙いなので、
/// 消費側で `_ => {}` を書いて黙らせないこと。
pub enum StmtPart<'a> {
    /// 文が直接持つ部分式（初期化式・条件・添字代入の左右・`raise` の例外式…）。
    ///
    /// ⚠ **式の中にブロック式が入りうる**ので、宣言を集める walker はここへ降りる必要がある
    /// （`Stmt::Static` を落としていたのが #84 で見つかった実バグ）。
    Expr(&'a Expr),
    /// `match` アームの `case <expr>` パターン。
    ///
    /// ⚠⚠ **参照を集める walker だけが降りる**（#75）。宣言・slot 採番の walker は
    /// **降りない** — 降りるとパターン内のブロック式が slot を取り、**採番がずれる**
    /// （[`crate::expr_walk::SubPart::MatchPattern`] と同じ判断）。
    MatchPattern(&'a Expr),
    /// **同じフレーム**の制御フロー本体（`if` / `while` / `for` / `match` / `block:` / `try`）。
    /// ⇒ 宣言も参照も外側と地続きなので、**降りる walker が最も多い**。
    Control(&'a [Stmt]),
    /// `for` ターゲット。⚠ **参照ではなく束縛**で、**採番順に意味がある**。
    ForTarget(&'a str),
    /// `except E as e:` の別名。⚠ **束縛**であり、**handler 本体の直前**に来る（採番順）。
    ExceptAlias(&'a str),
    /// 文が直接**名指しする既存の名前**（`x = …` / `x += …` の左辺・`freeze x`・
    /// `mng <- async->T:` の `mng`）。
    ///
    /// ⚠ **束縛ではない** — どれも「既にある名前」を指す。参照を集める walker だけが拾い、
    /// 宣言・採番の walker は拾わない（拾うと `mng` がシャドウ候補に化ける・#27-c）。
    TargetName(&'a str),
    /// `fn` の本体 ＝ **別フレーム**（ローカルも slot も独立）。
    ///
    /// ⚠ `params` も一緒に渡す — 自由変数解析（`nested_fn_free_names`）は
    /// 「仮引数 ＋ 本体の宣言」を自前の名前とするので、**これが無いと walker 側に
    /// 2 つ目の `match stmt` が残ってしまう**。
    FnBody {
        params: &'a [Param],
        body: &'a [Stmt],
    },
    /// `gen` の本体 ＝ 別フレーム。
    ///
    /// ⚠ **`fn` と分けてある**。`nested_fn_free_names` は `fn` だけを特別扱いしており、
    /// 畳むと `gen` にも自由変数解析が走って挙動が変わる。
    /// **入れ子 `gen` は `decl-prepass:GenDef` で必ず bail する**ので現状は到達しない（実測）。
    /// ⚠ `params` を渡さないのはそのため（要るのは `fn` の自由変数解析だけ）。
    GenBody(&'a [Stmt]),
    /// `class` / `trait` の本体（メンバ定義の集まり）。
    TypeBody(&'a [Stmt]),
    /// `protocol` の本体。
    ///
    /// ⚠ **`class`/`trait` と分けてある**。中身は**シグネチャ宣言だけ**で、
    /// `collect_bound_names` は「束縛を作らない」として意図的に降りない。
    ProtocolBody(&'a [Stmt]),
    /// `import` / `from ... import` が持つ**別モジュール**の本体。
    ///
    /// ⚠ 呼び出し元のフレームとは無関係なので、**7 本すべてが降りないと決めている**。
    /// ⇒ 本体を読む消費者が 1 つも無いため `dead_code` が出る。
    /// **`allow` の範囲はこのフィールド 1 つだけ**（#51 の「属性が別の警告を食っていないか疑う」
    /// に対して、enum の 1 フィールドは他の警告を隠しようがない）。
    /// 降りる walker ができたら `allow` ごと外すこと。
    ModuleBody(#[allow(dead_code)] &'a [Stmt]),
    /// `mng <- async->T:` の本体。
    ///
    /// ⚠ 送出時に**ディープクローン**され、別チャンクとしてコンパイルされる
    /// （＝採番側は降りない）。⚠ `target` は**束縛ではない**（#27-c）。
    AsyncBody(&'a [Stmt]),
}

/// `stmt` の直下の部分を**出現順に**列挙する（#84）。
///
/// ⚠⚠ `match` は **exhaustive**。`Stmt` に variant を足すとここが止まる。
/// ⚠⚠ **順序を変えない**（slot 採番が依存している。モジュール doc 参照）。
pub fn each_subpart(stmt: &Stmt, f: &mut impl FnMut(StmtPart<'_>)) {
    use StmtPart as P;
    match stmt {
        // ── 部分式を 1 つだけ持つ文 ──
        Stmt::Expr(e)
        | Stmt::Let(_, _, e)
        | Stmt::Const(_, _, e)
        | Stmt::Mut(_, _, e)
        | Stmt::Static(_, e, _)
        | Stmt::BlockReturn(e, _)
        | Stmt::LoopYield(e)
        | Stmt::Yield(e)
        | Stmt::DebugLet(_, e) => f(P::Expr(e)),
        Stmt::LetTuple { value, .. } => f(P::Expr(value)),
        // 左辺は**既存の名前**（束縛ではない）。名前 → 右辺の順。
        Stmt::Assign { name, value, .. } | Stmt::CompoundAssign { name, value, .. } => {
            f(P::TargetName(name));
            f(P::Expr(value));
        }
        Stmt::Return(e) | Stmt::Raise { exc: e, .. } => {
            if let Some(e) = e {
                f(P::Expr(e));
            }
        }
        // ── 部分式を 2 つ持つ文（左辺 → 右辺の順）──
        Stmt::AttrAssign { target, value } | Stmt::AttrCompoundAssign { target, value, .. } => {
            f(P::Expr(target));
            f(P::Expr(value));
        }
        Stmt::EventSubscribe {
            source, handler, ..
        }
        | Stmt::EventUnsubscribe {
            source, handler, ..
        } => {
            f(P::Expr(source));
            f(P::Expr(handler));
        }

        // ── 同じフレームの制御フロー ──
        Stmt::If {
            branches,
            else_body,
        } => {
            for (cond, body) in branches {
                f(P::Expr(cond));
                f(P::Control(body));
            }
            if let Some(body) = else_body {
                f(P::Control(body));
            }
        }
        Stmt::While { cond, body } => {
            f(P::Expr(cond));
            f(P::Control(body));
        }
        // ⚠⚠ 順序は「ターゲット → iter → 本体」（`collect_nested_decls` の採番順）。
        Stmt::For {
            targets,
            iter,
            body,
        } => {
            for t in targets {
                f(P::ForTarget(t));
            }
            f(P::Expr(iter));
            f(P::Control(body));
        }
        Stmt::Match { subject, arms, .. } => {
            f(P::Expr(subject));
            for arm in arms {
                if let MatchPattern::Case(e) = &arm.pattern {
                    f(P::MatchPattern(e));
                }
                f(P::Control(&arm.body));
            }
        }
        Stmt::Block(body) => f(P::Control(body)),
        // ⚠⚠ 順序は「本体 → 別名 → handler 本体 → finally」（採番順）。
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => {
            f(P::Control(body));
            for h in handlers {
                if let Some(alias) = &h.name {
                    f(P::ExceptAlias(alias));
                }
                f(P::Control(&h.body));
            }
            if let Some(body) = finally_body {
                f(P::Control(body));
            }
        }

        // ── 別フレーム／別スコープの本体 ──
        // ⚠ `decorators` と仮引数の既定値は**意図的に列挙しない**（モジュール doc）。
        Stmt::FnDef { params, body, .. } => f(P::FnBody { params, body }),
        Stmt::GenDef { body, .. } => f(P::GenBody(body)),
        Stmt::ClassDef { body, .. } | Stmt::TraitDef { body, .. } => f(P::TypeBody(body)),
        Stmt::ProtocolDef { body, .. } => f(P::ProtocolBody(body)),
        Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => f(P::ModuleBody(body)),
        // ⚠ `target` は**束縛ではない**（#27-c）。`get_var` するだけで新しい名前を作らない。
        Stmt::AsyncAssign { target, stmts, .. } => {
            f(P::TargetName(target));
            f(P::AsyncBody(stmts));
        }

        // ── 定義文が持つ「値の式」──
        // クラス／トレイト本体のフィールド宣言の既定値。
        Stmt::Field { default, .. } => {
            if let Some(e) = default {
                f(P::Expr(e));
            }
        }
        // `enum E: A = 1, B = 2` の初期化式。
        Stmt::EnumDef { variants, .. } => {
            for (_, e) in variants {
                if let Some(e) = e {
                    f(P::Expr(e));
                }
            }
        }

        // ── 直下に何も持たない文。⚠ 理由を消さずに列挙すること（#59 / #75 / #81 と同じ方針）──
        // 制御を移すだけの文。
        Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::BreakPoint { .. } => {}
        // 既存の名前を指すだけ（可変性を降格する）。部分式も本体も持たない。
        Stmt::Freeze(name, _) => f(P::TargetName(name)),
        // 型の別名定義。原型は**型名の文字列**であって式ではない。
        Stmt::NewTypeDef { .. } => {}
    }
}
