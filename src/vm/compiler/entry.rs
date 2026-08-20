// vm/compiler/entry.rs — **公開入口**（6 つ）とその `_inner`。
//
// `compile_fn`（関数本体）／`compile_toplevel_stmt`（最上位の 1 文・#10-b）／
// `compile_module_stmt`（import モジュール本体・#42）／`compile_definition_expr`（定義文脈の式・#41）／
// `compile_async_body`（async 本体・#9）／`compile_debug`（デバッガ REPL・V-E）。
//
// ⚠ **新しい入口を足すときは `CompileMode` にバリアントを足す**（mod.rs）。
// ここで bool を並べない。


use std::collections::{HashMap, HashSet};

use crate::ast::{
    Expr, Param, Stmt,
};

use crate::vm::chunk::Chunk;
use crate::vm::op::Op;
use super::*;


/// 関数本体を Chunk へコンパイルする。非対応構文があれば `None`。
///
/// - `params`: 仮引数（可変長は `local::args` として末尾 slot に採番）。
/// - `body`: 解決済み関数本体（リゾルバが `res` を付与済み。**入れ子 `fn` の本体は未解決**）。
/// - `captures`: **不変キャプチャの名前**（#27-d）。クロージャ本体をコンパイルするときだけ非空。
///   末尾に slot を採番し、呼び出し側が `chunk.captured_slots` を見て値を書き込む。
pub fn compile_fn(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
    mut_captures: &[String],
) -> Option<Chunk> {
    // 実行時間分布の計測（`--features prof`）。既定ビルドでは消える。
    #[cfg(feature = "prof")]
    let _ct = crate::prof::CompileTimer::new();
    // 診断フック（#10）: 失敗したのに bail 地点が 1 件も記録されなかったら「未帰属」として計上する。
    // 個々の `?` を全て計装する代わりに、取りこぼしを総量で可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_fn_inner(params, body, annotations, captures, mut_captures);
    }
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_fn_inner(params, body, annotations, captures, mut_captures);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        bail("unattributed", None);
    }
    out
}

/// モジュール最上位の**単一文**を Chunk へコンパイルする（#10-b）。
///
/// `compile_fn` との違いは 2 点だけ:
/// - **パラメータも base ローカルも無い**。slot は文の内側（ループ本体・for ターゲット等）の
///   宣言にだけ割り当てる（`collect_nested_decls`）。最上位の宣言は slot ではなくグローバル。
/// - **書き込み先にグローバルを許す**（`toplevel_globals`）。読み取りは従来どおり AST の
///   `Resolution::Global`（#21-b）に従うので、ここで渡すのは書き込み判定のためだけ。
///
/// 対象は**定義文以外のすべて**（#10-b/#10-c/#10-c2）。
///
/// 許可リストではなく**定義文の除外リスト**にしてある。新しい文種別が増えたとき、
/// 許可リストだと黙って取りこぼす（#10-c2 の着手時、式文 909 件が
/// 「まだ試行すらしていない」状態で残っていた）。`compile_stmt` が対応していない文は
/// そこで bail するので、除外リストは「試すだけ無駄と分かっているもの」だけでよい。
///
/// 宣言文（`let`/`mut`/`const`）を含む理由は「宣言が多いから」ではない。
/// **初期化子がループ式**のとき（`mut xs = for i in range(N) -> list[T]: ... loop_yield ...`）
/// 本体が N 回まわるからで、実測では最上位のツリーウォークの **93% がこの形**だった。
/// 「1 回しか実行されない文はコンパイル損」と素朴に考えると取り逃す。
/// 最上位文のうち **Chunk 化を試みる対象か**（#10-c2 / #27-c）。
///
/// 定義文（＝インタプリタ状態への登録）は #10-d の担当で、試しても必ず bail する。
/// ⚠ **「対象外」と「コンパイル失敗」は別物**として扱うこと。呼び出し側がこれを見ずに
/// `compile_toplevel_stmt` の `None` を一律「失敗」と数えると、定義文が失敗に化けて
/// 数字が読めなくなる（実際 `toplevel_FAILED` 511 のうち **336 件が定義文**だった）。
pub fn is_toplevel_compile_target(stmt: &Stmt) -> bool {
    !matches!(
        stmt,
        Stmt::FnDef { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. }
            | Stmt::NewTypeDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::FromImport { .. }
            | Stmt::Field { .. }
    )
}

pub fn compile_toplevel_stmt(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
) -> Option<Chunk> {
    compile_toplevel_or_module(stmt, annotations, toplevel_globals, CompileMode::Toplevel)
}

