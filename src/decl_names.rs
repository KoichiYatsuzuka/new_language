// decl_names.rs — 「この文はその場のスコープにどの名前を束縛するか」の**唯一の定義**（#59）。
//
// ## なぜ要るのか
//
// 同じ木を歩いて「宣言される名前」を集める walker が **6 本**あり、それぞれが
// 同じ `match stmt` を複製したうえ `_ => {}` で落としていた。`Stmt` に variant を足しても
// **何も強制されない**ので、足した人が 6 本すべてを直したかどうかは誰にも分からなかった。
//
// 実際にずれた:
// - #27-c が `EnumDef` を `resolver::collect_program_globals` に**だけ**足した
// - #68 で `exec::collect_declared_names` の抜けが**実バグとして発火**した
//   （関数本体の `enum` を宣言するクロージャがその名前を自由変数と見なし、
//   コンパイラが採番した slot とぶつかって `capture-slot-conflict` で bail）
//
// ## どう強制するか（**この 2 段が仕掛けの全部**）
//
// 1. [`each_declared_name`] の `match stmt` は **exhaustive**（`_` を書かない）。
//    ⇒ `Stmt` に variant を足すと**まずここがコンパイルエラーになる**。
// 2. [`DeclOrigin`] を消費側は**すべて exhaustive match** で受ける。
//    ⇒ 束縛する variant だと分類した瞬間、**全 walker がコンパイルエラーになり**、
//    「この walker では拾うのか・拾わないのか」を必ず決めさせられる。
//
// ⚠ **`_ => {}` を書き足してこの仕掛けを無効化しないこと。** 拾わないなら
// **拾わない理由を書いてバリアントを列挙する**（それが #59 で得た唯一のもの）。
//
// ## ⚠ ここが答えないこと（意図的に walker 側へ残した差）
//
// - **本体・部分式へどう降りるか。** 降り方は walker ごとに本当に違う
//   （`collect_program_globals` は最上位だけ／`collect_bound_names` は入れ子定義の本体まで）。
// - **`for` ターゲットと `except ... as` の別名。** これらは「その場のスコープ」ではなく
//   **入れ子スコープ**の束縛で、しかも**採番順が walker ごとに違う**。
//   ⚠⚠ 特に `vm::compiler::decls::collect_nested_decls` の `Try` アームは
//   「try 本体 → 別名 → handler 本体」の順に slot を振る。ここで別名を先に報告すると
//   **slot 番号がずれて `LoadLocal` が別の変数を読む**（プランの「採番はリゾルバと同順・同数」）。
//   ⇒ 順序に意味がある束縛はここに載せない。
//
// ## ⚠ 統合しなかった walker と、その理由
//
// | walker | 統合しない理由 |
// |---|---|
// | `resolver::collect_base_decls` | 答える問いが違う（「**この文を理解できるか**」で、未知の文は `false` を返して解決を諦める）。**既定が安全側**なので取りこぼしてもバグにならない |
// | `vm::compiler::decls::scan_shadow_stmts` | 集めるのは `for` ターゲット**との衝突候補**で、定義文（`fn`/`class`…）を意図的に入れない。上記の「順序に意味がある束縛」側 |
// | `resolver::collect_shadowing_binders` | 自前の判断を持たず `collect_bound_names` に委譲するだけ（既に 1 本） |
// | `exec::collect_referenced_names` | **参照**を集める walker で、宣言とは逆向きの問い |

use crate::ast::{Stmt, TupleTarget};

/// 束縛の出どころ（#59）。**消費側は必ず exhaustive に match する。**
///
/// ⚠ ここに 1 つ足すと**全消費者がコンパイルエラーになる**。それが狙いなので、
/// エラーを消すために `_ => {}` を足してはいけない。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeclOrigin {
    /// `let x = …` / `const x = …`（不変）。
    Let,
    /// `mut x = …`（可変）。
    Mut,
    /// `static mut x = …`（可変・記憶域は `Interpreter::static_cells`）。
    Static,
    /// `let a, b = t` の要素のうち `let` / 束縛のみ（不変）。
    TupleLet,
    /// `let a, mut b = t` の `mut` 要素（可変）。
    TupleMut,
    /// `fn f(…)` 定義。
    Fn,
    /// `gen g(…)` 定義。
    Gen,
    /// `class C` 定義。
    Class,
    /// `trait T` 定義。
    Trait,
    /// `protocol P` 定義。
    Protocol,
    /// `enum E` 定義（#68 で実バグを出した所）。
    Enum,
    /// `new_type N = …` 定義。
    NewType,
    /// `import[lang] a.b as c` の束縛名。
    Import,
    /// `from a.b import[lang] X as Y` の各束縛名。
    FromImport,
}

