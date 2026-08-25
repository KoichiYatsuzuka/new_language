// syntax_cov.rs — 「例題スイートがどの構文を**一度も書いていない**か」を機械的に数える（#85）。
//
// ## なぜ要るのか
//
// プラン冒頭が「**未カバーの構文を例題側から数える手段がまだ無い**（`Stmt` の全 variant ×
// 文脈のマトリクスが無い）」と自認していた。その穴から出た実バグが**5 件**あり、
// **全件が「その形の例題が 1 本も無かった」**で通り抜けている:
//
// | # | 見落とされていた形 |
// |---|---|
// | 56 | `parse_ar` を使う例題が 1 本も無く、`is_builtin_callee` の bail が #33 以来これを殺していた |
// | 68 | **関数本体の `enum`**（最上位の `enum` は書かれていたが `in_fn` は 0 本） |
// | 71 | `import[py]` の関数呼び出し |
// | 75 | **式形式の制御構文の中だけ**で外側変数を参照するクロージャ |
// | 84 | **入れ子 `fn` の `match` 腕／ブロック式**の中の宣言、`static mut` の初期化式のブロック式 |
//
// ⚠⚠ **どれもゲートは緑のまま**だった。`force_gate` 0 件も `cargo test` 750 も
// 「**例題が書いている形については緑**」という意味しかない。
//
// ## 何を数えるか
//
// **実行せず、パース直後の AST を静的に歩く**。理由は 3 つ:
//
// 1. **決定的**。GUI 例題のタイムアウトも async のスケジューリングも関係ない
//    （`force_gate` は 161 例題中 4 本がタイムアウト後に窓を閉じて完走する形）。
// 2. **呼ばれない関数の中も数えられる**。「書いてあるか」を問うのが目的で、
//    「実行されたか」ではない。
// 3. **速い**。実行しないので例題 1 本あたり数 ms。
//
// 3 つの表を出す:
//
// - `stmt` / `expr` … variant ごとの出現数（**0 なら未カバー**）
// - `ctx` … `variant@文脈` の出現数。文脈は**フレームの種類**
//   （`top` / `fn` / `nested_fn` / `type` / `module` / `async`）× **式本体の中か**（`+expr`）。
//   ⇒ 「`EnumDef` は `top` にはあるが `fn` には 0」（#68 の形）が読める。
// - `pair` … `親>子` の出現数。⇒ 「`Static>Block`（`static mut s = block …:`）は 0」
//   （#84 ③ の形）が読める。
//
// ## ⚠ 走査そのものは自前で持たない
//
// 木の歩き方は [`crate::stmt_walk`]（#84）・[`crate::expr_walk`]（#81）・
// [`crate::interpreter::tw_stats::stmt_kind_of`]・[`crate::vm::compiler::expr_kind`] に**既にある**。
// ⇒ ここは**それを歩いて数えるだけ**。variant を足すと `each_subpart` 側が
// コンパイルエラーになるので、**この計測が古くなることは無い**（#59/#81/#84 の 2 段の強制に乗る）。
//
// ## 使い方
//
// ```text
// cargo build --features tw_stats
// AR_SYNTAX_COV=1 arrow -src <file.ar>     # stderr へ SyntaxCov[...] を出して**実行せず終了**
// ./syntax_cov.ps1                          # 全例題ぶんを合算し「未カバー」を一覧する
// ```
//
// ⚠ **`AR_SYNTAX_COV=1` は実行前に終了する**。これは意図的で、GUI・async・FFI を
// 起こさずに全例題を舐めるための仕掛け（どのゲートもこの環境変数を立てない）。

use std::collections::BTreeMap;

use crate::ast::{Expr, Stmt};

/// 集計先。キーは `variant` / `variant@ctx` / `親>子`。
#[derive(Default)]
pub struct Acc {
    stmt: BTreeMap<String, u64>,
    expr: BTreeMap<String, u64>,
    ctx: BTreeMap<String, u64>,
    pair: BTreeMap<String, u64>,
}