/// **import モジュール本体の 1 文**を Chunk へコンパイルする（#42）。
///
/// `compile_toplevel_stmt` との違いは `CompileMode::Module` だけ。モジュール本体は
/// `exec_module` が `push_scope` してから回すので、名前は `scopes[0]` ではなく
/// **push 済みスコープ**に入る。⇒ 代入を `StoreName`（`assign_var` = チェーン探索）にする。
/// 宣言（`DeclareGlobal` → `declare_var` → `scopes.last_mut()`）と
/// 読み（`LoadName`）はもともとチェーンを見るので変更不要。
pub fn compile_module_stmt(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    module_globals: &HashSet<String>,
) -> Option<Chunk> {
    compile_toplevel_or_module(stmt, annotations, module_globals, CompileMode::Module)
}

fn compile_toplevel_or_module(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
    mode: CompileMode,
) -> Option<Chunk> {
    debug_assert!(matches!(mode, CompileMode::Toplevel | CompileMode::Module));
    if !is_toplevel_compile_target(stmt) {
        return None;
    }
    // 実行時間分布の計測（`--features prof`）。既定ビルドでは消える。
    #[cfg(feature = "prof")]
    let _ct = crate::prof::CompileTimer::new();
    // 診断フック（#10）: `compile_fn` と同じく取りこぼしを「未帰属」として可視化する。
    if !crate::interpreter::tw_stats::enabled() {
        return compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals, mode);
    }
    // bail の分類先を「最上位」へ切り替える（#27。関数側と残タスクが別物なので混ぜない）。
    let _g = crate::interpreter::tw_stats::ToplevelCompileGuard::new(stmt);
    let before = crate::interpreter::tw_stats::bail_count();
    let out = compile_toplevel_stmt_inner(stmt, annotations, toplevel_globals, mode);
    if out.is_none() && crate::interpreter::tw_stats::bail_count() == before {
        // 未帰属でも**どの文種別か**は分かる。これが無いと 46 件の出所を探せない（#27-c）。
        bail("unattributed", Some(stmt));
    }
    out
}

/// **定義文脈の式**を Chunk へコンパイルする（#41）。
///
/// クラスのフィールド既定値・`enum` の値・デコレータ式は**定義文の一部**なので
/// `compile_toplevel_stmt` の対象外（定義文は除外リストに入っている）。それでも中身は
/// 任意の式で、`block:` / `if` / `for` 式を書ける。ここを VM に載せないと
/// **ツリーウォークの制御フローが生き続ける**（#33 が削除できない理由だった）。
///
/// `compile_toplevel_stmt` との違い:
/// - **文ではなく式**をコンパイルし、値を `Return` で返す。
/// - **自由な識別子は `LoadName`**（`name_lookup`）。定義文の実行位置は最上位とは限らず
///   （import モジュール本体の中など）、`scopes[0]` 限定の `LoadGlobal` では
///   ツリーウォークの `eval()` と答えが変わる。名前引きなら深さを問わず一致する。
/// - **書き込み先は slot だけ**（`store_target` が `name_lookup` で bail する）。
///   式の中で宣言したローカルは `collect_expr_decls` が slot を振るので影響しない。
pub fn compile_definition_expr(
    expr: &Expr,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
) -> Option<Chunk> {
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    collect_expr_decls(expr, &mut slots, &mut slot_mut, &mut slot_type, &mut n)?;

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        slots,
        slot_mut,
        slot_type,
        // Chunk の先頭がそのまま式なので、深さ 0 を最初の式へ伝える（#34）。
        pending: Some(0),
        named_locals: n,
        n_locals: n as usize,
        ..Compiler::base(CompileMode::DefinitionExpr, annotations)
    };

    c.compile_expr(expr)?;
    c.emit(Op::Return);
    Some(c.finish(ChunkMeta { local_names, ..ChunkMeta::default() }))
}

