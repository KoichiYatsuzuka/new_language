// resolver.rs — Phase R / R1: ローカル読み取りの slot 解決パス。
//
// 型検査後・実行前に **メインプログラムのトップレベル関数** を走査し、確実に
// 「関数 base スコープ（実行時は `scopes[frame_floor]`）」へ解決できる `Expr::Ident` を
// `Expr::Ident` の `res` へ `Resolution::Local(slot)` を書く。実行時はスコープ遡り＋文字列ハッシュが
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

use crate::ast::{CallArg, Expr, ExceptHandler, MatchArm, MatchPattern, Param, Stmt, TupleTarget, Resolution};

/// **VM が要求する解決情報を 1 箇所で揃える**（#88）。
///
/// ## なぜ要るのか
///
/// プランが明記しているとおり **VM は「解決情報が揃っている」前提**（#3/#36）で、
/// `resolve_program` ＋ 型検査の注釈 ＋ 最上位グローバル集合を供給しない入口では
/// **正しいコードでも `VmForceError` になる**。ところがその配線は **入口ごとの手写し**だった。
///
/// #88 で 5 入口を実測したところ、**理由の無い差が 2 つ**あった:
///
/// | 差 | 実測 | 判定 |
/// |---|---|---|
/// | **順序** | `run_program`/`--compile` は「型検査 → 解決」、REPL / テストヘルパーは「解決 → 型検査」 | ⚠ **理由なし**（型検査は [`crate::ast::Resolution`] を**一切読まない**と確認）⇒ ここへ畳んだ |
/// | **型検査の入口** | 2 種あった（片方は警告を落としただけの逐語コピー） | ⚠ #88 で [`check_program`](crate::type_check::TypeChecker::check_program) **1 本へ削除統合**した |
///
/// ## ⚠ ここで畳まないもの（**実測して理由があると分かった差**・#72 と同じ判断）
///
/// - **型エラーをどうするか**: `run_program` は停止／`--compile` は `exit(1)`／
///   REPL は**無視して続行**／テストヘルパーは**無視**（注釈だけ欲しい・型エラーを
///   意図的に踏むテストがある）。⇒ **戻り値で返して呼び出し側に決めさせる**。
/// - **グローバル集合の入れ方**: REPL だけ `extend_toplevel_globals`
///   （ブロックを跨いで積み増さないと後のブロックの代入が VM に載らない）。
///   ⇒ `toplevel_declared_globals` は呼び出し側が呼ぶ。
/// - **`--compile` は `Interpreter` を持たない**（ネイティブ codegen は注釈だけ消費する）。
///
/// ## ⚠ 供給しない入口が 1 つある（**意図的**）
///
/// **import モジュール本体**（`exec_module`）は解決も注釈も供給しない。
/// `Resolution::Unresolved` のまま名前引き・注釈なしで特化しないだけで、
/// **どちらも安全側へ倒れる**（注釈は最適化ヒントであって意味論の根拠ではない・#15e）。
/// グローバル集合だけは `toplevel_declared_globals(body)` をその場で作って渡している。
pub(crate) fn resolve_and_annotate(
    stmts: &mut [Stmt],
) -> (
    Vec<crate::type_check::StaticTypeError>,
    Vec<crate::type_check::StaticTypeWarning>,
    crate::type_check::AstAnnotations,
) {
    // ⚠⚠ **順序はここが唯一の定義**。型検査を先に走らせる（`run_program` の順序に揃えた）。
    // 現時点では型検査が `Resolution` を読まないのでどちらでも同じ結果になるが、
    // **読むようになった瞬間に入口ごとの挙動差になる**ので 1 箇所に固定する。
    #[cfg(feature = "prof")]
    let _p_tc = crate::prof::Timer::new(crate::prof::Phase::TypeCheck);
    let (errors, warnings, annotations) = crate::type_check::TypeChecker::check_program(stmts);
    #[cfg(feature = "prof")]
    drop(_p_tc);

    #[cfg(feature = "prof")]
    let _p_res = crate::prof::Timer::new(crate::prof::Phase::Resolve);
    resolve_program(stmts);
    #[cfg(feature = "prof")]
    drop(_p_res);

    (errors, warnings, annotations)
}