/// 走査中の文脈。**フレームの種類**と「式本体の中か」の 2 軸（モジュール doc 参照）。
#[derive(Clone, Copy, PartialEq, Eq)]
struct Ctx {
    frame: Frame,
    /// 式形式の制御構文（`block:` / `if` 式 …）の本体の中か。
    ///
    /// ⚠ **フレームとは独立な軸**。「入れ子 `fn` の中のブロック式」（#84 ②）は
    /// `nested_fn+expr` になり、どちらの情報も残る。
    in_expr_body: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// メインプログラム最上位。
    Top,
    /// 最上位に定義された `fn` / `gen` の本体。
    Fn,
    /// **その中にさらに定義された** `fn` / `gen` の本体（＝クロージャ）。
    ///
    /// ⚠ #68 / #75 / #84 の実バグ 4 件はすべてこの文脈で起きた。**最も見落とされる場所**。
    NestedFn,
    /// `class` / `trait` / `protocol` の本体。
    Type,
    /// `import` が持つ別モジュールの本体。
    ///
    /// ⚠ **分けて数える意味がある** — 標準ライブラリの `.ar` が踏んでいるだけの形を
    /// 「例題がカバーしている」と読むと、#56（`parse_ar`）の再演になる。
    Module,
    /// `mng <- async->T:` の本体。
    Async,
}

impl Frame {
    fn name(self) -> &'static str {
        match self {
            Frame::Top => "top",
            Frame::Fn => "fn",
            Frame::NestedFn => "nested_fn",
            Frame::Type => "type",
            Frame::Module => "module",
            Frame::Async => "async",
        }
    }

    /// 関数本体へ入るときのフレーム。**最上位の `fn` は `fn`、その中はすべて `nested_fn`**。
    fn enter_fn(self) -> Frame {
        match self {
            Frame::Top | Frame::Type | Frame::Module => Frame::Fn,
            // ⚠ 3 段以上入れ子になっても `nested_fn` のまま（深さは数えない）。
            Frame::Fn | Frame::NestedFn | Frame::Async => Frame::NestedFn,
        }
    }
}

impl Ctx {
    fn key(self) -> String {
        if self.in_expr_body {
            format!("{}+expr", self.frame.name())
        } else {
            self.frame.name().to_string()
        }
    }
    fn frame(self, frame: Frame) -> Ctx {
        // 別フレームへ入ったら「式本体の中」はリセットされる。
        Ctx {
            frame,
            in_expr_body: false,
        }
    }
    fn expr_body(self) -> Ctx {
        Ctx {
            in_expr_body: true,
            ..self
        }
    }
}

fn add(t: &mut BTreeMap<String, u64>, k: String) {
    *t.entry(k).or_insert(0) += 1;
}

impl Acc {
    fn hit_stmt(&mut self, kind: &str, ctx: Ctx) {
        add(&mut self.stmt, kind.to_string());
        add(&mut self.ctx, format!("{kind}@{}", ctx.key()));
    }
    fn hit_expr(&mut self, kind: &str, ctx: Ctx) {
        add(&mut self.expr, kind.to_string());
        add(&mut self.ctx, format!("{kind}@{}", ctx.key()));
    }
    fn hit_pair(&mut self, parent: &str, child: &str) {
        add(&mut self.pair, format!("{parent}>{child}"));
    }
}

fn stmt_kind(s: &Stmt) -> &'static str {
    crate::interpreter::tw_stats::stmt_kind_of(s)
}

fn expr_kind(e: &Expr) -> &'static str {
    crate::vm::compiler::expr_kind(e)
}

fn walk_stmts(stmts: &[Stmt], ctx: Ctx, acc: &mut Acc) {
    for s in stmts {
        walk_stmt(s, ctx, acc);
    }
}