fn compile_toplevel_stmt_inner(
    stmt: &Stmt,
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    toplevel_globals: &HashSet<String>,
    mode: CompileMode,
) -> Option<Chunk> {
    let body = std::slice::from_ref(stmt);
    // 関数側と同じ扱い（#27）。最上位でも 1 文の中で `for` 変数が宣言を覆いうる。
    let shadowed_for_targets = for_target_shadows(&[], body);

    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    // ⚠ 宣言文は **`collect_nested_decls` に渡してはいけない**（#10-c）。
    // あれは「その文が宣言する名前」に slot を割り当てるが、最上位の宣言はグローバルであって
    // slot ではない。slot を振ると `store_target` が slot 側を優先してしまい、
    // `DeclareGlobal` が出ずに値がフレームへ消える。初期化子の**内側**の宣言だけを採番する。
    let collected = match stmt {
        Stmt::Let(_, _, e) | Stmt::Mut(_, _, e) | Stmt::Const(_, _, e) => {
            collect_expr_decls(e, &mut slots, &mut slot_mut, &mut slot_type, &mut n)
        }
        // `let a, b = t` も宣言文（#27-c）。ターゲットは最上位ではグローバルなので
        // slot を振らない — 振ると `Op::LetTuple` が slot 束縛側を選び、値がフレームへ消える
        // （`collection.ar` の `tx` が実例）。
        Stmt::LetTuple { value, .. } => {
            collect_expr_decls(value, &mut slots, &mut slot_mut, &mut slot_type, &mut n)
        }
        _ => collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n),
    };
    if collected.is_none() {
        bail("nested-decls", None);
        return None;
    }

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        slots,
        slot_mut,
        slot_type,
        named_locals: n,
        n_locals: n as usize,
        toplevel_globals: toplevel_globals.clone(),
        shadowed_for_targets,
        // 最上位に `static` は無い（あれば定義文として #10-d の担当）。
        ..Compiler::base(mode, annotations)
    };

    c.compile_stmt(stmt)?;
    // 最上位文は値を返さない（`Return` は型検査が最上位で禁じる）。
    c.emit(Op::ReturnNil);
    let chunk = c.finish(ChunkMeta { local_names, ..ChunkMeta::default() });

    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", crate::vm::disasm::disassemble(&chunk, "<toplevel>"));
    }

    Some(chunk)
}