/// メインプログラムのトップレベル文列を走査し、解決可能な関数本体を書き換える。
pub(crate) fn resolve_program(stmts: &mut [Stmt]) {
    // プログラム最上位で宣言される名前（＝グローバル）を先に集める（R2-b）。
    // 型検査が**グローバル名の再宣言を禁止**している（入れ子ブロックでも
    // `variable 'x' is already declared in an accessible scope`）ため、
    // ここに載った名前は関数内でシャドウされないと保証できる。
    let globals = collect_program_globals(stmts);

    // ── 最上位文列そのものを解決する（#21-b）──
    // 最上位のローカルは存在せず、宣言はそのままグローバル（`scopes[0]`）になるので、
    // 付けられるのは `Resolution::Global` だけ。それでも意味がある — 実測で
    // **`Global(SlotCache)` 読みは `Local(slot)` 読みと同速**であり（#21-a）、
    // 最上位の書き込みは `Stmt::Assign` の `SlotCache`（R2）で既に索引化済みなので、
    // 残っていた差は「読みが名前引きであること」だけだった。
    resolve_toplevel(stmts, &globals);

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

/// 最上位文列の `Expr::Ident` を解決する（#21-b）。
///
/// 関数側（`resolve_function`）との決定的な違い:
/// - **base slot が無い**。最上位の宣言は `scopes[0]`（グローバル）に入るので `Local` は付かない。
/// - **直下宣言を差し引いてはいけない**。関数では「本体の宣言はグローバルを覆う」が、
///   最上位では**直下宣言こそがグローバル**。ここを取り違えると解決対象が空になる。
///
/// 覆うのは**入れ子ブロック（if/for/while/block/match/try）の束縛と for ターゲット**だけ。
/// これらは push されたスコープに入るので `scopes[0]` には居ない
/// （関数側で `collection.ar` の set を for ターゲットが覆って落ちた実例がある）。
fn resolve_toplevel(stmts: &mut [Stmt], globals: &HashSet<String>) {
    let visible = toplevel_visible_globals_with(stmts, globals);
    if visible.is_empty() {
        return;
    }
    // base は空 = `Local` は決して付かない。`rewrite_expr` の `globals` 経路だけが働く。
    let base: HashMap<String, u32> = HashMap::new();
    rewrite_stmts(stmts, &base, &visible);
}

/// 最上位で**宣言された**グローバル名の集合（シャドウ減算なし・#27-c）。
///
/// `toplevel_visible_globals_with` との違いと、**なぜ VM コンパイラはこちらでよいのか**:
/// - リゾルバは AST のノードに `Resolution::Global` を**一度だけ焼く**。そのノードは
///   `for i in ...` の本体の中でも評価されうるので、**プログラム全体のシャドウを引く**必要がある。
/// - VM コンパイラは**最上位文を 1 つずつ**コンパイルし、その文の中で束縛される名前は
///   すべて `slots` に入る（`collect_nested_decls` / `collect_expr_decls` / for ターゲット）。
///   最上位に他の囲みスコープは無いので、**`slots` に無い名前は必ず `scopes[0]` を指す**。
///   ⇒ 減算後の集合を渡すと、別の文の `for i in ...` のせいで `while i < N` の `i` まで
///   解決できなくなる（実測でこの形の bail が出ていた）。
///
/// ⚠ 使う側は**必ず `slots` を先に引くこと**。順序を逆にすると本当にシャドウしている
/// ローカルをグローバルとして読んでしまう。
pub(crate) fn toplevel_declared_globals(stmts: &[Stmt]) -> HashSet<String> {
    collect_program_globals(stmts)
}

/// 最上位の**可視**グローバル集合（宣言 − 入れ子スコープのシャドウ）。`resolve_toplevel` 専用。
/// ⚠ VM コンパイラへ渡すのは**こちらではなく** `toplevel_declared_globals`（上の doc を参照）。
fn toplevel_visible_globals_with(stmts: &[Stmt], globals: &HashSet<String>) -> HashSet<String> {
    let mut shadowing: HashSet<String> = HashSet::new();
    collect_shadowing_binders(stmts, &mut shadowing);
    globals.difference(&shadowing).cloned().collect()
}

/// **直下宣言以外**で束縛される名前を集める（#21-b）。
///
/// 「その文の並びを囲むスコープの名前を覆いうる名前」＝ for ターゲット・入れ子ブロックの
/// 宣言・`except ... as` の別名など。`collect_bound_names` をそのまま使うと**直下宣言まで
/// 拾ってしまい**、差し引いた結果が空になる（＝何も解決されない）ので、直下宣言だけを除く。
///
/// 消費者は 2 つで、**どちらも「覆われる側の集合から差し引く」**用途:
/// - 最上位のグローバル（`toplevel_visible_globals_with`）
/// - 関数の base スコープ（`resolve_function`）
fn collect_shadowing_binders(stmts: &[Stmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            // 直下宣言 = グローバルそのもの。名前は覆わないが、初期化式の中は見る
            // （ブロック式が内部で束縛を作りうる）。
            Stmt::Let(_, _, e) | Stmt::Const(_, _, e) | Stmt::Mut(_, _, e) => {
                collect_bound_in_expr(e, out);
            }
            Stmt::Static(_, e, _) => collect_bound_in_expr(e, out),
            Stmt::LetTuple { value, .. } => collect_bound_in_expr(value, out),
            // 入れ子定義の本体は**別フレーム**なので最上位のスコープを覆わない
            // （`rewrite_stmts` もこれらには踏み込まない）。
            Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. } => {}
            // それ以外（for ターゲット・入れ子ブロック・async ブロック等）は保守的に全部集める。
            other => collect_bound_names(std::slice::from_ref(other), out),
        }
    }
}

