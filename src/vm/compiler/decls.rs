// vm/compiler/decls.rs — **slot 採番**と、そのための AST 走査。
//
// ⚠⚠ **採番はリゾルバと同順・同数**（`push_base` される名前は `slots` に入れなくても
// 必ず 1 slot 消費する）。ずれると `LoadLocal` が範囲外を読む。
// ⚠ **`compile_stmt` に文種別を足したら必ず `collect_nested_decls` も見る**（#27-c で 2 回踏んだ）。


use std::collections::{HashMap, HashSet};

use crate::ast::{
    Expr, Param, Stmt,
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
            // 文の直下の構造は 1 箇所（#84）。⚠ **`_ => {}` を書かない** — `StmtPart` に
            // 種類が増えるとここが止まり、「この walker ではどう扱うか」を決めさせられる。
            crate::stmt_walk::each_subpart(s, &mut |part| {
                use crate::stmt_walk::StmtPart as P;
                match part {
                    // 入れ子 `fn` 本体へは**降りない**（その中の `fn` の自由変数は
                    // `collect_referenced_names` が `Stmt::FnDef` の本体へ降りるので
                    // 下の `referenced` に含まれる）。
                    P::FnBody { params, body } => {
                        let mut own: HashSet<String> =
                            params.iter().map(|p| p.name.clone()).collect();
                        crate::interpreter::collect_declared_names(body, &mut own);
                        let mut referenced: HashSet<String> = HashSet::new();
                        crate::interpreter::collect_referenced_names(body, &mut referenced);
                        out.extend(referenced.into_iter().filter(|n| !own.contains(n)));
                    }
                    // 制御フローの中に置かれた `fn` も拾う（同じフレーム）。
                    P::Control(b) => walk(b, out),
                    // ⚠ 入れ子 `gen` は **`decl-prepass:GenDef` で必ず bail する**ので到達しない（実測）。
                    //   拾うようにするなら `gen` の VM 対応と同時にやること。
                    P::GenBody { .. } => {}
                    // 別スコープの定義集合。クロージャのキャプチャ対象ではない。
                    P::TypeBody(_) | P::ProtocolBody(_) => {}
                    // 別モジュールの本体。
                    P::ModuleBody(_) => {}
                    // async 本体は送出時にディープクローンされ別チャンクになる。
                    P::AsyncBody(_) => {}
                    // 式の中のブロック式に置かれた `fn` はここでは拾わない（従来どおり）。
                    // ⚠ 拾うとセル化の対象が増えてバイトコードが動くので別タスク。
                    P::Expr(_) | P::MatchPattern(_) => {}
                    // 束縛。自由変数とは逆向きの問い。
                    P::ForTarget(_) | P::ExceptAlias(_) => {}
                    // 既存の名前への代入。宣言でも自由変数解析の入力でもない
                    // （参照側は `collect_referenced_names` が別に拾う）。
                    P::TargetName(_) => {}
                }
            });
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
        // ① この 1 文が直接束縛する名前（判断は 1 箇所・#59）。
        crate::decl_names::each_declared_name(s, &mut |name, origin, _| {
            use crate::decl_names::DeclOrigin as D;
            match origin {
                // `for` ターゲットに覆われうるのは「値を持つ束縛」だけ。
                D::Let | D::Mut | D::Static | D::TupleLet | D::TupleMut => {
                    decl_names.insert(name.to_string());
                }
                // ⚠ **定義文（`fn`/`class`…）は意図的に入れない**（#59 の doc に明記）。
                // ここが集めるのは「`for` ターゲットとの**衝突候補**」であって
                // 宣言名の一覧ではない。
                D::Fn | D::Gen | D::Class | D::Trait | D::Protocol | D::Enum | D::NewType => {}
                // 関数本体の `import` は `compile_stmt` にアームが無く bail する。
                D::Import | D::FromImport => {}
            }
        });

        // ② どこへ降りるか＋入れ子スコープの束縛（#84）。
        // ⚠ **`_ => {}` を書かない** — `StmtPart` に種類が増えるとここが止まる。
        crate::stmt_walk::each_subpart(s, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                P::Expr(e) => scan_shadow_expr(e, for_names, decl_names),
                P::Control(b) => scan_shadow_stmts(b, for_names, decl_names),
                // `for` ターゲットは**衝突候補**として集める（この walker 固有の判断）。
                P::ForTarget(t) => {
                    for_names.insert(t.to_string());
                }
                // `except ... as e` は入れ子スコープの束縛＝覆われうる側。
                P::ExceptAlias(a) => {
                    decl_names.insert(a.to_string());
                }
                // 別フレーム／別スコープ。`for` ターゲットの覆いは跨がない。
                P::FnBody { .. }
                | P::GenBody { .. }
                | P::TypeBody(_)
                | P::ProtocolBody(_)
                | P::ModuleBody(_)
                | P::AsyncBody(_) => {}
                // ⚠ パターンへは降りない（`collect_expr_decls` と揃える・採番がずれる）。
                P::MatchPattern(_) => {}
                // 既存の名前への代入は**宣言ではない**ので衝突候補に入れない。
                P::TargetName(_) => {}
            }
        });
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

        // ② どこへ降りるか＋入れ子スコープの束縛（**この walker 固有**・#84）。
        //
        // ⚠⚠ **列挙の順序＝採番の順序**。`each_subpart` は `For` を
        // 「ターゲット → iter → 本体」、`Try` を「本体 → 別名 → handler 本体 → finally」の
        // 順に返す（#84 以前の手書きと同じ順序）。崩すと `LoadLocal` が別の変数を読む。
        //
        // ⚠ `add_decl` は slot が溢れると `None` を返して**打ち切る**。クロージャからは `?` で
        // 抜けられないので、`ok` が落ちたら以降は**何もしない**（`collect_expr_decls` と同じ）。
        let mut ok = Some(());
        crate::stmt_walk::each_subpart(stmt, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            if ok.is_none() {
                return;
            }
            ok = match part {
                // ⚠ 初期化式・条件・`raise` の例外式などの中にも**ブロック式**がありうる。
                // #84 以前は `Stmt::Static` のアームが無く、`static mut s = block ->T: …` の
                // 本体宣言が slot を取れずに `decl-no-slot` で bail していた（実バグ）。
                P::Expr(e) => collect_expr_decls(e, slots, slot_mut, slot_type, n),
                // 同じフレームの制御フロー本体。⚠ `block:` 文の中の `let` も slot を持つ（#27-c）。
                P::Control(b) => collect_nested_decls(b, slots, slot_mut, slot_type, n),
                // ループ変数は可変（tree-walk は `Var::new(item, true)`）。型注釈なし。
                P::ForTarget(t) => add_decl(t, None, true, slots, slot_mut, slot_type, n),
                // `except E as e:` の別名は不変束縛（tree-walk は `Var::new(exc, false)`）。
                P::ExceptAlias(a) => add_decl(a, None, false, slots, slot_mut, slot_type, n),
                // 別フレーム。ローカルも slot も独立なので採番しない
                // （`compile_stmt` が本体を別 Chunk としてコンパイルする）。
                P::FnBody { .. }
                | P::GenBody { .. }
                | P::TypeBody(_)
                | P::ProtocolBody(_)
                | P::ModuleBody(_)
                | P::AsyncBody(_) => Some(()),
                // ⚠⚠ パターンへは**降りない**。降りるとパターン内のブロック式が slot を取り、
                // **採番がずれる**（#81 と同じ判断）。
                P::MatchPattern(_) => Some(()),
                // 既存の名前への代入。slot は宣言側（①）が既に振っている。
                P::TargetName(_) => Some(()),
            };
        });
        ok?;
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
    stmts.iter().any(|s| {
        if matches!(s, Stmt::Return(_)) {
            return true;
        }
        // 文の直下の構造は 1 箇所（#84）。⚠ **`_ => {}` を書かない**。
        let mut bails = false;
        crate::stmt_walk::each_subpart(s, &mut |part| {
            use crate::stmt_walk::StmtPart as P;
            match part {
                // 同じフレームなので、中の `return` もこのブロック式を跨いで抜ける。
                P::Control(b) => bails |= block_body_bails(b),
                // 別フレーム／別チャンクの本体。中の `return` は**そちらの関数**から抜ける。
                P::FnBody { .. }
                | P::GenBody { .. }
                | P::TypeBody(_)
                | P::ProtocolBody(_)
                | P::ModuleBody(_)
                | P::AsyncBody(_) => {}
                // 式の中のブロック式は**自分で** `block_return` を処理する（従来どおり）。
                P::Expr(_) | P::MatchPattern(_) => {}
                // 束縛・既存名の指定。どちらも脱出とは無関係。
                P::ForTarget(_) | P::ExceptAlias(_) | P::TargetName(_) => {}
            }
        });
        bails
    })
}