fn compile_fn_inner(
    params: &[Param],
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
    mut_captures: &[String],
) -> Option<Chunk> {
    // 外側変数をシャドウする `for` ループ変数は、本体のコンパイル中だけ専用 slot へ差し替える（#27）。
    let shadowed_for_targets = for_target_shadows(params, body);
    // base slot をリゾルバと同順で採番する: パラメータ → トップレベル let/mut/const。
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut self_slot: Option<u16> = None;
    let mut n: u16 = 0;
    for (i, p) in params.iter().enumerate() {
        // 可変長パラメータ（#27）。`bind_args` は**受け取った値を `local::args` という名前で
        // 末尾に 1 つ**束縛する（引数が無ければ `Value::None`）ので、slot も同じ名前・同じ位置に
        // 採番すれば一般バインド経路（`bindings[i]` → slot i）とそのまま一致する。
        // ⚠ **並びが `bind_args` と一致することが健全性そのもの**。パーサは可変長を末尾に
        // しか許さないが、崩れたときに黙って値がずれないよう明示的に確かめる。
        if p.variadic {
            if i + 1 != params.len() {
                bail("variadic-param-not-last", None);
                return None;
            }
            slots.insert("local::args".to_string(), n);
            slot_mut.push(p.mutable);
            // 型注釈は**要素の型**（`let ...: int`）であって束縛される list の型ではない。
            slot_type.push(None);
            n = n.checked_add(1)?;
            continue;
        }
        if p.name == "self" {
            self_slot = Some(n);
        }
        slots.insert(p.name.clone(), n);
        slot_mut.push(p.mutable);
        slot_type.push(p.type_ann.clone());
        n = n.checked_add(1)?;
    }
    // パラメータ数（`self` 含む）。デバッガが「まだ宣言されていないローカル」を隠すのに使う（#1）。
    let n_params = n as usize;
    // `static mut` の名前 → 宣言位置（#27-d）。slot ではなく `static_cells` を指す名前の集合。
    let mut statics: HashMap<String, crate::token::Span> = HashMap::new();
    // トップレベル宣言を事前採番。LetTuple/Static/入れ子定義など slot をずらす形は非対応。
    for stmt in body {
        match stmt {
            Stmt::Let(name, ty, _) | Stmt::Const(name, ty, _)
                if name != "_" && !slots.contains_key(name) =>
            {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            Stmt::Mut(name, ty, _) if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(true);
                slot_type.push(ty.clone());
                n = n.checked_add(1)?;
            }
            // `_` 名・既出名の宣言は base slot を増やさない（no-op）。
            Stmt::Let(..) | Stmt::Const(..) | Stmt::Mut(..) => {}
            // 入れ子 `fn` の名前も base slot を占める（リゾルバの `collect_base_decls` と同順・#27）。
            // ⚠ **採番だけここで行い、載せられるかの判定は `compile_stmt` で行う**
            //    （自由変数の判定に `slots` の完成が要るため）。載せられなければそこで bail する。
            Stmt::FnDef { name, .. } if name != "_" && !slots.contains_key(name) => {
                slots.insert(name.clone(), n);
                slot_mut.push(false);
                slot_type.push(None);
                n = n.checked_add(1)?;
            }
            Stmt::FnDef { .. } => {}
            // `static mut x = e`（#27-d）。記憶域はフレームではなく `Interpreter::static_cells`
            // （宣言位置がキーの共有セル）で、呼び出しをまたいで生き残る。名前 → 宣言位置を
            // 控えておき、本体の読み書きを `LoadStatic`/`StoreStatic` に落とす。
            //
            // ⚠⚠ **slot は「使わないが必ず 1 つ消費する」**。リゾルバの `collect_base_decls` は
            // `Stmt::Static` にも `push_base` するので、ここで飛ばすと**以降の base slot が
            // 全部 1 つずれ**、`Resolution::Local(k)` が別の変数（または範囲外）を指す。
            // 実際に `LoadLocal` の添字 out-of-bounds で落ちた。**採番はリゾルバと同順・同数**が契約。
            Stmt::Static(name, _, span) => {
                statics.insert(name.clone(), span.clone());
                if name != "_" {
                    slot_mut.push(true);
                    slot_type.push(None);
                    n = n.checked_add(1)?; // 穴を空けるだけ（`slots` には入れない）
                }
            }
            // slot を採番する可能性のある未対応の宣言的文があれば、番号ずれを避けて丸ごと諦める。
            Stmt::LetTuple { .. }
            | Stmt::GenDef { .. }
            | Stmt::ClassDef { .. }
            | Stmt::TraitDef { .. }
            | Stmt::ProtocolDef { .. }
            | Stmt::NewTypeDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::FromImport { .. } => {
                bail("decl-prepass", Some(stmt));
                return None;
            }
            _ => {}
        }
    }
    // ネストしたブロック（if/while/match のボディ）内の Let/Const/Mut にも
    // フレーム内固定 slot を割り当てる（R0-B: 関数内の全ローカルが平坦 slot）。
    // トップレベル decl は上で採番済みなのでスキップされる。順序は問わない
    // （compile は slots 引きで参照する）。シャドウイング禁止＝同名は非同時生存なので
    // slot 再利用は健全。リゾルバは nested 名を解決しない（Ident のまま）ので衝突しない。
    if collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n).is_none() {
        bail("nested-decls", None);
        return None;
    }

    // クロージャの不変キャプチャに slot を採番する（#27-d）。
    //
    // **必ず最後**に採番する。`Resolution::Local` が付いた本体（＝リゾルバが解決した
    // トップレベル関数）は「パラメータ → 本体直下の宣言」の並びを前提に番号が焼かれているので、
    // 途中に差し込むとずれる。末尾に足すぶんには既存の番号を動かさない。
    // （実際にはクロージャ本体はリゾルバが降りないので全て `Unresolved` だが、
    //   この不変条件を崩さないでおく方が安全。）
    //
    // ⚠ **既に slot がある名前とぶつかったら諦める**。`capture_env` は
    // 「パラメータでも本体の宣言でもない自由変数」だけを捕まえるので本来ぶつからないが、
    // ぶつかるということは `collect_declared_names`（キャプチャ側）と `slots`（コンパイラ側）の
    // 木の歩き方がずれているということ。**黙って上書きすると閉包変数が消える**ので
    // 計測できる形で落とす。
    let mut captured_slots: Vec<(String, u16)> = Vec::with_capacity(captures.len());
    // 採番順を安定させる（`captured_env` は HashMap なので反復順が実行ごとに変わる）。
    let mut capture_names: Vec<&String> = captures.iter().collect();
    capture_names.sort();
    for name in capture_names {
        if slots.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        slots.insert(name.clone(), n);
        slot_mut.push(false); // 不変キャプチャ（`CapturedVar::Immutable`）だけを載せる
        slot_type.push(None);
        captured_slots.push((name.clone(), n));
        n = n.checked_add(1)?;
    }

    // **可変キャプチャ**にセル index を採番する（#27-d 段階 2b）。
    //
    // slot ではなくセルなのは、ツリーウォークが `CapturedVar::Mutable(cell)` として
    // **外側と同じ `Rc<RefCell<Value>>` を共有**するから。slot（`Value` 直値）へ値を
    // コピーすると、クロージャ内の書き込みが外側へ返らない。
    // 実行時は `build_cells` が `captured_env` のセルを**そのまま**この index へ入れる。
    let mut cells: HashMap<String, u16> = HashMap::new();
    let mut captured_cells: Vec<(String, u16)> = Vec::with_capacity(mut_captures.len());
    let mut mut_capture_names: Vec<&String> = mut_captures.iter().collect();
    mut_capture_names.sort(); // 採番順を安定させる（HashMap 由来）
    for name in mut_capture_names {
        // 不変キャプチャと同じ理由で、slot と衝突したら諦める。
        if slots.contains_key(name) || cells.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        let idx = u16::try_from(cells.len()).ok()?;
        cells.insert(name.clone(), idx);
        captured_cells.push((name.clone(), idx));
    }

    // **入れ子 `fn` に可変キャプチャされる自分のローカル**もセルへ移す（#27-d 段階 2b）。
    //
    // ⚠ **slot は解放しない**（穴のまま残す）。`Resolution::Local(k)` はリゾルバの採番で
    // 焼かれているので、詰め直すと別の変数を指す（`static` で実際に踏んだ）。
    // 読み書きは `cells` を先に引くことで slot 側へ行かないようにする。
    let mut cell_by_slot: HashMap<u16, u16> = HashMap::new();
    let mut free_in_nested: Vec<String> = nested_fn_free_names(body).into_iter().collect();
    free_in_nested.sort(); // 採番順を安定させる
    for name in free_in_nested {
        let Some(&slot) = slots.get(&name) else { continue };
        if !slot_mut.get(slot as usize).copied().unwrap_or(false) {
            continue; // 不変ローカルのキャプチャは値の複製で足りる（従来どおり slot）
        }
        let idx = u16::try_from(cells.len()).ok()?;
        slots.remove(&name); // slot 番号は残したまま名前だけ外す
        cells.insert(name.clone(), idx);
        cell_by_slot.insert(slot, idx);
    }
    let n_cells = cells.len();

    // V-E: slot → 変数名 のデバッグ名テーブル（named slot のみ。temp は無名）。
    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }

    let mut c = Compiler {
        slots,
        slot_mut,
        slot_type,
        self_slot,
        named_locals: n,
        n_locals: n as usize,
        shadowed_for_targets,
        statics,
        cells,
        cell_by_slot,
        n_cells,
        ..Compiler::base(CompileMode::Function, annotations)
    };

    for stmt in body {
        c.compile_stmt(stmt)?;
    }
    // 本体末尾までフォールオフしたら None を返す。
    c.emit(Op::ReturnNil);


    let chunk = c.finish(ChunkMeta {
        local_names,
        n_params,
        captured_slots,
        captured_cells,
    });

    // 開発用フック: `AR_VM_DUMP=1` で生成バイトコードを標準エラーへ逆アセンブルする。
    // どの式に型特化 op が乗ったかを目視で確認するために使う（disasm.rs の唯一の呼び元）。
    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", crate::vm::disasm::disassemble(&chunk, "<fn>"));
    }

    Some(chunk)
}