/// `stmt` が**その場のスコープに直接束縛する名前**を、**宣言順に**列挙する（#59）。
///
/// 引数の `f` は `(名前, 出どころ, 型注釈)` を受け取る。型注釈は `let`/`mut`/`static` のみ
/// `Some`（タプル要素・定義文・import は常に `None`）。
///
/// ⚠ **本体・部分式には降りない。** 降り方は walker 固有なので呼び出し側が決める。
/// ⚠ **`for` ターゲットと `except ... as` の別名は報告しない**（モジュール doc の理由）。
///
/// ⚠⚠ **下の `match` に `_` を足さないこと。** `Stmt` に variant を足したとき
/// ここが壊れることだけが「6 本の walker を見直す」きっかけになる。
pub fn each_declared_name(stmt: &Stmt, f: &mut impl FnMut(&str, DeclOrigin, Option<&String>)) {
    match stmt {
        Stmt::Let(name, ty, _) | Stmt::Const(name, ty, _) => f(name, DeclOrigin::Let, ty.as_ref()),
        Stmt::Mut(name, ty, _) => f(name, DeclOrigin::Mut, ty.as_ref()),
        // ⚠ `static mut` は slot を持たない（記憶域は `Interpreter::static_cells`）。
        // 「束縛する」ことは確かなので報告し、slot を振るかは消費側が決める。
        Stmt::Static(name, _, _) => f(name, DeclOrigin::Static, None),
        Stmt::LetTuple { targets, .. } => {
            for t in targets {
                match t {
                    TupleTarget::Let(n) | TupleTarget::Bare(n) => f(n, DeclOrigin::TupleLet, None),
                    TupleTarget::Mut(n) => f(n, DeclOrigin::TupleMut, None),
                    TupleTarget::Wildcard => {}
                }
            }
        }
        Stmt::FnDef { name, .. } => f(name, DeclOrigin::Fn, None),
        Stmt::GenDef { name, .. } => f(name, DeclOrigin::Gen, None),
        Stmt::ClassDef { name, .. } => f(name, DeclOrigin::Class, None),
        Stmt::TraitDef { name, .. } => f(name, DeclOrigin::Trait, None),
        Stmt::ProtocolDef { name, .. } => f(name, DeclOrigin::Protocol, None),
        Stmt::EnumDef { name, .. } => f(name, DeclOrigin::Enum, None),
        Stmt::NewTypeDef { name, .. } => f(name, DeclOrigin::NewType, None),

        // ⚠ 既定の束縛名は `alias` か**モジュールパスの末尾**。
        // cpp 系の実際の束縛名はヘッダのファイル stem（`Interpreter::import_bind_name`・#58）で、
        // ここと**食い違っている**。#59 では既存の挙動をそのまま保存した（変えるとバイトコードが動く）。
        Stmt::Import { module, alias, .. } => {
            if let Some(b) = alias.as_ref().or_else(|| module.last()) {
                f(b, DeclOrigin::Import, None);
            }
        }
        Stmt::FromImport { names, .. } => {
            for (orig, alias) in names {
                f(alias.as_ref().unwrap_or(orig), DeclOrigin::FromImport, None);
            }
        }

        // --- ここから下は「その場のスコープに名前を束縛しない」文 ---
        //
        // ⚠ **1 つずつ列挙してある**（`_ => {}` を書かないため）。`Stmt` に variant を
        // 足したらコンパイルエラーになるので、束縛するかどうかをここで必ず分類すること。
        //
        // ⚠ `For` / `Try` は**束縛するが報告しない**（`for` ターゲット・`except as` の別名は
        // 入れ子スコープで、採番順が walker ごとに違う ⇒ モジュール doc の「答えないこと」）。
        Stmt::For { .. } | Stmt::Try { .. } => {}
        // 制御フロー・式・代入は名前を作らない（`Assign` は既存の束縛への代入）。
        Stmt::Expr(_)
        | Stmt::Assign { .. }
        | Stmt::AttrAssign { .. }
        | Stmt::AttrCompoundAssign { .. }
        | Stmt::CompoundAssign { .. }
        | Stmt::If { .. }
        | Stmt::Match { .. }
        | Stmt::While { .. }
        | Stmt::Block(_)
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Pass
        | Stmt::BlockReturn(..)
        | Stmt::LoopYield(_)
        | Stmt::Yield(_)
        | Stmt::Freeze(..)
        | Stmt::Raise { .. }
        | Stmt::BreakPoint { .. }
        | Stmt::EventSubscribe { .. }
        | Stmt::EventUnsubscribe { .. } => {}
        // `mng <- async->T: body`。⚠ **`target` は束縛ではない**（`exec_async_assign` は
        // `get_var(target)` するだけで新しい名前を作らない）。束縛扱いすると `mng` が
        // シャドウ候補になり `Resolution::Global` が付かず VM が bail する（実測 18 件・#27-c）。
        Stmt::AsyncAssign { .. } => {}
        // クラス本体のフィールド宣言。作るのは**インスタンスのフィールド**であって
        // スコープの名前ではない。
        Stmt::Field { .. } => {}
        // デバッガ REPL の `let dbg::x`。束縛先は `DebugState::vars` で、
        // 通常のスコープではない（walker はどれもこの文を見ない）。
        Stmt::DebugLet(..) => {}
    }
}