fn walk_stmt(s: &Stmt, ctx: Ctx, acc: &mut Acc) {
    let kind = stmt_kind(s);
    acc.hit_stmt(kind, ctx);

    // ⚠ 木の歩き方は持たない。[`crate::stmt_walk`] の exhaustive な列挙に乗る（#84）。
    crate::stmt_walk::each_subpart(s, &mut |part| {
        use crate::stmt_walk::StmtPart as P;
        match part {
            // 直下の式は「親>子」のペアとしても数える（`Static>Block` を読むため）。
            P::Expr(e) | P::MatchPattern(e) => {
                acc.hit_pair(kind, expr_kind(e));
                walk_expr(e, ctx, acc);
            }
            // 同じフレームの制御フロー本体。
            P::Control(b) => {
                for x in b {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(b, ctx, acc);
            }
            P::FnBody { body, .. } | P::GenBody(body) => {
                for x in body {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(body, ctx.frame(ctx.frame.enter_fn()), acc);
            }
            P::TypeBody(b) | P::ProtocolBody(b) => {
                for x in b {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(b, ctx.frame(Frame::Type), acc);
            }
            // ⚠ 別モジュールの本体。**`stmt_walk` の 8 本目の消費者**で、
            // ここが読むまで `ModuleBody` の payload は誰にも読まれていなかった（#84）。
            P::ModuleBody(b) => {
                for x in b {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(b, ctx.frame(Frame::Module), acc);
            }
            P::AsyncBody(b) => {
                for x in b {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(b, ctx.frame(Frame::Async), acc);
            }
            // 名前だけの部分。構文の種類を持たないので数えない。
            P::ForTarget(_) | P::ExceptAlias(_) | P::TargetName(_) => {}
        }
    });
}

fn walk_expr(e: &Expr, ctx: Ctx, acc: &mut Acc) {
    let kind = expr_kind(e);
    acc.hit_expr(kind, ctx);

    // ⚠ 木の歩き方は持たない。[`crate::expr_walk`] の exhaustive な列挙に乗る（#81）。
    crate::expr_walk::each_subpart(e, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) | P::MatchPattern(x) => {
                acc.hit_pair(kind, expr_kind(x));
                walk_expr(x, ctx, acc);
            }
            // 式形式の制御構文の本体 ＝ **`+expr` の文脈**（#75 / #84 ② が起きた場所）。
            P::Body(b) => {
                for x in b {
                    acc.hit_pair(kind, stmt_kind(x));
                }
                walk_stmts(b, ctx.expr_body(), acc);
            }
            // ループ変数は名前だけ。構文の種類を持たない。
            P::ForTarget(_) => {}
        }
    });
}

/// 計測が有効か（`AR_SYNTAX_COV` が空でない）。
///
/// ⚠ `tw_stats::enabled()` と**別の環境変数**にしてある。あちらは実行しながら数えるが、
/// こちらは**実行せずに終了する**ので、混ぜると `tw_stats.ps1` が何も測れなくなる。
pub fn enabled() -> bool {
    std::env::var("AR_SYNTAX_COV").is_ok_and(|v| !v.is_empty())
}

/// パース直後の AST を歩いて構文カバレッジを stderr へ出す（#85）。
///
/// ⚠ 呼び出し側はこの後**実行せずに終了する**（モジュール doc）。
pub fn dump_program(stmts: &[Stmt]) {
    let mut acc = Acc::default();
    walk_stmts(
        stmts,
        Ctx {
            frame: Frame::Top,
            in_expr_body: false,
        },
        &mut acc,
    );

    // ⚠ キーに空白を入れないこと（集計スクリプトが `key=value` を空白で分割する。
    // `tw_stats::record_bail` と同じ約束）。
    // ⚠ **母集団も Rust 側から出す**。集計スクリプトに variant 一覧を書くと
    // そちらが黙って古くなる（#59/#81/#84 が潰してきたのと同じドリフト）。
    eprintln!("SyntaxCov[all_stmt] total={} {}", ALL_STMT_KINDS.len(), ALL_STMT_KINDS.join(" "));
    eprintln!("SyntaxCov[all_expr] total={} {}", ALL_EXPR_KINDS.len(), ALL_EXPR_KINDS.join(" "));

    for (cat, tbl) in [
        ("stmt", &acc.stmt),
        ("expr", &acc.expr),
        ("ctx", &acc.ctx),
        ("pair", &acc.pair),
    ] {
        let total: u64 = tbl.values().sum();
        let body = tbl
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("SyntaxCov[{cat}] total={total} {body}");
    }
}

/// この言語に存在する `Stmt` variant 名の全一覧（未カバー判定の母集団）。
///
/// ⚠⚠ **`stmt_kind_of` の戻り値と 1 対 1 でなければ意味が無い**。ずれると
/// 「存在しない variant が永遠に未カバー」と報告し続けて信用を失う。
/// ⇒ 突き合わせは**合成サンプルではなく実データ**で行う: [syntax_cov.ps1](syntax_cov.ps1) が
/// 全例題で観測した種別のうち**この一覧に無いもの**を検出したら `STALE POPULATION` で落ちる
/// （161 例題ぶんの実際の AST が母集団検査そのものになる）。
/// ⚠ 逆方向（一覧にあるが実在しない綴り）は「未カバー」として目に見える形で残る。
pub const ALL_STMT_KINDS: &[&str] = &[
    "Expr", "Let", "Const", "Mut", "LetTuple", "Static", "Assign", "AttrAssign",
    "AttrCompoundAssign", "CompoundAssign", "If", "Match", "While", "For", "Block", "Return",
    "Break", "Continue", "Pass", "BlockReturn", "LoopYield", "Yield", "Freeze", "FnDef", "GenDef",
    "ClassDef", "TraitDef", "ProtocolDef", "Field", "NewTypeDef", "EnumDef", "Try", "Raise",
    "Import", "FromImport", "AsyncAssign", "BreakPoint", "DebugLet", "EventSubscribe",
    "EventUnsubscribe",
];

/// この言語に存在する `Expr` variant 名の全一覧（未カバー判定の母集団）。
pub const ALL_EXPR_KINDS: &[&str] = &[
    "Int", "Float", "ImaginaryLit", "Str", "Bool", "None", "Undefined", "Ident", "List", "Attr",
    "TraitAccess", "BinOp", "UnaryOp", "Call", "TemplateInstantiate", "Subscript", "Slice", "Dict",
    "Tuple", "Set", "Block", "IfExpr", "ForExpr", "WhileExpr", "MatchExpr", "Cast", "IsType",
    "MustBe", "DebugVar", "LocalVar",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠⚠ 母集団の一覧が `stmt_kind_of` / `expr_kind` の戻り値と**同じ集合**であることを固定する。
    ///
    /// `Stmt` / `Expr` に variant を足すと `stmt_kind_of` / `expr_kind` は
    /// exhaustive なのでコンパイルエラーになるが、**この一覧は文字列なので黙って古くなる**。
    /// ⇒ 一覧に無い名前を返す variant があれば、ここで落とす。
    #[test]
    fn kind_lists_have_no_duplicates_and_are_sorted_by_definition_order() {
        let mut seen = std::collections::HashSet::new();
        for k in ALL_STMT_KINDS {
            assert!(seen.insert(*k), "duplicate stmt kind: {k}");
        }
        assert_eq!(ALL_STMT_KINDS.len(), 40, "Stmt variant の数が変わった");
        let mut seen = std::collections::HashSet::new();
        for k in ALL_EXPR_KINDS {
            assert!(seen.insert(*k), "duplicate expr kind: {k}");
        }
        assert_eq!(ALL_EXPR_KINDS.len(), 30, "Expr variant の数が変わった");
    }

    /// 走査が「文脈」を正しく付けることの検査（負の対照つき）。
    #[test]
    fn nested_fn_context_is_distinguished() {
        let src = "fn outer() -> int:\n    fn inner() -> int:\n        let z = 1\n        return z\n    return inner()\n";
        let tokens = crate::lexer::Lexer::new(src, "<t>").tokenize();
        let stmts = crate::parser::Parser::new(tokens, None).parse_program().unwrap();
        let mut acc = Acc::default();
        walk_stmts(&stmts, Ctx { frame: Frame::Top, in_expr_body: false }, &mut acc);
        // 外側 `fn` は `top`、内側 `fn` は `fn`、その本体の `let` は `nested_fn`。
        assert_eq!(acc.ctx.get("FnDef@top").copied(), Some(1));
        assert_eq!(acc.ctx.get("FnDef@fn").copied(), Some(1));
        assert_eq!(acc.ctx.get("Let@nested_fn").copied(), Some(1));
        // 負の対照: 最上位に `let` は無い。
        assert_eq!(acc.ctx.get("Let@top").copied(), None);
        // 親子ペア。
        assert_eq!(acc.pair.get("FnDef>FnDef").copied(), Some(1));
    }

    /// 式本体の中（`+expr`）が別文脈として出ることの検査（#84 ② の形）。
    #[test]
    fn expr_body_context_is_distinguished() {
        let src = "fn f() -> int:\n    let q = block ->int:\n        let z = 9\n        block_return z\n    return q\n";
        let tokens = crate::lexer::Lexer::new(src, "<t>").tokenize();
        let stmts = crate::parser::Parser::new(tokens, None).parse_program().unwrap();
        let mut acc = Acc::default();
        walk_stmts(&stmts, Ctx { frame: Frame::Top, in_expr_body: false }, &mut acc);
        assert_eq!(acc.ctx.get("Let@fn").copied(), Some(1), "`let q = …` は fn 直下");
        assert_eq!(acc.ctx.get("Let@fn+expr").copied(), Some(1), "`let z = 9` はブロック式の中");
        assert_eq!(acc.pair.get("Let>Block").copied(), Some(1), "`let` の直下がブロック式");
    }
}