/// `mng <- async->T: body` の**本体**を Chunk へコンパイルする（#32）。
///
/// 本体は「値を返すブロック式」（`block_return v` が結果）なので、`compile_fn` ではなく
/// `compile_block_expr` の上に `Return` を載せた形にする。
///
/// `captures` は submit 時に deep-clone された環境の変数名（D5 share-nothing）。
/// #27-d 段階 1 と同じく**末尾に slot を採番**し、実行側が `chunk.captured_slots` を見て
/// 値を書き込む（名前で引くので `env` の並びに依存しない）。
///
/// ⚠ **worker スレッドは型注釈を持てない**（`AstAnnotations` は `Rc` ベースで `Send` でない）。
/// 呼び出し側は空の注釈を渡すので**型特化 op は乗らない**が、
/// 注釈は最適化ヒントであって意味論の根拠ではない（#15e の原則）ので結果は変わらない。
///
/// ⚠ **worker のスコープは `[globals, env]` の 2 段**。ここで slot に載らない自由名は
/// `LoadName`（`get_val`）に落ちる必要があるので、`toplevel_globals` に env の名前を入れて
/// 最上位モード扱いにする（`frame_floor` は 0 のままで、覗かれて困る呼び出し元も居ない）。
pub fn compile_async_body(
    body: &[Stmt],
    annotations: std::rc::Rc<crate::type_check::AstAnnotations>,
    captures: &[String],
) -> Option<Chunk> {
    let mut slots: HashMap<String, u16> = HashMap::new();
    let mut slot_mut: Vec<bool> = Vec::new();
    let mut slot_type: Vec<Option<String>> = Vec::new();
    let mut n: u16 = 0;
    // 本体の宣言（入れ子ブロックを含む）を先に採番する。
    if collect_nested_decls(body, &mut slots, &mut slot_mut, &mut slot_type, &mut n).is_none() {
        bail("nested-decls", None);
        return None;
    }
    // 捕捉環境は末尾（#27-d 段階 1 と同じ理由・同じ表）。
    let mut captured_slots: Vec<(String, u16)> = Vec::with_capacity(captures.len());
    let mut capture_names: Vec<&String> = captures.iter().collect();
    capture_names.sort();
    for name in capture_names {
        // ⚠ **本体が同名を宣言していたら諦める**（#27-d 段階 1 と同じ判断）。
        // ツリーウォークでは捕捉値が push されたスコープに居るので**宣言より前は捕捉値が見える**。
        // slot を宣言側に取られると、その読みが未初期化 slot になって黙って値が変わる。
        if slots.contains_key(name) {
            bail("capture-slot-conflict", None);
            return None;
        }
        slots.insert(name.clone(), n);
        slot_mut.push(false);
        slot_type.push(None);
        captured_slots.push((name.clone(), n));
        n = n.checked_add(1)?;
    }

    let mut local_names = vec![String::new(); n as usize];
    for (name, &slot) in &slots {
        if let Some(entry) = local_names.get_mut(slot as usize) {
            *entry = name.clone();
        }
    }
    let shadowed_for_targets = for_target_shadows(&[], body);

    let mut c = Compiler {
        slots,
        slot_mut,
        slot_type,
        named_locals: n,
        n_locals: n as usize,
        // ⚠ **空でないと `reads_by_name` が偽になり `LoadGlobal` へ落ちる**。名前の中身は
        // 書き込み判定にしか使わないので、捕捉名を入れておけば足りる（`CompileMode` の doc）。
        toplevel_globals: captures.iter().cloned().collect(),
        shadowed_for_targets,
        // async 本体は可変キャプチャを持たない（submit 時に deep-clone される・D5）。
        ..Compiler::base(CompileMode::AsyncBody, annotations)
    };

    // async 本体は Chunk の先頭＝オペランドスタックは空（#34）。
    // アノテーションは持たない（`mng <- async->T:` の T は代入先の型で、`block_return` の
    // 実行時検査はツリーウォークでも走らない＝`BLOCK_RETURN_EXPECTED_TYPE` は空・#35）。
    c.compile_block_expr(body, Some(0), None, true)?;
    c.emit(Op::Return);
    let chunk = c.finish(ChunkMeta {
        local_names,
        captured_slots,
        ..ChunkMeta::default()
    });
    if std::env::var("AR_VM_DUMP").is_ok_and(|v| !v.is_empty()) {
        eprintln!("{}", crate::vm::disasm::disassemble(&chunk, "<async>"));
    }
    Some(chunk)
}