/// プログラム最上位で宣言される名前を集める（R2-b の `Resolution::Global` 判定用）。
///
/// ⚠ **最上位の並びだけを見る**（入れ子ブロックの宣言はグローバルではない）＝ 降りない walker。
/// 直接の束縛の判断は [`crate::decl_names`] に集約してある（#59）。
///
/// ⚠ `enum` / `new_type` が抜けていたため `MyEnum` のような読みが `Resolution::Global` に
/// ならず VM が bail していた（#27-c で修正）。**同じ抜けが `collect_declared_names` 側に
/// 残って #68 の実バグになった** — その再発を止めるのが #59。
fn collect_program_globals(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in stmts {
        crate::decl_names::each_declared_name(stmt, &mut |name, origin, _| {
            use crate::decl_names::DeclOrigin as D;
            // ⚠ **全バリアントを拾う**（最上位の束縛はすべてグローバル）。
            // `|_|` で済ませず match で書くのは、`DeclOrigin` が増えたとき
            // **ここでも判断を強制する**ため（#59 の仕掛け）。
            match origin {
                D::Let
                | D::Mut
                | D::Static
                | D::TupleLet
                | D::TupleMut
                | D::Fn
                | D::Gen
                | D::Class
                | D::Trait
                | D::Protocol
                | D::Enum
                | D::NewType
                | D::Import
                | D::FromImport => {
                    out.insert(name.to_string());
                }
            }
        });
    }
    out
}

/// 関数本体の**どこかで束縛される名前**をすべて集める（R2-b の安全条件）。
///
/// `base`（引数＋本体直下の宣言）だけでは足りない。**for ループのターゲットは
/// `base` に入らないのにグローバルをシャドウできる**（型検査は `let`/`mut` の再宣言は
/// 禁じるが、for ターゲットは許す）。実際 `collection.ar` の
/// `let x = {1,2,3}`（最上位・set）を `fn sum_ints` の `for x in items:` が覆っており、
/// これを見落として `Resolution::Global` にしたところ set を読んで落ちた。
///
/// そこで入れ子ブロック・ブロック式の中まで含め、束縛の可能性がある名前を保守的に集め、
/// **1 つでも該当したらその名前は `Resolution::Global` にしない**。
fn collect_bound_names(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        // ① この 1 文が直接束縛する名前（判断は [`crate::decl_names`] に集約・#59）。
        crate::decl_names::each_declared_name(stmt, &mut |name, origin, _| {
            use crate::decl_names::DeclOrigin as D;
            match origin {
                D::Let
                | D::Mut
                | D::Static
                | D::TupleLet
                | D::TupleMut
                | D::Fn
                | D::Gen
                | D::Class
                | D::Trait
                | D::Protocol => {
                    out.insert(name.to_string());
                }
                // ⚠⚠ **#59 時点で拾っていない既知の穴**（挙動を保存するため変えていない）。
                // 拾わないと、入れ子ブロックで `enum E` を宣言した関数の中で
                // 外側の同名グローバルが `Resolution::Global` のまま残る。
                // **実害が出ていないのは VM コンパイラが注釈より先に `slots` を引くから**
                // （#59 で実測確認: `if` の中の `enum` が外側の同名 enum を読むかを検査 → 読まない）。
                // ⇒ 直すと `LoadGlobal` が動いてバイトコードが変わるので**独立タスク**にすること。
                D::Enum | D::NewType => {}
                // 関数本体の `import` は `collect_base_decls` が `false` を返して
                // 関数ごと解決を諦めるので、ここへ来ても意味がない。
                D::Import | D::FromImport => {}
            }
        });

        // ② どこへ降りるか＋入れ子スコープの束縛（**この walker 固有**・保守的に広く取る・#84）。
        // ⚠ **`_ => {}` を書かない** — `StmtPart` に種類が増えるとここが止まる。
        crate::stmt_walk::each_subpart(stmt, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                // ⚠ #84 で `AttrAssign` / `AttrCompoundAssign` / `Raise` / `enum` の初期化式などへも
                // 降りるようになった（それ以前は `_ => {}` に落ちて**見ていなかった**）。
                // **束縛は取りこぼすと危険側に倒れる**ので、網が広がるのは正しい方向。
                P::Expr(e) => collect_bound_in_expr(e, out),
                P::Control(b) => collect_bound_names(b, out),
                // `for` ターゲット・`except ... as` の別名は入れ子スコープの束縛。
                P::ForTarget(t) => {
                    out.insert(t.to_string());
                }
                P::ExceptAlias(a) => {
                    out.insert(a.to_string());
                }
                // 入れ子定義の本体。⚠ 名前そのものは①が入れる。ここは**降りる**ためだけ。
                P::FnBody { body, .. } | P::GenBody(body) | P::TypeBody(body) => {
                    collect_bound_names(body, out);
                }
                // ⚠ `protocol` の本体は**シグネチャ宣言だけ**なので降りない（従来どおり）。
                P::ProtocolBody(_) => {}
                // 別モジュールの本体。この関数の解決とは無関係。
                P::ModuleBody(_) => {}
                // `mng <- async->T:` の本体は束縛を作りうる（#27-c）。
                // ⚠ `target` は**束縛ではない**ので①も②も入れない。
                P::AsyncBody(b) => collect_bound_names(b, out),
                // ⚠ パターンへは降りない（従来どおり）。
                P::MatchPattern(_) => {}
                // ⚠ 既存の名前への代入は**束縛ではない**。`mng <- async` の `mng` を
                // 束縛扱いするとシャドウ候補に化けて VM が bail する（#27-c の実バグ）。
                P::TargetName(_) => {}
            }
        });
    }
}