/// デバッガ REPL の 1 文をデバッグモード（名前引きアクセス）でコンパイルする。
/// 対応: 式文（値を `Return`）・`let/const dbg::name = 式`（`DeclareName`）。
/// メソッド呼び出し・添字・制御フロー等は `None`（呼び出し側がツリーウォークへフォールバック）。
pub fn compile_debug(stmt: &Stmt) -> Option<Chunk> {
    let mut c = Compiler {
        // デバッグ経路は注釈を持たない（空＝型特化しない）。
        ..Compiler::base(
            CompileMode::DebugRepl,
            std::rc::Rc::new(crate::type_check::AstAnnotations::default()),
        )
    };
    match stmt {
        Stmt::Expr(e) => {
            c.compile_expr(e)?;
            c.emit(Op::Return); // 式の値を返す（呼び出し側が表示）
        }
        Stmt::Let(name, _, e) | Stmt::Const(name, _, e) if name != "_" => {
            c.compile_expr(e)?;
            let ni = c.add_name(name);
            c.emit(Op::DeclareName(ni));
            c.emit(Op::ReturnNil);
        }
        _ => return None,
    }
    // ⚠ **覗き穴最適化を掛けない**（#52 以前からの挙動。差を消すのは別タスク）。
    Some(c.into_chunk(ChunkMeta::default()))
}