/// 式の内部（ブロック式・for 式など）に現れる束縛も集める。
fn collect_bound_in_expr(expr: &Expr, out: &mut HashSet<String>) {
    // 部分式の構造は 1 箇所（#81）。⚠ **`_ => {}` を書かない**。
    //
    // ⚠ #81 で `Slice` と `TemplateInstantiate` へも降りるようになった（それ以前は
    // `_ => {}` に落ちて**見ていなかった**）。束縛は取りこぼすと危険側に倒れるので、
    // 網が広がるのは正しい方向。挙動が動かないことは compare_bytecode で確認した。
    crate::expr_walk::each_subpart(expr, &mut |part| {
        use crate::expr_walk::SubPart as P;
        match part {
            P::Plain(x) | P::Control(x) => collect_bound_in_expr(x, out),
            P::Body(b) => collect_bound_names(b, out),
            // `for` ターゲットは入れ子スコープの束縛（この walker は拾う）。
            P::ForTarget(t) => {
                out.insert(t.to_string());
            }
            // ⚠ パターンは**束縛を作らない**（`case` は値比較・#81 以前も見ていない）。
            P::MatchPattern(_) => {}
        }
    });
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

    // ⚠ **入れ子スコープで覆われうる base 名は解決しない**（#27）。
    //
    // `for i in ...` は base に載らないが、ツリーウォークは**内側スコープに**ループ変数を
    // 宣言する。base に `mut i` がある関数でその読みを `Resolution::Local` に書き換えると、
    // `eval_local_ref` が `scopes[frame_floor]`（＝base スコープ）を直接引くため、
    // **ループ本体の `i` が外側の値を読んでしまう**。
    // 実測: `for i in range(4): s += i` が毎回 100 を読み、Python 実装の 6 に対し
    // Rust が 400 を返していた（最上位では正しく 6 で、関数内だけずれていた）。
    //
    // 判定は最上位のグローバル側と**同じ関数**を使う（同じ判断をする 2 実装を作らない）。
    // slot 番号は `order` の並びで決まるので、**採番後に**除外する（番号はずらさない）。
    let mut shadowing: HashSet<String> = HashSet::new();
    collect_shadowing_binders(body, &mut shadowing);

    let base: HashMap<String, u32> = order
        .into_iter()
        .enumerate()
        .filter(|(_, n)| !shadowing.contains(n))
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
// Rewrite: Ident の res → Resolution::Local（base 名のみ）。入れ子定義には踏み込まない。
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
        // 解決結果は `res` フィールドへ書く（変種の差し替えではない）。
        // `name` / `node_id` はそのまま残るので、書き換えで注釈やフォールバックを失わない。
        Expr::Ident { name, res, .. } => {
            if let Some(&slot) = base.get(name) {
                *res = Resolution::Local(slot);
            } else if globals.contains(name) {
                // base（引数＋本体直下宣言）に無く、最上位で宣言された名前 → グローバル確定。
                // 型検査がグローバル名の再宣言を禁じているのでシャドウの心配はない。
                *res = Resolution::Global(crate::ast::SlotCache::default());
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
        // ImaginaryLit）は書き換え不要。
        _ => {}
    }
}
